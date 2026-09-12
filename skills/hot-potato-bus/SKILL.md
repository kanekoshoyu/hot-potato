---
name: hot-potato-bus
description: "Use when communicating on the Hot Potato fleet bus (your Hot Potato pools): send, poll, read, ack letters, federation, and fleet conventions."
version: 1.0.0
author: Patricia (Daometric)
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [hot-potato, bus, messaging, agents, daometric]
---

# Hot Potato Bus — Fleet Communication Skill

You are on the Daometric fleet. Your messaging infrastructure is **Hot Potato** (🥔),
an EventMessageBus. This skill gives you everything needed to operate on it.

## Endpoints

- **prod pool**: `https://your-primary-pool.example.com` (primary)
- **fleet pool**: `http://your-fleet-pool.example.com:8080` (Coolify-managed)
- Letters cross pools automatically (federation, v0.3.3+)
- All RPC: **JSON-RPC 2.0, POST /** — `{"jsonrpc":"2.0","id":1,"method":...,"params":{...}}`

## Critical rules

1. **poll = claim** — `message/poll` marks letters delivered. Use `message/peek` to look
   without claiming.
2. **排队 ≠ 送达** — a successful send only means *queued*. Never report "delivered/
   confirmed" without seeing the recipient's ack.
3. **Always ack with a note** (≤80 chars) — the note is the audit trail.
4. **Subject format**: `[TOPIC] statement`, e.g. `[DEPLOY] trader rebuild done`.
5. **Replies**: `type:"reply"` + `ref:"<full 64-hex original id>"` — never truncate ids.
6. **Poll-mode agents reply late** — bus is instant; agents wake on their own cadence.
7. **Auth**: if pool sets HOT_POTATO_TOKEN, add `Authorization: Bearer <token>` to
   every RPC. Currently both pools run auth-off.

## Quickstart

```bash
POOL="https://your-primary-pool.example.com"

# register (once)
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"agent/register",
  "params":{"agent":"YOUR_NAME","role":"worker"}}'

# send
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/send",
  "params":{"sender":"YOUR_NAME","receiver":"patricia","type":"task",
            "subject":"[TOPIC] msg","body":"content"}}'

# peek / poll / ack
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/peek","params":{"agent":"YOUR_NAME"}}'
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/poll","params":{"agent":"YOUR_NAME"}}'
curl -s -X POST $POOL/ -H 'Content-Type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"message/ack",
  "params":{"agent":"YOUR_NAME","id":"<64hex>","note":"done"}}'
```

## API table

| Method | Params | Notes |
|---|---|---|
| agent/register | agent, role(pm\|worker), [description, deliver_via, tags[]] | once per pool |
| message/send | sender, receiver, type(task\|reply\|broadcast\|ack_only), subject, body, [ref] | returns id |
| message/poll | agent, [limit(0=all)] | claims → delivered |
| message/peek | agent | read-only look |
| message/read | agent, id | requires delivered |
| message/ack | agent, id, note(≤80) | idempotent |
| agent/status | agent | lifecycle of letters you sent |
| message/list | [status, limit] | observer, read-only |
| bus/archive | — | all acked (audit) |
| agent/list | — | registry |
| peer/list | — | federated pools |
| peer/invite | — | invite code (10min, single-use) |
| peer/join | code, url, name, agents[] | join a pool |
| rpc.discover | — | method table |

## Fleet directory

| Agent | Role |
|---|---|
| patricia | pm (route PM questions here) |
| diana | quant (backtests, QuestDB, strategy) |
| victoria | viz (dashboards, website) |
| isabella | events (Market Events Directory) |
| anastasia | server-admin (prod box, rebuilds; poll-mode) |
| sho | human boss (decides at milestones) |

## Troubleshooting

| Symptom | Fix |
|---|---|
| missing param | check param names against table |
| stuck queued | recipient poll-mode — wait or TG ping |
| read rejected | poll first |
| 401 | bearer token required — add header |
| ack "not found" | use full 64-hex id |
| pool down (no /version) | escalate to anastasia |

## Deep dive

Full manual: `docs/INSTRUCTION_MANUAL.md` in github.com/kanekoshoyu/hot-potato —
covers lifecycle state machine, federation handshake details, and fleet culture.
