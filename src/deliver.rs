//! Delivery registry + push dispatcher (RFC-002, v0.2).
//!
//! Sho's architecture: the bus is a **super-connector router agent** with its
//! own identity. Agents register once (name, description, deliver_via); the
//! bus then **pushes** letters to them on arrival instead of waiting for poll.
//!
//! - A2A is the transport, not the contract: delivery is async notification,
//!   the receiver decides when to process. Poll survives as fallback
//!   (unregistered endpoints, debug, push-failure retry).
//! - Push failure never loses a letter: it stays queued, poll still works.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// How the bus should deliver to a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliverVia {
    /// Push a notification to an A2A endpoint (SendMessage with the letter).
    A2a { url: String },
    /// POST the letter as JSON to a webhook.
    Webhook { url: String },
    /// Fire-and-forget door-knock via an HTTP relay (e.g. ntfy, TG relay).
    Relay { url: String },
    /// No push — classic poll-only semantics (v0.1 behavior).
    #[serde(other)]
    Poll,
}

impl DeliverVia {
    pub fn is_push(&self) -> bool {
        !matches!(self, DeliverVia::Poll)
    }
}

/// One agent's registry entry: who they are + how to reach them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub agent: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub deliver_via: DeliverVia,
}

impl Default for DeliverVia {
    fn default() -> Self {
        DeliverVia::Poll
    }
}

/// The central registry: Sho's "register once, the bus finds you."
#[derive(Default)]
pub struct Registry {
    agents: RwLock<HashMap<String, AgentEntry>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or update. Description/deliver_via are idempotent overwrites.
    pub async fn register(
        &self,
        agent: &str,
        description: &str,
        deliver_via: DeliverVia,
    ) -> AgentEntry {
        let mut agents = self.agents.write().await;
        let entry = AgentEntry {
            agent: agent.to_string(),
            description: description.to_string(),
            deliver_via,
        };
        agents.insert(agent.to_string(), entry.clone());
        entry
    }

    pub async fn lookup(&self, agent: &str) -> Option<AgentEntry> {
        self.agents.read().await.get(agent).cloned()
    }

    pub async fn all(&self) -> Vec<AgentEntry> {
        let mut v: Vec<_> = self.agents.read().await.values().cloned().collect();
        v.sort_by(|a, b| a.agent.cmp(&b.agent));
        v
    }
}

/// What a push transport must do. A2a/Webhook/Relay each get an impl;
/// a failed push is reported, never fatal — the letter stays queued.
#[async_trait::async_trait]
pub trait PushTransport: Send + Sync {
    /// Returns Ok(()) if the receiver (probably) got the notification.
    async fn push(&self, target: &DeliverVia, letter: &serde_json::Value) -> Result<(), String>;
}

/// Real HTTP transport: webhook = POST JSON; a2a/relay = POST with method shape.
/// A2A SendMessage is a synchronous task call (waits for the peer agent to
/// finish), so the client timeout is generous — the dispatcher runs spawned,
/// so a slow push never blocks the original message/send.
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
        }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PushTransport for HttpTransport {
    async fn push(&self, target: &DeliverVia, letter: &serde_json::Value) -> Result<(), String> {
        match target {
            DeliverVia::Poll => Err("poll-only agent: nothing to push".into()),
            DeliverVia::Webhook { url } => self
                .client
                .post(url)
                .json(letter)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map(|_| ())
                .map_err(|e| format!("webhook push failed: {e}")),
            DeliverVia::A2a { url } => {
                // A2A v1.0 SendMessage shape: the bus notifies, receiver polls
                // or reads the attached letter from the notification payload.
                let payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "method": "message/send",
                    "params": {
                        "role": "user",
                        "kind": "message",
                        "parts": [{"kind": "text", "text": letter.to_string()}]
                    }
                });
                self.client
                    .post(url)
                    .json(&payload)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map(|_| ())
                    .map_err(|e| format!("a2a push failed: {e}"))
            }
            DeliverVia::Relay { url } => self
                .client
                .post(url)
                .header("Title", "hot-potato: new mail")
                .body(letter.to_string())
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map(|_| ())
                .map_err(|e| format!("relay push failed: {e}")),
        }
    }
}

