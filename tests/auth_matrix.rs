//! #11 Auth matrix: every RPC and surface — anonymous 401 / token 200.
//!
//! Complements `bearer_token_rejects_when_set` (one RPC, one rejection) with a
//! full sweep across every RPC method + the unauth-by-design surfaces, so a
//! new RPC added without auth consideration fails here immediately.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use hot_potato::bus::EventBus;
use hot_potato::server::{router, ServerConfig};
use hot_potato::store::memory::InMemoryStore;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "test-bearer-0123456789";

fn sessions() -> Arc<tokio::sync::RwLock<std::collections::HashSet<String>>> {
    Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()))
}

fn cfg() -> Arc<ServerConfig> {
    Arc::new(ServerConfig {
        name: "auth-matrix".into(),
        description: "d".into(),
        public_url: "u".into(),
        version: "0".into(),
        bearer_token: Some(TOKEN.into()),
        dashboard_user: "admin".into(),
        dashboard_password: "88888888".into(),
        peers: hot_potato::federation::Peers::default(),
        pool_name: "auth-matrix".into(),
    })
}

async fn seeded_bus() -> Arc<EventBus> {
    let bus = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));
    bus.register("alice", hot_potato::bus::Role::Pm).await.unwrap();
    bus.register("bob", hot_potato::bus::Role::Worker).await.unwrap();
    bus
}

fn app(bus: Arc<EventBus>) -> Router {
    router(bus, cfg(), sessions())
}

async fn status(app: &Router, method: &str, params: Value, auth: Option<&str>) -> StatusCode {
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    let mut req = Request::post("/").header("content-type", "application/json");
    if let Some(t) = auth {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap())
        .await
        .unwrap();
    res.status()
}

/// Every agent-facing RPC must reject anonymous callers with 401.
#[tokio::test]
async fn all_agent_rpcs_reject_anonymous() {
    let bus = seeded_bus().await;
    let a = app(bus);

    let rpcs: Vec<(&str, Value)> = vec![
        ("message/send", json!({"sender":"alice","receiver":"bob","type":"task","subject":"s","body":"b"})),
        ("message/list", json!({})),
        ("message/poll", json!({"agent":"bob"})),
        ("agent/register", json!({"agent":"dave","role":"worker","deliver_via":"a2a","url":"http://localhost:1/"})),
        ("agent/list", json!({})),
        ("bus/archive", json!({})),
    ];

    for (method, params) in rpcs {
        let code = status(&a, method, params, None).await;
        assert_eq!(
            code,
            StatusCode::UNAUTHORIZED,
            "{method} accepted an anonymous caller — auth gap"
        );
    }
}

/// Every read RPC must accept the correct bearer token (writes tested by
/// roundtrip tests elsewhere; here we prove the gate admits, not just blocks).
#[tokio::test]
async fn all_read_rpcs_accept_bearer() {
    let bus = seeded_bus().await;
    let a = app(bus);

    let read_rpcs: Vec<(&str, Value)> = vec![
        ("message/list", json!({})),
        ("agent/list", json!({})),
        ("bus/archive", json!({})),
    ];
    for (method, params) in read_rpcs {
        let code = status(&a, method, params, Some(TOKEN)).await;
        assert_eq!(
            code,
            StatusCode::OK,
            "{method} rejected a valid bearer token"
        );
    }
}

/// Unauthenticated-by-design surfaces stay open; everything else is gated.
#[tokio::test]
async fn public_surfaces_stay_open_but_private_require_auth() {
    let bus = seeded_bus().await;
    let a = app(bus);

    // health + version + agent-card are public by design (dashboards, agent
    // discovery). They must NOT 401.
    for path in ["/health", "/version", "/.well-known/agent-card.json"] {
        let res = a
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path} should be public");
    }

    // /log leaks letter contents — it must be gated when a token is set.
    let res = a
        .oneshot(Request::get("/log").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "/log accepted an anonymous caller — information leak"
    );
}
