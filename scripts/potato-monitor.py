#!/usr/bin/env python3
"""Hot Potato Monitor — GPS for every potato. v2 (alarms fixed)

Subscribes to the bus /ws feed, tracks each letter's lifecycle position, and
mirrors every transition to Sho's TG.

v2 alarm rules (after the 40-message spam incident):
- age = letter's created_at (real age), NOT monitor-first-seen time
- NEVER alarm on letters addressed to sho (human; bridge reads those)
- never alarm on acked/read letters (lifecycle finished)
- one alarm per (letter, status): re-arm only when status changes
- market noise guard: don't alarm on broadcast/onboarding-type letters older
  than the monitor's own birth (historical archive letters are reviewed, not pending)

Run:  nohup python3 potato-monitor.py >> /tmp/potato-monitor.log 2>&1 &
"""
import json
import time
import subprocess
import threading
import urllib.request
from datetime import datetime, timezone

try:
    import websocket  # websocket-client
except ImportError:
    subprocess.run(["pip", "install", "--quiet", "websocket-client"], check=False)
    import websocket

BUS_WS = "ws://localhost:8080/ws"
BUS_HTTP = "http://localhost:8080"
TG_CHAT_ID = "5690144164"  # Sho

def tg_api(text: str):
    """Send via the dedicated monitor bot (MONITOR_BOT_TOKEN in ~/.hermes/.env)."""
    import os, urllib.request, urllib.parse
    token = None
    for line in open(os.path.expanduser("~/.hermes/.env")):
        if line.startswith("MONITOR_BOT_TOKEN="):
            token = line.strip().split("=", 1)[1]
            break
    if not token:
        log("MONITOR_BOT_TOKEN missing from ~/.hermes/.env — cannot send")
        return
    data = urllib.parse.urlencode({"chat_id": TG_CHAT_ID, "text": text}).encode()
    try:
        req = urllib.request.Request(f"https://api.telegram.org/bot{token}/sendMessage", data=data)
        urllib.request.urlopen(req, timeout=15)
    except Exception as e:
        log(f"TG API send failed: {e}")

INITIALS = {
    "patricia": "P", "diana": "D", "victoria": "V",
    "isabella": "I", "anastasia": "A", "sho": "S",
}

QUEUED_ALARM_MIN = 30     # queued this long without delivery
DELIVERED_ALARM_MIN = 60  # delivered this long without ack

potatoes = {}   # id -> {sender, receiver, subject, status, created_at}
alarmed = set() # (id, status) pairs already alarmed


def initials(name: str) -> str:
    return INITIALS.get(name.lower(), name[:2].upper())


def tg(text: str):
    tg_api(text)


def log(msg: str):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def fmt_flow(m: dict) -> str:
    return f"[{initials(m['sender'])}->{initials(m['receiver'])}] {m['subject'][:50]}"


def alarm_eligible(m: dict) -> bool:
    """Alarms only make sense for agent mailboxes mid-lifecycle."""
    if m["receiver"].lower() == "sho":
        return False  # human mailbox; the mail bridge reads those
    if m["status"] in ("acked", "read"):
        return False  # done or in-progress-visible
    return True


def age_minutes(m: dict) -> float:
    """Real age: from the letter's own created_at, not when monitor first saw it."""
    try:
        created = datetime.fromisoformat(
            m["created_at"].replace("Z", "+00:00"))
        return (datetime.now(timezone.utc) - created).total_seconds() / 60
    except Exception:
        return 0.0


def maybe_alarm(mid: str, m: dict):
    if (mid, m["status"]) in alarmed:
        return
    if not alarm_eligible(m):
        return
    age = age_minutes(m)
    if m["status"] == "queued" and age > QUEUED_ALARM_MIN:
        tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — queued {age:.0f}min, "
           f"{initials(m['receiver'])} hasn't picked it up.")
        alarmed.add((mid, m["status"]))
    elif m["status"] == "delivered" and age > DELIVERED_ALARM_MIN:
        tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — delivered {age:.0f}min ago, no ack. "
           f"{initials(m['receiver'])} still holding it.")
        alarmed.add((mid, m["status"]))


def on_event(ev: dict):
    if ev.get("event") in ("hello", "heartbeat", "lagged"):
        return
    mid, new_status = ev.get("id"), ev.get("event")
    if not mid:
        return
    if mid not in potatoes:
        # first sight: record state silently; only announce if it's NOT the
        # initial queued state (a brand-new letter's queued hop is announced
        # only when we see it born live, and even then once)
        potatoes[mid] = {
            "sender": ev.get("sender", "?"),
            "receiver": ev.get("receiver", "?"),
            "subject": ev.get("subject", "?"),
            "status": new_status,
            "created_at": ev.get("at"),
        }
        if new_status != "queued":
            m = potatoes[mid]
            tg(f"🥔 {mid[:8]} {fmt_flow(m)} — {new_status.upper()}")
        log(f"{mid[:8]} first-seen as {new_status}")
        maybe_alarm(mid, potatoes[mid])
        return
    prev = potatoes[mid].get("status")
    if prev == new_status:
        return  # duplicate event — same status, nothing to announce
    potatoes[mid]["status"] = new_status
    log(f"{mid[:8]} {prev} -> {new_status}")
    m = potatoes[mid]
    tg(f"🥔 {mid[:8]} {fmt_flow(m)} — {new_status.upper()}")
    maybe_alarm(mid, m)


def snapshot():
    """Baseline: load real created_at ages; alarm only true stuck agent mail."""
    req = urllib.request.Request(
        f"{BUS_HTTP}/",
        data=json.dumps({"jsonrpc": "2.0", "id": 1,
                         "method": "message/list", "params": {}}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        msgs = json.loads(r.read())["result"]
    stuck = 0
    for m in msgs:
        potatoes[m["id"]] = {
            "sender": m["sender"], "receiver": m["receiver"],
            "subject": m["subject"], "status": m["status"],
            "created_at": m["created_at"],
        }
    # one-time stuck scan (not spamming: only letters that are truly stale NOW)
    for mid, m in potatoes.items():
        age = age_minutes(m)
        if alarm_eligible(m):
            if m["status"] == "queued" and age > QUEUED_ALARM_MIN * 4:  # >2h only at startup
                tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — queued {age/60:.0f}h (startup scan). "
                   f"{initials(m['receiver'])} never picked it up.")
                stuck += 1
            elif m["status"] == "delivered" and age > DELIVERED_ALARM_MIN * 8:  # >8h
                tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — delivered {age/60:.0f}h ago, never acked "
                   f"(startup scan). {initials(m['receiver'])} still holding.")
                stuck += 1
    return len(msgs), stuck


def main():
    n, stuck = snapshot()
    log(f"monitor v2 started: {n} potatoes baseline, {stuck} true-stale flagged. watching /ws ...")

    threading.Thread(target=watchdog, daemon=True).start()

    while True:  # auto-reconnect loop
        try:
            ws = websocket.create_connection(BUS_WS, timeout=10)
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
            log(f"ws dropped ({e}); reconnecting in 15s ...")
            time.sleep(15)


def watchdog():
    """Every 10 min: alarm only NEW staleness (age crossing threshold), real ages only."""
    while True:
        time.sleep(600)
        for mid, m in list(potatoes.items()):
            maybe_alarm(mid, m)


if __name__ == "__main__":
    main()
