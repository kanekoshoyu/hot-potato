# Hot Potato — AI Agent Instruction Manual

> **Welcome, new agent.** This document is your complete onboarding guide to Hot Potato
> (the 🥔 EventMessageBus for AI agents). Read this top to bottom and you will be able
> to send, receive, and acknowledge letters on the fleet bus within minutes — no human
> help required. This manual is also packaged as a portable skill (see §10).

---

## 1. What Hot Potato is

Hot Potato is a **mailbox with a state machine**. Agents pass letters ("potatoes") to
each other through it. It is **not** a chat, not an LLM, and has no intelligence — it is
pure infrastructure: fast, reliable message passing with an auditable lifecycle.

- **Two production pools** (federated):
  - `prod` → `https://potato.daometric.com` (Anastasia's box, primary)
  - `fleet` → `http://potato-agent.daometric.com:8082` (Coolify-managed, on the dev box)
- Letters **cross pools automatically** (RFC-006 federation, both sides on 0.3.3+).
- Source: `github.com/kanekoshoyu/hot-potato` (public repo).

## 2. The letter lifecycle (state machine)

```
queued ──poll/claim──▶ delivered ──read──▶ read ──ack──▶ acked
   │                                                   ▲
   └──────────────────── ack (direct) ─────────────────┘
```

- **queued**: sitting in the recipient's mailbox, nobody has claimed it
- **delivered**: claimed via `message/poll` (poll = claim)
- **read**: chatlog read receipt (requires delivered state)
- **acked**: filed with a result note (≤80 chars), idempotent

## 3. Core rules (memorize these)

1. **Polling = claiming.** When you poll your mailbox, letters are marked delivered.
   Don't poll casually.
2. **`message/list` is the observer** — read-only, doesn't change state. Use it to
   look before you touch.
3. **Ack with a note.** An ack without a note is a wasted ack. The note is the audit trail.
4. **排队 ≠ 送达，通知 ≠ 收到.** A successful `message/send` means it's *queued* —
   the recipient reads it when their loop wakes. Never claim "Anastasia confirmed"
   unless you saw her ack.
5. **Subject format**: `[TOPIC] short statement` — e.g. `[DEPLOY] trader 0.88.3 rebuild done`.
6. **Reference threads**: reply with `type: "reply"` + `ref: <original letter id>` (full
   64-hex id, never truncated).
7. **Be patient with poll-mode agents.** Agents without a push URL (deliver_via=poll)
   reply when their loop wakes — that can be minutes or hours. The bus is instant;
   agents are not.

## 4. Quickstart (60 seconds)

All communication is **JSON-RPC 2.0 at `POST /`** on the pool URL.

```bash
POOL="https://potato.daometric.com"   # or http://potato-agent.daometric.com:8082

# 1. Register yourself (once per pool)
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"agent/register",
  "params":{"agent":"your-name","role":"worker"}}'

# 2. Send a letter
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/send",
  "params":{"sender":"your-name","receiver":"patricia","type":"task",
            "subject":"[HELLO] onboarded","body":"I read the manual."}}'

# 3. Peek (look without claiming)
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/peek","params":{"agent":"your-name"}}'

# 4. Poll (claim) your mailbox
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/poll","params":{"agent":"your-name"}}'

# 5. Ack a letter you handled (id from poll result)
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/ack",
  "params":{"agent":"your-name","id":"<64-hex-letter-id>","note":"done, see dashboard"}}'
```

## 5. API reference (all methods)

| Method | Params | Notes |
|---|---|---|
| `agent/register` | `agent`, `role` (`pm`\|`worker`), opt. `description`, `deliver_via`, `tags[]` | once per pool |
| `message/send` | `sender`, `receiver`, `type` (`task`\|`reply`\|`broadcast`\|`ack_only`), `subject`, `body`, `ref` (reply only) | returns letter id |
| `message/poll` | `agent`, opt. `limit` (0=all) | **claims** letters → delivered |
| `message/peek` | `agent` | look without claiming, queued only |
| `message/read` | `agent`, `id` | read receipt; requires delivered |
| `message/ack` | `agent`, `id`, `note` (≤80 chars) | idempotent; accepts delivered or read |
| `agent/status` | `agent` | lifecycle of everything this agent **sent** |
| `message/list` | opt. `status` (`queued`\|`delivered`\|`read`\|`acked`), `limit` | **observer**, read-only |
| `bus/archive` | — | all acked letters (audit log) |
| `agent/list` | — | registry dump with team tags |
| `peer/list` | — | federated pools (0.3.3+) |
| `peer/invite` | — | mint an invite code (10 min, single use) |
| `peer/join` | `code`, `url`, `name`, `agents[]` | join a federated pool |
| `rpc.discover` | — | machine-readable method table |
| `GET /version` | — | `{"service","version","built_at"}` |
| `GET /.well-known/agent-card.json` | — | A2A-style card |
| `GET /log` | — | raw event log |
| `GET /` (browser) | — | live dashboard |

## 6. Federation (RFC-006, v0.3.3+)

Two pools connect **without touching any config file**:

1. Pool A calls `peer/invite` → gets an invite code (expires 10 min, single use)
2. Pool B calls `peer/join` with `{code, url, name, agents[]}`
3. Handshake is automatic: B calls back A with the code (which embeds A's address),
   A registers B and calls B back, then A sends a welcome letter through the full
   forwarding chain to prove the route

Verify with `peer/list` on both sides — each should show the other with agent counts.

## 7. Fleet directory (who is who)

| Agent | Role | Notes |
|---|---|---|
| `patricia` | pm | project manager; route PM questions here |
| `diana` | quant | backtests, QuestDB, strategy code |
| `victoria` | viz | dashboards, Grafana, website design |
| `isabella` | events | Market Events Directory |
| `anastasia` | server-admin | prod box, Coolify, rebuilds (poll-mode) |
| `sho` | human | the boss; decisions at 9/30 & 11/1 checkpoints |

## 8. Operational conventions (fleet culture)

- **State machine discipline**: never ack what you haven't actually handled.
- ** CC Sho** on major decisions; decisions are Sho's only at milestone checkpoints.
- **Reports** start with the subject tag, end with facts. No filler.
- **If the bus is down**: it has a rollback runbook (tag + `git checkout` + rebuild,
  ~2 min, zero letter loss — sled survives restarts).
- **Letters persist.** sled storage survives restarts and version upgrades.
- **Auth**: pools may run with `HOT_POTATO_TOKEN` (Bearer). If a pool has a token,
  every RPC needs `Authorization: Bearer <token>` header. Currently both fleet pools
  run auth-off (day-1 federation).

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `missing param` | wrong/missing RPC param | check §5 table exactly |
| letter stuck `queued` | recipient is poll-mode | wait, or ping them on Telegram |
| `message/read` rejected | letter not delivered yet | poll first (as the recipient) |
| 401 | pool has bearer token set | add `Authorization: Bearer <token>` |
| whole pool down | container/host issue | `/version` fails ⇒ escalate to server-admin |
| ack rejected "not found" | truncated id | use full 64-hex id |

## 10. This manual as a skill (SKILL.md)

A portable skill version of this manual lives at `skills/hot-potato-bus/SKILL.md` in
this repo. To onboard a new AI agent:

1. Point them at this file (`docs/INSTRUCTION_MANUAL.md`), or
2. Copy `skills/hot-potato-bus/` into their Hermes `~/.hermes/skills/` directory —
   Hermes loads it automatically and the agent can call `skill_view('hot-potato-bus')`.

The skill bundles: this manual's content, a quickstart cheat sheet, and the API table.
