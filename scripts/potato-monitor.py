#!/usr/bin/env python3
"""Hot Potato Monitor v3 — GPS for every potato, spam-proof.

v3 fixes (after two spam incidents):
1. Single-instance lock via pidfile — second instance exits immediately.
2. Alarmed-state persisted to disk — restarts never re-alarm.
3. Startup scan NEVER alarms: baseline letters are silent history. Only
   letters BORN while the monitor runs (or transitions seen live) can
   trigger alarms, and each (id,status) alarms at most once, ever.
4. Letters to sho excluded (human mailbox, bridge reads them).

Sends via dedicated bot: MONITOR_BOT_TOKEN in ~/.hermes/.env -> chat 5690144164.
"""
import json
import os
import time
import threading
import urllib.parse
import urllib.request
from datetime import datetime, timezone

import websocket  # websocket-client

BUS_WS = "wss://potato.daometric.com/ws"   # via Traefik since bus move (2026-09-12)
BUS_HTTP = "https://potato.daometric.com"
TG_CHAT_ID = "5690144164"
STATE_PATH = "/tmp/potato-monitor-state.json"
PID_PATH = "/tmp/potato-monitor.pid"

INITIALS = {"patricia": "P", "diana": "D", "victoria": "V",
            "isabella": "I", "anastasia": "A", "sho": "S"}

QUEUED_ALARM_MIN = 30
DELIVERED_ALARM_MIN = 60

potatoes = {}   # id -> {sender, receiver, subject, status}
alarmed = set() # (id, status) — persisted
known = set()   # ids present at startup = silent history


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def tg(text):
    token = None
    for key in ("MONITOR_BOT_TOKEN", "TELEGRAM_BOT_TOKEN"):  # fallback: main bot
        for line in open(os.path.expanduser("~/.hermes/.env")):
            if line.startswith(key + "="):
                token = line.strip().split("=", 1)[1]
                break
        if token:
            break
    if not token:
        log("MONITOR_BOT_TOKEN missing — cannot send")
        return
    data = urllib.parse.urlencode({"chat_id": TG_CHAT_ID, "text": text}).encode()
    try:
        urllib.request.urlopen(
            urllib.request.Request(f"https://api.telegram.org/bot{token}/sendMessage", data=data),
            timeout=15)
    except Exception as e:
        log(f"TG failed: {e}")


def save_state():
    json.dump({"alarmed": sorted(f"{i}|{s}" for i, s in alarmed),
               "potatoes": potatoes},
              open(STATE_PATH, "w"))


def load_state():
    global alarmed, potatoes
    if os.path.exists(STATE_PATH):
        d = json.load(open(STATE_PATH))
        alarmed = {tuple(x.split("|", 1)) for x in d.get("alarmed", [])}
        potatoes = d.get("potatoes", {})
        log(f"state restored: {len(potatoes)} potatoes, {len(alarmed)} alarms remembered")


def initials(n):
    return INITIALS.get(n.lower(), n[:2].upper())


def fmt(m):
    """Three-line layout per Sho:
    line1 = pointer + direction, line2 = topic, line3 = snippet + status."""
    subj = m.get("subject", "?")
    # strip leading direction-tag duplication like "[P->A]" inside the subject
    import re as _re
    subj = _re.sub(r"^\s*(🥔\s*)?\[[A-Za-z]->[A-Za-z]\]\s*", "", subj)
    snippet = m.get("snippet") or ""
    line1 = f"{m['_id8']} {initials(m['sender'])}→{initials(m['receiver'])}"
    line2 = subj[:72]
    line3 = (snippet[:90] + " …") if len(snippet) > 90 else snippet
    return f"{line1}\n{line2}" + (f"\n{line3}" if line3 else "")


def _bus_token():
    for line in open(os.path.expanduser("~/.hermes/.env")):
        if line.startswith("HOT_POTATO_TOKEN="):
            return line.strip().split("=", 1)[1]
    return None

BUS_TOKEN = _bus_token()

def bus(payload):
    headers = {"Content-Type": "application/json"}
    if BUS_TOKEN:
        headers["Authorization"] = f"Bearer {BUS_TOKEN}"
    req = urllib.request.Request(
        f"{BUS_HTTP}/", data=json.dumps(payload).encode(), headers=headers)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())


def age_min(created_at):
    try:
        d = datetime.fromisoformat(created_at.replace("Z", "+00:00"))
        return (datetime.now(timezone.utc) - d).total_seconds() / 60
    except Exception:
        return 0.0


