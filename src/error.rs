//! Bus errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BusError {
    #[error("message {0} not found")]
    NotFound(String),

    #[error("agent `{0}` is not registered on this bus")]
    UnknownAgent(String),

    #[error("message {0} is not in `delivered` state (current: {1}) — ack requires delivery first")]
    NotDeliverable(String, String),

    #[error("mailbox for `{0}` is full ({1} messages) — consumer is not polling")]
    MailboxFull(String, usize),

    #[error("storage error: {0}")]
    Storage(String),
}

pub type BusResult<T> = Result<T, BusError>;
