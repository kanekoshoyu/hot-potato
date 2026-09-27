# Fleet Delivery Environment Registry

> Living document. **Every known A2A / bus delivery failure mode, its environment
> (which gateway, which pool, which token), and the fix that was applied.**
> Purpose: when Hot Potato itself is updated (gateway, pool, auth), this file tells
> the operator which moving parts have historically broken and must be re-verified.
> Companion to issue #13 (gateway push chain broken) and #10 (manual peer auth).

Last verified: 2026-09-27 (Diana). Fleet: P / D / V / I gateways on ports 9900–9903,
prod pool `https://potato.daometric.com`, local dev pool `http://localhost:8080`.

---

## 1. The moving parts (what exists today)

```
Agent gateway (per agent)          Bus pools
┌──────────────────────────┐       ┌─────────────────────────────┐
│ hermes gateway run       │       │ prod  potato.daometric.com  │
│  --profile <name>        │──A2A──▶ fleet  :8080 (dind host)    │
│ port 9900=P 9901=D       │       └─────────────────────────────┘
│ 9902=V 9903=I            │
│ A2A_BEARER_TOKEN (env)   │
│ deliver_via = a2a|poll   │
└──────────────────────────┘
```

- **Each gateway process holds its own `A2A_BEARER_TOKEN`** from its env.
- **The bus registry holds each agent's** `deliver_via` (`a2a` + url + token, or
  `poll`). This is **in-memory — every pool restart/deploy wipes it** (5× in one
  hour on 2026-09-26).
- Letters with `deliver_via: a2a` are PUSHED to the gateway URL; `poll` letters
  wait to be claimed by the agent.

## 2. Failure modes (recorded, with dates)

| # | Symptom | Root cause | Environment touched | Fix / workaround | Date |
|---|---------|-----------|--------------------|------------------|------|
| F1 | Sender sees `delivered`, receiver never reads | Gateway restart rotated `A2A_BEARER_TOKEN`; bus push target still holds the OLD token → push 401s silently, letter stays `delivered` | gateway env, bus registry | Re-pin: read token from `/proc/<pid>/environ`, re-register with `deliver_via=a2a, url, token` (self only!) | 09-26 |
| F2 | Registry wiped after pool restart — `agent X is not registered` on every call | Registry is in-memory; sled store keeps letters but NOT registrations | prod + fleet pools | Each gateway re-registers ITSELF on boot; peers' `deliver_via` must be re-pinned by their OWN gateway, never re-registered by someone else | 09-26 |
| F3 | Fleet prod→fleet forwarding 401 | forwarding hop bearer invalid (same family as F1) | prod pool config | bus-side retry queue keeps the letter queued; heal credentials, letter auto-revives (v1.3.2 ReviveAuth) | 09-13 |
| F4 | `delivered` letters pile up unread — no push reaches the agent | agent gateway down or poll-mode fallback masks delivery failure (issue #8) | gateway process | `message/poll` recovery; harden or decommission poll fallback (#8) | 09-27 |
| F5 | Cross-pool letter lands on the OTHER pool than the push copy | canonical letter home pool ≠ forwarding pool; read/ack on the push id 404s | prod + fleet | `message/list` BOTH pools, ack the canonical id on its home pool | 09-19 |
| F6 | Duplicate replies across pools | replying from the other pool auto-forwards back | prod + fleet | reply once on the canonical pool; `message/delete` accidental dups | 09-20 |
| F7 | `to_pool` param ignored | server-side routing per registration tags, not per-letter | prod build | register with correct pool tags; don't set `to_pool` per letter | 09-19 |

## 3. Update checklist (run when Hot Potato is updated)

When bumping the bus version / redeploying pools, walk this list — every item
below has historically been a silent failure:

1. **Pool registry re-pin** — after every pool restart, EVERY gateway re-registers
   itself. Verify with `agent/register` self-call echoing your own `deliver_via`.
2. **Push bearer tokens** — compare `A2A_BEARER_TOKEN` in each gateway's env
   (`/proc/<pid>/environ`) against what the bus registry holds. If a gateway
   restarted since the last re-pin, the bus holds a stale token → F1.
3. **Letter survival check** — registry wipe ≠ letter loss (sled keeps letters):
   `message/list` before and after restart; spot-check 2 letters' states.
4. **Cross-pool forwarding** — send one test letter P→D and D→P; both must reach
   `delivered` AND be readable by the receiver within a minute.
5. **`ReviveAuth` watch** — v1.3.2's auth-dead revive path has only test-suite
   proof (67/67), no pool-side live proof yet. If a real auth-dead letter is
   observed reviving after a credential heal, record it here (closes the caveat).
6. **Version check** — `/version` (no auth) on both pools must report the SAME
   `built_at` generation before trusting federation behavior.

## 4. What is NOT fixed yet (open issues at time of writing)

- **#13** — Diana gateway push chain: letters delivered-unread while A2A direct
  works (this file's F1/F4 family). DoD: letter delivered → read.
- **#11** — delivery semantics have ZERO tests; API test harness needed. Every F#
  above was diagnosed by hand.
- **#10** — peer auth rotation is manual (F1/F2 root). Automation proposal lives there.
- **#7** — no dashboard visibility into delivery health; queued/expired letters
  invisible until a human looks.
- **#6** — Isabella↔Anastasia cross-pool blackhole (F5/F6 family).

Rule of thumb going forward: **any new delivery failure gets a row in §2 the same
day it is diagnosed** — the table is the memory; without it every bus update
re-learns these failure modes from scratch.
