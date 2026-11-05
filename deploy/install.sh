#!/usr/bin/env bash
# Install tinycode on the target machine — no Rust toolchain needed there.
# From the build host:
#   scp target/x86_64-unknown-linux-musl/release/tinycode deploy/tinycode@.service deploy/opencode-auth-sync.sh deploy/install.sh vps:/tmp/
#   ssh vps 'bash /tmp/install.sh'
set -euo pipefail

[[ -f /tmp/tinycode ]] || { echo "binary missing; scp it to /tmp/tinycode first" >&2; exit 1; }
mkdir -p "$HOME/.local/bin" "$HOME/.config/systemd/user" "$HOME/.config/tinycode/tasks"
install -m 0755 /tmp/tinycode "$HOME/.local/bin/tinycode"
"$HOME/.local/bin/tinycode" --version

if [[ -f /tmp/tinycode@.service ]]; then
  install -m 0644 /tmp/tinycode@.service "$HOME/.config/systemd/user/"
  systemctl --user daemon-reload 2>/dev/null || true
  echo "user unit installed"
else
  echo "unit file not scp'ed; skipped (CLI still works)" >&2
fi

if [[ -f /tmp/opencode-auth-sync.sh ]]; then
  install -m 0755 /tmp/opencode-auth-sync.sh "$HOME/.local/bin/tinycode-opencode-auth"
  echo "OpenCode key sync installed (tinycode-opencode-auth)"
fi

cat <<'EOF'

Setup:
  1. KEY:          reused from OpenCode — the unit runs tinycode-opencode-auth
     before each task, copying OpenCode's stored keys (incl. opencode-go)
     into ~/.config/tinycode/opencode-auth.json (mode 600). For an
     interactive shell: tinycode-opencode-auth once, then
     export TINYCODE_OPENCODE_AUTH=~/.config/tinycode/opencode-auth.json
     Otherwise: echo "TINYCODE_API_KEY=<key>" > ~/.config/tinycode/env
     (or TINYCODE_BASE_URL/TINYCODE_MODEL there for a local/self-hosted
      OpenAI-compatible endpoint)
  2. MODEL:        echo "TINYCODE_MODEL=glm-5.3-flash" >> ~/.config/tinycode/env
     (or set "model": "opencode-go/glm-5.3-flash" in opencode.json)
  3. TASK:         echo "my task" > ~/.config/tinycode/tasks/<name>.prompt
  4. RUN:          systemctl --user start tinycode@<name>
  5. WATCH:        journalctl --user -u tinycode@<name> -f
  6. USAGE:        tinycode usage --since 24h   (tokens per provider/model)
  7. READ-ONLY:    default CLI mode — leave --auto off and nothing is
     written; the service file bundles --auto, so cap tasks with
     --max-turns (already set) and MemoryMax (set in the unit).
EOF
