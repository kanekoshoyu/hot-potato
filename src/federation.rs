//! Bus federation (RFC-004) — pools connect directly, bus-to-bus, over the
//! same HTTP JSON-RPC the agents already speak.
//!
//! Sho's design brief (2026-09-11):
//! - "每一个 Bus 是直接跟另一个 Bus 通过开放某个 Port 去连接的" — each bus opens
//!   one ordinary HTTP port and peers dial each other directly.
//! - "顶多就是说你有一些 password……做一个加密而已" — a shared bearer token per
//!   link is all the security a v1 needs.
//! - No link agents, no tunnels, no extra components.
//!
//! Model (prior art: SMTP MX + Matrix federation, cut down to our size):
//! - Each pool lists its remote peers in `HOT_POTATO_PEERS` (JSON env):
//!     [{"name":"fleet","url":"http://203.0.113.10:8081",
//!       "token":"<that bus's HOT_POTATO_TOKEN>",
//!       "agents":["patricia","diana","victoria","isabella"]}]
//! - A letter addressed to an agent in a peer's list is forwarded to that
//!   peer's bus as a plain `message/send` over HTTP with that peer's token.
//! - The receiving bus auto-registers federated senders (Worker role) when
//!   the request carries the `X-Potato-Pool` header + valid token, so
//!   cross-pool letters validate under the normal bus rules.
//! - Loop safety: `hops` counter on the letter (max 2). A letter that has
//!   already crossed MAX_HOPS links is refused, so a misconfigured ring of
//!   pools cannot ping-pong forever.

use crate::message::Message;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A letter may cross at most this many inter-bus links before it is
/// considered a routing loop and refused.
pub const MAX_HOPS: u8 = 2;

/// One remote pool: where it lives, how to auth, and which agents are at home there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// Pool name — used in logs and the `forwarded_from` envelope field.
    pub name: String,
    /// Base URL of the peer bus (its existing JSON-RPC endpoint).
    pub url: String,
    /// The peer bus's `HOT_POTATO_TOKEN` (bearer).
    pub token: String,
    /// Agent names whose home is this pool.
    pub agents: Vec<String>,
}

impl Peer {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn contains(&self, agent: &str) -> bool {
        self.agents.iter().any(|a| a == agent)
    }
}

/// The federation routing table. Empty = exactly yesterday's behaviour.
#[derive(Debug, Clone, Default)]
pub struct Peers(Arc<Vec<Peer>>);

impl Peers {
    /// Parse from a JSON string (the `HOT_POTATO_PEERS` env value).
    pub fn from_json(raw: &str) -> Result<Self, String> {
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let peers: Vec<Peer> =
            serde_json::from_str(raw).map_err(|e| format!("HOT_POTATO_PEERS invalid: {e}"))?;
        for p in &peers {
            if p.name.is_empty() || p.url.is_empty() || p.agents.is_empty() {
                return Err(format!(
                    "HOT_POTATO_PEERS: peer `{}` needs name, url and a non-empty agents list",
                    p.name
                ));
            }
        }
        Ok(Self(Arc::new(peers)))
    }

    /// From the env var. Absent/empty = no federation.
    pub fn from_env() -> Self {
        match std::env::var("HOT_POTATO_PEERS") {
            Ok(raw) => Self::from_json(&raw).unwrap_or_else(|e| {
                eprintln!("🥔 {e} — federation disabled");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn all(&self) -> Vec<Peer> {
        (*self.0).clone()
    }

    /// Which pool is home for this agent? `None` = treat as local.
    pub fn resolve(&self, agent: &str) -> Option<Peer> {
        self.0.iter().find(|p| p.contains(agent)).cloned()
    }

    /// Does any peer claim this agent? (Admin sanity check: a peer-listed
    /// agent should NOT also be registered locally on this bus.)
    pub fn claims(&self, agent: &str) -> bool {
        self.resolve(agent).is_some()
    }
}

/// Forward a letter to its home pool. Fire-and-forget friendly: errors are
/// returned, never panics; the caller keeps the local copy queued on failure.
pub async fn forward(peer: &Peer, letter: &Message, from_pool: &str) -> Result<(), String> {
    if letter.hops >= MAX_HOPS {
        return Err(format!(
            "hop limit {MAX_HOPS} reached (path: {})",
            letter.forwarded_from.as_deref().unwrap_or("?")
        ));
    }
    let mut params = serde_json::json!({
        "sender": letter.sender,
        "receiver": letter.receiver,
        "type": format!("{:?}", letter.msg_type).to_lowercase(),
        "subject": letter.subject,
        "body": letter.body,
        "hops": letter.hops + 1,
        "forwarded_from": from_pool,
    });
    if let Some(r) = &letter.r#ref {
        params["ref"] = serde_json::json!(r);
    }
    let client = reqwest::Client::new();
    let resp = client
        .post(&peer.url)
        .bearer_auth(&peer.token)
        .header("x-potato-pool", from_pool)
        .timeout(std::time::Duration::from_secs(10))
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "message/send", "params": params
        }))
        .send()
        .await
        .map_err(|e| format!("{} unreachable: {e}", peer.name()))?;
    if !resp.status().is_success() {
        return Err(format!("{} replied HTTP {}", peer.name(), resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("bad json: {e}"))?;
    if let Some(err) = body.get("error") {
        return Err(format!("{} rejected letter: {err}", peer.name()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"[
        {"name":"fleet","url":"http://203.0.113.10:8081","token":"t2",
         "agents":["patricia","diana","victoria","isabella"]}
    ]"#;

    #[test]
    fn parses_peer_config() {
        let p = Peers::from_json(CFG).unwrap();
        assert_eq!(p.len(), 1);
        assert!(!p.is_empty());
    }

    #[test]
    fn empty_config_is_no_federation() {
        let p = Peers::from_json("").unwrap();
        assert!(p.is_empty());
        assert!(Peers::from_env().is_empty()); // env unset in tests
    }

    #[test]
    fn rejects_malformed_config() {
        assert!(Peers::from_json("{not json").is_err());
        assert!(Peers::from_json(r#"[{"name":"x","url":""}]"#).is_err());
    }

    #[test]
    fn resolves_remote_agents_only() {
        let p = Peers::from_json(CFG).unwrap();
        assert!(p.claims("patricia"));
        assert_eq!(p.resolve("patricia").unwrap().name(), "fleet");
        assert!(!p.claims("anastasia"));
        assert!(p.resolve("anastasia").is_none());
    }

    #[tokio::test]
    async fn forward_refuses_hops_over_limit() {
        let p = Peers::from_json(CFG).unwrap().resolve("diana").unwrap();
        let mut letter = Message::new("anastasia", "diana", crate::message::MsgType::Task, "s", "b", None);
        letter.hops = MAX_HOPS;
        // Never touches the network: refused before dialing.
        let err = forward(&p, &letter, "sho").await.unwrap_err();
        assert!(err.contains("hop limit"));
    }
}
