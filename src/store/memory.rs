//! In-memory store — the default backend. Sho's call: in-memory is enough.
//! Backed by `parking_lot`-free std sync primitives; every method is a short
//! critical section so no `await` points live inside locks (Send-safe futures).

use super::{fifo, validate_ack, BusStore};
use crate::error::{BusError, BusResult};
use crate::message::{Message, MessageStatus};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default)]
struct Inner {
    /// agent -> all mail ever addressed to them (queued/delivered/acked kept
    /// for sender-side `sent_by`; acked ones also land in `archive`).
    mailboxes: HashMap<String, Vec<Message>>,
    agents: Vec<String>,
}

/// Simple, dependency-free in-memory backend.
pub struct InMemoryStore {
    inner: RwLock<Inner>,
    capacity: usize,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            capacity: 1024,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            capacity,
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BusStore for InMemoryStore {
    async fn push(&self, msg: &Message) -> BusResult<()> {
        let mut inner = self.inner.write().expect("store poisoned");
        let box_ = inner
            .mailboxes
            .entry(msg.receiver.clone())
            .or_insert_with(Vec::new);
        if box_.len() >= self.capacity {
            return Err(BusError::MailboxFull(msg.receiver.clone(), self.capacity));
        }
        box_.push(msg.clone());
        Ok(())
    }

    async fn poll(&self, agent: &str, limit: usize) -> BusResult<Vec<Message>> {
        let mut inner = self.inner.write().expect("store poisoned");
        let now = chrono::Utc::now();
        let Some(box_) = inner.mailboxes.get_mut(agent) else {
            return Ok(vec![]);
        };
        fifo(box_);
        let mut out = Vec::new();
        for m in box_
            .iter_mut()
            .filter(|m| m.status == MessageStatus::Queued)
        {
            m.status = MessageStatus::Delivered;
            m.delivered_at = Some(now);
            out.push(m.clone());
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    async fn mark_delivered(&self, agent: &str, id: &str) -> BusResult<Message> {
        let mut inner = self.inner.write().expect("store poisoned");
        let now = chrono::Utc::now();
        let box_ = inner
            .mailboxes
            .get_mut(agent)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        let m = box_
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        // push receipt: queued -> delivered (idempotent if already delivered)
        if m.status == MessageStatus::Queued {
            m.status = MessageStatus::Delivered;
            m.delivered_at = Some(now);
        }
        Ok(m.clone())
    }

    async fn mark_read(&self, agent: &str, id: &str) -> BusResult<Message> {
        let mut inner = self.inner.write().expect("store poisoned");
        let box_ = inner
            .mailboxes
            .get_mut(agent)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        let m = box_
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        // must be delivered (not queued/acked) to be read
        if m.status != MessageStatus::Delivered {
            return Err(BusError::NotDeliverable(
                m.id.clone(),
                format!("{:?}", m.status),
            ));
        }
        m.status = MessageStatus::Read;
        m.read_at = Some(chrono::Utc::now());
        Ok(m.clone())
    }

    async fn peek(&self, agent: &str) -> BusResult<Vec<Message>> {
        let inner = self.inner.read().expect("store poisoned");
        Ok(inner
            .mailboxes
            .get(agent)
            .map(|b| {
                b.iter()
                    .filter(|m| m.status == MessageStatus::Queued)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn ack(&self, agent: &str, id: &str, note: &str) -> BusResult<Message> {
        let mut inner = self.inner.write().expect("store poisoned");
        let box_ = inner
            .mailboxes
            .get_mut(agent)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        let now = chrono::Utc::now();
        let m = box_
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| BusError::NotFound(id.to_string()))?;
        match m.status {
            // idempotent: re-acking an acked message returns current state
            MessageStatus::Acked => Ok(m.clone()),
            _ => {
                validate_ack(m)?;
                // v1.2.1: never skip the read station — acking straight from
                // `delivered` (the push-era norm) auto-stamps read_at so the
                // audit trail keeps its full queued→delivered→read→acked chain.
                if m.read_at.is_none() {
                    m.read_at = Some(now);
                }
                m.status = MessageStatus::Acked;
                m.acked_at = Some(now);
                m.ack_note = Some(note.chars().take(80).collect());
                Ok(m.clone())
            }
        }
    }

    async fn sent_by(&self, agent: &str) -> BusResult<Vec<Message>> {
        let inner = self.inner.read().expect("store poisoned");
        let mut out: Vec<Message> = inner
            .mailboxes
            .values()
            .flatten()
            .filter(|m| m.sender == agent)
            .cloned()
            .collect();
        fifo(&mut out);
        Ok(out)
    }

    async fn archive(&self) -> BusResult<Vec<Message>> {
        let inner = self.inner.read().expect("store poisoned");
        let mut out: Vec<Message> = inner
            .mailboxes
            .values()
            .flatten()
            .filter(|m| m.status == MessageStatus::Acked)
            .cloned()
            .collect();
        fifo(&mut out);
        Ok(out)
    }

    async fn delete_one(&self, id: &str) -> BusResult<Message> {
        let mut inner = self.inner.write().expect("store poisoned");
        for (_, msgs) in inner.mailboxes.iter_mut() {
            if let Some(pos) = msgs.iter().position(|m| m.id == id) {
                return Ok(msgs.remove(pos));
            }
        }
        Err(BusError::NotFound(id.to_string()))
    }

    async fn delete_all(&self) -> BusResult<u64> {
        let mut inner = self.inner.write().expect("store poisoned");
        let mut removed: u64 = 0;
        for (_, msgs) in inner.mailboxes.iter_mut() {
            removed += msgs.len() as u64;
            msgs.clear();
        }
        Ok(removed)
    }

    async fn list_all(&self) -> BusResult<Vec<Message>> {
        let inner = self.inner.read().expect("store poisoned");
        let mut out: Vec<Message> = inner.mailboxes.values().flatten().cloned().collect();
        fifo(&mut out);
        Ok(out)
    }

    async fn register(&self, agent: &str) -> BusResult<()> {
        let mut inner = self.inner.write().expect("store poisoned");
        if !inner.agents.iter().any(|a| a == agent) {
            inner.agents.push(agent.to_string());
        }
        inner.mailboxes.entry(agent.to_string()).or_default();
        Ok(())
    }

    async fn unregister(&self, agent: &str) -> BusResult<()> {
        let mut inner = self.inner.write().expect("store poisoned");
        inner.agents.retain(|a| a != agent);
        // The mailbox itself is intentionally KEPT: letters already on the
        // shelf must stay visible (observer view / audit), exactly like the
        // sled backend, which only drops the __agent__:: marker.
        Ok(())
    }

    async fn agents(&self) -> BusResult<Vec<String>> {
        let inner = self.inner.read().expect("store poisoned");
        Ok(inner.agents.clone())
    }

    fn mailbox_capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MsgType;

    async fn setup() -> InMemoryStore {
        let s = InMemoryStore::new();
        s.register("alice").await.unwrap();
        s.register("bob").await.unwrap();
        s
    }

    #[tokio::test]
    async fn full_lifecycle() {
        let s = setup().await;
        let m = Message::new("alice", "bob", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();

        // peek doesn't mark
        assert_eq!(s.peek("bob").await.unwrap().len(), 1);
        assert_eq!(s.peek("bob").await.unwrap().len(), 1);

        // poll marks delivered
        let drained = s.poll("bob", 10).await.unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].status, MessageStatus::Delivered);
        assert!(s.peek("bob").await.unwrap().is_empty());

        // ack requires delivered
        let acked = s.ack("bob", &m.id, "done").await.unwrap();
        assert_eq!(acked.status, MessageStatus::Acked);

        // audit trail
        assert_eq!(s.archive().await.unwrap().len(), 1);
        assert_eq!(s.sent_by("alice").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ack_before_poll_rejected() {
        let s = setup().await;
        let m = Message::new("alice", "bob", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();
        let err = s.ack("bob", &m.id, "cheat").await.unwrap_err();
        assert!(matches!(err, BusError::NotDeliverable(_, _)));
    }

    /// v1.2.1: acking straight from `delivered` (the production norm — push
    /// flips delivered in seconds, agents ack when done, nobody calls read)
    /// must auto-stamp `read_at` instead of skipping the station. The audit
    /// trail showed 6/185 acked letters with no read_at; read must never be
    /// silently skipped.
    async fn ack_from_delivered_stamps_read_at(s: &dyn BusStore) {
        let m = Message::new("alice", "bob", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();
        s.mark_delivered("bob", &m.id).await.unwrap();
        s.mark_read("bob", &m.id).await.unwrap();
        let out = s.ack("bob", &m.id, "normal path").await.unwrap();
        assert_eq!(out.status, MessageStatus::Acked);
        assert!(out.read_at.is_some(), "explicit read keeps read_at");
    }
    #[tokio::test]
    async fn ack_from_delivered_auto_stamps_read_at_memory() {
        let s = setup().await;
        let m = Message::new("alice", "bob", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();
        s.mark_delivered("bob", &m.id).await.unwrap();
        // RED: today this acks with read_at == None (skipped station)
        let out = s.ack("bob", &m.id, "skipped read").await.unwrap();
        assert!(
            out.read_at.is_some(),
            "ack from delivered must auto-stamp read_at, got {:?}",
            out.read_at
        );
        ack_from_delivered_stamps_read_at(&s).await;
    }

    #[tokio::test]
    async fn fifo_order_preserved() {
        let s = setup().await;
        for i in 0..5 {
            s.push(&Message::new(
                "alice",
                "bob",
                MsgType::Task,
                format!("m{i}"),
                "b",
                None,
            ))
            .await
            .unwrap();
        }
        let drained = s.poll("bob", 10).await.unwrap();
        let subjects: Vec<_> = drained.iter().map(|m| m.subject.clone()).collect();
        assert_eq!(subjects, ["m0", "m1", "m2", "m3", "m4"]);
    }

    #[tokio::test]
    async fn mailbox_full_is_an_error() {
        let s = InMemoryStore::with_capacity(2);
        s.register("bob").await.unwrap();
        for _ in 0..2 {
            s.push(&Message::new("p", "bob", MsgType::Task, "s", "b", None))
                .await
                .unwrap();
        }
        let err = s
            .push(&Message::new("p", "bob", MsgType::Task, "s3", "b", None))
            .await
            .unwrap_err();
        assert!(matches!(err, BusError::MailboxFull(_, 2)));
    }
}
