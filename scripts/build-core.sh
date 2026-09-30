#!/bin/bash
#
# Owner: infra. Builds the Rust core and packages it for Swift.
#
#   1. cargo build --release --target aarch64-apple-darwin  (libfirstcut_core.a, staticlib)
#   2. uniffi-bindgen-swift  ->  App/Generated/FirstcutCore.swift
#                                App/Generated/FirstcutCoreFFI/{FirstcutCoreFFI.h,module.modulemap}
#   3. xcodebuild -create-xcframework  ->  App/Generated/FirstcutCore.xcframework
#
# Output layout (all git-ignored, see .gitignore):
#
#   App/Generated/FirstcutCore.swift     compiled into the app target by project.yml
#   App/Generated/FirstcutCoreFFI/       C declarations; on HEADER_SEARCH_PATHS
#   App/Generated/FirstcutCore.xcframework  linked, not embedded (static library)
#
# Swift agents: run this after pulling Rust changes, before `xcodegen`. The Xcode pre-build phase
# (project.yml) runs it automatically when the Rust sources are newer than the stamp file.
#
# Env: FIRSTCUT_RUST_TARGET (default aarch64-apple-darwin), FIRSTCUT_CORE_PROFILE=debug|release.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

ensure_cargo
require_tool xcodebuild "Install Xcode and run xcode-select --switch /Applications/Xcode.app."

PROFILE="${FIRSTCUT_CORE_PROFILE:-release}"
CORE_LIB="$CARGO_DIR/target/$RUST_TARGET/$PROFILE/libfirstcut_core.a"
BINDGEN="$CARGO_DIR/target/$RUST_TARGET/$PROFILE/uniffi-bindgen-swift"
STAMP="$CARGO_DIR/target/$RUST_TARGET/$PROFILE/.firstcut-bindings.stamp"

log "Building firstcut-core ($PROFILE, $RUST_TARGET)"
cargo build --manifest-path "$CARGO_DIR/Cargo.toml" \
  --package firstcut-core \
  --profile "$PROFILE" \
  --target "$RUST_TARGET"

[ -f "$CORE_LIB" ] || die "static library not found at $CORE_LIB"
[ -x "$BINDGEN" ] || die "bindings generator not found at $BINDGEN (it is a bin target of firstcut-core)"

log "Generating Swift bindings"
FFI_DIR="$GENERATED_DIR/FirstcutCoreFFI"
rm -rf "$GENERATED_DIR"
mkdir -p "$FFI_DIR"
# Run from the workspace: the generator shells out to `cargo metadata` to resolve the crate.
# The Clang module the generated Swift imports is `FirstcutCoreFFI` (uniffi.toml sets
# module_name = FirstcutCore, and uniffi derives the FFI module name from it), so the modulemap
# must declare that name -- not the Swift one.
(cd "$CARGO_DIR" && "$BINDGEN" \
  "$CORE_LIB" \
  "$GENERATED_DIR" \
  --swift-sources \
  --headers \
  --modulemap \
  --module-name FirstcutCoreFFI \
  --modulemap-filename FirstcutCoreFFI.modulemap)

# The Swift bindings call the C functions directly, so the C declarations must be reachable as a
# Clang module. Swift finds a module through a modulemap on the header search path, so the
# generated FirstcutCoreFFI.modulemap is renamed to the canonical module.modulemap. project.yml
# puts this directory on HEADER_SEARCH_PATHS.
mv "$GENERATED_DIR/FirstcutCoreFFI.modulemap" "$FFI_DIR/module.modulemap"
mv "$GENERATED_DIR/FirstcutCoreFFI.h" "$FFI_DIR/FirstcutCoreFFI.h"

# The same bindings again under a name Xcode will accept as a bundled resource.
#
# `CoreBridgeTests` asserts on the real export list by reading the generated source. Reading it from
# the repository meant reading ~/Documents from a GUI test host, which raises a TCC consent prompt
# and blocks the run forever with nobody there to click Allow — and because the app is ad-hoc signed,
# its identity changes on every rebuild, so the prompt came back every time. Bundling the file into
# the test target fixes it. Xcode silently refuses to copy a `.swift` file through a Copy Bundle
# Resources phase ("cannot be processed by a Copy Bundle Resources build phase"), so the copy is
# published as plain text. The content is byte-identical; only the name differs.
cp "$GENERATED_DIR/FirstcutCore.swift" "$GENERATED_DIR/FirstcutCore.bindings.txt"

log "Packaging FirstcutCore.xcframework"
rm -rf "$GENERATED_DIR/FirstcutCore.xcframework"
xcodebuild -create-xcframework \
  -library "$CORE_LIB" \
  -output "$GENERATED_DIR/FirstcutCore.xcframework"

# Stamp last: the Xcode pre-build phase compares Rust mtimes against this.
date >"$STAMP"

log "Core ready:"
printf '    %s\n' \
  "$GENERATED_DIR/FirstcutCore.swift" \
  "$GENERATED_DIR/FirstcutCore.bindings.txt" \
  "$GENERATED_DIR/FirstcutCoreFFI/" \
  "$GENERATED_DIR/FirstcutCore.xcframework/"
