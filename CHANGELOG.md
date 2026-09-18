# Changelog

All notable changes to Hot Potato are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/).

## [1.3.0] — 2026-09-18

Declarative letter state machine + honest delivery semantics. The letter
lifecycle is now defined by a single truth table, refusals are no longer
recorded as deliveries, and broadcasts materialize into one letter per
recipient.

### Added

- **Declarative state machine** (`src/state.rs`): a `TRANSITIONS` truth table
  plus a single `apply()` entry point. Every status change in the bus, stores,
  sweeper, and server funnels through it — illegal transitions are refused
  with an error instead of being silently written. Adding a state or event
  now means touching exactly one table.
- **`Pushing` state**: an in-flight state claimed via CAS (`claim_push`).
  A dispatcher and the sweeper can no longer double-push the same letter;
  the CAS loser skips and the winner owns the outcome.
- **`Dead` terminal state**: after 8 failed push attempts (`MAX_ATTEMPTS`)
  a poison letter is declared Dead with its `dead_reason` attached — visible
  in `message/list` instead of retrying forever in the shadows.
- **Delivery forensics on every letter**: `attempts`, `last_push_at`,
  `last_error`, `dead_reason` fields (serde-defaulted; existing stores need
  zero migration). Failed deliveries are now diagnosable from data, not
  guesswork.
- **Push outcome logging**: every push attempt emits one structured log line
  (`pushed` / `failed` / `skipped` + reason) via `log_push_outcome`.
- **`receiver="all"` broadcast materialization**: `message/send` with
  `receiver="all"` fans out into one independent letter per registered agent,
  each with its own lifecycle and push-on-arrival attempt, and returns
  `{ids, fanout}`. The literal `"all"` mailbox letter is gone.
- **Stale-`Pushing` recovery**: the sweeper reclaims letters stuck mid-push
  (dispatcher died between claim and outcome) back to `queued` after a
  liveness window, so a crashed process can no longer strand a letter.
- **11 new tests** covering the truth table (terminality, CAS gating,
  poison policy, exhaustive state×event verdicts), honest-failure E2E
  (refused pushes stay queued then go Dead, never delivered), and broadcast
  materialization. 65/65 green.

### Changed

- **HTTP 4xx/5xx refusals are failures, not deliveries.** `HttpTransport::push`
  previously mapped a rejected request to success, so the letter was marked
  `delivered` and never retried — the root cause of letters silently
  black-holing for hours. Refusals now keep the letter `queued` (with
  `attempts`/`last_error` recorded) for sweeper retry.
- **Connection errors mid-push** release the letter's in-flight claim back to
  `queued` instead of leaving it in limbo.
- **Sweeper is attempt-aware**: retries every queued letter on its exponential
  backoff schedule, then gives up honestly at `MAX_ATTEMPTS`.
- Push-on-arrival extracted into a shared `push_on_arrival()` path used by
  single sends, broadcast fan-out, and register re-push — one honest outcome
  handler instead of three divergent ones.

### Unchanged

- **A2A timeout semantics**: a timed-out push still counts as *notified*
  (the receiving agent has the task and will process it), matching the a2a
  request/response contract. Only hard refusals and connection errors are
  retried, which keeps retry volume bounded.
- Store schemas (sled / in-memory): new fields default cleanly, no data
  migration required.
- Poll semantics, ack flow, federation forwarding, and the RPC surface are
  byte-compatible; existing agents need no changes.

## [1.2.3] — 2026-09-16

- Sweeper: queue self-healing with per-letter exponential backoff
  (1m → 2m → … → 32m cap), re-dispatching letters whose push failed at
  send time.
- Federation hardening: hop limits, peer token checks, inbound envelope
  validation.
- Per-thread A2A `contextId` (v1.2.0): one receiver session per thread for
  prompt cache hits.
