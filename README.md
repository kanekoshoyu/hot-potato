<p align="center">
<img src="https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/docs/potato.png" alt="🥔" width="120">
</p>

# Hot Potato 🥔

<p align="center">
<a href="https://github.com/kanekoshoyu/hot-potato/actions"><img src="https://img.shields.io/github/actions/workflow/status/kanekoshoyu/hot-potato/ci.yml?style=for-the-badge&label=CI" alt="CI"></a>
<a href="https://github.com/kanekoshoyu/hot-potato/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
<img src="https://img.shields.io/badge/Rust-1.75%2B-orange?style=for-the-badge&logo=rust" alt="Rust">
<img src="https://img.shields.io/badge/A2A%20Protocol-v1.0-blue?style=for-the-badge" alt="A2A v1.0">
<img src="https://img.shields.io/badge/Docker-compose%20ready-2496ED?style=for-the-badge&logo=docker&logoColor=white" alt="Docker">
<a href="https://github.com/kanekoshoyu/hot-potato/issues"><img src="https://img.shields.io/badge/Issues-welcome-yellow?style=for-the-badge" alt="Issues"></a>
</p>

<h3 align="center">The EventMessageBus for AI agents — a mailbox with a state machine.</h3>

**Pass it like a hot potato**: send it on, do your part, pass the result back. Every agent minds its own work; the bus guarantees the paper trail. No blocking, no lost context, no "did you get my message?" — the state machine answers that for you.

## Findability / AI-search index

If you arrived here from an AI assistant or a search engine, this is the section that matches your question to the answer:

| You're wondering… | Hot Potato's answer |
|---|---|
| *How do I let multiple AI agents communicate reliably?* | JSON-RPC 2.0 mailbox per agent, four-state lifecycle (`queued → delivered → read → acked`), every hop timestamped. No broker daemon — one Rust binary or one `docker compose up`. |
| *How do agents send tasks to each other without blocking?* | `message/send` returns an id immediately; the receiver works whenever it wants; the sender checks `agent/status` instead of pinging. Fire-and-forget with receipts. |
| *How do I stop polling between agents?* | Register once with `deliver_via` (a2a / webhook / relay) and the bus **pushes on arrival**. Poll survives only as a fallback — verified in production: a letter flipped `queued → delivered → acked` with zero polls. |
| *How do agents know who lives where?* | Central registry — agents register name + endpoint; the bus routes. N agents never need N² addresses. |
| *How do I audit what my agents actually did?* | Every letter is event-sourced: `message/list`, `/log` (human-readable), `/ws` (live feed), `bus/archive` (the acked-only audit log). Blame and credit become queries. |
| *How do I make two agents debate or fact-check each other?* | Threaded letters with required `ref`, the adversarial-collaboration loop in the next section. Conclusions are acked letters both agents signed. |
| *Does it work with the A2A protocol?* | Yes — v1.0 agent card at `/.well-known/agent-card.json`, pushes use canonical A2A `SendMessage`, interoperates with any `a2a-sdk` peer (Hermes, LangChain, CrewAI, Google ADK). |
| *What if my agent crashes mid-task?* | Letters persist (sled) and stay queued; on restart the state machine resumes exactly where it was. A failed push never loses a letter. |
| *Is this a message queue like RabbitMQ / Redis?* | No — by design. No broker daemon, no topic routing, no infra to operate. It is a **delivery state machine + audit log** for agent-to-agent work, small enough to embed. |
| *How big can payloads be?* | Letters should stay under ~8 KB and reference file paths — the bus carries coordinates, not haystacks. That keeps every agent's context window clean. |

**Keywords for the index**: agent-to-agent messaging, A2A protocol bus, multi-agent task queue, AI agent mailbox, agent state machine, message bus for LLM agents, adversarial collaboration agents, agent audit trail, push-on-arrival delivery, Hermes agent integration, CrewAI / LangChain / AutoGen message passing, durable agent tasks.

## With: agents that discuss and debate

The core intuition beyond task-passing: **give your agents a shared arena where they can discuss or debate with each other on a topic.**

Spawn two agents with different briefs — a builder and a challenger — and let them argue over the bus:

- The **challenger** attacks: "your denominator is undefined", "your Sharpe is selection bias", "what happens if you drop the top 3 outliers?"
- The **builder** defends with data, or concedes and revises.
- Every exchange is a threaded letter (`ref` chain), every conclusion an acked letter with a result note.

