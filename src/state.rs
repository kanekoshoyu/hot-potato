//! Declarative letter state machine — the single source of truth.
//!
//! Every status change in the bus MUST go through [`apply`]. The
//! `TRANSITIONS` table below is the only place a (state, event) → state
//! mapping exists; anything not in the table is an [`Illegal`] transition
//! and is refused instead of silently rewritten.
//!
//! Rationale (2026-09-18 audit, Sho-commissioned): status writes were
//! previously scattered across deliver.rs, sweeper.rs and two store
//! implementations; each site had its own private opinion ("push rejected →
//! call it delivered"), which produced lying receipts that no sweeper could
//! rescue. This module collapses all of that into one table.

use crate::message::MessageStatus as S;
use chrono::{DateTime, Utc};

/// What happened to a letter — the only inputs the state machine accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A dispatcher (push-on-arrival or sweeper) starts pushing the letter.
    /// CAS gate: only legal from `Queued` — prevents double delivery when
    /// the dispatcher and the sweeper race on the same letter.
    PushStarted,
    /// The push returned HTTP 2xx — the peer's HTTP layer accepted it.
    PushOk,
    /// The push got a 4xx/5xx: the endpoint exists but refused the letter.
    /// HONEST semantics: refused ≠ delivered. Back to Queued for sweeper retry.
    PushRejected { reason: String },
    /// The push failed at the transport level (connect error, DNS, TLS…).
    PushErrored { reason: String },
    /// A poll-only receiver claimed the letter via drain.
    PollClaimed,
    /// Chatlog-style read receipt.
    ReadReceipt,
    /// Recipient finished their part and filed the result (≤80 char note).
    Ack { note: Option<String> },
    /// The letter exhausted its attempts (or was judged dead by an operator).
    GiveUp { reason: String },
}

impl Event {
    /// Short machine-readable name for logs and metrics.
    pub fn name(&self) -> &'static str {
        match self {
            Event::PushStarted => "push_started",
            Event::PushOk => "push_ok",
            Event::PushRejected { .. } => "push_rejected",
            Event::PushErrored { .. } => "push_errored",
            Event::PollClaimed => "poll_claimed",
            Event::ReadReceipt => "read_receipt",
            Event::Ack { .. } => "ack",
            Event::GiveUp { .. } => "give_up",
        }
    }
}

/// An (event, state) pair that is not in the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Illegal {
    pub event: &'static str,
    pub from: S,
}

impl std::fmt::Display for Illegal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "illegal transition: {} from {:?}", self.event, self.from)
    }
}
impl std::error::Error for Illegal {}

/// The truth table. One row per legal (from, event) → to.
/// If you are adding a state or an event, this table is the ONLY place to
/// define its transitions.
pub const TRANSITIONS: &[(S, &str, S)] = &[
    (S::Queued, "push_started", S::Pushing),
    (S::Pushing, "push_ok", S::Delivered),
    (S::Pushing, "push_rejected", S::Queued), // honest failure: sweeper will retry
    (S::Pushing, "push_errored", S::Queued),  // transport error: sweeper will retry
    (S::Queued, "poll_claimed", S::Delivered),
    (S::Delivered, "read_receipt", S::Read),
    (S::Delivered, "ack", S::Acked),
    (S::Read, "ack", S::Acked),
    (S::Queued, "give_up", S::Dead),
    (S::Pushing, "give_up", S::Dead), // poison letter detected mid-push
];

/// Outcome of applying an event to a letter's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Status changed to the returned state.
    To(S),
    /// The event does not change the status (e.g. a retry-state event on an
    /// already-retry-target letter). Callers may treat as a no-op.
    NoOp(S),
}

/// Apply `event` to `status`. Pure function — no I/O, no side effects.
/// Stores still own persistence; callers own logging.
pub fn apply(status: S, event: &Event) -> Result<Transition, Illegal> {
    let name = event.name();
    // PushStarted from Queued is legal; from any other state it is a CAS
    // violation (someone else is already pushing / letter already claimed).
    for (from, ev, to) in TRANSITIONS {
        if *from == status && *ev == name {
            return Ok(Transition::To(*to));
        }
    }
    // Duplicate PushStarted on a letter that is already Pushing: second racer
    // gets a NoOp so a benign dispatcher/sweeper race logs a skip instead of
    // an error storm. Every other state refuses — Dead is terminal.
    if let Event::PushStarted = event {
        if status == S::Pushing {
            return Ok(Transition::NoOp(status));
        }
    }
    Err(Illegal {
        event: name,
        from: status,
    })
}

/// Poison-letter policy: after this many failed push attempts the letter is
/// declared Dead instead of retrying forever.
pub const MAX_ATTEMPTS: u32 = 8;
/// A push that has not come back within this window is treated as errored by
/// the sweeper (the transport's own timeout is 120s; this is the state-level
/// liveness bound so a lost Pushing letter self-heals).
pub const PUSHING_TIMEOUT_SECS: i64 = 150;

/// Side-channel facts updated alongside a transition (not part of the table
/// because they are bookkeeping, not state).
#[derive(Debug, Clone, Default)]
pub struct Bookkeeping {
    pub attempts: Option<u32>,
    pub last_error: Option<String>,
    pub last_push_at: Option<DateTime<Utc>>,
    pub dead_reason: Option<String>,
}

