//! A2A connection layer — serves the bus over HTTP so any agent (or human
//! with curl) can pass potatoes.
//!
//! V1 scope (Sho's call: "A2A protocol is the standard, V1 is enough"):
//! - Agent Card discovery at `/.well-known/agent-card.json` (A2A v1.0 path)
//! - JSON-RPC 2.0 endpoint at `/` with bus methods
//! - Optional bearer-token auth (HOT_POTATO_TOKEN env)
//!
//! The bus core stays transport-free: this module is a shell around it.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::bus::{EventBus, Role};
use crate::message::MsgType;
use crate::store::memory::InMemoryStore;
use crate::ws::EventHub;

/// A2A Agent Card (v1.0 shape, trimmed to fields that matter for a bus).
#[derive(Debug, Clone, Serialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub protocol_version: String,
    pub capabilities: Value,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<Value>,
}

/// Run configuration, assembled from env + explicit registration.
pub struct ServerConfig {
    pub name: String,
    pub description: String,
    pub public_url: String,
    pub version: String,
    pub bearer_token: Option<String>,
    /// RFC-004: federation peers (remote pools + the agents they are home to).
    pub peers: crate::federation::Peers,
    /// RFC-004: this pool's own name (goes into `forwarded_from`).
    pub pool_name: String,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            name: std::env::var("HOT_POTATO_NAME").unwrap_or_else(|_| "hot-potato".into()),
            description: std::env::var("HOT_POTATO_DESCRIPTION").unwrap_or_else(|_| {
                "EventMessageBus for AI agents - a mailbox with a state machine".into()
            }),
            public_url: std::env::var("HOT_POTATO_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            bearer_token: std::env::var("HOT_POTATO_TOKEN").ok(),
            peers: crate::federation::Peers::from_env(),
            pool_name: std::env::var("HOT_POTATO_POOL").unwrap_or_else(|_| "local".into()),
        }
    }

    fn agent_card(&self) -> AgentCard {
        AgentCard {
            name: self.name.clone(),
            description: self.description.clone(),
            url: self.public_url.clone(),
            version: self.version.clone(),
            protocol_version: "1.0".into(),
            capabilities: json!({
                "streaming": false,
                "pushNotifications": false,
                "stateTransitionHistory": true
            }),
            default_input_modes: vec!["application/json".into()],
            default_output_modes: vec!["application/json".into()],
            skills: vec![
                json!({
                    "id": "bus-send",
                    "name": "Send a message",
                    "description": "Pass a potato to a registered agent (task/reply/broadcast)",
                    "tags": ["bus", "send"]
                }),
                json!({
                    "id": "bus-poll",
                    "name": "Poll my mailbox",
                    "description": "Drain queued messages (marks them delivered)",
                    "tags": ["bus", "poll"]
                }),
                json!({
                    "id": "bus-ack",
                    "name": "Ack a message",
                    "description": "File the result note; closes the loop",
                    "tags": ["bus", "ack"]
                }),
                json!({
                    "id": "bus-status",
                    "name": "Check my sent messages",
                    "description": "Lifecycle status of everything I sent",
                    "tags": ["bus", "status"]
                }),
            ],
        }
    }
}

#[derive(Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Add `rpc.discover` (JSON-RPC introspection, A2A-friendly) + a bus method table.
async fn discover() -> Value {
    json!({
        "methods": [
            {"name":"agent/register","params":["agent","role (pm|worker)"],"desc":"register an agent on the bus (optional: description, deliver_via, tags[])"},
            {"name":"message/send","params":["sender","receiver","type (task|reply|broadcast|ack_only)","subject","body","ref (required for reply)"],"desc":"pass a potato"},
            {"name":"message/poll","params":["agent","limit (optional, 0=all)"],"desc":"drain my mailbox; marks returned letters delivered (poll = claim)"},
            {"name":"message/peek","params":["agent"],"desc":"look without marking (queued letters only)"},
            {"name":"message/read","params":["agent","id"],"desc":"chatlog read receipt; requires delivered state"},
            {"name":"message/ack","params":["agent","id","note (<=80 chars)"],"desc":"file the result; idempotent on already-acked; accepts delivered or read"},
            {"name":"agent/status","params":["agent"],"desc":"lifecycle of everything this agent SENT"},
            {"name":"bus/archive","params":[],"desc":"all acked letters (audit log)"},
            {"name":"message/list","params":["status (optional: queued|delivered|read|acked)","limit (optional, 0=all)"],"desc":"OBSERVER: every letter on the bus, any status, read-only (v0.2)"},
            {"name":"agent/list","params":[],"desc":"registry dump with team tags (dashboard legend) (RFC-005)"},
            {"name":"peer/invite","params":[],"desc":"generate a one-time invite code (dashboard: Invite a peer) (RFC-006)"},
            {"name":"peer/join","params":["code","url","name","agents[]"],"desc":"join a remote pool with an invite code (dashboard: Join a peer); three-way handshake completes automatically (RFC-006)"},
            {"name":"peer/list","params":[],"desc":"federation peers (env + dashboard-learned)"},
            {"name":"rpc.discover","params":[],"desc":"this document"}
        ]
    })
}

