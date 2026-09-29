#!/bin/bash
#
# Owner: infra. Re-runs scripts/build-core.sh only when the Rust side is newer than the last
# successful build. Used as the Xcode pre-build phase (project.yml) so Swift agents never have to
# think about the Rust core, and so a plain `cargo build` is never triggered by a Swift-only change.
#
# Xcode build phases run with the project directory as the working directory and without the
# rustup PATH, which scripts/common.sh handles.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

STAMP="$CARGO_DIR/target/$RUST_TARGET/release/.firstcut-bindings.stamp"

if ! rust_is_newer_than "$STAMP"; then
  exit 0
fi

ensure_cargo
"$FIRSTCUT_ROOT/scripts/build-core.sh"
