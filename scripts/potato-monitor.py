#!/usr/bin/env python3
"""Hot Potato Monitor — GPS for every potato.

Subscribes to the bus /ws feed, tracks each letter's lifecycle position, and
mirrors every transition to Sho's TG. Stuck-potato alarms: queued >30min or
delivered >60min without ack.

Non-agent by design: a dumb, reliable pipe. No LLM, no opinions.

Run:  nohup python3 potato-monitor.py >> /tmp/potato-monitor.log 2>&1 &
Stop: pkill -f potato-monitor.py
"""
import json
import time
import subprocess
import threading
import urllib.request

try:
    import websocket  # websocket-client
except ImportError:
    subprocess.run(["pip", "install", "--quiet", "websocket-client"], check=False)
    import websocket

BUS_WS = "ws://localhost:8080/ws"
BUS_HTTP = "http://localhost:8080"
TG_TARGET = "telegram:5690144164"

INITIALS = {
    "patricia": "P", "diana": "D", "victoria": "V",
    "isabella": "I", "anastasia": "A", "sho": "S",
}

# letter_id -> {"sender","receiver","subject","status","last_change"}
potatoes = {}
# letter_id -> epoch when it entered current status
entered = {}


def initials(name: str) -> str:
    return INITIALS.get(name.lower(), name[:2].upper())


def tg(text: str):
    try:
        subprocess.run(
            ["hermes", "send", "--to", TG_TARGET, text],
            capture_output=True, text=True, timeout=60,
        )
    except Exception as e:  # never die on TG hiccups
        log(f"TG send failed: {e}")


def log(msg: str):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def fmt_flow(m: dict) -> str:
    return f"[{initials(m['sender'])}->{initials(m['receiver'])}] {m['subject'][:50]}"


def on_event(ev: dict):
    if ev.get("event") in ("hello", "heartbeat", "lagged"):
        return
    mid, new_status = ev.get("id"), ev.get("event")
    if not mid:
        return
    prev = potatoes.get(mid, {}).get("status")
    if prev == new_status:
        return
    if mid not in potatoes:
        potatoes[mid] = {
            "sender": ev.get("sender", "?"),
            "receiver": ev.get("receiver", "?"),
            "subject": ev.get("subject", "?"),
            "status": new_status,
        }
        entered[mid] = time.time()
    potatoes[mid]["status"] = new_status
    entered[mid] = time.time()
    m = potatoes[mid]
    log(f"{mid[:8]} {prev} -> {new_status}")
    tg(f"🥔 {mid[:8]} {fmt_flow(m)} — {new_status.upper()}")


def watchdog():
    """Every 5 min: flag stuck potatoes (queued>30m, delivered>60m)."""
    while True:
        time.sleep(300)
        now = time.time()
        for mid, m in potatoes.items():
            age_min = (now - entered.get(mid, now)) / 60
            st = m["status"]
            if st == "queued" and age_min > 30:
                tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — QUEUED for {age_min:.0f}min. "
                   f"{initials(m['receiver'])} hasn't picked it up.")
                entered[mid] = now  # re-arm so we don't spam every 5 min
            elif st == "delivered" and age_min > 60:
                tg(f"⚠️ 🥔 {mid[:8]} {fmt_flow(m)} — DELIVERED {age_min:.0f}min ago, "
                   f"no ack. {initials(m['receiver'])} still holding it.")
                entered[mid] = now


def snapshot():
    """Full potato table on demand (also used as startup baseline, silent)."""
    req = urllib.request.Request(
        f"{BUS_HTTP}/",
        data=json.dumps({"jsonrpc": "2.0", "id": 1,
                         "method": "message/list", "params": {}}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        msgs = json.loads(r.read())["result"]
    for m in msgs:
        potatoes[m["id"]] = {
            "sender": m["sender"], "receiver": m["receiver"],
            "subject": m["subject"], "status": m["status"],
        }
        entered.setdefault(m["id"], time.time())
    return len(msgs)


def main():
    n = snapshot()
    log(f"monitor started, baseline {n} potatoes loaded. watching /ws ...")

    threading.Thread(target=watchdog, daemon=True).start()

    while True:  # auto-reconnect loop
        try:
            ws = websocket.create_connection(BUS_WS, timeout=10)
            ws.settimeout(30)  # recv window; heartbeat arrives every 60s so 30s recv -> Timeout -> ping
            log("connected to /ws")
            while True:
                try:
                    raw = ws.recv()
                except websocket.WebSocketTimeoutException:
                    ws.ping()  # keepalive probe; server responds pong
                    continue
                if not raw:
                    continue
                if raw == ws.Opcode.PING if isinstance(raw, bytes) else False:
                    continue
                try:
                    on_event(json.loads(raw))
                except (json.JSONDecodeError, TypeError):
                    pass  # control frames / non-json
        except Exception as e:
            log(f"ws dropped ({e}); reconnecting in 15s ...")
            time.sleep(15)


if __name__ == "__main__":
    main()