/// JSON-RPC handler — the single endpoint agents talk to.
async fn rpc(
    State((bus, hub, config, registry, invites, dynamic_peers)): State<BusState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    if let Some(expected) = &config.bearer_token {
        let ok = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == format!("Bearer {expected}"))
            .unwrap_or(false);
        if !ok {
            return (
                StatusCode::UNAUTHORIZED,
                Json(
                    json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32001,"message":"unauthorized"}}),
                ),
            );
        }
    }

    let result: Result<Value, String> = match req.method.as_str() {
        "agent/register" => {
            parse2(&req.params, ["agent", "role"], |p, agent, role| {
                let role = if role == "pm" { Role::Pm } else { Role::Worker };
                let agent = agent.to_string();
                // Delivery registration (RFC-002): optional description + deliver_via.
                let deliver_via: crate::deliver::DeliverVia = match p.get("deliver_via") {
                    Some(Value::String(s)) if s == "a2a" || s == "webhook" || s == "relay" => {
                        let url = p
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        match s.as_str() {
                            "a2a" => crate::deliver::DeliverVia::A2a {
                                url,
                                token: p.get("token").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            },
                            "webhook" => crate::deliver::DeliverVia::Webhook { url },
                            _ => crate::deliver::DeliverVia::Relay { url },
                        }
                    }
                    _ => crate::deliver::DeliverVia::Poll,
                };
                let description = p
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // RFC-005: optional team tags (["team","fleet",...]).
                let tags: Vec<String> = p
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let registry = registry.clone();
                async move {
                    let entry = registry
                        .register_tagged(&agent, &description, deliver_via, tags)
                        .await;
                    bus.register(&agent, role)
                        .await
                        .map(|_| {
                            json!({
                                "registered": agent,
                                "deliver_via": entry.deliver_via,
                                "tags": entry.tags,
                            })
                        })
                        .map_err(|e| e.to_string())
                }
            })
            .await
        }
        "agent/list" => {
            // RFC-005: registry dump with tags — the dashboard's team legend.
            let entries = registry.all().await;
            Ok(json!(entries
                .iter()
                .map(|e| json!({
                    "agent": e.agent,
                    "description": e.description,
                    "deliver_via": e.deliver_via,
                    "tags": e.tags,
                }))
                .collect::<Vec<_>>()))
        }
        "message/send" => {
            parse5(&req.params, |sender, receiver, msg_type, subject, body| {
                let mt = match msg_type {
                    "reply" => MsgType::Reply,
                    "broadcast" => MsgType::Broadcast,
                    "ack_only" => MsgType::AckOnly,
                    _ => MsgType::Task,
                };
                let r#ref = req
                    .params
                    .get("ref")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                // RFC-004 envelope: letters that crossed an inter-bus link.
                let hops = req.params.get("hops").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let forwarded_from = req
                    .params
                    .get("forwarded_from")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let (sender, receiver, subject, body) = (
                    sender.to_string(),
                    receiver.to_string(),
                    subject.to_string(),
                    body.to_string(),
                );

                // --- RFC-004: cross-pool routing -------------------------------
                // 1) Inbound federated letter: the peer's bus dialed us with the
                //    peer token + X-Potato-Pool header. Auto-register the remote
                //    sender (Worker) so the bus's normal rules accept the letter.
                // 2) Outbound: if the receiver's home pool is a configured peer,
                //    hand the letter over HTTP to that peer's bus instead of
                //    queueing locally. Local copy goes to `forwarded` state.
                let fed_headers = headers
                    .get("x-potato-pool")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let authed = config
                    .bearer_token
                    .as_ref()
                    .map(|t| {
                        headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(|v| v == format!("Bearer {t}"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                let peers = config.peers.clone();
                let pool_name = config.pool_name.clone();
                let bus3 = bus.clone();
                let registry3 = registry.clone();
                let hub3 = hub.clone();
                let dyn_peers3 = dynamic_peers.clone();
                async move {
                    // (1) inbound from a federated peer bus?
                    if let Some(from_pool) = fed_headers {
                        // Token required only when this bus has one configured.
                        // Auth-off pools (day-1 federation) accept the letter;
                        // the X-Potato-Pool header + envelope are still mandatory.
                        if config.bearer_token.is_some() && !authed {
                            return Err("federated send requires the pool bearer token".into());
                        }
                        if hops == 0 || forwarded_from.is_none() {
                            return Err(
                                "federated send must carry hops and forwarded_from".into()
                            );
                        }
                        if hops > crate::federation::MAX_HOPS {
                            return Err(format!(
                                "hop limit {} exceeded (from pool `{from_pool}`)",
                                crate::federation::MAX_HOPS
                            ));
                        }
                        // Federated senders are auto-registered: the peer bus
                        // authenticated with the shared token, that's the trust.
                        if !bus3.is_registered(&sender).await {
                            registry3
                                .register_tagged(
                                    &sender,
                                    &format!("federated from {from_pool}"),
                                    crate::deliver::DeliverVia::Poll,
                                    vec![from_pool.clone()],
                                )
                                .await;
                            bus3.register(&sender, crate::bus::Role::Worker)
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                        let id = bus3
                            .send_envelope(
                                &sender, &receiver, mt, &subject, &body, r#ref,
                                hops, forwarded_from,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        let stored = bus3.peek(&receiver).await;
                        if let Ok(letters) = stored {
                            if let Some(letter) = letters.into_iter().find(|m| m.id == id) {
                                hub3.emit("queued", &letter);
                            }
                        }
                        // RFC-007: federated inbound letters get the same
                        // push-on-arrival as local ones — otherwise cross-pool
                        // letters wait for the receiver's poll cycle.
                        let registry4 = registry3.clone();
                        let bus4 = bus3.clone();
                        let transport: Arc<dyn crate::deliver::PushTransport> =
                            Arc::new(crate::deliver::HttpTransport::new());
                        let push_receiver = receiver.clone();
                        let push_id = id.clone();
                        tokio::spawn(async move {
                            let stored = bus4.peek(&push_receiver).await;
                            let Some(letter) = stored.ok().and_then(|v| {
                                v.into_iter().find(|m| m.id == push_id)
                            }) else { return };
                            let push_letter = serde_json::to_value(&letter).expect("letter json");
                            let outcome = crate::deliver::dispatch_push(
                                &registry4, &transport, &push_receiver, &push_letter,
                            ).await;
                            if let crate::deliver::PushOutcome::Pushed = outcome {
                                if let Ok(m) = bus4.mark_delivered(&push_receiver, &push_id).await {
                                    hub3.emit("delivered", &m);
                                }
                            }
                        });
                        return Ok(json!({"id": id, "forwarded": true, "to_pool": from_pool}));
                    }

                    // (2) outbound to a federated peer?
                    if !peers.is_empty() || dynamic_peers.len().await > 0 {
                        // env peers win on name clash; dashboard-learned peers fill the rest.
                        let peer = match peers.resolve(&receiver) {
                            Some(p) => Some(p),
                            None => dyn_peers3.contains_agent(&receiver).await,
                        };
                        if let Some(peer) = peer {
                            // The receiver lives on the peer pool: shadow-register
                            // them locally so bus rules accept the letter (same
                            // pattern as the inbound auto-registration).
                            if !bus3.is_registered(&receiver).await {
                                registry3
                                    .register_tagged(
                                        &receiver,
                                        &format!("federated at {}", peer.name()),
                                        crate::deliver::DeliverVia::Poll,
                                        vec![peer.name().to_string()],
                                    )
                                    .await;
                                bus3.register(&receiver, crate::bus::Role::Worker)
                                    .await
                                    .map_err(|e| e.to_string())?;
                            }
                            let id = bus3
                                .send_envelope(
                                    &sender, &receiver, mt, &subject, &body, r#ref.clone(),
                                    0, Some(pool_name.clone()),
                                )
                                .await
                                .map_err(|e| e.to_string())?;
                            // Fetch the stored letter, mark it forwarded on
                            // successful handover; on failure it stays queued.
                            let stored = bus3.peek(&receiver).await;
                            let letter = stored.ok().and_then(|v| {
                                v.into_iter().find(|m| m.id == id)
                            });
                            match crate::federation::forward(&peer, &letter.clone().expect(
                                "letter just stored", ), &pool_name).await {
                                Ok(()) => {
                                    if let Ok(m) = bus3
                                        .mark_delivered(&receiver, &id)
                                        .await
                                    {
                                        hub3.emit("delivered", &m);
                                    }
                                    return Ok(json!({
                                        "id": id,
                                        "forwarded": true,
                                        "to_pool": peer.name()
                                    }));
                                }
                                Err(e) => {
                                    return Err(format!(
                                        "forward to pool `{}` failed: {e} (letter stays queued)",
                                        peer.name()
                                    ))
                                }
                            }
                        }
                    }

                    // (3) local delivery (unchanged v0.2 path)
                    let id = bus3
                        .send(&sender, &receiver, mt, &subject, &body, r#ref)
                        .await
                        .map_err(|e| e.to_string())?;
                    // Fetch the STORED letter (canonical id) — never synthesize
                    // a fresh one, or ws/push carry a ghost id that receivers
                    // can't read/ack against.
                    let stored = bus3.peek(&receiver).await;
                    let letter = stored
                        .ok()
                        .and_then(|v| v.into_iter().find(|m| m.id == id))
                        .unwrap_or_else(|| {
                            crate::message::Message::new(
                                sender.clone(),
                                receiver.clone(),
                                mt,
                                subject.clone(),
                                body.clone(),
                                None,
                            )
                        });
                    hub3.emit("queued", &letter);
                    // Push-on-arrival (RFC-002): fire-and-forget — a push
                    // failure never blocks the send or loses the letter.
                    // On success (or a2a "notified" semantics), mark the
                    // letter delivered: it left the shelf without a poll.
                    let registry = registry3.clone();
                    let bus2 = bus3.clone();
                    let transport: Arc<dyn crate::deliver::PushTransport> =
                        Arc::new(crate::deliver::HttpTransport::new());
                    let push_letter = serde_json::to_value(&letter).expect("letter json");
                    let push_receiver = receiver.clone();
                    let push_id = id.clone();
                    tokio::spawn(async move {
                        let outcome = crate::deliver::dispatch_push(
                            &registry,
                            &transport,
                            &push_receiver,
                            &push_letter,
                        )
                        .await;
                        match outcome {
                            crate::deliver::PushOutcome::Pushed => {
                                if let Ok(m) =
                                    bus2.mark_delivered(&push_receiver, &push_id).await
                                {
                                    hub.emit("delivered", &m);
                                }
                            }
                            crate::deliver::PushOutcome::Failed { error } => {
                                eprintln!(
                                    "🥔 push failed for {push_receiver}: {error}"
                                );
                            }
                            crate::deliver::PushOutcome::Skipped { .. } => {}
                        }
                    });
                    Ok(json!({"id": id}))
                }
            })
            .await
        }
        "message/poll" => {
            let agent = match req.params.get("agent").and_then(|v| v.as_str()) {
                Some(a) => a.to_string(),
                None => "missing param: agent".to_string(),
            };
            if agent.starts_with("missing param") {
                return (
                    StatusCode::OK,
                    Json(
                        json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32602,"message":agent}}),
                    ),
                );
            }
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            bus.poll(&agent, limit)
                .await
                .map(|msgs| {
                    for m in &msgs {
                        hub.emit("delivered", m);
                    }
                    json!(msgs)
                })
                .map_err(|e| e.to_string())
        }
        "message/read" => {
            parse2(&req.params, ["agent", "id"], |_p, agent, id| {
                let (agent, id) = (agent.to_string(), id.to_string());
                async move {
                    bus.mark_read(&agent, &id)
                        .await
                        .map(|m| {
                            hub.emit("read", &m);
                            json!(m)
                        })
                        .map_err(|e| e.to_string())
                }
            })
            .await
        }
        "message/ack" => {
            parse3(
                &req.params,
                ["agent", "id", "note"],
                |_p, agent, id, note| {
                    let (agent, id, note) = (agent.to_string(), id.to_string(), note.to_string());
                    async move {
                        bus.ack(&agent, &id, &note)
                            .await
                            .map(|m| {
                                hub.emit("acked", &m);
                                json!(m)
                            })
                            .map_err(|e| e.to_string())
                    }
                },
            )
            .await
        }
        "agent/status" => {
            parse1(&req.params, "agent", |agent| {
                let agent = agent.to_string();
                async move {
                    bus.status(&agent)
                        .await
                        .map(|m| json!(m))
                        .map_err(|e| e.to_string())
                }
            })
            .await
        }
        "message/peek" => {
            let agent = match req.params.get("agent").and_then(|v| v.as_str()) {
                Some(a) => a.to_string(),
                None => String::new(),
            };
            if agent.is_empty() {
                return param_error(&req.id, "agent", &["agent"]);
            }
            bus.peek(&agent)
                .await
                .map(|msgs| json!(msgs))
                .map_err(|e| e.to_string())
        }
        "bus/archive" => bus
            .archive()
            .await
            .map(|m| json!(m))
            .map_err(|e| e.to_string()),
        "message/list" => {
            let status = req
                .params
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            bus.list_all()
                .await
                .map(|mut msgs| {
                    if let Some(s) = &status {
                        msgs.retain(|m| {
                            format!("{:?}", m.status).to_lowercase() == s.to_lowercase()
                        });
                    }
                    fifo_last(&mut msgs, limit);
                    json!(msgs)
                })
                .map_err(|e| e.to_string())
        }
        "peer/invite" => {
            // RFC-006: dashboard "Invite a peer" button. One-time code, 10 min.
            let inv = crate::handshake::create_invite(&invites, &config.public_url).await;
            Ok(json!({
                "code": inv.code,
                "expires_at": inv.expires_at,
                "note": "paste into the peer's dashboard 'Join a peer' within 10 minutes; single use"
            }))
        }
        "peer/join" => {
            // RFC-006 three-way handshake. The invite code embeds the inviter's
            // URL: hp-<code>-<b64url of inviter base>. The JOINER's dashboard
            // calls ITS OWN bus's peer/join; this bus then dials the inviter
            // with the code (step 1), the inviter records the joiner and
            // dials back peer/register (step 2), then proves the link with a
            // welcome letter (step 3).
            let p = &req.params;
            let code = p.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let agents: Vec<String> = p.get("agents").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if code.is_empty() || url.is_empty() || name.is_empty() || agents.is_empty() {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32602,
                        "message":"code, url, name, agents[] all required"}})),
                );
            }
            // Decode inviter URL from the code tail (urlsafe b64 of json).
            let inviter_url = code.rsplit('-').next()
                .and_then(|tail| crate::handshake::b64url_decode(tail))
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_string));
            let Some(inviter_url) = inviter_url else {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32602,
                        "message":"cannot read inviter url from invite code"}})),
                );
            };
            let client = reqwest::Client::new();
            // Step 1: present the code to the inviter.
            let present = client
                .post(&inviter_url)
                .timeout(std::time::Duration::from_secs(10))
                .json(&json!({
                    "jsonrpc":"2.0","id":1,"method":"peer/accept","params":{
                        "code": code, "url": url, "name": name, "agents": agents
                    }
                }))
                .send()
                .await;
            let present = match present {
                Ok(r) if r.status().is_success() => r.json::<serde_json::Value>().await.unwrap_or_default(),
                Ok(r) => return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32001,
                        "message":format!("inviter replied HTTP {}", r.status())}})),
                ),
                Err(e) => return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32001,
                        "message":format!("cannot reach inviter at {inviter_url}: {e}")}})),
                ),
            };
            if let Some(err) = present.get("error") {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32001,
                        "message":format!("inviter rejected: {err}")}})),
                );
            }
            // The inviter's accept response carries its identity for step 2.
            let inviter = present.get("result").cloned().unwrap_or(json!({}));
            let inviter_name = inviter.get("name").and_then(|v| v.as_str()).unwrap_or("peer").to_string();
            let inviter_agents: Vec<String> = inviter.get("agents").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            dynamic_peers.add(crate::federation::Peer {
                name: inviter_name.clone(),
                url: inviter_url.clone(),
                token: String::new(),
                agents: inviter_agents,
            }).await;
            Ok(json!({
                "joined": inviter_name,
                "note": "handshake complete: peer learned, forward path proven by their welcome letter"
            }))
        }
        "peer/accept" => {
            // Step 1 counterpart, on the INVITER: validate the code, record
            // the joiner, dial back peer/register (step 2), push a welcome
            // letter (step 3).
            let p = &req.params;
            let code = p.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let agents: Vec<String> = p.get("agents").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if !crate::handshake::consume_invite(&invites, code).await {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32001,
                        "message":"invalid, expired or already-used invite code"}})),
                );
            }
            if url.is_empty() || name.is_empty() || agents.is_empty() {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32602,
                        "message":"url, name, agents[] all required"}})),
                );
            }
            dynamic_peers.add(crate::federation::Peer {
                name: name.clone(), url: url.clone(), token: String::new(),
                agents: agents.clone(),
            }).await;
            let client = reqwest::Client::new();
            // Step 2: tell the joiner who we are.
            let my_agents = registry.all().await.iter().map(|e| e.agent.clone()).collect::<Vec<_>>();
            let _ = client.post(&url).timeout(std::time::Duration::from_secs(10))
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"peer/register","params":{
                    "name": config.pool_name, "url": config.public_url, "agents": my_agents }}))
                .send().await;
            // Step 3: prove the forward path with a real letter.
            if let Some(first) = agents.first() {
                let _ = client.post(&url).timeout(std::time::Duration::from_secs(10))
                    .header("x-potato-pool", &config.pool_name)
                    .json(&json!({"jsonrpc":"2.0","id":2,"method":"message/send","params":{
                        "sender":"sho","receiver":first,"type":"task",
                        "subject":"[handshake] link established",
                        "body":format!("pool `{name}` joined federation; forward path verified."),
                        "hops":1,"forwarded_from":config.pool_name }}))
                    .send().await;
            }
            let my_agents2 = registry.all().await.iter().map(|e| e.agent.clone()).collect::<Vec<_>>();
            Ok(json!({"name": config.pool_name, "agents": my_agents2,
                "note": "joiner recorded; callback + welcome letter sent"}))
        }
        "peer/register" => {
            // Step 2 counterpart: the inviting bus registers itself on the
            // joiner. Authenticated implicitly by being dialed from a bus that
            // just consumed our invite (v1 trust: network-level reachability).
            let p = &req.params;
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let agents: Vec<String> = p.get("agents").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if name.is_empty() || url.is_empty() || agents.is_empty() {
                return (
                    StatusCode::OK,
                    Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32602,
                        "message":"name, url, agents[] required"}})),
                );
            }
            dynamic_peers.add(crate::federation::Peer {
                name, url, token: String::new(), agents,
            }).await;
            Ok(json!({"registered": true}))
        }
        "peer/list" => {
            let mut env_peers = config.peers.all();
            let dyn_peers = dynamic_peers.all().await;
            for p in dyn_peers {
                if !env_peers.iter().any(|e| e.name() == p.name()) {
                    env_peers.push(p);
                }
            }
            Ok(json!(env_peers.iter().map(|p| json!({
                "name": p.name(), "url": p.url, "agents": p.agents,
                "auth": if p.token.is_empty() { "none" } else { "bearer" },
            })).collect::<Vec<_>>()))
        }
        "rpc.discover" => Ok(discover().await),
        other => Err(format!(
            "unknown method: {other} (call rpc.discover for the method table)"
        )),
    };

    match result {
        Ok(v) => (
            StatusCode::OK,
            Json(json!({"jsonrpc":"2.0","id":req.id,"result":v})),
        ),
        Err(e) => (
            StatusCode::OK, // JSON-RPC errors ride 200 with an error object
            Json(json!({"jsonrpc":"2.0","id":req.id,"error":{"code":-32603,"message":e}})),
        ),
    }
}

