#!/usr/bin/env bash
#
# install.sh — Install MyWorkTable dashboard, hooks, and systemd service.
#
# Usage:
#   ./install.sh [SERVER_HOST]
#
# SERVER_HOST defaults to "localhost". Use "dockerhost" if Claude Code runs
# inside a Docker container that resolves "dockerhost" to the host machine.
#
# Safe to re-run — all steps are idempotent.

set -euo pipefail

SERVER_HOST="${1:-localhost}"
SERVER_PORT=5548
SERVER_URL="http://${SERVER_HOST}:${SERVER_PORT}"

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
CLAUDE_DIR="${HOME}/.claude"
SETTINGS_FILE="${CLAUDE_DIR}/settings.json"
SERVICE_NAME="myworktable"
BINARY="${REPO_DIR}/target/release/server"

# ── helpers ────────────────────────────────────────────────────────────────

info()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m==> WARNING:\033[0m %s\n' "$*"; }
error() { printf '\033[1;31m==> ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || error "'$1' is required but not found in PATH"
}

# ── preflight ──────────────────────────────────────────────────────────────

require cargo
require jq
require systemctl

[ -d "$CLAUDE_DIR" ] || error "~/.claude does not exist — is Claude Code installed?"

# ── build ──────────────────────────────────────────────────────────────────

info "Building release binary..."
cargo build --release --manifest-path "${REPO_DIR}/Cargo.toml"
[ -x "$BINARY" ] || error "Build succeeded but binary not found at ${BINARY}"

# ── configure Claude Code hooks ───────────────────────────────────────────

info "Configuring Claude Code hooks (server: ${SERVER_URL})..."

HOOK_EVENTS=(
    SessionStart
    UserPromptSubmit
    PermissionRequest
    PreToolUse
    PostToolUse
    PostCompact
    Notification
    Stop
    SubagentStart
    SubagentStop
    SessionEnd
    TaskCreated
    TaskCompleted
    CwdChanged
)

# Build the hooks object with jq
HOOKS_JSON=$(jq -n --arg url "$SERVER_URL" '
    [$ARGS.positional[] | {
        key: .,
        value: [{ hooks: [{ type: "http", url: ($url + "/hooks/" + .), timeout: 5 }] }]
    }] | from_entries
' --args -- "${HOOK_EVENTS[@]}")

# Merge into existing settings (preserve all other keys)
if [ -f "$SETTINGS_FILE" ]; then
    EXISTING=$(cat "$SETTINGS_FILE")
else
    EXISTING='{}'
fi

echo "$EXISTING" | jq \
    --argjson hooks "$HOOKS_JSON" \
    '.hooks = ((.hooks // {}) * $hooks) | del(.statusLine)' \
    > "${SETTINGS_FILE}.tmp"
mv -f "${SETTINGS_FILE}.tmp" "$SETTINGS_FILE"

# ── systemd user service ──────────────────────────────────────────────────

info "Installing systemd user service..."
SERVICE_DIR="${HOME}/.config/systemd/user"
mkdir -p "$SERVICE_DIR"

cat > "${SERVICE_DIR}/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=MyWorkTable dashboard
After=default.target

[Service]
ExecStart=${BINARY}
WorkingDirectory=${REPO_DIR}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable "$SERVICE_NAME"
systemctl --user restart "$SERVICE_NAME"

# ── done ───────────────────────────────────────────────────────────────────

info "Done!"
echo
echo "  Dashboard:   http://localhost:${SERVER_PORT}"
echo "  Service:     systemctl --user status ${SERVICE_NAME}"
echo "  Hooks:       ${SERVER_URL}/hooks/*"
echo
echo "  Re-run this script any time to rebuild and update."