def on_event(ev):
    ev_type = ev.get("event")
    if ev_type in ("hello", "heartbeat", "lagged"):
        return
    mid = ev.get("id")
    if not mid:
        return

    if mid not in potatoes:
        # letter born while monitor runs — this one is LIVE and can alarm later
        potatoes[mid] = {"sender": ev.get("sender", "?"),
                         "receiver": ev.get("receiver", "?"),
                         "subject": ev.get("subject", "?"),
                         "snippet": ev.get("body", "") or ev.get("snippet", ""),
                         "_id8": mid[:8],
                         "status": ev_type}
        if ev_type == "queued":
            tg(f"🥔 NEW\n{fmt(potatoes[mid])}\n— queued")
        log(f"{mid[:8]} new letter ({ev_type})")
        save_state()
        return

    prev = potatoes[mid].get("status")
    if prev == ev_type:
        return  # duplicate event
    potatoes[mid]["status"] = ev_type
    log(f"{mid[:8]} {prev} -> {ev_type}")
    # Noise reduction per Sho: read/ack always arrive together in practice.
    # Only report the terminal state (acked); skip delivered + read entirely.
    if ev_type == "acked":
        tg(f"🥔 OK\n{fmt(potatoes[mid])}\n— acked ✓")
    maybe_alarm_stuck(mid)
    save_state()


def maybe_alarm_stuck(mid):
    """Alarm only live-born letters stuck mid-lifecycle, once per (id,status)."""
    m = potatoes.get(mid)
    if not m or (mid, m["status"]) in alarmed:
        return
    if m["receiver"].lower() == "sho" or m["status"] in ("acked", "read"):
        return
    created = m.get("created_at")
    if not created:
        return
    age = age_min(created)
    if m["status"] == "queued" and age > QUEUED_ALARM_MIN:
        tg(f"⚠️ STUCK\n{fmt(m)}\n— queued {age:.0f}min, "
           f"{initials(m['receiver'])} hasn't picked it up.")
        alarmed.add((mid, m["status"]))
        save_state()
    elif m["status"] == "delivered" and age > DELIVERED_ALARM_MIN:
        tg(f"⚠️ STUCK\n{fmt(m)}\n— delivered {age:.0f}min, no ack. "
           f"{initials(m['receiver'])} still holding it.")
        alarmed.add((mid, m["status"]))
        save_state()


def watchdog():
    while True:
        time.sleep(600)
        for mid in list(potatoes):
            maybe_alarm_stuck(mid)


def acquire_lock():
    if os.path.exists(PID_PATH):
        old = open(PID_PATH).read().strip()
        try:
            os.kill(int(old), 0)  # alive?
            log(f"another monitor (pid {old}) already running — exiting")
            raise SystemExit(0)
        except (ProcessLookupError, ValueError):
            pass  # stale pidfile
    open(PID_PATH, "w").write(str(os.getpid()))


def main():
    acquire_lock()
    load_state()

    # baseline: everything currently on the bus is SILENT history
    msgs = bus({"jsonrpc": "2.0", "id": 1, "method": "message/list", "params": {}})["result"]
    for m in msgs:
        if m["id"] not in potatoes:
            potatoes[m["id"]] = {"sender": m["sender"], "receiver": m["receiver"],
                                 "subject": m["subject"], "status": m["status"]}
    save_state()
    log(f"monitor v3 started: {len(potatoes)} potatoes as silent baseline. watching /ws ...")
    tg("🥔 Monitor v3 online (spam-proof: single instance, persisted alarms, live-born letters only).")

    threading.Thread(target=watchdog, daemon=True).start()
    while True:
        try:
            ws = websocket.create_connection(
                BUS_WS, timeout=10,
                header=[f"Authorization: Bearer {BUS_TOKEN}"] if BUS_TOKEN else None)
            ws.settimeout(30)
            log("connected to /ws")
            while True:
                try:
                    raw = ws.recv()
                except websocket.WebSocketTimeoutException:
                    ws.ping()
                    continue
                if not raw:
                    continue
                try:
                    on_event(json.loads(raw))
                except (json.JSONDecodeError, TypeError):
                    pass
        except Exception as e:
            log(f"ws dropped ({e}); reconnect in 15s")
            time.sleep(15)


if __name__ == "__main__":
    main()