// --- small param extractors to keep the match arms readable ---

type P<'a> = &'a Value;

/// Keep the last `limit` letters (newest window); limit 0 = all.
fn fifo_last(msgs: &mut Vec<crate::message::Message>, limit: usize) {
    if limit > 0 && msgs.len() > limit {
        let drain = msgs.len() - limit;
        msgs.drain(0..drain);
    }
}

async fn parse1<F, Fut>(p: P<'_>, k1: &str, f: F) -> Result<Value, String>
where
    F: FnOnce(&str) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let v1 = p.get(k1).and_then(|v| v.as_str()).ok_or("missing param")?;
    f(v1).await
}

async fn parse2<F, Fut>(p: P<'_>, keys: [&str; 2], f: F) -> Result<Value, String>
where
    F: FnOnce(P<'_>, &str, &str) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let v1 = p
        .get(keys[0])
        .and_then(|v| v.as_str())
        .ok_or("missing param")?;
    let v2 = p
        .get(keys[1])
        .and_then(|v| v.as_str())
        .ok_or("missing param")?;
    f(p, v1, v2).await
}

async fn parse3<F, Fut>(p: P<'_>, keys: [&str; 3], f: F) -> Result<Value, String>
where
    F: FnOnce(P<'_>, &str, &str, &str) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let (v1, v2, v3) = (
        p.get(keys[0])
            .and_then(|v| v.as_str())
            .ok_or("missing param")?,
        p.get(keys[1])
            .and_then(|v| v.as_str())
            .ok_or("missing param")?,
        p.get(keys[2])
            .and_then(|v| v.as_str())
            .ok_or("missing param")?,
    );
    f(p, v1, v2, v3).await
}

async fn parse5<F, Fut>(p: P<'_>, f: F) -> Result<Value, String>
where
    F: FnOnce(&str, &str, &str, &str, &str) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let g = |k: &str| p.get(k).and_then(|v| v.as_str()).ok_or("missing param");
    f(
        g("sender")?,
        g("receiver")?,
        g("type").unwrap_or("task"),
        g("subject")?,
        g("body")?,
    )
    .await
}

/// 400-class JSON-RPC error that names the missing param and lists expected ones.
fn param_error(id: &Option<Value>, missing: &str, expected: &[&str]) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc":"2.0","id":id,
            "error":{
                "code":-32602,
                "message": format!("missing required param: {missing}"),
                "data": {"expected_params": expected}
            }
        })),
    )
}

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok", "service": "hot-potato"}))
}

