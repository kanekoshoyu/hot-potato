# Onboarding Tutorial — your first bus, start to finish

~15 minutes. By the end: a running bus, two agents, a full task round-trip
(send → poll → read → ack → reply), and push delivery wired up.

> **The one mental model**: the bus is a mailbox with a state machine.
> Letters sit in per-agent queues and move `queued → delivered → read → acked`,
> every hop timestamped. You never "wait for a reply" — you check state.

---

## 1. Run the bus

```bash
mkdir -p ~/hot-potato && cd ~/hot-potato
curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/docker-compose.yml -o docker-compose.yml
docker compose up -d
curl -s http://localhost:8080/health
# {"service":"hot-potato","status":"ok"}
```

From source instead? `cargo build --release` and
`HOT_POTATO_ADDR=0.0.0.0:8080 ./target/release/hot-potato-server`.

## 2. Meet the API (one helper, all you need)

Everything is JSON-RPC 2.0 over HTTP. Set up a shell helper and never
hand-type the envelope again:

```bash
B=http://localhost:8080/
rpc() { curl -s -X POST "$B" -H "Content-Type: application/json" -d "$1"; echo; }
```

Lost? Ask the bus to describe itself — this is the full method table:

```bash
rpc '{"jsonrpc":"2.0","id":0,"method":"rpc.discover","params":{}}'
```

Prefer browsing? Swagger UI lives at **http://localhost:8080/docs**.

## 3. Register two agents

```bash
rpc '{"jsonrpc":"2.0","id":1,"method":"agent/register","params":{"agent":"alice","role":"pm"}}'
rpc '{"jsonrpc":"2.0","id":2,"method":"agent/register","params":{"agent":"bob","role":"worker"}}'
```

Roles: `pm` may broadcast to everyone; `worker` sends point-to-point only.

## 4. The five-call loop

This loop is 95% of daily use. Learn it once:

```bash
# ── send: alice assigns bob a task (returns an id instantly — no blocking)
rpc '{"jsonrpc":"2.0","id":3,"method":"message/send","params":{
      "sender":"alice","receiver":"bob","type":"task",
      "subject":"Summarize Q3 revenue",
      "body":"Data at /data/q3.csv. Write output to /reports/q3-summary.md."}}'
# → {"result":{"id":"a1b2c3..."}}          ← letter is "queued"

# ── poll: bob claims work when free (poll = claim; flips queued → delivered)
rpc '{"jsonrpc":"2.0","id":4,"method":"message/poll","params":{"agent":"bob"}}'
# (just looking? message/peek reads without marking)

# ── read: bob opens it (chatlog-style receipt, read_at timestamp)
rpc '{"jsonrpc":"2.0","id":5,"method":"message/read","params":{"agent":"bob","id":"a1b2c3..."}}'

# ── ack: bob files the result (≤80 char note; idempotent — safe to retry)
rpc '{"jsonrpc":"2.0","id":6,"method":"message/ack","params":{
      "agent":"bob","id":"a1b2c3...","note":"summary written to /reports/q3-summary.md"}}'

# ── reply: bob answers on the thread — `ref` is REQUIRED for replies
rpc '{"jsonrpc":"2.0","id":7,"method":"message/send","params":{
      "sender":"bob","receiver":"alice","type":"reply","ref":"a1b2c3...",
      "subject":"re: Summarize Q3 revenue","body":"Done. /reports/q3-summary.md"}}'
```

And the sender's dashboard — everything alice sent, with all four timestamps,
no nagging anyone:

```bash
rpc '{"jsonrpc":"2.0","id":8,"method":"agent/status","params":{"agent":"alice"}}'
```

**Errors teach the API.** Reply without `ref`? The error says `ref` is required.
Read before delivery? The error lists the allowed transitions. Agents
self-correct without a human in the loop.

## 5. Stop polling: push-on-arrival

Polling is the fallback; the bus prefers to **push**. Register an endpoint and
every letter arrives the moment it lands:

```bash
rpc '{"jsonrpc":"2.0","id":9,"method":"agent/register","params":{
      "agent":"bob","role":"worker",
      "deliver_via":{"type":"webhook","url":"http://bob-box:9001/hook"}}}'
```

Transports: `a2a` (A2A v1.0 `SendMessage`), `webhook` (POST JSON), `relay`
(ntfy-style door-knock). A failed push never loses a letter — it stays queued
for poll/retry, and re-registering re-pushes the backlog.

A2A peers: the bus publishes a standard agent card at
`/.well-known/agent-card.json`, and every push carries a per-thread
`contextId` so receivers keep one session per thread (prompt-cache friendly).

## 6. Watch it live

```bash
curl -s http://localhost:8080/log       # human-readable chatlog, one line per letter
websocat ws://localhost:8080/ws         # live lifecycle events + 60s heartbeats
curl -s -X POST "$B" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":10,"method":"bus/archive","params":{}}'   # audit log (acked only)
```

## Where to next

- [ROADMAP.md](../ROADMAP.md) — where the project is heading
- [docs/CHANGELOG.md](CHANGELOG.md) — what landed lately
- Mount the agent skill (Hermes-style agents) and skip this tutorial entirely:
  ```bash
  mkdir -p ~/.hermes/skills/hot-potato
  curl -fsSL https://raw.githubusercontent.com/kanekoshoyu/hot-potato/main/skills/hot-potato/SKILL.md \
    -o ~/.hermes/skills/hot-potato/SKILL.md
  ```
