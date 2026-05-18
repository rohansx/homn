#!/usr/bin/env bash
# homn — end-to-end demo
# =============================================================================
# Walks the whole policy pipeline so you can SEE it work:
#   1. `homn rule trace`  — which rule decides a call, and why
#   2. daemon + hook      — a real PermissionRequest decided + audited
#   3. `homn log`         — the audit trail
#   4. MCP `query_policy` — an agent introspecting its own policy
#
# Everything runs in an isolated sandbox under /tmp — your real ~/.config/homn
# and ~/.local/share/homn are never touched. Re-runnable; cleans up after itself.
#
# Usage:  ./scripts/demo.sh
set -euo pipefail
cd "$(dirname "$0")/.."

SANDBOX=/tmp/homn-demo
export XDG_CONFIG_HOME="$SANDBOX/config"
export XDG_DATA_HOME="$SANDBOX/data"
export XDG_RUNTIME_DIR="$SANDBOX/run"
SOCK="$XDG_RUNTIME_DIR/homn.sock"
rule() { printf '\n\033[1;36m════════ %s ════════\033[0m\n' "$1"; }

rm -rf "$SANDBOX"
mkdir -p "$XDG_CONFIG_HOME/homn/policies" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"

rule "build"
cargo build -p homn-bin
BIN="$PWD/target/debug/homn"
cp policies/default.rhai "$XDG_CONFIG_HOME/homn/policies/default.rhai"

rule "1. homn rule trace — why a call is decided the way it is"
"$BIN" rule trace Bash "rm -rf /etc"          2>/dev/null | tail -3
"$BIN" rule trace Bash "cargo build --release" 2>/dev/null | tail -2
"$BIN" rule trace WebFetch "https://unknown.example.com" 2>/dev/null | tail -2

rule "2. daemon + hook — a real PermissionRequest, decided and audited"
"$BIN" daemon --foreground 2>"$SANDBOX/daemon.log" &
DAEMON=$!
trap 'kill $DAEMON 2>/dev/null || true' EXIT
timeout 8 bash -c "until [ -S '$SOCK' ]; do :; done"
echo "daemon up at $SOCK"
for payload in \
  '{"session_id":"01DEMO","tool_name":"Bash","tool_input":{"command":"rm -rf /etc"},"cwd":"/home/you/dev/x"}' \
  '{"session_id":"01DEMO","tool_name":"Bash","tool_input":{"command":"cargo build"},"cwd":"/home/you/dev/x"}' \
  '{"session_id":"01DEMO","tool_name":"Read","tool_input":{"path":"'"$HOME"'/notes.md"},"cwd":"/home/you/dev/x"}'
do
  printf '  request: %s\n  verdict: ' "$(echo "$payload" | python3 -c 'import sys,json;p=json.load(sys.stdin);print(p["tool_name"],p["tool_input"])')"
  echo "$payload" | "$BIN" hook permission-request 2>/dev/null
done

rule "3. homn log — the audit trail"
"$BIN" log 2>/dev/null

rule "4. MCP query_policy — an agent checking policy before acting"
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"demo","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query_policy","arguments":{"tool":"Bash","tool_input":{"command":"rm -rf /etc"}}}}' \
 | timeout 8 "$BIN" mcp stdio 2>/dev/null \
 | python3 -c 'import sys,json
for ln in sys.stdin:
    m=json.loads(ln)
    if m.get("id")==2: print("  query_policy(rm -rf /etc) ->", m["result"]["content"][0]["text"])'

printf '\n\033[1;32mdone.\033[0m sandbox left at %s — `rm -rf %s` to clear it.\n' "$SANDBOX" "$SANDBOX"
