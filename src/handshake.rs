//! RFC-006: Dashboard-driven federation — peers connect through the panel,
//! three-way handshake, zero config files.
//!
//! Sho's rule (2026-09-12): peer-to-peer must not require editing env vars.
//! Two bus operators connect by exchanging a one-time invite code through the
//! dashboards:
//!
//!   1. A 的 dashboard 点 "Invite a peer" → 生成邀请码（含 A 的 URL + 一次性
//!      handshake token，10 分钟有效，单次使用）
//!   2. 把邀请码粘到 B 的 dashboard "Join a peer" → B 立即回拨 A 完成三方握手：
//!      B→A: peer/join(code)  →  A 验证并记住 B 的 URL
//!      A→B: 响应里带回 A 的注册信息 → B 记住 A
//!      A→B: 主动发一封欢迎信走完整转发路径（链路自证）
//!   3. 两边 registry 里自动出现对方 pool 的影子条目，dashboard 上立即
//!      可见对岸 agent（带 pool tag 着色）
//!
//! Trust model: handshake token 是 bearer secret——谁拿到邀请码谁就能把自己
//! 注册为 peer（10 分钟窗口 + 单次使用）。后续可升级互配 token（已有
//! HOT_POTATO_TOKEN 机制不变）。Invite/join 走普通 RPC 端点，受全局 bearer
//! gate 保护（若配置了）。
//!
//! Peers learned this way live in the same in-memory+JSON-persisted store as
//! env-configured peers; env wins on conflict (ops override panel).

use crate::federation::Peer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// One pending invite: single-use, 10-minute TTL.
#[derive(Debug, Clone, Serialize)]
pub struct Invite {
    pub code: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing)]
    pub used: bool,
}

impl Invite {
    pub fn new(inviter_url: &str) -> Self {
        let now = chrono::Utc::now();
        let payload = serde_json::json!({ "url": inviter_url }).to_string();
        let code = format!(
            "hp-{}-{}",
            Uuid::new_v4().simple(),
            b64url_encode(&payload)
        );
        Self {
            code,
            created_at: now,
            expires_at: now + chrono::Duration::minutes(10),
            used: false,
        }
    }

    pub fn valid(&self) -> bool {
        !self.used && chrono::Utc::now() < self.expires_at
    }
}

/// Peers learned through the handshake (persisted alongside env peers).
#[derive(Debug, Default)]
pub struct DynamicPeers {
    inner: RwLock<Vec<Peer>>,
    path: std::sync::Mutex<Option<std::path::PathBuf>>,
}

impl DynamicPeers {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(Vec::new()),
            path: std::sync::Mutex::new(None),
        })
    }

    pub fn with_persistence(self: Arc<Self>, path: std::path::PathBuf) -> Arc<Self> {
        if let Ok(raw) = std::fs::read(&path) {
            if let Ok(peers) = serde_json::from_str::<Vec<Peer>>(&String::from_utf8_lossy(&raw)) {
                if let Ok(mut w) = self.inner.try_write() {
                    *w = peers;
                }
            }
        }
        *self.path.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);
        self
    }

    async fn persist(&self, peers: &[Peer]) {
        let guard = self.path.lock().unwrap_or_else(|p| p.into_inner());
        let Some(path) = guard.as_ref() else { return };
        if let Ok(json) = serde_json::to_vec_pretty(peers) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    pub async fn add(&self, peer: Peer) {
        let mut w = self.inner.write().await;
        w.retain(|p| p.name != peer.name);
        w.push(peer);
        self.persist(&w).await;
    }

    /// All dynamic peers (for routing: env peers take precedence on name clash).
    pub async fn all(&self) -> Vec<Peer> {
        self.inner.read().await.clone()
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn contains_agent(&self, agent: &str) -> Option<Peer> {
        self.inner
            .read()
            .await
            .iter()
            .find(|p| p.contains(agent))
            .cloned()
    }
}

/// Generate an invite for the dashboard button.
pub async fn create_invite(store: &RwLock<Vec<Invite>>, inviter_url: &str) -> Invite {
    let inv = Invite::new(inviter_url);
    // opportunistic cleanup of expired ones
    let mut w = store.write().await;
    w.retain(|i| i.valid());
    w.push(inv.clone());
    inv
}

/// Consume an invite code; returns true when valid+unused.
pub async fn consume_invite(store: &RwLock<Vec<Invite>>, code: &str) -> bool {
    let mut w = store.write().await;
    if let Some(inv) = w.iter_mut().find(|i| i.code == code && i.valid()) {
        inv.used = true;
        return true;
    }
    false
}

/// URL-safe base64 (no padding) decode — used by invite codes to carry the
/// inviter's URL so the joiner only ever needs the code itself.
pub fn b64url_decode(input: &str) -> Option<String> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut buf = Vec::new();
    let bytes: Vec<u8> = input.bytes().collect();
    let mut chunk = [0u8; 4];
    for group in bytes.chunks(4) {
        for (i, b) in chunk.iter_mut().enumerate() {
            *b = match group.get(i) {
                Some(&c) if c == b'=' => 0,
                Some(&c) => TABLE.iter().position(|&t| t == c)? as u8,
                None => 0,
            };
        }
        buf.push((chunk[0] << 2) | (chunk[1] >> 4));
        if group.len() > 2 {
            buf.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        if group.len() > 3 {
            buf.push((chunk[2] << 6) | chunk[3]);
        }
    }
    String::from_utf8(buf).ok()
}

/// URL-safe base64 (no padding) encode.
pub fn b64url_encode(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = input.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        out.push(TABLE[(b[0] >> 2) as usize] as char);
        out.push(TABLE[((b[0] & 0x03) << 4 | b[1] >> 4) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((b[1] & 0x0f) << 2 | b[2] >> 6) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b[2] & 0x3f) as usize] as char);
        }
    }
    out
}

/// B→A join payload.

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invite_single_use_and_ttl() {
        let store = RwLock::new(Vec::new());
        let inv = create_invite(&store, "http://inviter.example").await;
        assert!(inv.valid());
        assert!(consume_invite(&store, &inv.code).await);
        assert!(!consume_invite(&store, &inv.code).await); // second use rejected
        assert!(!consume_invite(&store, "hp-bogus").await);
    }

    #[tokio::test]
    async fn dynamic_peers_persist_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hp-peers-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("peers.json");
        {
            let dp = DynamicPeers::new().with_persistence(path.clone());
            dp.add(Peer {
                name: "fleet".into(),
                url: "http://x:8082/".into(),
                token: String::new(),
                agents: vec!["patricia".into()],
            })
            .await;
        }
        // rehydrate
        let dp2 = DynamicPeers::new().with_persistence(path);
        assert!(dp2.contains_agent("patricia").await.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
