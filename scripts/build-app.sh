#!/bin/bash
#
# Owner: infra. Builds the shippable app: Rust core -> bindings -> Xcode project -> Release app
# -> ad-hoc signature -> dist/Firstcut.app.
#
# This is the "clean clone to working app" path (task.md §13). No manual steps, no paid Apple
# Developer account, no notarization: the app is ad-hoc signed, which is what the Homebrew cask and
# the curl download rely on.
#
# Usage:
#   scripts/build-app.sh [--open] [--configuration Release|Debug]
#
# Env:
#   BUILD_NUMBER       CFBundleVersion (default 1; the release workflow passes the tag's build)
#   FIRSTCUT_RUST_TARGET  Rust target triple (default aarch64-apple-darwin)

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

ensure_cargo
require_tool xcodegen "Install it with: brew install xcodegen"
require_tool xcodebuild "Install Xcode and run xcode-select --switch /Applications/Xcode.app."

OPEN_AFTER=0
CONFIGURATION="Release"
while [ $# -gt 0 ]; do
  case "$1" in
  --open) OPEN_AFTER=1 ;;
  --configuration) CONFIGURATION="${2:?--configuration needs a value}"; shift ;;
  -h | --help)
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *) die "unknown option: $1" ;;
  esac
  shift
done

VERSION="$(repo_version)"
BUILD_NUMBER="${BUILD_NUMBER:-1}"

log "Firstcut $VERSION (build $BUILD_NUMBER, $CONFIGURATION)"

# 1. The Rust core and its Swift bindings.
"$FIRSTCUT_ROOT/scripts/build-core.sh"

# 2. The Xcode project (git-ignored, generated from project.yml).
log "Generating Firstcut.xcodeproj"
(cd "$FIRSTCUT_ROOT" && xcodegen)

# 3. The app.
log "xcodebuild $CONFIGURATION"
xcodebuild \
  -project "$FIRSTCUT_ROOT/Firstcut.xcodeproj" \
  -scheme Firstcut \
  -configuration "$CONFIGURATION" \
  -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath "$FIRSTCUT_ROOT/build/dd" \
  MARKETING_VERSION="$VERSION" \
  CURRENT_PROJECT_VERSION="$BUILD_NUMBER" \
  build

APP="$FIRSTCUT_ROOT/build/dd/Build/Products/$CONFIGURATION/Firstcut.app"
[ -d "$APP" ] || die "xcodebuild did not produce $APP"

# 4. Stage and sign. The Rust core is statically linked, so the .app is self-contained: there is no
#    dylib to embed and only one code signature to make. Ad-hoc (-s -) is deliberate: without a
#    paid Developer ID we cannot notarize (task.md §13).
log "Staging dist/Firstcut.app"
rm -rf "$FIRSTCUT_ROOT/dist"
mkdir -p "$FIRSTCUT_ROOT/dist"
ditto "$APP" "$FIRSTCUT_ROOT/dist/Firstcut.app"

log "Ad-hoc signing"
codesign --force --sign - --timestamp=none "$FIRSTCUT_ROOT/dist/Firstcut.app"
codesign --verify --deep --strict --verbose=1 "$FIRSTCUT_ROOT/dist/Firstcut.app"

SIZE="$(du -sh "$FIRSTCUT_ROOT/dist/Firstcut.app" | cut -f1)"
log "Done: dist/Firstcut.app ($SIZE, version $VERSION, build $BUILD_NUMBER)"

if [ "$OPEN_AFTER" = "1" ]; then
  log "Opening Firstcut.app"
  open "$FIRSTCUT_ROOT/dist/Firstcut.app"
fi