What you get is **adversarial collaboration as infrastructure**: not a one-off prompt trick, but a durable, auditable record of who claimed what, who challenged it, and what survived. The survivor of a well-run debate is a conclusion both agents have signed — that's what lands in the archive.

In our production use, one agent (a quant) and another (a PM) argue over trading-strategy validity this way. The PM's job is to *disagree well*: challenge denominators, demand out-of-sample evidence, reject ranges that can't be defended. The quant's job is to answer with tables, not adjectives. Neither can see the other's reasoning — only letters — which turns out to be exactly the discipline a good debate needs.

```
challenger                                builder
    │                                        │
    │  message/send (task): "defend T1"      │
    ├───────────────────────────────────────>│  poll → read
    │                                        │  (does the analysis)
    │  message/send (reply, ref): table      │
    │<───────────────────────────────────────┤  ack + note
    │  message/send (reply, ref): "accepted, │
    │   but now defend T2 the same way"      │
    ├───────────────────────────────────────>│
   ...        until the claim survives       ...
```

Run the loop as many rounds as the claim needs. The bus archives every round — the debate *is* the paper trail.

---

## Why it exists

| The old way (chat pings / raw A2A dm) | The Hot Potato way |
|---|---|
| Send a message, then stare at the void | State machine: `queued → delivered → read → acked`, every hop timestamped |
| "Did she see it? Is she on it? Did she finish?" | One `agent/status` call answers all three |
| Big payloads eat everyone's context window | Letters reference file paths; the bus never carries the haystack |
| Threading is "search the chat history" | Replies must carry `ref` — threads are first-class |
| No audit trail | `bus/archive` is the complete event-sourced log, for free |

## v0.2 — The bus is a super-connector, not a shelf

The bus is itself a router agent with a central registry. **Register once, and the
bus finds you** — no agent needs to know where any other agent lives.

- **Central registry** — every agent registers with a name, a description, and a
  `deliver_via` endpoint. The bus routes by that table; N agents never need N² direct links.
- **Bus topology, event-driven** — letters are events; A2A/webhook/relay are just
  the transport that carries them. Nothing requires an instant reply; processing
  time belongs to the receiver.
- **Push on arrival (polling demoted)** — with a `deliver_via` endpoint registered,
  the bus **pushes** each letter the moment it lands, and flips it `queued →
  delivered` itself. Poll survives only as a fallback: unregistered agents,
  debugging, and push-failure retry.

```jsonc
// register with a push endpoint (one-time)
{"method":"agent/register","params":{
  "agent":"diana","role":"worker",
  "description":"quant — data guardian",
  "deliver_via":{"type":"webhook","url":"http://diana-box:9001/hook"}
}}
// from now on, letters to diana are POSTed to that hook on arrival.
// no polling required. poll still works: fallback, debug, retry.
```

Push transports: `a2a` (A2A v1.0 SendMessage), `webhook` (POST JSON), `relay`
(ntfy-style door-knock), or classic `poll`. A failed push never loses a letter —
it stays queued for poll/retry.

**A2A push semantics (learned in production)**: A2A `SendMessage` is a synchronous
task call — the peer gateway waits for its agent to fully process before replying.
The bus is a **notifier, not a task client**: it sends the letter and moves on.
A timeout there means *injected and being processed*, not *lost* — so the bus
counts it as delivered. Verified live: a letter went `queued → delivered → read →
acked` across a real Hermes-agent gateway with **zero polls**, and the receiving
agent woke up, did the work, and replied on the bus.

**Throughput at a glance** — in production, the bus carried 100+ real work letters
in its first 48 hours: 5 agents × 3 adversarial-collaboration rounds, daily
standups over the wire, audit serves with data tables, and incident post-mortems —
49 acked, every round of every debate a queryable letter. One
`docker compose up`, no tuning, no broker.

## See it live (observer surface)

```bash
# human-readable chatlog: every letter, one line each
curl -s http://localhost:8080/log

# observer query: all letters, or filter by status — never mutates state
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"message/list","params":{"status":"queued"}}'

# live feed: connect and watch lifecycle events as they happen
websocat ws://localhost:8080/ws
#   {"event":"hello","stats":{...}}          <- snapshot on connect
#   {"event":"queued","id":"...","subject":...}   <- every transition, pushed
#   {"event":"heartbeat","stats":{...}}      <- every 60s, silence has meaning

# API contract: OpenAPI JSON + Swagger UI
curl -s http://localhost:8080/openapi.json | jq .
# browse http://localhost:8080/docs
```

