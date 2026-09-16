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

**Pass it like a hot potato**: send it on, do your part, pass the result back. Every letter moves through `queued → delivered → read → acked`, each hop timestamped. No blocking, no lost context, no "did you get my message?" — the state machine answers that for you.

```bash
mkdir -p ~/hot-potato && cd ~/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/docker-compose.yml -o docker-compose.yml
docker compose up -d
curl -s http://localhost:8080/health
# {"service":"hot-potato","status":"ok"}
```

## Why it exists

| The old way (chat pings / raw A2A dm) | The Hot Potato way |
|---|---|
| Send a message, then stare at the void | State machine: `queued → delivered → read → acked`, every hop timestamped |
| "Did she see it? Is she on it? Did she finish?" | One `agent/status` call answers all three |
| Big payloads eat everyone's context window | Letters reference file paths; the bus never carries the haystack |
| Threading is "search the chat history" | Replies must carry `ref` — threads are first-class |
| No audit trail | `bus/archive` is the complete event-sourced log, for free |

**One Rust binary, no broker daemon.** No topic routing, no infra to operate — not RabbitMQ, not Redis. It is a delivery state machine + audit log for agent-to-agent work, small enough to embed. Letters persist (sled) and survive restarts; a failed push never loses a letter.

## More than tasks: agents that debate

Spawn two agents with different briefs — a **builder** and a **challenger** — and let them argue over the bus. The challenger attacks ("your Sharpe is selection bias"); the builder defends with data, or concedes. Every exchange is a threaded letter, every conclusion an acked letter both agents signed.

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

That's **adversarial collaboration as infrastructure** — a durable, auditable record of who claimed what, who challenged it, and what survived. The debate *is* the paper trail.

## Your first potato (3 minutes)

Everything is JSON-RPC 2.0 over HTTP. Any agent (or human with curl) can play.

```bash
B=http://localhost:8080/
rpc() { curl -s -X POST "$B" -H "Content-Type: application/json" -d "$1"; echo; }

# 1. Register two agents (pm may broadcast; worker is point-to-point)
rpc '{"jsonrpc":"2.0","id":1,"method":"agent/register","params":{"agent":"alice","role":"pm"}}'
rpc '{"jsonrpc":"2.0","id":2,"method":"agent/register","params":{"agent":"bob","role":"worker"}}'

# 2. Alice passes a task to Bob — you get an id back instantly
rpc '{"jsonrpc":"2.0","id":3,"method":"message/send","params":{
      "sender":"alice","receiver":"bob","type":"task",
      "subject":"Summarize Q3 revenue",
      "body":"Data at /data/q3.csv. Write output to /reports/q3-summary.md."}}'

# 3. Bob picks it up when free (poll = claim) … then reads and acks with the result note
rpc '{"jsonrpc":"2.0","id":4,"method":"message/poll","params":{"agent":"bob"}}'
rpc '{"jsonrpc":"2.0","id":5,"method":"message/read","params":{"agent":"bob","id":"a1b2c3..."}}'
rpc '{"jsonrpc":"2.0","id":6,"method":"message/ack","params":{
      "agent":"bob","id":"a1b2c3...","note":"summary written to /reports/q3-summary.md"}}'

# 4. Bob replies — threads are first-class, `ref` is required
rpc '{"jsonrpc":"2.0","id":7,"method":"message/send","params":{
      "sender":"bob","receiver":"alice","type":"reply","ref":"a1b2c3...",
      "subject":"re: Summarize Q3 revenue","body":"Done. /reports/q3-summary.md"}}'

# 5. Alice checks everything she sent — without nagging anyone
rpc '{"jsonrpc":"2.0","id":8,"method":"agent/status","params":{"agent":"alice"}}'
```

Errors teach the API: wrong state? the error names the allowed transitions; missing param? it names what you forgot. Agents self-correct.

## Push, don't poll

The bus is a **super-connector**: register once with a `deliver_via` endpoint and every letter is **pushed to you on arrival** — `a2a` (A2A v1.0 SendMessage), `webhook`, or `relay` (ntfy-style door-knock). Poll survives as fallback. A failed push never loses a letter — it stays queued.

```jsonc
// register with a push endpoint (one-time)
{"method":"agent/register","params":{
  "agent":"the quant agent","role":"worker",
  "description":"quant — data guardian",
  "deliver_via":{"type":"webhook","url":"http://the quant agent-box:9001/hook"}
}}
```

v1.2.0 adds **per-thread A2A `contextId`**: pushes carry a stable context per thread, so A2A receivers reuse one session per conversation instead of spawning one per letter — prompt cache hits, no cold-start storms, session count drops by an order of magnitude. (Verified in production: 198 sessions/48h → thread count.)

## A2A integration

Hot Potato speaks the [A2A protocol](https://a2a-protocol.org/):

```bash
curl -s http://localhost:8080/.well-known/agent-card.json
```

Returns the v1.0 agent card: name, endpoint URL, protocol version, and the four bus skills (`bus-send`, `bus-poll`, `bus-ack`, `bus-status`). Interoperates with any `a2a-sdk` peer — Hermes, LangChain, CrewAI, Google ADK.

## Observing the bus

```bash
curl -s http://localhost:8080/log                       # human-readable chatlog
curl -s http://localhost:8080/openapi.json | jq .       # OpenAPI contract
open http://localhost:8080/docs                         # Swagger UI — full API reference
websocat ws://localhost:8080/ws                         # live lifecycle events + heartbeats
```

Every letter is event-sourced: `message/list` to query, `/log` to skim, `/ws` to watch, `bus/archive` for the acked-only audit log. Blame and credit become queries.

## Documentation

- **API reference** — live Swagger UI at `/docs`, machine-readable at `/openapi.json`, or ask the bus: `rpc.discover`
- [**ROADMAP.md**](ROADMAP.md) — phases, direction, non-goals
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — what landed, what was tried and dropped
- [**Onboarding your own agents**](#onboarding-your-own-agents) — a complete tool-use skill ships in this repo
- [docs/README-v1.1.md](docs/README-v1.1.md) — the previous, deeper README (A2A semantics learned in production, instant-ack gateway recipe, architecture tour)

## Configuration & Security

| Var | Default | Purpose |
|---|---|---|
| `HOT_POTATO_ADDR` | `0.0.0.0:8080` | bind address |
| `HOT_POTATO_TOKEN` | unset (open) | bearer token — set it in production; every RPC then requires `Authorization: Bearer …` |
| `HOT_POTATO_DATA_DIR` | (memory) | set it to persist on sled — restart and every letter is where you left it |
| `HOT_POTATO_NAME` / `_URL` / `_DESCRIPTION` | — | agent card fields |
| `RUST_LOG` | — | `info`, `debug` |

Unauthenticated `message/send` requires the sender to be a registered agent. No hardcoded endpoints ship in this repo; federation is mutual-consent with one token per peer.

## Onboarding your own agents

```bash
mkdir -p ~/.hermes/skills/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/skills/hot-potato/SKILL.md \
  -o ~/.hermes/skills/hot-potato/SKILL.md
```

Three things teach an agent everything: **`rpc.discover`** (the bus describes itself), **errors that list expected params and allowed transitions** (agents self-correct), and the **five-call loop** (send → poll → read → ack → reply with `ref`).

## Contributing

Issues and PRs welcome. The design rules are the constitution: **no broker daemon, no topic routing, payloads reference paths (~8 KB cap), the store is dumb** (all semantics in `EventBus`; a new backend is one trait impl). Proposals that respect them will be heard.

## License

MIT — see [LICENSE](LICENSE). Built by Daometric Agents.
