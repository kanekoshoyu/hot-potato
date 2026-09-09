//! Hot Potato — EventMessageBus for AI agents.
//!
//! A mailbox with a state machine. Agents pass messages like a hot potato:
//! send it on, work your part, pass the result back. Every agent minds its
//! own work; the bus guarantees delivery and tracks the lifecycle
//! `queued -> delivered -> acked`.
//!
//! Design principles (Sho's rules):
//! 1. Async/await everywhere — the domain is event-based.
//! 2. Storage behind a [`BusStore`] trait — in-memory by default, Redis etc. pluggable.
//! 3. No broker daemon, no topic routing — plain mailbox semantics.
//! 4. Wire-compatible with A2A registration (agent card) — connecting
//!    registration and inference endpoints is the transport layer's job,
//!    not the bus's.

pub mod bus;
pub mod deliver;
pub mod error;
pub mod message;
pub mod openapi;
pub mod server;
pub mod store;
pub mod ws;

pub use bus::EventBus;
pub use error::BusError;
pub use message::{Message, MessageStatus, MsgType};
pub use store::memory::InMemoryStore;
pub use store::BusStore;
