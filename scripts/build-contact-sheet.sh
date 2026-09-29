#!/bin/bash
#
# Builds the contact-sheet renderer, a standalone Swift tool.
#
#   tools/contact-sheet/main.swift  ->  build/tools/contact-sheet
#
# It is standalone rather than part of the app target for the same reason the decode spike is: a
# Swift file that is not `main.swift` cannot have top-level code at all, and `main.swift` is the
# only place Swift allows it. `firstcut contact-sheet` builds this if it is missing and then pipes
# the boundary list in on stdin.
#
# Env: FIRSTCUT_CONTACT_SHEET_BIN overrides the output path.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/tools/contact-sheet/main.swift"
OUT="${FIRSTCUT_CONTACT_SHEET_BIN:-$ROOT/build/tools/contact-sheet}"

if [ ! -f "$SRC" ]; then
  echo "contact-sheet: missing $SRC" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"

# Rebuild only when the source is newer than the binary: this sits on the path of every
# `firstcut contact-sheet` run and a full rebuild each time would dominate the cost.
if [ -x "$OUT" ] && [ "$OUT" -nt "$SRC" ]; then
  exit 0
fi

# -O because it decodes 2,880 CR3 previews; -swift-version 6 so it cannot drift from the app's
# language mode and break later in a confusing way.
swiftc -O -swift-version 6 -framework AppKit -framework CoreText -framework ImageIO \
  -o "$OUT" "$SRC"

echo "built $OUT" >&2