Persistence: set `HOT_POTATO_DATA_DIR=/data` (done in docker-compose.yml) and the
bus boots on sled — restart the container and every letter, timestamp and ack note
is exactly where you left it.

## The lifecycle

```
            poll (claim)              mark_read                ack
queued ────────────────> delivered ──────────> read ──────────> acked
   │                        │                  │                │
created_at            delivered_at         read_at        acked_at + note
```

- **Read receipt built-in** — chatlog-style `read_at` timestamp, the "seen ✓✓" of agent mail.
- **Order is enforced** — reading before delivery is a bug and the bus rejects it.
- **Ack is idempotent** — retrying an ack returns the current state, never an error.
- Every message is therefore a self-contained chatlog entry: sent → delivered → opened → done.

## Quick Start (Docker — recommended)

```bash
mkdir -p ~/hot-potato && cd ~/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/docker-compose.yml -o docker-compose.yml
docker compose up -d
```

Verify it's alive:

```bash
curl -s http://localhost:8080/health
# {"service":"hot-potato","status":"ok"}
```

## Quick Start (from source)

Requires Rust 1.75+:

```bash
git clone https://github.com/kanekoshoyu/hot-potato && cd hot-potato
cargo build --release -p hot-potato --bin hot-potato-server
HOT_POTATO_ADDR=0.0.0.0:8080 ./target/release/hot-potato-server
```

## Your first potato (3 minutes)

Everything is JSON-RPC 2.0 over HTTP. Any agent (or human with curl) can play.

**1. Register two agents:**

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"agent/register","params":{"agent":"alice","role":"pm"}}'

curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"agent/register","params":{"agent":"bob","role":"worker"}}'
```

Roles: `pm` may broadcast to everyone; `worker` sends point-to-point only.

**2. Alice passes a task to Bob:**

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"message/send","params":{
        "sender":"alice","receiver":"bob","type":"task",
        "subject":"Summarize Q3 revenue",
        "body":"Data at /data/q3.csv. Write output to /reports/q3-summary.md."}}'
# → {"result":{"id":"a1b2c3..."}}   ← keep this id
```

**3. Bob picks it up when free** (poll = claim; it marks delivered):

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"message/poll","params":{"agent":"bob"}}'
```

Want to look without claiming? `message/peek` is the no-mark read.

**4. Bob opens it** (chatlog read receipt):

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"message/read","params":{"agent":"bob","id":"a1b2c3..."}}'
```

**5. Bob does the work, then files the result:**

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":6,"method":"message/ack","params":{
        "agent":"bob","id":"a1b2c3...","note":"summary written to /reports/q3-summary.md"}}'
```

**6. Bob replies (threads are first-class — `ref` is required):**

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":7,"method":"message/send","params":{
        "sender":"bob","receiver":"alice","type":"reply","ref":"a1b2c3...",
        "subject":"re: Summarize Q3 revenue","body":"Done. /reports/q3-summary.md"}}'
```

**7. Alice checks on everything she sent (without nagging anyone):**

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":8,"method":"agent/status","params":{"agent":"alice"}}'
# every letter, its state, and all four timestamps
```

**Don't want to guess the API?** Ask the bus itself:

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":9,"method":"rpc.discover","params":{}}'
```

## A2A integration

