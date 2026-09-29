#!/bin/bash
#
# Owner: infra. One command to get a worktree ready to build after a pull:
#
#   scripts/generate-project.sh
#
# It regenerates the Rust bindings if the Rust sources changed, then regenerates Firstcut.xcodeproj
# from project.yml. After this, `xcodebuild` (or Xcode) just works. Nothing else in the tree is
# generated: Swift sources are picked up by folder, so adding a file never needs this.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

ensure_cargo
require_tool xcodegen "Install it with: brew install xcodegen"

"$FIRSTCUT_ROOT/scripts/build-core-if-stale.sh"

log "Generating Firstcut.xcodeproj"
(cd "$FIRSTCUT_ROOT" && xcodegen)

log "Ready. Build with:"
printf '    %s\n' \
  "xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test" \
  "scripts/build-app.sh --open          # the shippable app in dist/"
