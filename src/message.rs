//! Message model — the hot potato itself.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Message priority-free by design: a mailbox, not a queue with SLAs.
/// If you need priority, send a more urgent subject line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    /// A task assignment (Patricia -> Diana).
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acked_at: Option<DateTime<Utc>>,
    /// <= 80 chars completion summary written on ack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_note: Option<String>,
}

impl Message {
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
            acked_at: None,
            ack_note: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_message_starts_queued() {
        let m = Message::new("patricia", "diana", MsgType::Task, "do X", "body", None);
        assert_eq!(m.status, MessageStatus::Queued);
        assert!(m.delivered_at.is_none());
        assert_eq!(m.sender, "patricia");
    }

    #[test]
    fn serializes_snake_case() {
        let m = Message::new("a", "b", MsgType::AckOnly, "s", "b", None);
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"ack_only\""));
        assert!(!j.contains("delivered_at")); // skip if none
    }
}
