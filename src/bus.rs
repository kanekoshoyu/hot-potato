//! The bus itself — mailbox semantics on top of any [`BusStore`].
//!
//! This is the only type agent-facing code needs. It enforces the rules the
//! Python prototype evolved during the Daometric pilot:
//! - senders must be registered (know your agents)
//! - replies must carry `ref` (threads are first-class, everything else is chat)
//! - broadcasts are permission-gated (only the PM role broadcasts)
//! - `status` gives senders a zero-LLM answer to "did my potato land?"

use crate::error::{BusError, BusResult};
use crate::message::{Message, MsgType};
use crate::store::BusStore;
use std::sync::Arc;

/// Roles for permission checks. More can be added; nothing here is a topic router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Project manager — may broadcast.
    Pm,
    /// Worker agent — sends tasks/replies to named peers.
    Worker,
}

pub struct EventBus {
    store: Arc<dyn BusStore>,
    /// agent -> role
    roles: tokio::sync::RwLock<std::collections::HashMap<String, Role>>,
}

impl EventBus {
    pub fn new(store: Arc<dyn BusStore>) -> Self {
        Self {
            store,
            roles: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Register an agent with a role. Idempotent.
    pub async fn register(&self, agent: &str, role: Role) -> BusResult<()> {
        self.store.register(agent).await?;
        self.roles.write().await.insert(agent.to_string(), role);
        Ok(())
    }

    /// Hot potato: write the letter and let go. Returns the id to track.
    pub async fn send(
        &self,
        sender: &str,
        receiver: &str,
        msg_type: MsgType,
        subject: &str,
        body: &str,
        r#ref: Option<String>,
    ) -> BusResult<String> {
        self.ensure_registered(sender).await?;

        if msg_type == MsgType::Broadcast {
            let role = self.role_of(sender).await;
            if role != Some(Role::Pm) {
                return Err(BusError::Storage(format!(
                    "`{sender}` is not allowed to broadcast (PM role required)"
                )));
            }
        }
        if msg_type == MsgType::Reply && r#ref.is_none() {
            return Err(BusError::Storage(
                "reply messages must carry `ref` (the parent id)".into(),
            ));
        }
        if msg_type == MsgType::Broadcast {
            // receiver is ignored for broadcasts; fan-out handled in `broadcast()`
        } else {
            self.ensure_registered(receiver).await?;
        }

        let msg = Message::new(sender, receiver, msg_type, subject, body, r#ref);
        self.store.push(&msg).await?;
        Ok(msg.id)
    }

    /// Broadcast = one letter per registered agent (fan-out at the bus edge,
    /// not inside the store — keeps the store dumb and the semantics clear).
    pub async fn broadcast(
        &self,
        sender: &str,
        subject: &str,
        body: &str,
    ) -> BusResult<Vec<String>> {
        let agents = self.store.agents().await?;
        let mut ids = Vec::with_capacity(agents.len());
        for a in agents {
            if a == sender {
                continue;
            }
            ids.push(
                self.send(sender, &a, MsgType::Broadcast, subject, body, None)
                    .await?,
            );
        }
        Ok(ids)
    }

    /// Drain my mailbox (marks delivered). `limit` 0 = all.
    pub async fn poll(&self, agent: &str, limit: usize) -> BusResult<Vec<Message>> {
        self.ensure_registered(agent).await?;
        let limit = if limit == 0 { usize::MAX } else { limit };
        self.store.poll(agent, limit).await
    }

    /// Read receipt: I opened it (chatlog `read_at` timestamp).
    pub async fn mark_read(&self, agent: &str, id: &str) -> BusResult<Message> {
        self.ensure_registered(agent).await?;
        self.store.mark_read(agent, id).await
    }

    /// Peek without marking — "what's waiting if I pick it up?"
    pub async fn peek(&self, agent: &str) -> BusResult<Vec<Message>> {
        self.ensure_registered(agent).await?;
        self.store.peek(agent).await
    }

    /// File the result: I'm done with my part of this potato.
    pub async fn ack(&self, agent: &str, id: &str, note: &str) -> BusResult<Message> {
        self.ensure_registered(agent).await?;
        self.store.ack(agent, id, note).await
    }

    /// Sender-side: status of everything I sent (the "did they get it?" view).
    pub async fn status(&self, agent: &str) -> BusResult<Vec<Message>> {
        self.ensure_registered(agent).await?;
        self.store.sent_by(agent).await
    }

    /// The audit log — every acked letter, FIFO.
    pub async fn archive(&self) -> BusResult<Vec<Message>> {
        self.store.archive().await
    }

    /// Observer view: every letter on the bus, any status. Read-only.
    pub async fn list_all(&self) -> BusResult<Vec<Message>> {
        self.store.list_all().await
    }

    async fn ensure_registered(&self, agent: &str) -> BusResult<()> {
        let roles = self.roles.read().await;
        if roles.contains_key(agent) {
            Ok(())
        } else {
            Err(BusError::UnknownAgent(agent.to_string()))
        }
    }

    async fn role_of(&self, agent: &str) -> Option<Role> {
        self.roles.read().await.get(agent).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::InMemoryStore;

    async fn bus() -> EventBus {
        let store = Arc::new(InMemoryStore::new());
        let bus = EventBus::new(store);
        bus.register("patricia", Role::Pm).await.unwrap();
        bus.register("diana", Role::Worker).await.unwrap();
        bus.register("victoria", Role::Worker).await.unwrap();
        bus
    }

    #[tokio::test]
    async fn send_poll_reply_ack_roundtrip() {
        let b = bus().await;
        let id = b
            .send("patricia", "diana", MsgType::Task, "run X", "body", None)
            .await
            .unwrap();

        let mail = b.poll("diana", 0).await.unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].id, id);

        let reply_id = b
            .send(
                "diana",
                "patricia",
                MsgType::Reply,
                "re: run X",
                "done, see csv",
                Some(id.clone()),
            )
            .await
            .unwrap();
        assert!(!reply_id.is_empty());

        let acked = b.ack("diana", &id, "delivered csv").await.unwrap();
        assert_eq!(acked.status, crate::message::MessageStatus::Acked);

        let p_status = b.status("patricia").await.unwrap();
        assert_eq!(p_status.len(), 1); // only the task she sent (reply is diana's)
    }

    #[tokio::test]
    async fn worker_cannot_broadcast() {
        let b = bus().await;
        let err = b
            .send("diana", "*", MsgType::Broadcast, "spam", "body", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not allowed to broadcast"));
    }

    #[tokio::test]
    async fn pm_can_broadcast_fans_out() {
        let b = bus().await;
        let ids = b
            .broadcast("patricia", "standup 9:00", "be there")
            .await
            .unwrap();
        assert_eq!(ids.len(), 2); // diana + victoria, not patricia
        assert_eq!(b.peek("diana").await.unwrap().len(), 1);
        assert_eq!(b.peek("victoria").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reply_without_ref_rejected() {
        let b = bus().await;
        let err = b
            .send("diana", "patricia", MsgType::Reply, "re:", "x", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must carry `ref`"));
    }

    #[tokio::test]
    async fn unregistered_sender_rejected() {
        let b = bus().await;
        let err = b
            .send("mallory", "diana", MsgType::Task, "hi", "b", None)
            .await
            .unwrap_err();
        assert!(matches!(err, BusError::UnknownAgent(_)));
    }

    #[tokio::test]
    async fn chatlog_timestamps_flow() {
        let b = bus().await;
        let id = b
            .send("patricia", "diana", MsgType::Task, "t", "b", None)
            .await
            .unwrap();

        // queued: no read receipt yet
        let s0 = b.status("patricia").await.unwrap();
        assert!(s0[0].read_at.is_none());

        b.poll("diana", 0).await.unwrap();
        let read = b.mark_read("diana", &id).await.unwrap();
        assert_eq!(read.status, crate::message::MessageStatus::Read);
        assert!(read.read_at.is_some());
        assert!(read.delivered_at.is_some());

        // read->ack legal; delivered_at <= read_at
        let acked = b.ack("diana", &id, "done").await.unwrap();
        assert!(acked.delivered_at.unwrap() <= acked.read_at.unwrap());
        assert!(acked.read_at.unwrap() <= acked.acked_at.unwrap());
    }

    #[tokio::test]
    async fn read_before_delivery_rejected() {
        let b = bus().await;
        let id = b
            .send("patricia", "diana", MsgType::Task, "t", "b", None)
            .await
            .unwrap();
        let err = b.mark_read("diana", &id).await.unwrap_err();
        assert!(matches!(err, BusError::NotDeliverable(_, _)));
    }

    #[tokio::test]
    async fn archive_only_counts_acked() {
        let b = bus().await;
        let id = b
            .send("patricia", "diana", MsgType::Task, "t", "b", None)
            .await
            .unwrap();
        assert!(b.archive().await.unwrap().is_empty());
        b.poll("diana", 0).await.unwrap();
        b.ack("diana", &id, "done").await.unwrap();
        assert_eq!(b.archive().await.unwrap().len(), 1);
    }
}