/// GET /version — the running binary's cargo version (Sho: confirm which build is live).
async fn version() -> impl IntoResponse {
    Json(json!({
        "service": "hot-potato",
        "version": env!("CARGO_PKG_VERSION"),
        "built_at": env!("BUILD_TS"),
    }))
}

/// GET / — the live dashboard (static HTML, baked into the binary at compile time).
async fn dashboard() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../dashboard/index.html"),
    )
}

/// GET /log?limit=N — human-readable lifecycle page (RFC-001 F2).
/// The chatlog view: every letter on the bus, newest last, one line each.
async fn log_page(
    State((bus, _hub, _config, _registry, _invites, _dynamic_peers)): State<BusState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let msgs = bus.list_all().await.unwrap_or_default();
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let start = msgs.len().saturating_sub(limit);
    let mut body = String::from("🥔 hot-potato lifecycle log\n\n");
    body.push_str(&format!(
        "{:<6} {:<10} {:<10} {:<11} {:<26} {}\n",
        "status", "sender", "receiver", "type", "id", "subject"
    ));
    body.push_str(&"-".repeat(90));
    body.push('\n');
    for m in &msgs[start..] {
        body.push_str(&format!(
            "{:<6} {:<10} {:<10} {:<11} {:<26} {}\n",
            format!("{:?}", m.status).to_lowercase(),
            m.sender,
            m.receiver,
            format!("{:?}", m.msg_type).to_lowercase(),
            m.id,
            m.subject,
        ));
    }
    body.push_str(&format!(
        "\n{} letters shown ({} total)\n",
        msgs.len() - start,
        msgs.len()
    ));
    axum::http::header::HeaderMap::new(); // keep type inference happy
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        body,
    )
}

