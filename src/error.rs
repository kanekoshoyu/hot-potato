//! Bus errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BusError {
    #[error("message {0} not found")]
    NotFound(String),

    #[error("agent `{0}` is not registered on this bus")]
    UnknownAgent(String),

    #[error(
        "message {0} is not in a state that allows this operation (current: {1}); \
         allowed transitions: queued→delivered(poll)→read(mark_read)→acked(ack); \
         ack also accepts read state"
    )]
    NotDeliverable(String, String),

    /// Retrying an ack with the same intent — treated as success (idempotent).
    #[error("message {0} was already acked (at {1})")]
    AlreadyAcked(String, String),

    #[error("mailbox for `{0}` is full ({1} messages) — consumer is not polling")]
    MailboxFull(String, usize),

    #[error("missing required param: {0} (expected params: {1})")]
    MissingParam(String, String),

    #[error("storage error: {0}")]
    Storage(String),
}

pub type BusResult<T> = Result<T, BusError>;
