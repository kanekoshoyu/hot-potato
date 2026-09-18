//! Queue sweeper — the self-healing dispatcher.
//!
//! `dispatch_push` is point-in-time: a push that fails at send time (receiver's
//! gateway restarting, endpoint down, DNS hiccup) leaves the letter queued
//! forever — the only recovery paths were the receiver's poll or a fresh
//! `agent/register` re-push. The 2026-09-16 production audit found 33 letters
//! stuck queued for 17–45h this way. The sweeper re-dispatches queued letters
//! on a fixed cadence with per-letter exponential backoff, so a transient push
//! failure self-heals within one sweep period.
//!
//! Semantics (each deliberate):
//! - only letters still `queued` are touched — delivered/read/acked are never
//!   re-pushed (poll = claim remains the single transition into delivered)
//! - enumeration goes through `list_all()` (store level), not per-agent peek:
//!   no registration gate, so letters to a since-unregistered receiver are
//!   visible and skippable instead of invisible
//! - push success → `mark_delivered`, the same receipt path as push-on-arrival
//!   (RFC-002); a2a timeouts already count as notified success inside
//!   `HttpTransport::push`, so duplicate delivery only risks the connect-error
//!   window — the same contract the register re-push has always had, and the
//!   receiver's ack path dedupes by id
//! - backoff is derived statelessly from the letter's age (1m → 2m → 4m …
//!   capped at ~32m): no per-letter attempt counter to persist, a restart
//!   resumes the same schedule
//! - `min_backoff` is an argument (DI) so tests run at zero delay without
//!   touching process-global env

use crate::bus::EventBus;
use crate::deliver::{dispatch_push, PushOutcome, PushTransport, Registry};
use std::sync::Arc;
use std::time::Duration;