/// Derive bookkeeping deltas for a transition. `prev_attempts` is the letter's
/// current attempt count.
pub fn bookkeep(event: &Event, prev_attempts: u32, now: DateTime<Utc>) -> Bookkeeping {
    match event {
        Event::PushStarted => Bookkeeping {
            attempts: Some(prev_attempts + 1),
            last_push_at: Some(now),
            ..Default::default()
        },
        Event::PushRejected { reason } | Event::PushErrored { reason } => Bookkeeping {
            last_error: Some(reason.clone()),
            ..Default::default()
        },
        Event::GiveUp { reason } => Bookkeeping {
            dead_reason: Some(reason.clone()),
            ..Default::default()
        },
        _ => Bookkeeping::default(),
    }
}

/// Should this letter be declared dead? Called by the sweeper after a failed
/// push when the attempt count has run out.
pub fn should_give_up(attempts: u32) -> bool {
    attempts >= MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal(status: S, event: Event) -> S {
        match apply(status, &event).expect("expected legal transition") {
            Transition::To(s) => s,
            Transition::NoOp(s) => s,
        }
    }

    #[test]
    fn happy_path_queued_to_acked() {
        assert_eq!(legal(S::Queued, Event::PushStarted), S::Pushing);
        assert_eq!(legal(S::Pushing, Event::PushOk), S::Delivered);
        assert_eq!(legal(S::Delivered, Event::ReadReceipt), S::Read);
        assert_eq!(
            legal(
                S::Read,
                Event::Ack {
                    note: Some("done".into())
                }
            ),
            S::Acked
        );
    }

    #[test]
    fn rejected_push_is_honest_back_to_queued() {
        // THE fix for the lying-receipt bug: refusal is NOT delivery.
        assert_eq!(
            legal(
                S::Pushing,
                Event::PushRejected {
                    reason: "403".into()
                }
            ),
            S::Queued
        );
        assert_eq!(
            legal(
                S::Pushing,
                Event::PushErrored {
                    reason: "conn refused".into()
                }
            ),
            S::Queued
        );
    }

    #[test]
    fn push_started_is_cas_gated() {
        // second racer on a Pushing letter → NoOp, status unchanged
        assert_eq!(
            apply(S::Pushing, &Event::PushStarted),
            Ok(Transition::NoOp(S::Pushing))
        );
    }

    #[test]
    fn illegal_transitions_are_refused() {
        // cannot read before delivered
        assert!(apply(S::Queued, &Event::ReadReceipt).is_err());
        // cannot ack from queued
        assert!(apply(S::Queued, &Event::Ack { note: None }).is_err());
        // cannot poll-claim a letter mid-push
        assert!(apply(S::Pushing, &Event::PollClaimed).is_err());
        // dead is terminal
        assert!(apply(S::Dead, &Event::PushStarted).is_err());
        assert!(apply(S::Dead, &Event::Ack { note: None }).is_err());
    }

    #[test]
    fn give_up_is_legal_from_queued_and_pushing_only() {
        assert_eq!(
            legal(
                S::Queued,
                Event::GiveUp {
                    reason: "poison".into()
                }
            ),
            S::Dead
        );
        assert_eq!(
            legal(
                S::Pushing,
                Event::GiveUp {
                    reason: "poison".into()
                }
            ),
            S::Dead
        );
        assert!(apply(S::Delivered, &Event::GiveUp { reason: "x".into() }).is_err());
    }

    #[test]
    fn ack_from_delivered_skips_read() {
        assert_eq!(legal(S::Delivered, Event::Ack { note: None }), S::Acked);
    }

    #[test]
    fn bookkeeping_tracks_attempts_and_errors() {
        let now = Utc::now();
        let bk = bookkeep(&Event::PushStarted, 3, now);
        assert_eq!(bk.attempts, Some(4));
        assert_eq!(bk.last_push_at, Some(now));

        let bk = bookkeep(
            &Event::PushRejected {
                reason: "401".into(),
            },
            4,
            now,
        );
        assert_eq!(bk.last_error.as_deref(), Some("401"));

        let bk = bookkeep(
            &Event::GiveUp {
                reason: "poison".into(),
            },
            8,
            now,
        );
        assert_eq!(bk.dead_reason.as_deref(), Some("poison"));
    }

    #[test]
    fn poison_policy_threshold() {
        assert!(!should_give_up(MAX_ATTEMPTS - 1));
        assert!(should_give_up(MAX_ATTEMPTS));
    }
    #[test]
    fn dead_is_fully_terminal() {
        // No event may leave Dead — including the CAS-racer NoOp path that
        // previously let PushStarted through.
        for ev in [
            Event::PushStarted,
            Event::PushOk,
            Event::PushRejected { reason: "x".into() },
            Event::PushErrored { reason: "x".into() },
            Event::ReadReceipt,
            Event::PollClaimed,
            Event::Ack { note: None },
            Event::GiveUp { reason: "x".into() },
        ] {
            assert!(apply(S::Dead, &ev).is_err(), "Dead must refuse {ev:?}");
        }
    }

    #[test]
    fn every_state_event_pair_has_a_verdict() {
        // Exhaustive table audit: every (state, event) pair resolves to a
        // legal transition, a benign NoOp, or an explicit refusal — never a
        // panic, never an unhandled path.
        let states = [
            S::Queued,
            S::Pushing,
            S::Delivered,
            S::Read,
            S::Acked,
            S::Dead,
        ];
        let events = [
            Event::PushStarted,
            Event::PushOk,
            Event::PushRejected { reason: "r".into() },
            Event::PushErrored { reason: "r".into() },
            Event::ReadReceipt,
            Event::PollClaimed,
            Event::Ack { note: None },
            Event::GiveUp { reason: "r".into() },
        ];
        for st in states {
            for ev in &events {
                let _ = apply(st, ev);
            }
        }
    }

}
