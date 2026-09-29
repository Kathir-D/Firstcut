#!/bin/bash
#
# Owner: infra. Pushes the release cask into Kathir-D/homebrew-tap, so
# `brew upgrade --cask firstcut` works without a human editing a file (task.md §13).
#
# Run by .github/workflows/release.yml after the cask in this repo has been stamped with the
# release's version and SHA-256. Needs HOMEBREW_TAP_TOKEN: a token with write access to the tap.
# The workflow's default GITHUB_TOKEN cannot write to another repository, which is why this is a
# separate secret.
#
# Usage (locally, to rehearse): HOMEBREW_TAP_TOKEN=… scripts/bump-cask.sh
#   DRY_RUN=1 prints the diff and writes nothing.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

TAP_REPO="${FIRSTCUT_TAP_REPO:-Kathir-D/homebrew-tap}"
TAP_CASK_PATH="Casks/firstcut.rb"
VERSION="$(repo_version)"
SOURCE_CASK="$FIRSTCUT_ROOT/Casks/firstcut.rb"
SHA="$(cut -d' ' -f1 "$FIRSTCUT_ROOT/dist/SHA256SUMS.txt" 2>/dev/null || true)"

[ -n "$SHA" ] || die "no dist/SHA256SUMS.txt -- run scripts/package-release.sh first"
grep -q "REPLACE_WITH_RELEASE_SHA256" "$SOURCE_CASK" \
  && die "Casks/firstcut.rb still holds the placeholder; the release workflow must stamp it first"

if [ -n "${DRY_RUN:-}" ]; then
  log "DRY RUN: would update $TAP_REPO/$TAP_CASK_PATH to $VERSION ($SHA)"
  diff -u <(gh api "repos/$TAP_REPO/contents/$TAP_CASK_PATH" --jq '.content' 2>/dev/null | base64 -d) \
    "$SOURCE_CASK" && log "no change"
  exit 0
fi

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is required"
[ -n "${HOMEBREW_TAP_TOKEN:-}" ] || die "HOMEBREW_TAP_TOKEN is not set"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

log "Cloning $TAP_REPO"
gh repo clone "$TAP_REPO" "$WORK/tap" -- --depth=1 --quiet
mkdir -p "$WORK/tap/Casks"
cp "$SOURCE_CASK" "$WORK/tap/$TAP_CASK_PATH"

cd "$WORK/tap"
if git diff --quiet -- "$TAP_CASK_PATH"; then
  log "Cask is already at $VERSION; nothing to push"
  exit 0
fi

git -c user.name="github-actions[bot]" -c user.email="github-actions[bot]@users.noreply.github.com" \
  commit -am "firstcut $VERSION" -- "$TAP_CASK_PATH" >/dev/null
git push origin HEAD:main 2>&1 | tail -2

log "Pushed $TAP_REPO/$TAP_CASK_PATH: firstcut $VERSION ($SHA)"
log "Users can now run: brew upgrade --cask firstcut"