/// Delay between sweeps. Set `HOT_POTATO_SWEEP_PERIOD_SECS` to enable the
/// background sweeper (both compose files ship 300); unset = disabled, which
/// keeps `router()` test-side-effect-free.
pub fn sweep_period() -> Option<Duration> {
    let secs = std::env::var("HOT_POTATO_SWEEP_PERIOD_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())?;
    Some(Duration::from_secs(secs.max(1)))
}

/// Production backoff floor for [`retry_window`].
pub const MIN_BACKOFF: Duration = Duration::from_secs(60);

/// A queued letter is retried only when its age covers the next backoff
/// window: 1m, 2m, 4m, 8m, 16m, 32m (capped). Stateless — derived from age.
pub fn retry_window(age: Duration, min_backoff: Duration) -> Duration {
    let step = min_backoff.as_secs().max(1);
    let attempts = (age.as_secs() / step).checked_ilog2().unwrap_or(0).min(5);
    Duration::from_secs(step << attempts)
}

/// Re-push every queued letter whose backoff window has elapsed. Returns how
/// many letters were pushed. Honest-failure semantics (2026-09-18 rewrite):
/// a refused push keeps the letter queued and records attempts/last_error;
/// after `MAX_ATTEMPTS` failures the letter goes terminal `Dead` instead of
/// retrying forever. Stale `Pushing` letters (dispatcher died mid-push) are
/// reclaimed to queued first. Nothing here can silently lose a letter — the
/// worst case is a visible Dead letter with a reason attached.
pub async fn sweep_once(
    bus: &Arc<EventBus>,
    registry: &Arc<Registry>,
    transport: &Arc<dyn PushTransport>,
    hub: &Arc<crate::ws::EventHub>,
    min_backoff: Duration,
) -> usize {
    let Ok(all) = bus.list_all().await else {
        return 0;
    };
    let now = chrono::Utc::now();
    let mut pushed = 0usize;

    // Liveness recovery (audit D2): a letter stuck in Pushing past the window
    // means its dispatcher died between PushStarted and the outcome. Reclaim
    // to Queued so the normal retry path below can pick it up again.
    for letter in all
        .iter()
        .filter(|l| l.status == crate::message::MessageStatus::Pushing)
    {
        let stale_for = (now - letter.last_push_at.unwrap_or(letter.created_at)).num_seconds();
        if stale_for > crate::state::PUSHING_TIMEOUT_SECS {
            let _ = bus
                .reclaim_stale_pushing(
                    &letter.receiver,
                    &letter.id,
                    &format!("pushing stale {stale_for}s (dispatcher died mid-push)"),
                )
                .await;
        }
    }

    let Ok(all) = bus.list_all().await else {
        return 0;
    };
    for letter in all
        .iter()
        .filter(|l| l.status == crate::message::MessageStatus::Queued)
    {
        let age = (now - letter.created_at).to_std().unwrap_or_default();
        // min_backoff ZERO disables gating entirely (tests); production uses
        // MIN_BACKOFF (60s) so retries pace out 1m → 2m → … → 32m.
        if min_backoff > Duration::ZERO && age < retry_window(age, min_backoff) {
            continue;
        }
        // Dead letters are never touched (terminal); Pushing letters are owned
        // by a live dispatcher (CAS claim below also guards this).
        let mut push_letter = serde_json::to_value(letter).expect("letter json");
        // v1.2.0: per-thread A2A contextId — resolve the thread root once per
        // letter so receivers keep reusing the session they already have.
        push_letter["contextId"] = serde_json::json!(format!(
            "bus-t-{}",
            crate::message::Message::thread_root_id(&all, &letter.id)
        ));

        // CAS claim: Queued → Pushing. If we don't win (dispatcher racing),
        // skip — the winner owns the outcome.
        if !bus
            .claim_push(&letter.receiver, &letter.id)
            .await
            .unwrap_or(false)
        {
            continue;
        }

        let outcome = dispatch_push(registry, transport, &letter.receiver, &push_letter).await;
        crate::deliver::log_push_outcome(&letter.id, &letter.receiver, &outcome);
        match outcome {
            PushOutcome::Pushed => {
                if let Ok(m) = bus.push_succeeded(&letter.receiver, &letter.id).await {
                    hub.emit("delivered", &m);
                    pushed += 1;
                }
            }
            PushOutcome::Failed { error } => {
                // Honest failure: Pushing → Queued (sweeper retries with
                // backoff), or → Dead once attempts are exhausted.
                if let Ok(m) = bus.push_failed(&letter.receiver, &letter.id, &error).await {
                    if m.status == crate::message::MessageStatus::Dead {
                        eprintln!(
                            "🥔 sweeper: letter {} declared DEAD after {} attempts: {error}",
                            letter.id, m.attempts
                        );
                    }
                }
            }
            PushOutcome::Skipped { reason } => {
                // Nothing was attempted — release the claim so a later sweep
                // (or the register re-push) can take it.
                let _ = bus
                    .reclaim_stale_pushing(&letter.receiver, &letter.id, &reason)
                    .await;
            }
        }
    }
    pushed
}

/// Spawn the periodic sweeper if `HOT_POTATO_SWEEP_PERIOD_SECS` is set.
/// Called from `router()`; a no-op in tests and bare local runs.
pub fn spawn_if_enabled(
    bus: Arc<EventBus>,
    registry: Arc<Registry>,
    transport: Arc<dyn PushTransport>,
    hub: Arc<crate::ws::EventHub>,
) {
    let Some(period) = sweep_period() else { return };
    tokio::spawn(async move {
        eprintln!(
            "🥔 sweeper: re-pushing queued letters every {}s",
            period.as_secs()
        );
        let mut ticker = tokio::time::interval(period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let n = sweep_once(&bus, &registry, &transport, &hub, MIN_BACKOFF).await;
            if n > 0 {
                eprintln!("🥔 sweeper: re-pushed {n} queued letter(s)");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deliver::DeliverVia;
    use crate::message::MsgType;
    use crate::store::memory::InMemoryStore;
    use std::sync::atomic::{AtomicBool, Ordering};

    // -- backoff windows ---------------------------------------------------

    #[test]
    fn backoff_fresh_letter_waits_one_minute() {
        let w = retry_window(Duration::from_secs(30), MIN_BACKOFF);
        assert_eq!(w, Duration::from_secs(60));
        assert!(Duration::from_secs(30) < w);
    }

    #[test]
    fn backoff_windows_grow_and_cap() {
        let mb = Duration::from_secs(60);
        // ~1m old → window 1m (retry now)
        assert_eq!(retry_window(Duration::from_secs(61), mb), mb);
        // ~2.5m old → window 2m
        assert_eq!(
            retry_window(Duration::from_secs(150), mb),
            Duration::from_secs(120)
        );
        // ~4m old → window 4m
        assert_eq!(
            retry_window(Duration::from_secs(245), mb),
            Duration::from_secs(240)
        );
        // 4h old → capped at 32m
        assert_eq!(
            retry_window(Duration::from_secs(14400), mb),
            Duration::from_secs(60 << 5)
        );
    }

    // -- sweep e2e ---------------------------------------------------------

    /// Fails the first push, succeeds afterwards — simulates a receiver that
    /// was down at send time and came back.
    struct FlakyTransport(AtomicBool);

    #[async_trait::async_trait]
    impl PushTransport for FlakyTransport {
        async fn push(&self, _t: &DeliverVia, _l: &serde_json::Value) -> Result<(), String> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err("receiver down (first attempt)".into())
            } else {
                Ok(())
            }
        }
    }

    async fn bus_with_registry() -> (Arc<EventBus>, Arc<Registry>) {
        let bus = Arc::new(EventBus::new(Arc::new(InMemoryStore::new())));
        bus.register("alice", crate::bus::Role::Pm).await.unwrap();
        bus.register("carol", crate::bus::Role::Worker)
            .await
            .unwrap();
        let registry = Arc::new(Registry::new());
        registry
            .register(
                "carol",
                "test",
                DeliverVia::Webhook {
                    url: "http://mock".into(),
                },
            )
            .await;
        (bus, registry)
    }

    #[tokio::test]
    async fn sweep_recovers_letter_push_failed_at_send_time() {
        let (bus, registry) = bus_with_registry().await;
        let hub = Arc::new(crate::ws::EventHub::new());
        let transport: Arc<dyn PushTransport> = Arc::new(FlakyTransport(AtomicBool::new(true)));

        bus.send("alice", "carol", MsgType::Task, "subject", "body", None)
            .await
            .unwrap();

        // 1st sweep: transport fails → still queued
        let n = sweep_once(&bus, &registry, &transport, &hub, Duration::ZERO).await;
        assert_eq!(n, 0, "failed push must not claim delivery");
        assert_eq!(count_queued(&bus).await, 1);

        // 2nd sweep: transport recovered → delivered
        let n = sweep_once(&bus, &registry, &transport, &hub, Duration::ZERO).await;
        assert_eq!(n, 1);
        assert_eq!(
            count_queued(&bus).await,
            0,
            "sweep must mark delivered on push success"
        );
    }

    /// Always refuses — the lying-receipt E2E: a 4xx-style rejection must
    /// NEVER flip the letter to delivered, and must go Dead (not retry
    /// forever) once attempts are exhausted.
    struct RejectingTransport;

    #[async_trait::async_trait]
    impl PushTransport for RejectingTransport {
        async fn push(&self, _t: &DeliverVia, _l: &serde_json::Value) -> Result<(), String> {
            Err("401 invalid token".into())
        }
    }

    #[tokio::test]
    async fn rejected_pushes_stay_queued_then_go_dead() {
        let (bus, registry) = bus_with_registry().await;
        let hub = Arc::new(crate::ws::EventHub::new());
        let transport: Arc<dyn PushTransport> = Arc::new(RejectingTransport);

        let id = bus
            .send("alice", "carol", MsgType::Task, "poison", "body", None)
            .await
            .unwrap();

        // MAX_ATTEMPTS sweeps: every push is refused.
        for i in 0..crate::state::MAX_ATTEMPTS {
            let n = sweep_once(&bus, &registry, &transport, &hub, Duration::ZERO).await;
            assert_eq!(n, 0, "refused push must not claim delivery (sweep {i})");
        }
        // After MAX_ATTEMPTS failed pushes the letter is terminal Dead with
        // the last error attached — visible, not silently lost, NOT delivered.
        let letters = bus.list_all().await.unwrap();
        let m = letters.iter().find(|l| l.id == id).unwrap();
        assert_eq!(
            m.status,
            crate::message::MessageStatus::Dead,
            "poison letter must die honestly, got {:?}",
            m.status
        );
        assert_eq!(m.attempts, crate::state::MAX_ATTEMPTS);
        assert_eq!(m.last_error.as_deref(), Some("401 invalid token"));
        assert!(m.delivered_at.is_none(), "dead ≠ delivered");
    }

    #[tokio::test]
    async fn sweep_respects_backoff_and_never_touches_nonqueued() {
        let (bus, registry) = bus_with_registry().await;
        let hub = Arc::new(crate::ws::EventHub::new());
        let transport: Arc<dyn PushTransport> = Arc::new(FlakyTransport(AtomicBool::new(false)));

        bus.send("alice", "carol", MsgType::Task, "fresh", "body", None)
            .await
            .unwrap();

        // Fresh letter + 60s backoff floor → skipped despite healthy transport.
        let n = sweep_once(&bus, &registry, &transport, &hub, MIN_BACKOFF).await;
        assert_eq!(n, 0, "backoff must gate the retry");
        assert_eq!(count_queued(&bus).await, 1);

        // Delivered letters are never re-pushed even with zero backoff.
        let queued_id = {
            let all = bus.list_all().await.unwrap();
            all.iter()
                .find(|l| l.status == crate::message::MessageStatus::Queued)
                .unwrap()
                .id
                .clone()
        };
        bus.mark_delivered("carol", &queued_id).await.unwrap();
        let n = sweep_once(&bus, &registry, &transport, &hub, Duration::ZERO).await;
        assert_eq!(n, 0, "delivered letters must never be re-pushed");
    }

    #[tokio::test]
    async fn sweep_skips_unregistered_receiver_without_touching_mailbox() {
        let (bus, registry) = bus_with_registry().await;
        let hub = Arc::new(crate::ws::EventHub::new());
        let transport: Arc<dyn PushTransport> = Arc::new(FlakyTransport(AtomicBool::new(false)));

        // Letter to an agent with a mailbox but no push registry entry.
        bus.register("sho", crate::bus::Role::Worker).await.unwrap();
        bus.send(
            "alice",
            "sho",
            MsgType::Task,
            "to a poll-only human",
            "body",
            None,
        )
        .await
        .unwrap();

        let n = sweep_once(&bus, &registry, &transport, &hub, Duration::ZERO).await;
        assert_eq!(n, 0, "no push registry entry → skipped");
        assert_eq!(count_queued(&bus).await, 1, "letter stays queued, not lost");
    }

    async fn count_queued(bus: &Arc<EventBus>) -> usize {
        bus.list_all()
            .await
            .unwrap()
            .iter()
            .filter(|l| l.status == crate::message::MessageStatus::Queued)
            .count()
    }
}
