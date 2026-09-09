#!/usr/bin/env bash
# sho-mail-bridge: mirror sho's Hot Potato mailbox to Telegram (his visible surface).
# Polls the bus for sho's queued letters (poll marks them delivered), sends each via hermes send.
# Designed for a 5-min cron (no_agent mode). Exit 0 always; idempotent.
BUS=http://localhost:8080
TMP=$(mktemp)
curl -s -X POST "$BUS/" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"message/poll","params":{"agent":"sho"}}' > "$TMP"

python3 - "$TMP" <<'PY' > "$TMP.parsed"
import json, sys
d = json.load(open(sys.argv[1]))
letters = d.get("result") or []
out = []
for m in letters:
    out.append({"subject": m["subject"], "body": m["body"][:900], "id": m["id"]})
print(json.dumps(out))
PY

python3 - "$TMP.parsed" <<'PY'
import json, subprocess, sys
letters = json.load(open(sys.argv[1]))
for m in letters:
    text = f"🥔 [bus mail for sho] {m['subject']}\n\n{m['body']}"
    subprocess.run(["hermes", "send", "--to", "telegram:5690144164", text], timeout=60)
    print(f"bridged: {m['id'][:8]} {m['subject'][:50]}")
PY

rm -f "$TMP" "$TMP.parsed"
