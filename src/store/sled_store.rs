//! Sled-backed store — persistence for the observer narrative.
//!
//! A bus that forgets its letters on restart cannot claim to be a chatlog.
//! Sled keeps every mailbox on disk; the bus survives `docker restart`.
//!
//! Layout: one sled tree, key = `"{receiver}::{created_at_millis}::{id}"`,
//! value = the serialized Message. FIFO order falls out of the key sort
//! within a receiver prefix; `list_all` scans everything.

use super::{fifo, validate_ack, BusStore};
use crate::error::{BusError, BusResult};
use crate::message::{Message, MessageStatus};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

pub struct SledStore {
    db: sled::Db,
    capacity: usize,
}

impl SledStore {
    /// Open (or create) the store at `path`.
    pub fn open(path: impl Into<PathBuf>) -> sled::Result<Self> {
        let db = sled::Config::new().path(path.into()).open()?;
        Ok(Self { db, capacity: 1024 })
    }

    fn key(receiver: &str, msg: &Message) -> Vec<u8> {
        format!(
            "{receiver}::{:020}::{}",
            msg.created_at.timestamp_millis(),
            msg.id
        )
        .into_bytes()
    }

    /// Find and update one message inside a receiver's prefix. Returns the
    /// updated clone. `update` gets a mutable message; error aborts.
    async fn update_one<F>(&self, agent: &str, id: &str, update: F) -> BusResult<Message>
    where
        F: FnOnce(&mut Message) -> BusResult<()>,
    {
        let prefix = format!("{agent}::");
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (_k, v) = item.map_err(sled_err)?;
            let mut m: Message = serde_json::from_slice(&v).map_err(serde_err)?;
            if m.id == id {
                update(&mut m)?;
                let key = Self::key(&m.receiver, &m);
                self.db
                    .insert(key, serde_json::to_vec(&m).map_err(serde_err)?)
                    .map_err(sled_err)?;
                return Ok(m);
            }
        }
        Err(BusError::NotFound(id.to_string()))
    }

    /// Collect messages matching a predicate, FIFO ordered.
    async fn collect<F>(&self, pred: F) -> BusResult<Vec<Message>>
    where
        F: Fn(&Message) -> bool,
    {
        let mut out = Vec::new();
        for item in self.db.iter() {
            let (_k, v) = item.map_err(sled_err)?;
            if let Ok(m) = serde_json::from_slice::<Message>(&v) {
                if pred(&m) {
                    out.push(m);
                }
            }
        }
        fifo(&mut out);
        Ok(out)
    }
}

fn sled_err(e: sled::Error) -> BusError {
    BusError::Storage(format!("sled: {e}"))
}

fn serde_err(e: serde_json::Error) -> BusError {
    BusError::Storage(format!("serde: {e}"))
}

#[async_trait]
impl BusStore for SledStore {
    async fn push(&self, msg: &Message) -> BusResult<()> {
        let prefix = format!("{}::", msg.receiver);
        let count = self.db.scan_prefix(prefix.as_bytes()).count();
        if count >= self.capacity {
            return Err(BusError::MailboxFull(msg.receiver.clone(), self.capacity));
        }
        self.db
            .insert(
                Self::key(&msg.receiver, msg),
                serde_json::to_vec(msg).map_err(serde_err)?,
            )
            .map_err(sled_err)?;
        Ok(())
    }

