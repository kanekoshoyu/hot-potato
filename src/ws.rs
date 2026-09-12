//! WebSocket feed — live lifecycle events, hot off the bus.
//!
//! Connect to `ws://host:8080/ws` and receive a JSON event per lifecycle
//! transition: `queued` (letter created), `delivered` (polled), `read`,
//! `acked`. Plus a heartbeat every 60s with bus stats, so silence is
//! meaningful.
//!
//! This is the engineering face of Sho's mailman principle: the bus pushes,
//! observers don't poll.

use axum::extract::State;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::bus::EventBus;
use crate::message::Message;
use crate::store::memory::InMemoryStore;

/// Shared state tuple for the ws router. Slots 4-6 mirror BusState's registry
/// + RFC-006 federation state (unused here, kept so one state tuple flows
/// through both routers).
pub type WsState = (
    Arc<EventBus>,
    Arc<EventHub>,
    Arc<crate::server::ServerConfig>,
    Arc<crate::deliver::Registry>,
    Arc<tokio::sync::RwLock<Vec<crate::handshake::Invite>>>,
    Arc<crate::handshake::DynamicPeers>,
);

/// One lifecycle event, as seen by observers.
#[derive(Debug, Clone)]
pub struct BusEvent {
    pub event: &'static str, // queued | delivered | read | acked
    pub message: Message,
}

impl BusEvent {
    pub fn to_json(&self) -> Value {
        json!({
            "event": self.event,
            "at": chrono::Utc::now(),
            "id": self.message.id,
            "sender": self.message.sender,
            "receiver": self.message.receiver,
            "type": self.message.msg_type,
            "subject": self.message.subject,
            "status": self.message.status,
        })
    }
}

/// Broadcast hub. The bus emits into `tx`; every `/ws` client holds a rx.
/// `RECEIVERS` per event is fine: slow clients drop events rather than
/// block the bus (observers are optional, the bus never waits on them).
pub struct EventHub {
    pub tx: broadcast::Sender<BusEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn emit(&self, event: &'static str, message: &Message) {
        let _ = self.tx.send(BusEvent {
            event,
            message: message.clone(),
        });
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

/// GET /ws — upgrade and stream events. First frame = hello+stats snapshot.
async fn ws_handler(
    State((bus, hub, config, _registry, _invites, _dynamic_peers)): State<WsState>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    upgrade.on_upgrade(move |socket| stream(bus, hub, config, socket))
}

/// The actual pump: hello snapshot, then broadcast + heartbeat until close.
async fn stream(
    bus: Arc<EventBus>,
    hub: Arc<EventHub>,
    _config: Arc<crate::server::ServerConfig>,
    socket: axum::extract::ws::WebSocket,
) {
    let (mut sink, mut rx_client) = socket.split();
    let mut events = BroadcastStream::new(hub.tx.subscribe());

    // hello frame: current bus snapshot so the client can render immediately.
    // RFC-003 P4: per-mailbox backlog counts — a reconnecting agent sees its
    // queue depth (and everyone else's) without a single extra RPC.
    let all = bus.list_all().await.unwrap_or_default();
    let total = all.len();
    let acked = all.iter().filter(|m| m.acked_at.is_some()).count();
    let mut backlog: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    for m in &all {
        let e = backlog
            .entry(m.receiver.clone())
            .or_insert(json!({"queued": 0, "unread": 0}));
        if m.status == crate::message::MessageStatus::Queued {
            e["queued"] = json!(e["queued"].as_u64().unwrap_or(0) + 1);
        }
        if m.status == crate::message::MessageStatus::Delivered {
            e["unread"] = json!(e["unread"].as_u64().unwrap_or(0) + 1);
        }
    }
    let hello = json!({
        "event": "hello",
        "version": env!("CARGO_PKG_VERSION"),
        "stats": {"total_letters": total, "acked": acked},
        "mailbox_backlog": backlog,
        "heartbeat_secs": 60,
    });
    if sink
        .send(axum::extract::ws::Message::text(hello.to_string()))
        .await
        .is_err()
    {
        return;
    }

    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(60));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // lifecycle event from the bus
            maybe = events.next() => {
                match maybe {
                    Some(Ok(ev)) => {
                        if sink.send(axum::extract::ws::Message::text(ev.to_json().to_string())).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(_lagged)) => {
                        let note = json!({"event": "lagged", "dropped": "some events (slow client)"});
                        if sink.send(axum::extract::ws::Message::text(note.to_string())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            // heartbeat: silence is ambiguous, stats are not
            _ = heartbeat.tick() => {
                let all = bus.list_all().await.unwrap_or_default();
                let total = all.len();
                let acked = all.iter().filter(|m| m.acked_at.is_some()).count();
                let mut backlog: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
                for m in &all {
                    let e = backlog.entry(m.receiver.clone()).or_insert(json!({"queued": 0, "unread": 0}));
                    if m.status == crate::message::MessageStatus::Queued {
                        e["queued"] = json!(e["queued"].as_u64().unwrap_or(0) + 1);
                    }
                    if m.status == crate::message::MessageStatus::Delivered {
                        e["unread"] = json!(e["unread"].as_u64().unwrap_or(0) + 1);
                    }
                }
                let beat = json!({"event": "heartbeat", "stats": {"total_letters": total, "acked": acked}, "mailbox_backlog": backlog});
                if sink.send(axum::extract::ws::Message::text(beat.to_string())).await.is_err() {
                    break;
                }
            }
            // client said goodbye (or sent anything — we don't require input)
            res = rx_client.next() => {
                if res.is_none() {
                    break;
                }
            }
        }
    }
}

/// Mount the /ws route onto the main router.
pub fn router() -> Router<WsState> {
    Router::new().route("/ws", get(ws_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hub_fans_out_to_two_observers() {
        let hub = Arc::new(EventHub::new());
        let mut rx1 = hub.tx.subscribe();
        let mut rx2 = hub.tx.subscribe();
        let msg = Message::new(
            "patricia",
            "diana",
            crate::message::MsgType::Task,
            "t",
            "b",
            None,
        );
        hub.emit("queued", &msg);
        assert_eq!(rx1.recv().await.unwrap().event, "queued");
        assert_eq!(rx2.recv().await.unwrap().event, "queued");
    }

    #[tokio::test]
    async fn event_json_carries_lifecycle_fields() {
        let hub = EventHub::new();
        let mut rx = hub.tx.subscribe();
        let msg = Message::new(
            "patricia",
            "diana",
            crate::message::MsgType::Task,
            "run X",
            "b",
            None,
        );
        hub.emit("delivered", &msg);
        let ev = rx.recv().await.unwrap();
        let j = ev.to_json();
        assert_eq!(j["event"], "delivered");
        assert_eq!(j["receiver"], "diana");
        assert!(j["at"].is_string());
    }
}
