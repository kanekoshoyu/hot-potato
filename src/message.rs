//! Message model — the hot potato itself.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Message priority-free by design: a mailbox, not a queue with SLAs.
/// If you need priority, send a more urgent subject line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    /// A task assignment.
    Task,
    /// A reply threading on a previous message (`ref` must be set).
    Reply,
    /// Broadcast to all agents. Permission-gated at the API layer.
    Broadcast,
    /// Pure lifecycle notification (no payload beyond the note).
    AckOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// Written to the recipient's mailbox, recipient unaware.
    Queued,
    /// Recipient polled it — hot potato is now in their hands.
    Delivered,
    /// Recipient opened/read the payload (chatlog-style read receipt).
    Read,
    /// Recipient finished their part and filed the result.
    Acked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub receiver: String,
    #[serde(rename = "type")]
    pub msg_type: MsgType,
    /// One-line summary — a scanning receiver decides urgency from this.
    pub subject: String,
    /// Full payload. Keep files as paths, not contents (8KB soft cap upstream).
    pub body: String,
    /// Thread parent for replies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    pub status: MessageStatus,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<DateTime<Utc>>,
    /// Chatlog-style read receipt: when the recipient actually opened it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acked_at: Option<DateTime<Utc>>,
    /// <= 80 chars completion summary written on ack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_note: Option<String>,
    /// Federation (RFC-004): how many inter-bus links this letter has crossed.
    #[serde(default)]
    pub hops: u8,
    /// Federation (RFC-004): pool that handed this letter over, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded_from: Option<String>,
}

impl Message {
    /// Walk the ref-chain from `id` to its thread root using the full letter
    /// list. Robust against cycles and dangling refs (caps at 32 hops). The
    /// root id is the stable per-thread key used for A2A `contextId`.
    pub fn thread_root_id(all: &[Message], id: &str) -> String {
        let by_id: std::collections::HashMap<&str, &Message> =
            all.iter().map(|m| (m.id.as_str(), m)).collect();
        let mut cur = id.to_string();
        for _ in 0..32 {
            match by_id.get(cur.as_str()).and_then(|m| m.r#ref.as_deref()) {
                Some(next) if by_id.contains_key(next) && next != cur => cur = next.to_string(),
                _ => break,
            }
        }
        cur
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sender: impl Into<String>,
        receiver: impl Into<String>,
        msg_type: MsgType,
        subject: impl Into<String>,
        body: impl Into<String>,
        r#ref: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().simple().to_string(),
            sender: sender.into(),
            receiver: receiver.into(),
            msg_type,
            subject: subject.into(),
            body: body.into(),
            r#ref,
            status: MessageStatus::Queued,
            created_at: Utc::now(),
            delivered_at: None,
            read_at: None,
            acked_at: None,
            ack_note: None,
            hops: 0,
            forwarded_from: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_message_starts_queued() {
        let m = Message::new("alice", "bob", MsgType::Task, "do X", "body", None);
        assert_eq!(m.status, MessageStatus::Queued);
        assert!(m.delivered_at.is_none());
        assert_eq!(m.sender, "alice");
    }

    #[test]
    fn thread_root_walks_full_chain_and_survives_cycles() {
        // a <- b <- c : root is a
        let a = Message::new("p", "d", MsgType::Task, "s", "b", None);
        let mut b = Message::new("d", "p", MsgType::Reply, "s", "b", None);
        b.id = "bbbb".into();
        b.r#ref = Some(a.id.clone());
        let mut c = Message::new("p", "d", MsgType::Reply, "s", "b", None);
        c.id = "cccc".into();
        c.r#ref = Some(b.id.clone());
        let all = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(Message::thread_root_id(&all, "cccc"), a.id);
        assert_eq!(Message::thread_root_id(&all, "bbbb"), a.id);
        assert_eq!(Message::thread_root_id(&all, &a.id), a.id);
        // cycle a<->b must not hang
        let mut x = Message::new("p", "d", MsgType::Task, "s", "b", None);
        x.id = "xxxx".into();
        let mut y = Message::new("d", "p", MsgType::Reply, "s", "b", None);
        y.id = "yyyy".into();
        y.r#ref = Some("xxxx".into());
        x.r#ref = Some("yyyy".into());
        let all2 = vec![x, y];
        let root = Message::thread_root_id(&all2, "yyyy");
        assert!(root == "xxxx" || root == "yyyy");
        // dangling ref: treated as root (self)
        let mut d = Message::new("p", "d", MsgType::Reply, "s", "b", None);
        d.id = "dddd".into();
        d.r#ref = Some("ghost".into());
        assert_eq!(Message::thread_root_id(&[d.clone()], "dddd"), "dddd");
    }

    #[test]
    fn serializes_snake_case() {
        let m = Message::new("a", "b", MsgType::AckOnly, "s", "b", None);
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"ack_only\""));
        assert!(!j.contains("delivered_at")); // skip if none
    }
}
