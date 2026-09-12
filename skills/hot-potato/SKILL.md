---
name: hot-potato
description: Use when an AI agent needs to install and use Hot Potato (EventMessageBus) to exchange tasks/results with other agents over a mailbox-with-state-machine. Covers Docker deployment, JSON-RPC API, and the full hot-potato workflow (send → poll → read → ack → reply).
---

# Hot Potato — EventMessageBus Skill

Pass messages like a hot potato: send it, work your part, pass the result back.
No waiting, no polling humans, no lost context. The bus tracks everything.

## What it is

A Rust-based message bus for AI agents, deployed as one Docker container:
- **Mailbox per agent** — not a queue, not pub/sub. Letters wait in your box.
- **State machine** — every message moves `queued → delivered → read → acked`
  with a timestamp at each hop (built-in chatlog).
- **JSON-RPC 2.0 over HTTP** — any agent that can POST JSON can use it.
- **A2A-compatible discovery** — agent card at `/.well-known/agent-card.json`.

Repo: https://github.com/kanekoshoyu/hot-potato

## Install (one command, Docker required)

```bash
# on any machine with Docker
mkdir -p ~/hot-potato && cd ~/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/docker-compose.yml -o docker-compose.yml
docker compose up -d
# verify
curl -s http://localhost:8080/health
# expected: {"service":"hot-potato","status":"ok"}
```

No Docker? Build from source (Rust 1.75+):
```bash
git clone https://github.com/kanekoshoyu/hot-potato && cd hot-potato
cargo build --release -p hot-potato --bin hot-potato-server
HOT_POTATO_ADDR=0.0.0.0:8080 ./target/release/hot-potato-server
```

## Registration

The compose image can pre-register example agents (e.g. alice, bob, carol,
isabella, anastasia, sho. For your own fleet, register your agents first:

```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"agent/register","params":{"agent":"alice","role":"pm"}}'
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"agent/register","params":{"agent":"bob","role":"worker"}}'
```

Roles: `pm` may broadcast; `worker` sends point-to-point only.

## The workflow (memorize this loop)

**Sender** — pass the potato:
```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"message/send","params":{
        "sender":"alice","receiver":"bob","type":"task",
        "subject":"One-line summary",
        "body":"Full context. Reference files by path, do not inline big data."}}'
# → {"result":{"id":"<message_id>"}}
```

**Receiver** — when free, pick it up:
```bash
# 1. drain your mailbox (marks delivered)
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"message/poll","params":{"agent":"bob"}}'
# 2. open the letter (chatlog read receipt)
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"message/read","params":{"agent":"bob","id":"<id>"}}'
# 3. do the work, then file the result (≤80 char note)
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"message/ack","params":{"agent":"bob","id":"<id>","note":"done, output at /path/out.csv"}}'
# 4. reply (must carry ref = original id; threads are first-class)
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"message/send","params":{
        "sender":"bob","receiver":"alice","type":"reply","ref":"<id>",
        "subject":"re: One-line summary","body":"Result summary + artifact path."}}'
```

**Sender** — check without nagging:
```bash
curl -s -X POST http://localhost:8080/ -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":6,"method":"agent/status","params":{"agent":"alice"}}'
# every letter you sent, with its current lifecycle state + all timestamps
```

## Rules (violating these is a bug, not a style choice)

1. **Reply must carry `ref`.** Unthreaded replies are rejected by the bus.
2. **Never inline big payloads.** Body references file paths; keep letters < 8 KB.
3. **Ack means done**, not "I saw it". The ack_note is the deliverable summary.
4. **Broadcast is PM-only.** Workers send to named peers.
5. **Check status before re-sending.** `queued` = they have not polled yet;
   `delivered`/`read` = it is in their hands, give them time.

## Discovery

```bash
curl -s http://localhost:8080/.well-known/agent-card.json
```
Returns the A2A v1.0 agent card: name, endpoint URL, and the four bus skills
(send / poll / ack / status) with JSON-RPC method names.

## Auth (recommended if not on localhost)

Set `HOT_POTATO_TOKEN=your-secret` in docker-compose.yml, then add
`-H "Authorization: Bearer your-secret"` to every request.
401 response = missing/wrong token.

## Environment variables (all optional)

| Var | Default | Purpose |
|---|---|---|
| `HOT_POTATO_ADDR` | `0.0.0.0:8080` | bind address |
| `HOT_POTATO_NAME` | `hot-potato` | agent card name |
| `HOT_POTATO_DESCRIPTION` | see repo | agent card blurb |
| `HOT_POTATO_URL` | `http://localhost:8080` | public URL in the card |
| `HOT_POTATO_TOKEN` | unset (open) | bearer token; set it in production |

## Troubleshooting

- **`unknown agent` error** → the sender/receiver was never registered. Call `agent/register`.
- **`missing param: agent`** → your JSON body is missing `"agent"` in params.
- **`not in delivered state` on ack** → poll before acking; you cannot file a letter you never picked up.
- **401** → token mismatch; check `Authorization: Bearer <token>` header.
- **Container restarts lose mail** → in-memory store is intentional (v0.1).
  For durability, persist or wait for the sled store (roadmap).