/// The dispatcher the bus calls on every send: look up the receiver, push if
/// registered with a push transport. Fire-and-forget by design — a push error
/// is logged into the result, never blocks or loses the letter.
pub async fn dispatch_push(
    registry: &Arc<Registry>,
    transport: &Arc<dyn PushTransport>,
    receiver: &str,
    letter: &serde_json::Value,
) -> PushOutcome {
    let Some(entry) = registry.lookup(receiver).await else {
        return PushOutcome::Skipped {
            reason: "not in registry".into(),
        };
    };
    if !entry.deliver_via.is_push() {
        return PushOutcome::Skipped {
            reason: "poll-only".into(),
        };
    }
    match transport.push(&entry.deliver_via, letter).await {
        Ok(()) => PushOutcome::Pushed,
        Err(e) => PushOutcome::Failed { error: e },
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PushOutcome {
    Pushed,
    Failed { error: String },
    Skipped { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registry_register_lookup_roundtrip() {
        let reg = Registry::new();
        reg.register(
            "diana",
            "quant, data guardian",
            DeliverVia::A2a {
                url: "http://localhost:8643/".into(),
            },
        )
        .await;
        let e = reg.lookup("diana").await.unwrap();
        assert_eq!(e.description, "quant, data guardian");
        assert!(e.deliver_via.is_push());
        assert!(reg.lookup("ghost").await.is_none());
    }

    #[tokio::test]
    async fn poll_only_is_the_default_semantics() {
        let reg = Registry::new();
        reg.register("victoria", "viz", DeliverVia::Poll).await;
        let e = reg.lookup("victoria").await.unwrap();
        assert!(!e.deliver_via.is_push());
    }

    /// A recording transport: pushes are observable, failures injectable.
    struct RecordingTransport {
        fail_for: Vec<String>,
        pushed: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl PushTransport for RecordingTransport {
        async fn push(
            &self,
            target: &DeliverVia,
            _letter: &serde_json::Value,
        ) -> Result<(), String> {
            let url = match target {
                DeliverVia::A2a { url } => url.clone(),
                DeliverVia::Webhook { url } => url.clone(),
                DeliverVia::Relay { url } => url.clone(),
                DeliverVia::Poll => return Err("poll-only".into()),
            };
            if self.fail_for.contains(&url) {
                Err(format!("injected failure for {url}"))
            } else {
                self.pushed.lock().unwrap().push(url);
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn dispatch_pushes_to_registered_endpoint() {
        let reg = Arc::new(Registry::new());
        reg.register(
            "diana",
            "quant",
            DeliverVia::Webhook {
                url: "http://diana-hook".into(),
            },
        )
        .await;
        reg.register("victoria", "viz", DeliverVia::Poll).await;

        let transport = Arc::new(RecordingTransport {
            fail_for: vec![],
            pushed: std::sync::Mutex::new(vec![]),
        });

        // push lands for diana
        let out = dispatch_push(
            &reg,
            &(transport.clone() as Arc<dyn PushTransport>),
            "diana",
            &serde_json::json!({"id": "m1"}),
        )
        .await;
        assert!(matches!(out, PushOutcome::Pushed));
        assert_eq!(transport.pushed.lock().unwrap().len(), 1);

        // victoria is poll-only → skipped, not failed
        let out = dispatch_push(
            &reg,
            &(transport.clone() as Arc<dyn PushTransport>),
            "victoria",
            &serde_json::json!({"id": "m2"}),
        )
        .await;
        assert!(matches!(out, PushOutcome::Skipped { .. }));
    }

    #[tokio::test]
    async fn push_failure_is_reported_not_fatal() {
        let reg = Arc::new(Registry::new());
        reg.register(
            "diana",
            "quant",
            DeliverVia::Webhook {
                url: "http://broken-hook".into(),
            },
        )
        .await;
        let transport = Arc::new(RecordingTransport {
            fail_for: vec!["http://broken-hook".into()],
            pushed: std::sync::Mutex::new(vec![]),
        });
        let out = dispatch_push(
            &reg,
            &(transport as Arc<dyn PushTransport>),
            "diana",
            &serde_json::json!({"id": "m1"}),
        )
        .await;
        match out {
            PushOutcome::Failed { error } => assert!(error.contains("injected failure")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