/// Shared app state: the bus, its config, the ws event hub, the delivery
/// registry that makes push-on-arrival real (RFC-002), and the RFC-006
/// dashboard-driven federation state (invites + dynamically learned peers).
pub type BusState = (
    Arc<EventBus>,
    Arc<EventHub>,
    Arc<ServerConfig>,
    Arc<crate::deliver::Registry>,
    Arc<tokio::sync::RwLock<Vec<crate::handshake::Invite>>>,
    Arc<crate::handshake::DynamicPeers>,
);

/// Build the router (exposed for tests + compose).
pub fn router(bus: Arc<EventBus>, config: Arc<ServerConfig>) -> Router {
    let hub = Arc::new(EventHub::new());
    // Registry persistence: survive restarts. The registry file lives next to
    // the sled data dir (HOT_POTATO_DATA_DIR) when set — no dir, no file.
    let registry = Arc::new(match std::env::var("HOT_POTATO_DATA_DIR") {
        Ok(dir) if !dir.is_empty() => crate::deliver::Registry::new()
            .with_persistence(std::path::Path::new(&dir).join("registry.json")),
        _ => crate::deliver::Registry::new(),
    });
    // RFC-006: dashboard-driven federation state — invites + dynamically
    // learned peers, persisted next to the registry.
    let invites: Arc<tokio::sync::RwLock<Vec<crate::handshake::Invite>>> =
        Arc::new(tokio::sync::RwLock::new(Vec::new()));
    let dynamic_peers = match std::env::var("HOT_POTATO_DATA_DIR") {
        Ok(dir) if !dir.is_empty() => crate::handshake::DynamicPeers::new()
            .with_persistence(std::path::Path::new(&dir).join("peers.json")),
        _ => crate::handshake::DynamicPeers::new(),
    };
    let card = config.agent_card();
    let app = crate::ws::router()
        .route("/", get(dashboard).post(rpc))
        .route("/log", get(log_page))
        .route("/health", get(health))
        .route("/version", get(version))
        .route(
            "/.well-known/agent-card.json",
            get(move || async move {
                Json(serde_json::to_value(card.clone()).expect("card serializes"))
            }),
        )
        .with_state((bus, hub, config, registry, invites, dynamic_peers));
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/docs")
        .url("/openapi.json", crate::openapi::openapi_doc());
    app.merge(swagger)
}

