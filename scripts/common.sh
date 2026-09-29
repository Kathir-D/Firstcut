#!/bin/bash
#
# Owner: infra. Shared helpers for the scripts in this directory. Sourced, not executed.
#
# All scripts resolve paths from their own location, so they work from any working directory and
# from Xcode's build phases (which run with the project directory as the working directory).

set -euo pipefail

# Repo root = parent of scripts/
FIRSTCUT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export FIRSTCUT_ROOT

# Where the generated Swift bindings, the FFI module and the xcframework land.
# Git-ignored; regenerate with scripts/build-core.sh (docs/contracts/build.md).
GENERATED_DIR="$FIRSTCUT_ROOT/App/Generated"
export GENERATED_DIR

# The Rust workspace and the single target triple we support (Apple Silicon only, task.md §2).
CARGO_DIR="$FIRSTCUT_ROOT/core"
export CARGO_DIR
RUST_TARGET="${FIRSTCUT_RUST_TARGET:-aarch64-apple-darwin}"
export RUST_TARGET

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# rustup installs into ~/.cargo/bin, which a GUI-launched Xcode or a fresh shell may not have on
# PATH. Source the env file when the toolchain is not already visible.
ensure_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    if [ -f "$HOME/.cargo/env" ]; then
      # shellcheck disable=SC1091
      . "$HOME/.cargo/env"
    fi
  fi
  command -v cargo >/dev/null 2>&1 || die \
    "cargo not found. Install Rust with rustup (https://rustup.rs) and re-run, or open a new shell so ~/.cargo/env is sourced."
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not on PATH. ${2:-}"
}

# Version string from the VERSION file at the repo root (semver, e.g. 0.1.0).
repo_version() {
  [ -f "$FIRSTCUT_ROOT/VERSION" ] || die "VERSION file missing at $FIRSTCUT_ROOT/VERSION"
  tr -d '[:space:]' <"$FIRSTCUT_ROOT/VERSION"
}

# True when any Rust source or manifest is newer than the given stamp file. Used to keep the
# Xcode pre-build phase cheap: it only re-runs cargo when the Rust side actually changed.
rust_is_newer_than() {
  local stamp="$1"
  [ -e "$stamp" ] || return 0
  local newer
  newer="$(find "$CARGO_DIR/firstcut-core/src" "$CARGO_DIR/firstcut-cli/src" \
    "$CARGO_DIR/Cargo.toml" "$CARGO_DIR/Cargo.lock" \
    "$CARGO_DIR/firstcut-core/Cargo.toml" "$CARGO_DIR/firstcut-core/uniffi.toml" \
    -type f -newer "$stamp" -print -quit 2>/dev/null || true)"
  [ -n "$newer" ]
}
