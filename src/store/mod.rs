//! Storage abstraction — the trait that keeps the bus flexible.
//!
//! Sho's rule: the bus never talks to a concrete database. Anything that can
//! hold mailboxes and survive a bus restart can sit under [`BusStore`]:
//! in-memory (default, good enough), Redis (if ever needed), sled, Postgres…
//!
//! Semantics are mailbox-shaped, not queue-shaped:
//! - one mailbox per agent
//! - `poll` drains *and marks* delivered (hot potato leaves the shelf)
//! - `ack` files the letter into the archive (audit trail = event sourcing)

pub mod memory;
pub mod sled_store;

use crate::error::BusResult;
use crate::message::Message;
use async_trait::async_trait;
use std::collections::BTreeMap;

/// What a storage backend must provide. All async — Redis/network backends
/// need it, and the in-memory default costs nothing for it.
#[async_trait]
pub trait BusStore: Send + Sync {
    /// Append a message. Store assigns nothing — id/status are already set.
    async fn push(&self, msg: &Message) -> BusResult<()>;

    /// Drain up to `limit` queued messages for `agent`, marking them delivered.
    /// Order: FIFO by creation time.
    async fn poll(&self, agent: &str, limit: usize) -> BusResult<Vec<Message>>;

    /// Mark exactly one queued letter delivered — push-on-arrival receipt
    /// (RFC-002): the bus pushed it to the agent's endpoint, so it has
    /// "left the shelf" without a poll.
    async fn mark_delivered(&self, agent: &str, id: &str) -> BusResult<Message>;

    /// Mark one delivered message as read (chatlog receipt: `read_at` set).
    async fn mark_read(&self, agent: &str, id: &str) -> BusResult<Message>;

    /// Peek without marking (inspection, dashboards).
    async fn peek(&self, agent: &str) -> BusResult<Vec<Message>>;

    /// Mark an agent's message acked with a note; returns the updated message.
    async fn ack(&self, agent: &str, id: &str, note: &str) -> BusResult<Message>;

    /// Sender-side status: all messages sent by `agent` with their lifecycle.
    async fn sent_by(&self, agent: &str) -> BusResult<Vec<Message>>;

    /// Everything ever acked — the audit log.
    async fn archive(&self) -> BusResult<Vec<Message>>;

    /// Delete exactly one letter by id (any status). Returns the removed
    /// letter (Sho, 2026-09-13: selective cleanup — v0.3.8).
    async fn delete_one(&self, id: &str) -> BusResult<Message>;

    /// Delete every letter in the bus, any status. Returns how many were
    /// removed (Sho, 2026-09-13: full cleanup — v0.3.8).
    async fn delete_all(&self) -> BusResult<u64>;

    /// Every letter in the bus, any status. Read-only observer view
    /// (`message/list`, `/log`, WebSocket stats). Never mutates state.
    async fn list_all(&self) -> BusResult<Vec<Message>>;

    /// Register an agent (mailbox comes into existence). Idempotent.
    async fn register(&self, agent: &str) -> BusResult<()>;

    /// Known agents.
    async fn agents(&self) -> BusResult<Vec<String>>;

    /// Sanity hook for backends with capacity (in-memory: mailbox cap).
    fn mailbox_capacity(&self) -> usize {
        1024
    }
}

/// Helper: sort messages FIFO (by created_at, then id for stability).
pub fn fifo(messages: &mut [Message]) {
    messages.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
}

/// Helper shared by in-memory and future backends: validate ack target.
pub fn validate_ack(msg: &Message) -> BusResult<()> {
    use crate::error::BusError;
    use crate::message::MessageStatus;
    match msg.status {
        MessageStatus::Delivered | MessageStatus::Read => Ok(()),
        other => Err(BusError::NotDeliverable(
            msg.id.clone(),
            format!("{other:?}"),
        )),
    }
}

/// Convenience: group messages by receiver (for status dashboards).
pub fn group_by_receiver(messages: &[Message]) -> BTreeMap<String, Vec<&Message>> {
    let mut map: BTreeMap<String, Vec<&Message>> = BTreeMap::new();
    for m in messages {
        map.entry(m.receiver.clone()).or_default().push(m);
    }
    map
}
