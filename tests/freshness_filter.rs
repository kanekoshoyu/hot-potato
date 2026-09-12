//! API-level freshness-filter tests: `message/list` `max_age_secs` across a
//! matrix of window configurations, against letters of known ages.
//! Motivated by Sho's report (2-min window showing 5-min-old potatoes).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use hot_potato::bus::EventBus;
use hot_potato::deliver::Registry;
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

/// Letters of known ages: 10s, 70s, 5min, 15min, 50min, 3h — pushed straight
/// into the store with backdated created_at (no bus.send, which stamps now).
async fn bus_with_aged_letters() -> Arc<EventBus> {
    use hot_potato::store::BusStore;
    let store = Arc::new(InMemoryStore::new());
    let bus = Arc::new(EventBus::new(store.clone()));
    store.register("patricia").await.unwrap();
    store.register("anastasia").await.unwrap();
    let ages = [10i64, 70, 300, 900, 3000, 10800];
    for age in ages {
        let mut m = hot_potato::message::Message::new(
            "patricia", "anastasia", hot_potato::message::MsgType::AckOnly,
            format!("age-{}", age), "x", None);
        m.created_at = chrono::Utc::now() - chrono::Duration::seconds(age);
        store.push(&m).await.unwrap();
    }
    bus
}

#[tokio::test]
async fn freshness_window_matrix() {
    let bus = bus_with_aged_letters().await;
    let app = router(bus, cfg(), sessions());

    // (window_secs, expected subject count inside window)
    // NOTE: max_age_secs=0 matches only letters created within the same second
    // (age <= 0) — effectively "nothing". The dashboard's "all history" option
    // OMITS the param instead. 0 is tested to pin that exact API semantics.
    let cases: Vec<(Option<i64>, usize)> = vec![
        (None, 6),          // no filter -> all 6
        (Some(0), 0),       // 0 = age<=0 -> nothing (documented quirk)
        (Some(30), 1),      // 30s window -> only the 10s letter
        (Some(60), 1),      // 1 min -> 10s
        (Some(120), 2),     // 2 min -> 10s + 70s   (Sho's case: 5-min MUST NOT show)
        (Some(3600), 5),    // 1 h -> all but the 3h letter (15-min letter MUST show)
        (Some(7200), 5),    // 2 h -> same 5
        (Some(86400), 6),   // 24 h -> all 6
    ];

    for (window, expect) in cases {
        let mut params = json!({});
        if let Some(w) = window {
            params["max_age_secs"] = json!(w);
        }
        let res = rpc_call(&app, "message/list", params).await;
        let got = res.as_array().unwrap().len();
        assert_eq!(got, expect, "window {:?}: expected {} letters, got {}", window, expect, got);
    }

    // spot-check Sho's exact scenario: 1h window must INCLUDE the 15-min letter
    let res = rpc_call(&app, "message/list", json!({"max_age_secs": 3600})).await;
    let subjects: Vec<String> = res
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["subject"].as_str().unwrap().to_string())
        .collect();
    assert!(subjects.contains(&"age-900".to_string()), "15-min letter must be visible in 1h window");
    assert!(!subjects.contains(&"age-10800".to_string()), "3h letter must NOT be visible in 1h window");

    // combined with status filter: queued + 2min window
    let res = rpc_call(&app, "message/list", json!({"max_age_secs": 120, "status": "queued"})).await;
    assert_eq!(res.as_array().unwrap().len(), 2);
}
