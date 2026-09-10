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
            {"name":"agent/register","params":["agent","role (pm|worker)"],"desc":"register an agent on the bus"},
            {"name":"message/send","params":["sender","receiver","type (task|reply|broadcast|ack_only)","subject","body","ref (required for reply)"],"desc":"pass a potato"},
            {"name":"message/poll","params":["agent","limit (optional, 0=all)"],"desc":"drain my mailbox; marks returned letters delivered (poll = claim)"},
            {"name":"message/peek","params":["agent"],"desc":"look without marking (queued letters only)"},
            {"name":"message/read","params":["agent","id"],"desc":"chatlog read receipt; requires delivered state"},
            {"name":"message/ack","params":["agent","id","note (<=80 chars)"],"desc":"file the result; idempotent on already-acked; accepts delivered or read"},
            {"name":"agent/status","params":["agent"],"desc":"lifecycle of everything this agent SENT"},
            {"name":"bus/archive","params":[],"desc":"all acked letters (audit log)"},
            {"name":"message/list","params":["status (optional: queued|delivered|read|acked)","limit (optional, 0=all)"],"desc":"OBSERVER: every letter on the bus, any status, read-only (v0.2)"},
            {"name":"rpc.discover","params":[],"desc":"this document"}
        ]
    })
}

/// JSON-RPC handler — the single endpoint agents talk to.
async fn rpc(
    State((bus, hub, config, registry)): State<BusState>,
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
                            "a2a" => crate::deliver::DeliverVia::A2a { url },
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
                let registry = registry.clone();
                async move {
                    let entry = registry.register(&agent, &description, deliver_via).await;
                    bus.register(&agent, role)
                        .await
                        .map(|_| {
                            json!({
                                "registered": agent,
                                "deliver_via": entry.deliver_via,
                            })
                        })
                        .map_err(|e| e.to_string())
                }
            })
            .await
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
                let (sender, receiver, subject, body) = (
                    sender.to_string(),
                    receiver.to_string(),
                    subject.to_string(),
                    body.to_string(),
                );
                async move {
                    let id = bus
                        .send(&sender, &receiver, mt, &subject, &body, r#ref)
                        .await
                        .map_err(|e| e.to_string())?;
                    // Fetch the STORED letter (canonical id) — never synthesize
                    // a fresh one, or ws/push carry a ghost id that receivers
                    // can't read/ack against.
                    let stored = bus.peek(&receiver).await;
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
                    hub.emit("queued", &letter);
                            // Push-on-arrival (RFC-002): fire-and-forget — a push
                            // failure never blocks the send or loses the letter.
                            // On success (or a2a "notified" semantics), mark the
                            // letter delivered: it left the shelf without a poll.
                            let registry = registry.clone();
                            let bus2 = bus.clone();
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
    State((bus, _hub, _config, _registry)): State<BusState>,
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

/// Shared app state: the bus, its config, the ws event hub, and the delivery
/// registry that makes push-on-arrival real (RFC-002).
pub type BusState = (
    Arc<EventBus>,
    Arc<EventHub>,
    Arc<ServerConfig>,
    Arc<crate::deliver::Registry>,
);

/// Build the router (exposed for tests + compose).
pub fn router(bus: Arc<EventBus>, config: Arc<ServerConfig>) -> Router {
    let hub = Arc::new(EventHub::new());
    let registry = Arc::new(crate::deliver::Registry::new());
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
        .with_state((bus, hub, config, registry));
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
        bus
    }

    fn app(bus: Arc<EventBus>) -> Router {
        let cfg = Arc::new(ServerConfig {
            name: "test".into(),
            description: "d".into(),
            public_url: "http://test".into(),
            version: "0.1.0".into(),
            bearer_token: None,
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
}