    async fn poll(&self, agent: &str, limit: usize) -> BusResult<Vec<Message>> {
        let now = chrono::Utc::now();
        let prefix = format!("{agent}::");
        let mut out = Vec::new();
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (k, v) = item.map_err(sled_err)?;
            let mut m: Message = serde_json::from_slice(&v).map_err(serde_err)?;
            if m.status == MessageStatus::Queued {
                m.status = MessageStatus::Delivered;
                m.delivered_at = Some(now);
                self.db
                    .insert(&k, serde_json::to_vec(&m).map_err(serde_err)?)
                    .map_err(sled_err)?;
                out.push(m);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn mark_delivered(&self, agent: &str, id: &str) -> BusResult<Message> {
        let now = chrono::Utc::now();
        self.update_one(agent, id, |m| {
            if m.status == MessageStatus::Queued {
                m.status = MessageStatus::Delivered;
                m.delivered_at = Some(now);
            }
            Ok(())
        })
        .await
    }

    async fn mark_read(&self, agent: &str, id: &str) -> BusResult<Message> {
        self.update_one(agent, id, |m| {
            if m.status != MessageStatus::Delivered {
                return Err(BusError::NotDeliverable(
                    m.id.clone(),
                    format!("{:?}", m.status),
                ));
            }
            m.status = MessageStatus::Read;
            m.read_at = Some(chrono::Utc::now());
            Ok(())
        })
        .await
    }

    async fn peek(&self, agent: &str) -> BusResult<Vec<Message>> {
        self.collect(|m| m.receiver == agent && m.status == MessageStatus::Queued)
            .await
    }

    async fn ack(&self, agent: &str, id: &str, note: &str) -> BusResult<Message> {
        self.update_one(agent, id, |m| match m.status {
            MessageStatus::Acked => Ok(()), // idempotent re-ack
            _ => {
                validate_ack(m)?;
                m.status = MessageStatus::Acked;
                m.acked_at = Some(chrono::Utc::now());
                m.ack_note = Some(note.chars().take(80).collect());
                Ok(())
            }
        })
        .await
    }

    async fn sent_by(&self, agent: &str) -> BusResult<Vec<Message>> {
        self.collect(|m| m.sender == agent).await
    }

    async fn archive(&self) -> BusResult<Vec<Message>> {
        self.collect(|m| m.status == MessageStatus::Acked).await
    }

    async fn delete_one(&self, id: &str) -> BusResult<Message> {
        // Ids are unique bus-wide, but the key embeds the receiver — scan
        // everything for the matching id and remove it.
        let mut found: Option<(Vec<u8>, Message)> = None;
        for item in self.db.iter() {
            let (k, v) = item.map_err(sled_err)?;
            if let Ok(m) = serde_json::from_slice::<Message>(&v) {
                if m.id == id {
                    found = Some((k.to_vec(), m));
                    break;
                }
            }
        }
        let (key, msg) = found.ok_or_else(|| BusError::NotFound(id.to_string()))?;
        self.db.remove(&key).map_err(sled_err)?;
        Ok(msg)
    }

    async fn delete_all(&self) -> BusResult<u64> {
        let mut removed: u64 = 0;
        let mut keys: Vec<Vec<u8>> = Vec::new();
        for item in self.db.iter() {
            let (k, v) = item.map_err(sled_err)?;
            // skip agent markers — they are registry state, not letters
            if k.starts_with(b"__agent__::") {
                continue;
            }
            if serde_json::from_slice::<Message>(&v).is_ok() {
                keys.push(k.to_vec());
            }
        }
        for k in &keys {
            self.db.remove(k).map_err(sled_err)?;
            removed += 1;
        }
        Ok(removed)
    }

    async fn list_all(&self) -> BusResult<Vec<Message>> {
        self.collect(|_| true).await
    }

    async fn register(&self, agent: &str) -> BusResult<()> {
        // mailboxes come into existence with the first letter; registration
        // is tracked by the bus's role table. Nothing to persist here yet —
        // but we write a marker so `agents()` can list known agents.
        self.db
            .insert(format!("__agent__::{agent}").into_bytes(), b"" as &[u8])
            .map_err(sled_err)?;
        Ok(())
    }

    async fn agents(&self) -> BusResult<Vec<String>> {
        let mut out = Vec::new();
        for item in self.db.scan_prefix(b"__agent__::") {
            let (k, _) = item.map_err(sled_err)?;
            if let Ok(s) = std::str::from_utf8(&k) {
                if let Some(name) = s.strip_prefix("__agent__::") {
                    out.push(name.to_string());
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn mailbox_capacity(&self) -> usize {
        self.capacity
    }
}

/// Convenience for tests + single-node deploys.
pub fn temp_store() -> Arc<SledStore> {
    let dir = std::env::temp_dir().join(format!("hot-potato-test-{}", uuid::Uuid::new_v4()));
    Arc::new(SledStore::open(dir).expect("sled temp store"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MsgType;

    async fn store() -> Arc<SledStore> {
        let s = temp_store();
        s.register("patricia").await.unwrap();
        s.register("diana").await.unwrap();
        s
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("hot-potato-persist-{}", uuid::Uuid::new_v4()));
        {
            let s = SledStore::open(&dir).unwrap();
            s.register("diana").await.unwrap();
            s.push(&Message::new(
                "patricia",
                "diana",
                MsgType::Task,
                "survive",
                "b",
                None,
            ))
            .await
            .unwrap();
        }
        // reopen from disk — the letter is still there
        let s = SledStore::open(&dir).unwrap();
        let all = s.list_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject, "survive");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn full_lifecycle_on_sled() {
        let s = store().await;
        let m = Message::new("patricia", "diana", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();

        assert_eq!(s.peek("diana").await.unwrap().len(), 1);
        let drained = s.poll("diana", 10).await.unwrap();
        assert_eq!(drained[0].status, MessageStatus::Delivered);
        assert!(s.peek("diana").await.unwrap().is_empty());

        let acked = s.ack("diana", &m.id, "done").await.unwrap();
        assert_eq!(acked.status, MessageStatus::Acked);
        assert_eq!(s.archive().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ack_before_poll_rejected_on_sled() {
        let s = store().await;
        let m = Message::new("patricia", "diana", MsgType::Task, "T", "b", None);
        s.push(&m).await.unwrap();
        let err = s.ack("diana", &m.id, "cheat").await.unwrap_err();
        assert!(matches!(err, BusError::NotDeliverable(_, _)));
    }
}
