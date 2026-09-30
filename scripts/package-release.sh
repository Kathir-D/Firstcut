#!/bin/bash
#
# Owner: infra. Turns dist/Firstcut.app into the release artifacts (todo.md §13):
#
#   dist/Firstcut-<version>.zip   the app, ready to unzip into /Applications
#   dist/SHA256SUMS.txt           its SHA-256, so the Homebrew cask and users can verify it
#
# Run after scripts/build-app.sh. Release artifacts are ad-hoc signed and not notarized (no paid
# Apple Developer account), which is deliberate; see the release workflow for the consequences.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

require_tool shasum "Part of macOS; if this is missing, something is very wrong."

VERSION="$(repo_version)"
APP="$FIRSTCUT_ROOT/dist/Firstcut.app"
[ -d "$APP" ] || die "no app at $APP -- run scripts/build-app.sh first"

ZIP_NAME="Firstcut-${VERSION}.zip"
ZIP_PATH="$FIRSTCUT_ROOT/dist/$ZIP_NAME"

# `ditto` is the only archiver that keeps the executable bit, the symlinks inside the bundle and
# the resource forks. `zip -r` would produce an app that Gatekeeper refuses to launch.
log "Packaging $ZIP_NAME"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP_PATH"

SHA="$(shasum -a 256 "$ZIP_PATH" | cut -d' ' -f1)"
printf '%s  %s\n' "$SHA" "$ZIP_NAME" >"$FIRSTCUT_ROOT/dist/SHA256SUMS.txt"

# Same archive Homebrew will download and check against the cask. Rebuilding it here rather than
# re-zipping in the workflow keeps the checksum honest: one file, one hash.
log "SHA-256: $SHA  $ZIP_NAME"

SIZE="$(du -h "$ZIP_PATH" | cut -f1)"
log "Done: dist/$ZIP_NAME ($SIZE) and dist/SHA256SUMS.txt"