/// Bind and serve. Called from main.
pub async fn serve(
    bus: Arc<EventBus>,
    config: Arc<ServerConfig>,
    addr: &str,
) -> std::io::Result<()> {
    let app = router(bus, config);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageStatus;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // oneshot

    async fn test_bus() -> Arc<EventBus> {
        let bus = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));
        bus.register("patricia", Role::Pm).await.unwrap();
        bus.register("diana", Role::Worker).await.unwrap();
        bus.register("victoria", Role::Worker).await.unwrap();
        bus
    }

    fn app(bus: Arc<EventBus>) -> Router {
        let cfg = Arc::new(ServerConfig {
            name: "test".into(),
            description: "d".into(),
            public_url: "http://test".into(),
            version: "0.1.0".into(),
            bearer_token: None,
            peers: crate::federation::Peers::default(),
            pool_name: "test".into(),
        });
        router(bus, cfg)
    }

    async fn rpc_call(app: Router, method: &str, params: Value) -> Value {
        let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        let res = app
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    }

    #[tokio::test]
    async fn agent_card_is_served() {
        let res = app(test_bus().await)
            .oneshot(
                Request::get("/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let card: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(card["protocol_version"], "1.0");
        assert_eq!(card["skills"].as_array().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn full_hot_potato_roundtrip_over_http() {
        let bus = test_bus().await;
        let a = app(bus.clone());

        // send
        let res = rpc_call(
            a,
            "message/send",
            json!({
                "sender":"patricia","receiver":"diana","type":"task",
                "subject":"run X","body":"b"
            }),
        )
        .await;
        assert!(res.get("result").is_some(), "send failed: {res}");
        let id = res["result"]["id"].as_str().unwrap().to_string();

        // poll (new router instance shares the bus)
        let res = rpc_call(app(bus.clone()), "message/poll", json!({"agent":"diana"})).await;
        assert!(res.get("result").is_some(), "poll failed: {res}");
        assert_eq!(res["result"].as_array().unwrap().len(), 1);

        // read receipt
        let res = rpc_call(
            app(bus.clone()),
            "message/read",
            json!({"agent":"diana","id":id}),
        )
        .await;
        assert_eq!(res["result"]["status"], "read");

        // ack
        let res = rpc_call(
            app(bus.clone()),
            "message/ack",
            json!({"agent":"diana","id":id,"note":"done"}),
        )
        .await;
        assert_eq!(res["result"]["status"], "acked");

        // status shows the lifecycle to the sender
        let res = rpc_call(
            app(bus.clone()),
            "agent/status",
            json!({"agent":"patricia"}),
        )
        .await;
        let arr = res["result"].as_array().unwrap();
        assert_eq!(arr[0]["status"], "acked");
        assert!(arr[0]["acked_at"].is_string());
    }

    #[tokio::test]
    async fn message_list_is_read_only_observer_view() {
        let bus = test_bus().await;
        let a = app(bus.clone());

        // two letters in different states
        let res = rpc_call(
            a,
            "message/send",
            json!({
                "sender":"patricia","receiver":"diana","type":"task",
                "subject":"one","body":"b"
            }),
        )
        .await;
        let id1 = res["result"]["id"].as_str().unwrap().to_string();
        let _ = rpc_call(
            app(bus.clone()),
            "message/send",
            json!({
                "sender":"patricia","receiver":"diana","type":"task",
                "subject":"two","body":"b"
            }),
        )
        .await;

        // list sees both, no state changed
        let res = rpc_call(app(bus.clone()), "message/list", json!({})).await;
        let arr = res["result"].as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // filter by status
        let res = rpc_call(app(bus.clone()), "message/list", json!({"status":"queued"})).await;
        assert_eq!(res["result"].as_array().unwrap().len(), 2);

        // poll one, now the filter splits
        let _ = rpc_call(
            app(bus.clone()),
            "message/poll",
            json!({"agent":"diana","limit":1}),
        )
        .await;
        let res = rpc_call(
            app(bus.clone()),
            "message/list",
            json!({"status":"delivered"}),
        )
        .await;
        let arr = res["result"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id1);
    }

    #[tokio::test]
    async fn log_page_renders_plain_text() {
        let bus = test_bus().await;
        let a = app(bus.clone());
        let _ = rpc_call(
            a,
            "message/send",
            json!({
                "sender":"patricia","receiver":"diana","type":"task",
                "subject":"hello log","body":"b"
            }),
        )
        .await;

        let res = app(bus)
            .oneshot(Request::get("/log").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let content_type = res
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            text.contains("hello log"),
            "log page missing subject: {text}"
        );
        assert!(text.contains("patricia"));
        assert!(content_type.starts_with("text/plain"));
    }

    #[tokio::test]
    async fn bearer_token_rejects_when_set() {
        let bus = test_bus().await;
        let cfg = Arc::new(ServerConfig {
            name: "t".into(),
            description: "d".into(),
            public_url: "u".into(),
            version: "0".into(),
            bearer_token: Some("secret".into()),
            peers: crate::federation::Peers::default(),
            pool_name: "t".into(),
        });
        let a = router(bus, cfg);
        let body = json!({"jsonrpc":"2.0","id":1,"method":"bus/archive","params":{}});
        let res = a
            .oneshot(
                Request::post("/")
                    .header("authorization", "Bearer wrong")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    // --- RFC-004: bus federation -------------------------------------------

    /// A bus app that routes `diana` to a fake peer at 127.0.0.1:1
    /// (connection refused — deterministic "peer down").
    fn fed_app_down_peer(bus: Arc<EventBus>) -> Router {
        let peers = crate::federation::Peers::from_json(
            r#"[{"name":"fleet","url":"http://127.0.0.1:1","token":"x",
                 "agents":["diana"]}]"#,
        )
        .unwrap();
        let cfg = Arc::new(ServerConfig {
            name: "t".into(),
            description: "d".into(),
            public_url: "u".into(),
            version: "0".into(),
            bearer_token: None,
            peers,
            pool_name: "sho".into(),
        });
        router(bus, cfg)
    }

    #[tokio::test]
    async fn federation_peer_down_letter_stays_queued() {
        let bus = test_bus().await;
        let res = rpc_call(
            fed_app_down_peer(bus.clone()),
            "message/send",
            json!({"sender":"patricia","receiver":"diana","type":"task",
                   "subject":"cross pool","body":"b"}),
        )
        .await;
        // send returns a JSON-RPC error (forward failed), letter stays queued
        assert!(res.get("error").is_some(), "expected forward failure: {res}");
        assert!(res["error"]["message"].as_str().unwrap().contains("stays queued"));
    }

    #[tokio::test]
    async fn federation_local_delivery_untouched_when_no_peer_matches() {
        // peers configured, but receiver is local → normal path
        let bus = test_bus().await;
        let res = rpc_call(
            fed_app_down_peer(bus.clone()),
            "message/send",
            json!({"sender":"patricia","receiver":"victoria","type":"task",
                   "subject":"local one","body":"b"}),
        )
        .await;
        assert!(res.get("result").is_some(), "local send failed: {res}");
        assert!(res["result"]["forwarded"].is_null());
        let peek = rpc_call(app(bus), "message/peek", json!({"agent":"victoria"})).await;
        assert_eq!(peek["result"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn federation_inbound_requires_token_and_envelope() {
        let bus = test_bus().await;
        let peers = crate::federation::Peers::default();
        let cfg = Arc::new(ServerConfig {
            name: "t".into(),
            description: "d".into(),
            public_url: "u".into(),
            version: "0".into(),
            bearer_token: Some("poolsecret".into()),
            peers,
            pool_name: "sho".into(),
        });
        let a = router(bus.clone(), cfg);

        let body = json!({"jsonrpc":"2.0","id":1,"method":"message/send",
            "params":{"sender":"patricia","receiver":"victoria","type":"task",
                      "subject":"x","body":"b"}});
        // federated header WITHOUT token → rejected
        let res = a
            .clone()
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .header("x-potato-pool", "fleet")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // token but no hops/forwarded_from → error
        let res = a
            .clone()
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer poolsecret")
                    .header("x-potato-pool", "fleet")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("hops"));

        // full envelope → accepted, remote sender auto-registered
        let body = json!({"jsonrpc":"2.0","id":2,"method":"message/send",
            "params":{"sender":"patricia","receiver":"victoria","type":"task",
                      "subject":"cross-pool hi","body":"b",
                      "hops":1,"forwarded_from":"fleet"}});
        let res = a
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer poolsecret")
                    .header("x-potato-pool", "fleet")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["result"]["forwarded"], true);

        // the letter landed in victoria's mailbox with the envelope intact
        let peek = rpc_call(app(bus), "message/peek", json!({"agent":"victoria"})).await;
        let arr = peek["result"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["sender"], "patricia");
        assert_eq!(arr[0]["forwarded_from"], "fleet");
        assert_eq!(arr[0]["hops"], 1);
    }

    #[tokio::test]
    async fn federation_inbound_hops_over_limit_rejected() {
        let bus = test_bus().await;
        let cfg = Arc::new(ServerConfig {
            name: "t".into(),
            description: "d".into(),
            public_url: "u".into(),
            version: "0".into(),
            bearer_token: Some("poolsecret".into()),
            peers: crate::federation::Peers::default(),
            pool_name: "sho".into(),
        });
        let a = router(bus, cfg);
        let body = json!({"jsonrpc":"2.0","id":1,"method":"message/send",
            "params":{"sender":"patricia","receiver":"victoria","type":"task",
                      "subject":"loop","body":"b","hops":9,"forwarded_from":"fleet"}});
        let res = a
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer poolsecret")
                    .header("x-potato-pool", "fleet")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("hop limit"));
    }

    #[tokio::test]
    async fn federation_end_to_end_two_real_buses() {
        // The real thing: two in-process buses wired to each other over HTTP.
        // Pool A (sho): patricia local, diana routed to pool B.
        // Pool B (fleet): diana local, receives over HTTP from pool A.
        use tokio::net::TcpListener;

        let bus_a = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));
        bus_a.register("patricia", Role::Pm).await.unwrap();
        let l_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_a = l_a.local_addr().unwrap();

        let bus_b = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));
        bus_b.register("diana", Role::Worker).await.unwrap();
        let l_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_b = l_b.local_addr().unwrap();
        let cfg_b = Arc::new(ServerConfig {
            name: "poolB".into(),
            description: "d".into(),
            public_url: format!("http://{addr_b}"),
            version: "0".into(),
            bearer_token: Some("shared-secret".into()),
            peers: crate::federation::Peers::default(),
            pool_name: "fleet".into(),
        });

        // Point A's routing at B (real addr), then serve both.
        let peers_a = crate::federation::Peers::from_json(&format!(
            r#"[{{"name":"fleet","url":"http://{addr_b}","token":"shared-secret",
                 "agents":["diana"]}}]"#
        ))
        .unwrap();

        let cfg_a = Arc::new(ServerConfig {
            name: "poolA".into(),
            description: "d".into(),
            public_url: format!("http://{addr_a}"),
            version: "0".into(),
            bearer_token: Some("shared-secret".into()),
            peers: peers_a,
            pool_name: "sho".into(),
        });

        tokio::spawn(async move {
            axum::serve(l_a, router(bus_a, cfg_a)).await.unwrap();
        });
        let bus_b2 = bus_b.clone();
        tokio::spawn(async move {
            axum::serve(l_b, router(bus_b2, cfg_b)).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // anastasia-side agent (a client of pool A) sends to diana (pool B)
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{addr_a}"))
            .bearer_auth("shared-secret")
            .json(&json!({
                "jsonrpc":"2.0","id":1,"method":"message/send",
                "params":{"sender":"patricia","receiver":"diana","type":"task",
                          "subject":"fed e2e","body":"across pools!"}
            }))
            .send()
            .await
            .unwrap();
        let v: Value = resp.json().await.unwrap();
        assert!(v.get("error").is_none(), "forward failed: {v}");
        assert_eq!(v["result"]["forwarded"], true);
        assert_eq!(v["result"]["to_pool"], "fleet");

        // diana polls her LOCAL bus on pool B and finds the letter
        let resp = client
            .post(format!("http://{addr_b}"))
            .bearer_auth("shared-secret")
            .json(&json!({"jsonrpc":"2.0","id":2,"method":"message/poll",
                          "params":{"agent":"diana"}}))
            .send()
            .await
            .unwrap();
        let v: Value = resp.json().await.unwrap();
        let letters = v["result"].as_array().unwrap();
        assert_eq!(letters.len(), 1);
        assert_eq!(letters[0]["sender"], "patricia");
        assert_eq!(letters[0]["subject"], "fed e2e");
        assert_eq!(letters[0]["forwarded_from"], "sho");
        assert_eq!(letters[0]["hops"], 1);
    }
}
