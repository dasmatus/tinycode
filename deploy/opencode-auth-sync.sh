#!/usr/bin/env bash
# Reuse OpenCode's stored provider keys without touching OpenCode's files.
#
# OpenCode v2 keeps credentials in its SQLite database. This copies the
# active API-key credentials into an auth.json-shaped file that tinycode
# reads via TINYCODE_OPENCODE_AUTH, e.g.
#   {"opencode-go": {"type": "api", "key": "…"}}
# Run it before each tinycode run (the user unit does, via ExecStartPre)
# so a rotated key in OpenCode is picked up automatically.
# Never prints key material.
set -euo pipefail

db="${OPENCODE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.db}"
out="${TINYCODE_OPENCODE_AUTH:-${XDG_CONFIG_HOME:-$HOME/.config}/tinycode/opencode-auth.json}"

[[ -f "$db" ]] || { echo "opencode-auth-sync: no OpenCode database at $db" >&2; exit 1; }
mkdir -p "$(dirname "$out")"
umask 077
tmp="$(mktemp "$out.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

python3 - "$db" "$tmp" <<'PY'
import json, sqlite3, sys
db_path, out_path = sys.argv[1], sys.argv[2]
db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
entries = {}
for integration, value in db.execute(
        "select integration_id, value from credential where active = 1"):
    try:
        cred = json.loads(value)
    except ValueError:
        continue
    if cred.get("type") == "key" and cred.get("key"):
        entries[integration] = {"type": "api", "key": cred["key"]}
with open(out_path, "w") as f:
    json.dump(entries, f)
print("opencode-auth-sync: providers:", ", ".join(sorted(entries)) or "(none)")
PY

chmod 600 "$tmp"
mv -f "$tmp" "$out"
trap - EXIT
