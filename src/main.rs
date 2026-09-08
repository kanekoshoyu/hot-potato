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
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bus = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));

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
    eprintln!("   agent card: {}/.well-known/agent-card.json", config.public_url);
    serve(bus, config, &addr).await
}
