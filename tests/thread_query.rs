//! v0.3.13 message/thread — trajectory query e2e tests.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use hot_potato::bus::EventBus;
use hot_potato::server::{router, ServerConfig};
use hot_potato::store::memory::InMemoryStore;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn sessions() -> Arc<tokio::sync::RwLock<std::collections::HashSet<String>>> {
    Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()))
}

fn cfg() -> Arc<ServerConfig> {
    Arc::new(ServerConfig {
        name: "t".into(),
        description: "d".into(),
        public_url: "u".into(),
        version: "0".into(),
        bearer_token: None,
        dashboard_user: "admin".into(),
        dashboard_password: "88888888".into(),
        peers: hot_potato::federation::Peers::default(),
        pool_name: "t".into(),
    })
}

async fn rpc_call(app: &Router, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    let res = app
        .clone()
        .oneshot(
            Request::post("/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
}

/// Thread: root A->B, reply B->A (ref root), reply A->B (ref reply1).
/// Plus an unrelated letter that must NOT appear.
async fn thread_bus() -> (Arc<EventBus>, String, String, String) {
    use hot_potato::store::BusStore;
    let store = Arc::new(InMemoryStore::new());
    let bus = Arc::new(EventBus::new(store.clone()));
    for a in ["patricia", "diana", "victoria"] {
        store.register(a).await.unwrap();
    }
    let mut root = hot_potato::message::Message::new(
        "patricia", "diana", hot_potato::message::MsgType::Task, "root", "b", None);
    root.created_at = chrono::Utc::now() - chrono::Duration::seconds(60);
    store.push(&root).await.unwrap();

    let mut r1 = hot_potato::message::Message::new(
        "diana", "patricia", hot_potato::message::MsgType::Reply, "re: root", "b", Some(root.id.clone()));
    r1.created_at = chrono::Utc::now() - chrono::Duration::seconds(30);
    store.push(&r1).await.unwrap();

    let mut r2 = hot_potato::message::Message::new(
        "patricia", "diana", hot_potato::message::MsgType::Reply, "re: re: root", "b", Some(r1.id.clone()));
    r2.created_at = chrono::Utc::now() - chrono::Duration::seconds(10);
    store.push(&r2).await.unwrap();

    let mut stray = hot_potato::message::Message::new(
        "victoria", "patricia", hot_potato::message::MsgType::Task, "unrelated", "b", None);
    stray.created_at = chrono::Utc::now() - chrono::Duration::seconds(5);
    store.push(&stray).await.unwrap();

    (bus, root.id, r1.id, stray.id)
}

#[tokio::test]
async fn thread_query_walks_ref_chain_from_any_member() {
    let (bus, root, r1, _stray) = thread_bus().await;
    let app = router(bus, cfg(), sessions());

    // query from EVERY member — all must return the same full thread
    for anchor in [&root, &r1] {
        let res = rpc_call(&app, "message/thread", json!({"id": anchor})).await;
        let arr = res.as_array().unwrap();
        assert_eq!(arr.len(), 3, "anchor={}", anchor);
        assert_eq!(arr[0]["subject"], "root");
        assert_eq!(arr[1]["subject"], "re: root");
        assert_eq!(arr[2]["subject"], "re: re: root");
        // oldest-first ordering
        let t0 = arr[0]["created_at"].as_str().unwrap();
        let t1 = arr[1]["created_at"].as_str().unwrap();
        assert!(t0 < t1);
    }
}

#[tokio::test]
async fn thread_query_excludes_unrelated_and_errors_on_missing() {
    let (bus, root, _r1, stray) = thread_bus().await;
    let app = router(bus, cfg(), sessions());

    let res = rpc_call(&app, "message/thread", json!({"id": root})).await;
    let ids: Vec<&str> = res.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(!ids.contains(&stray.as_str()), "unrelated letter must not leak into thread");

    // missing id -> JSON-RPC error body (still 200 at HTTP layer)
    let body = json!({"jsonrpc":"2.0","id":1,"method":"message/thread","params":{"id":"does-not-exist"}});
    let res = app
        .oneshot(
            Request::post("/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(v["error"]["message"].as_str().unwrap().contains("not found"));
}
