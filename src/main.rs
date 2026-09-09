//! hot-potato-server — run the bus as an HTTP service.
//!
//! Configure with env vars (all optional):
//!   HOT_POTATO_NAME         agent card name      (default: hot-potato)
//!   HOT_POTATO_DESCRIPTION  agent card blurb     (default: see below)
//!   HOT_POTATO_URL          public URL for card  (default: http://localhost:8080)
//!   HOT_POTATO_TOKEN        bearer token; unset = open (fine for localhost)
//!   HOT_POTATO_ADDR         bind address         (default: 0.0.0.0:8080)

use hot_potato::bus::{EventBus, Role};
use hot_potato::server::{serve, ServerConfig};
use hot_potato::store::memory::InMemoryStore;
use hot_potato::store::sled_store::SledStore;
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Backend choice: HOT_POTATO_DATA_DIR set -> sled persistence (survives
    // restarts); unset -> in-memory (v0.1 default, ephemeral by design).
    let store: Arc<dyn hot_potato::store::BusStore> = match std::env::var("HOT_POTATO_DATA_DIR") {
        Ok(dir) if !dir.is_empty() => {
            eprintln!("🥔 storage: sled at {dir} (restart-safe)");
            Arc::new(SledStore::open(&dir).expect("open sled store"))
        }
        _ => {
            eprintln!(
                "🥔 storage: in-memory (letters lost on restart; set HOT_POTATO_DATA_DIR for sled)"
            );
            Arc::new(InMemoryStore::new())
        }
    };
    let bus = Arc::new(EventBus::new(store));

    // Seed the Daometric default roster (harmless anywhere, useful in compose).
    for (name, role) in [
        ("patricia", Role::Pm),
        ("diana", Role::Worker),
        ("victoria", Role::Worker),
        ("isabella", Role::Worker),
        ("anastasia", Role::Worker),
        ("sho", Role::Pm),
    ] {
        bus.register(name, role).await.expect("seed registration");
    }

    let config = Arc::new(ServerConfig::from_env());
    let addr = std::env::var("HOT_POTATO_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());

    eprintln!("🥔 hot-potato server on http://{addr}");
    eprintln!(
        "   agent card: {}/.well-known/agent-card.json",
        config.public_url
    );
    serve(bus, config, &addr).await
}
