#!/usr/bin/env bash
# Runs v2's opt-in photo tests — the ones that read the real RAW files that live outside the repo.
#
# Milestone-only, never in the per-commit loop (docs/v2-plan.md §9): the photos are owned by the
# machine, not the repo, and `FIRSTCUT_ALLOW_PHOTO_TESTS=1` has to be set besides pointing
# `FIRSTCUT_TEST_PHOTOS` at the shoots. Both are exported here — one wrapper, no half-set state.
#
# Cargo has no per-target environment the way xcodebuild did, so v1's .xctestrun patching is gone
# and this is a thin `cargo test` wrapper with `--include-ignored` for the milestone runs.
#
# USAGE
#
#   scripts/test-with-photos.sh                            # ~/Documents/testing
#   FIRSTCUT_TEST_PHOTOS=/some/folder scripts/test-with-photos.sh

set -euo pipefail

cd "$(dirname "$0")/.."

PHOTOS="${FIRSTCUT_TEST_PHOTOS:-$HOME/Documents/testing}"
if [[ ! -d "$PHOTOS" ]]; then
  echo "error: no such folder: $PHOTOS" >&2
  echo "       set FIRSTCUT_TEST_PHOTOS to a folder of RAW files." >&2
  exit 1
fi
PHOTOS="$(cd "$PHOTOS" && pwd)"

export FIRSTCUT_TEST_PHOTOS="$PHOTOS"
export FIRSTCUT_ALLOW_PHOTO_TESTS=1
echo "==> Photos: $PHOTOS"
cargo test --workspace -- --include-ignored