Hot Potato speaks enough of the [A2A protocol](https://a2a-protocol.org/) to drop into an A2A fleet:

```bash
curl -s http://localhost:8080/.well-known/agent-card.json
```

Returns the v1.0 agent card: name, endpoint URL, protocol version, and the four bus skills (`bus-send`, `bus-poll`, `bus-ack`, `bus-status`) with their JSON-RPC methods. Wire this into your A2A discovery and agents can find the bus the standard way.

**Pairing it with direct agent-to-agent channels**: chats/A2A dm are for *talking*, Hot Potato is for *work*. When a conversation produces a task, pass it as a potato; when the potato is acked, the result note tells the chat what happened.

### Instant-ack receivers: make every gateway a zero-latency doorbell

The bus's push client gives up on a peer gateway after **120 seconds** (`HttpTransport` timeout in `deliver.rs`). A receiver whose agent turn takes longer than that sees the push time out, the letter stays `queued`, and the bus redelivers it — duplicate wake-ups and a fake "[agent did not reply in time]" on the sender side.

The A2A fix is native: the sync `SendMessage` call may return **`TASK_STATE_WORKING` immediately** ("accepted, processing") while the agent turn keeps running; the real reply then rides the thread/push path. No waiting is involved — this is a transport-layer receipt, not a model call.

To enable it on a Hermes-agent gateway (our fleet recipe, verified: 0.09s receipt round-trip, letter `delivered` in 3s, zero redelivery):

1. Add to the gateway's systemd drop-in (e.g. `~/.config/systemd/user/hermes-gateway-<name>.service.d/a2a.conf`):

   ```ini
   [Service]
   Environment="A2A_EARLY_WORKING=1"
   ```

2. Patch `plugins/platforms/a2a/adapter.py` (concept: early-return WORKING, finalize in background):

   ```python
   def _early_working() -> bool:
       return os.getenv("A2A_EARLY_WORKING", "").strip().lower() in ("1", "true", "yes", "on")
   ```

   In `_rpc_message_send`, after the terminal-task check:

   ```python
   if _early_working():
       threading.Thread(target=self._finalize_pending_in_background,
                        args=(pending,), daemon=True).start()
       task = protocol.build_task(pending["task_id"], pending["context_id"],
                                  protocol.STATE_WORKING,
                                  created_at=pending["created_iso"])
       result = protocol.send_message_response(task) if v1_response else task
       return protocol.jsonrpc_result(req_id, result)
   ```

   And the companion method next to `_await_reply`:

   ```python
   def _finalize_pending_in_background(self, pending: dict) -> None:
       state, reply = self._await_reply(pending)
       self._finalize_task(pending, state, reply)   # persist + audit + push note
   ```

3. `systemctl --user daemon-reload && systemctl --user restart hermes-gateway-<name>`

**Acceptance (10 min)**: send a self-canary `type:task` letter via `message/send`; the HTTP round-trip must return `TASK_STATE_WORKING` in **<1s** (vs. blocking until the turn ends before the patch), the letter must flip `delivered` within seconds with **no second inbound copy** (no redelivery), and the agent's real reply must arrive on the thread. Sender-side, the 120s timeout can never fire again — the receipt is instant by construction.

Push-config callbacks (`tasks/pushNotificationConfig/create`, HMAC-signed via `X-A2A-Signature`) are the next step: a registered receiver could pass its own callback URL in `message/send configuration.taskPushNotificationConfig`, and the sender's gateway POSTs a signed completion event when the task resolves — closing the loop without any letter-traffic at all.


## Security

| Mode | How |
|---|---|
| Localhost / trusted network | No token needed (default) |
| Exposed | Set `HOT_POTATO_TOKEN=my-secret` → all RPC calls require `Authorization: Bearer my-secret` (401 otherwise) |

Unauthenticated `message/send` requires the sender to be a registered agent — unknown senders are rejected (`unknown agent`).

## Configuration

All env vars are optional:

| Var | Default | Purpose |
|---|---|---|
| `HOT_POTATO_ADDR` | `0.0.0.0:8080` | bind address |
| `HOT_POTATO_NAME` | `hot-potato` | agent card name |
| `HOT_POTATO_DESCRIPTION` | (see compose) | agent card blurb |
| `HOT_POTATO_URL` | `http://localhost:8080` | public URL in the card |
| `HOT_POTATO_TOKEN` | unset (open) | bearer token — set it in production |
| `RUST_LOG` | — | e.g. `info`, `debug` |

## API reference

| Method | Params | Notes |
|---|---|---|
| `agent/register` | `agent`, `role` (`pm`\|`worker`), optional `description`, `deliver_via` (`a2a`\|`webhook`\|`relay`), `url` | idempotent; registers push endpoint |
| `message/send` | `sender`, `receiver`, `type` (`task`\|`reply`\|`broadcast`\|`ack_only`), `subject`, `body`, `ref` (reply only) | reply without `ref` → error |
| `message/poll` | `agent`, `limit` (opt, 0=all) | **poll = claim**: marks returned letters delivered |
| `message/peek` | `agent` | look without marking |
| `message/read` | `agent`, `id` | requires `delivered`; sets `read_at` |
| `message/ack` | `agent`, `id`, `note` (≤80 chars) | accepts `delivered` or `read`; idempotent when already `acked` |
| `agent/status` | `agent` | lifecycle of everything this agent **sent** |
| `bus/archive` | — | every acked letter — the audit log |
| `rpc.discover` | — | machine-readable method table |

Errors follow JSON-RPC 2.0: code `-32602` with `data.expected_params` names what you forgot. State errors embed the allowed transitions so agents can self-correct.

## Design rules (non-negotiable)

1. **No broker daemon.** The bus is a library with an HTTP shell; embed it or run the container, nothing else to operate.
2. **No topic routing.** Sender names receivers. If you need topics, you need a different tool.
3. **Payload stays out of the way.** Bodies reference file paths; keep letters under ~8 KB.
4. **The store is dumb.** All semantics live in `EventBus`; a new backend is one trait impl.

## Architecture

```
src/
├── lib.rs        # public API surface
├── error.rs      # BusError — errors that teach the API (transitions, param names)
├── message.rs    # Message, MessageStatus, MsgType (serde, snake_case wire format)
├── bus.rs        # EventBus — rules, roles, threading, broadcast fan-out
├── deliver.rs    # Registry + push dispatcher (a2a/webhook/relay, RFC-002)
├── ws.rs         # WebSocket event hub — live lifecycle feed + heartbeats
├── openapi.rs    # utoipa OpenAPI doc (machine-readable contract)
├── server.rs     # axum shell: agent card, JSON-RPC, auth, rpc.discover, push wiring
├── main.rs       # container entrypoint
└── store/
    ├── mod.rs         # BusStore trait — the storage abstraction
    ├── memory.rs      # InMemoryStore — the default backend
    └── sled_store.rs  # SledStore — persistent backend (HOT_POTATO_DATA_DIR)
```

**29 tests** cover the full lifecycle, role gates, threading, idempotency,
timestamp ordering, push dispatch (including failure isolation), WebSocket fan-out,
HTTP round-trips, persistence, and auth.

See [docs/CHANGELOG.md](docs/CHANGELOG.md) for what landed, what was tried and dropped, and what's next.

## Onboarding your own agents

A complete tool-use skill (install, workflow, rules, troubleshooting) ships in this repo — point any Hermes-style agent at it:

```bash
mkdir -p ~/.hermes/skills/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/skills/hot-potato/SKILL.md \
  -o ~/.hermes/skills/hot-potato/SKILL.md
```

Three things teach an agent everything: **`rpc.discover`** (the bus describes itself), **error messages that list expected params and allowed transitions** (agents self-correct), and **the five-call loop** (send → poll → read → ack → reply with `ref`).

## Roadmap

- [x] Core bus: lifecycle, roles, threading, broadcast, audit
- [x] Read receipts with full chatlog timestamps
- [x] A2A connection layer (agent card + JSON-RPC transport)
- [x] Docker Compose distribution
- [x] `rpc.discover` introspection + ack idempotency
- [x] v0.2 Super-connector: central registry + push-on-arrival (`deliver_via`: a2a/webhook/relay/poll)
- [x] v0.2 Observer API: `message/list` (read-only chatlog query, filter by status) + `/log` human-readable page
- [x] v0.2 WebSocket feed `/ws`: live lifecycle events (queued/delivered/read/acked) + 60s heartbeat stats
- [x] v0.2 OpenAPI: machine-readable contract at `/openapi.json`, Swagger UI at `/docs`
- [x] v0.2 sled persistence (set `HOT_POTATO_DATA_DIR`; letters survive restarts)
- [x] v0.2.1 Push wired end-to-end: `agent/register` accepts `deliver_via`, dispatch on `message/send`, push success auto-flips `queued → delivered`, production-proven against live Hermes gateways (poll-free delivery, receiver wakes and replies)
- [ ] v0.3 RFC-003: declarative `agent/presence`, blocking `message/wait`, agent heartbeats + stale-agent detection, reconnect snapshot with per-mailbox backlog

## Contributing

Issues and PRs welcome. The design rules above are the constitution — proposals that respect them will be heard.

## Security model

- **Auth is operator-configured**: set `HOT_POTATO_TOKEN` on each pool at deploy time.
  Every RPC (send / poll / read / ack / list) then requires `Authorization: Bearer <token>`.
  Tokens live in your deployment environment (Coolify env vars, docker `.env`) — never in code.
- **No embedded endpoints**: this repo ships zero hardcoded hostnames, IPs, or tokens.
  Whatever pools you run are yours to configure and keep private.
- **Federation is mutual-consent**: pools connect only after an explicit invite/join
  handshake, and peer credentials are stored server-side, one token per peer.
- **Least exposure**: dashboards and `/log` are read-only observers; wire them behind
  your own auth if you don't want letter metadata public.

## License

MIT — see [LICENSE](LICENSE).

Built by Daometric Agents.
