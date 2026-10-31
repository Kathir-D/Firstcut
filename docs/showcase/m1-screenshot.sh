#!/usr/bin/env bash
# Drives the real app headless-of-humans: open a shoot folder under a scratch home, wait for
# the first frames to decode, and take ONE screenshot through the control channel
# (docs/v2-plan.md §4.3: agent-driven screenshots produce the README images).
#
# The PNG lands in docs/showcase/out/ — a gitignored directory, because the pixels are the
# shoot's photographs and **photos never enter the repo**. The README references the file;
# whoever runs this script vouches for the pixels (an owner item, todo.md §0.2).
#
# USAGE
#   docs/showcase/m1-screenshot.sh <shoot folder> [out name]
#   docs/showcase/m1-screenshot.sh ~/Documents/testing/Game1JENKS

set -euo pipefail
cd "$(dirname "$0")/../.."

SHOOT="${1:?usage: m1-screenshot.sh <shoot folder> [out name]}"
NAME="${2:-m1-culling}"
PORT="${M1_PORT:-41791}"
HOME_DIR="$(mktemp -d)"
OUT="docs/showcase/out"
mkdir -p "$OUT"

if [[ ! -d "$SHOOT" ]]; then
  echo "usage: the shoot folder must exist: $SHOOT" >&2
  exit 1
fi

cargo build -q -p firstcut

HOME="$HOME_DIR" ./target/debug/firstcut --control "$PORT" "$SHOOT" &
APP_PID=$!
trap 'kill "$APP_PID" 2>/dev/null || true; rm -rf "$HOME_DIR"' EXIT

# The window + the decode bank need a few seconds; the screenshot should show the loupe with
# real pixels, not the decode placeholder.
sleep 8

python3 - "$PORT" "$SHOOT" "$OUT/$NAME.png" <<'EOF'
import glob, json, socket, shutil, sys, tempfile, urllib.request
port, shoot, out = int(sys.argv[1]), sys.argv[2], sys.argv[3]

def rpc(method, params=None):
    client = socket.create_connection(("127.0.0.1", port), timeout=20)
    client.sendall(json.dumps({"id": 1, "method": method, "params": params or {}}).encode() + b"\n")
    reply = json.loads(client.makefile().readline())
    client.close()
    return reply

state = rpc("engine.state")
if "result" not in state:
    sys.exit(f"the app did not answer: {state}")
print("state:", json.dumps(state["result"])[:160], "…")
shot = rpc("ui.screenshot")
if "result" not in shot:
    sys.exit(f"the screenshot did not come back: {shot}")
png = shot["result"]["path"]
shutil.copyfile(png, out)
print("==> saved", out, "from", png)
EOF
