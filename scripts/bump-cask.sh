#!/bin/bash
#
# Owner: infra. Pushes the release cask into Kathir-D/homebrew-tap, so
# `brew upgrade --cask firstcut` works without a human editing a file (todo.md §13).
#
# Run by .github/workflows/release.yml after the cask in this repo has been stamped with the
# release's version and SHA-256. Needs write access to the tap, in either of two forms (the
# workflow's default GITHUB_TOKEN cannot write to another repository, so it is a separate secret):
#
#   HOMEBREW_TAP_DEPLOY_KEY   the private half of a deploy key with write access, on the tap only.
#                             This is how the tap's other projects publish, and it is preferred:
#                             it can touch one repository and nothing else.
#   HOMEBREW_TAP_TOKEN        a fine-grained token with contents:write on the tap.
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
  diff -u <(curl -fsSL "https://raw.githubusercontent.com/$TAP_REPO/main/$TAP_CASK_PATH" 2>/dev/null) \
    "$SOURCE_CASK" && log "no change"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [ -n "${HOMEBREW_TAP_DEPLOY_KEY:-}" ]; then
  KEY="$WORK/deploy_key"
  printf '%s\n' "$HOMEBREW_TAP_DEPLOY_KEY" > "$KEY"
  chmod 600 "$KEY"
  export GIT_SSH_COMMAND="ssh -i $KEY -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
  CLONE_URL="git@github.com:$TAP_REPO.git"
elif [ -n "${HOMEBREW_TAP_TOKEN:-}" ]; then
  CLONE_URL="https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/$TAP_REPO.git"
else
  die "neither HOMEBREW_TAP_DEPLOY_KEY nor HOMEBREW_TAP_TOKEN is set"
fi

log "Cloning $TAP_REPO"
git clone --depth=1 --quiet "$CLONE_URL" "$WORK/tap"
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
