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

Born inside [Daometric](https://daometric.com) to coordinate a fleet of AI agents (a PM, a quant, a viz engineer, a server admin) that were drowning each other in chat pings. Chats are for talking; **Hot Potato is for work**.

---

## Why it exists

| The old way (chat pings / raw A2A dm) | The Hot Potato way |
|---|---|
| Send a message, then stare at the void | State machine: `queued → delivered → read → acked`, every hop timestamped |
| "Did Diana see it? Is she on it? Did she finish?" | One `agent/status` call answers all three |
| Big payloads eat everyone's context window | Letters reference file paths; the bus never carries the haystack |
| Threading is "search the chat history" | Replies must carry `ref` — threads are first-class |
| No audit trail | `bus/archive` is the complete event-sourced log, for free |

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

That's it. The bus now pre-registers a default roster (patricia, diana, victoria, isabella, anastasia, sho — the Daometric crew). Rename them or register your own agents (below).

## Quick Start (from source)

Requires Rust 1.75+:

```bash
git clone https://github.com/kanekoshoyu/hot-potato && cd hot-potato
cargo build --release -p hot-potato --bin hot-potato-server
HOT_POTATO_ADDR=0.0.0.0:8080 ./target/release/hot-potato-server
```

## Your first potato (3 minutes)

Everything is JSON-RPC 2.0 over HTTP. Any agent (or human with curl) can play.

**1. Register two agents** (skip if using the default roster):

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

**Pairing it with direct agent-to-agent channels** (the Daometric topology): chats/A2A dm are for *talking*, Hot Potato is for *work*. When a conversation produces a task, pass it as a potato; when the potato is acked, the result note tells the chat what happened.

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
| `agent/register` | `agent`, `role` (`pm`\|`worker`) | idempotent |
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
4. **The store is dumb.** All semantics live in `EventBus`; a new backend is one trait impl. In-memory is the default (v0.1); sled/Redis backends are trait-implementations away, not rewrites.

## Architecture

```
src/
├── lib.rs        # public API surface
├── error.rs      # BusError — errors that teach the API (transitions, param names)
├── message.rs    # Message, MessageStatus, MsgType (serde, snake_case wire format)
├── bus.rs        # EventBus — rules, roles, threading, broadcast fan-out
├── server.rs     # axum shell: agent card, JSON-RPC, auth, rpc.discover
├── main.rs       # container entrypoint (seeds the Daometric roster)
└── store/
    ├── mod.rs    # BusStore trait — the storage abstraction
    └── memory.rs # InMemoryStore — the default backend
```

**17 tests** cover the full lifecycle, role gates, threading, idempotency, timestamp ordering, HTTP round-trips, and auth.

## Battle-tested

Hot Potato's first production workload was Daometric's M1 adversarial-collaboration loop: a PM agent challenging a quant agent's backtest conclusions (denominator definitions, overfitting audits) with every exchange flowing through the bus. The friction report from that agent's first live session drove the v0.1 fixes: `peek`, named-param errors, `rpc.discover`, and ack idempotency.

## Roadmap

- [x] Core bus: lifecycle, roles, threading, broadcast, audit
- [x] Read receipts with full chatlog timestamps
- [x] A2A connection layer (agent card + JSON-RPC transport)
- [x] Docker Compose distribution
- [x] `rpc.discover` introspection + ack idempotency
- [ ] Persistence-backed store (sled) for restart survival
- [ ] Mailer: notify agents on new mail via their existing channels
- [ ] Skill/tutorial packs for common agent frameworks

## For agent builders

Teaching your AI to use Hot Potato? Point it at three things:

1. **`rpc.discover`** — the bus describes its own API at runtime.
2. **Error messages** — state errors list allowed transitions; param errors list expected params. An agent that reads its errors can self-correct without a human.
3. **The loop** — send → poll → read → ack → reply(`ref`). Five calls, one mental model.

## Contributing

Issues and PRs welcome at [github.com/kanekoshoyu/hot-potato](https://github.com/kanekoshoyu/hot-potato). The design rules above are the constitution — proposals that respect them will be heard.

## License

MIT — see [LICENSE](LICENSE).

Built by [Daometric](https://daometric.com) — *a place full of love and respect for intelligence.*
