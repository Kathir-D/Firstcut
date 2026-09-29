#!/usr/bin/env bash
# Runs the app's photo tests -- the ones that read the 42 GB of real RAW files.
#
# WHY THIS EXISTS
#
# `FIRSTCUT_TEST_PHOTOS=... xcodebuild test` looks like it should work and does not. The test
# bundles are hosted by Firstcut.app, and xcodebuild launches the host itself; the host's
# environment is the scheme's, not the shell's. So the variable never arrives, every photo test
# sees "no photos on this machine", and the suite reports success having asserted nothing about
# the photographs. This was confirmed, not assumed: a test that printed its own environment saw
# `FIRSTCUT_TEST_PHOTOS=nil` with the variable set in the invoking shell.
#
# The fix is the supported one: build the tests, inject the variables into the generated
# `.xctestrun` (which is where xcodebuild reads per-target test environment from), then run
# without rebuilding. The opt-in gate itself is untouched -- without FIRSTCUT_ALLOW_PHOTO_TESTS=1
# the tests still skip, so nothing here can make CI read 42 GB by accident.
#
# USAGE
#
#   scripts/test-with-photos.sh                    # ~/Documents/testing
#   FIRSTCUT_TEST_PHOTOS=/some/folder scripts/test-with-photos.sh
#   scripts/test-with-photos.sh FirstcutUnitTests  # one target only
#
# Reading ~/Documents triggers a TCC consent prompt for a GUI app. The app is ad-hoc signed, so
# a rebuild is a new code identity and the prompt reappears. Grant it once in System Settings >
# Privacy & Security > Files and Folders, or point FIRSTCUT_TEST_PHOTOS at a copy outside a
# protected folder (see the note in App/Tests/Integration/Support/FirstcutTestSupport.swift).

set -euo pipefail

cd "$(dirname "$0")/.."

PHOTOS="${FIRSTCUT_TEST_PHOTOS:-$HOME/Documents/testing}"
if [[ ! -d "$PHOTOS" ]]; then
  echo "error: no such folder: $PHOTOS" >&2
  echo "       set FIRSTCUT_TEST_PHOTOS to a folder of RAW files." >&2
  exit 1
fi
PHOTOS="$(cd "$PHOTOS" && pwd)"

DERIVED="${FIRSTCUT_TEST_DERIVED:-$PWD/.build/test-with-photos}"
ONLY_TESTING="${1:-}"

echo "==> Photos: $PHOTOS"

# XcodeGen writes the project; the scheme carries the targets and the fixtures in the bundles.
xcodegen >/dev/null

echo "==> Building tests"
build_args=(
  -project Firstcut.xcodeproj
  -scheme Firstcut
  -destination 'platform=macOS,arch=arm64'
  -derivedDataPath "$DERIVED"
  build-for-testing
)
# shellcheck disable=SC2206  # deliberate word splitting: -only-testing takes a comma-separated list
[[ -n "$ONLY_TESTING" ]] && build_args+=(-only-testing:"$ONLY_TESTING")
xcodebuild "${build_args[@]}" >/dev/null

runfile="$(find "$DERIVED/Build/Products" -maxdepth 1 -name '*.xctestrun' -print -quit)"
if [[ -z "$runfile" ]]; then
  echo "error: xcodebuild produced no .xctestrun in $DERIVED/Build/Products" >&2
  exit 1
fi
echo "==> Injecting the test environment into $(basename "$runfile")"

# Python rather than plutil: the .xctestrun is plist, but plutil's -insert cannot express "add a key
# to every test target in the file", which is the whole job here.
python3 - "$runfile" "$PHOTOS" <<'PY'
import plistlib, sys

path, photos = sys.argv[1], sys.argv[2]
with open(path, "rb") as fh:
    plist = plistlib.load(fh)

# A test target is a dict carrying a BlueprintName; the file also holds
# `__xctestrun_metadata__`, which is not one. Matching on the shape rather than on the name means a
# rename of a target, or an older/newer xctestrun layout, does not silently patch nothing -- the
# counter below turns "patched nothing" into a loud error.
patched = 0
for key, target in plist.items():
    if not isinstance(target, dict) or "BlueprintName" not in target:
        continue
    env = target.setdefault("EnvironmentVariables", {})
    env["FIRSTCUT_TEST_PHOTOS"] = photos
    env["FIRSTCUT_ALLOW_PHOTO_TESTS"] = "1"
    # The photo tests are slow (a full-folder scan reads thousands of files), so the default
    # execution-time allowance is not always enough. A generous cap is better than a red run that
    # says "timed out" when the code under test is fine.
    target["DefaultTestExecutionTimeAllowance"] = 1800
    patched += 1

if patched == 0:
    sys.exit("error: no test targets found in the .xctestrun; the format changed")

with open(path, "wb") as fh:
    plistlib.dump(plist, fh)
print(f"    {patched} test target(s) patched")
PY

echo "==> Running"
run_args=(
  -xctestrun "$runfile"
  -destination 'platform=macOS,arch=arm64'
  test-without-building
)
[[ -n "$ONLY_TESTING" ]] && run_args+=(-only-testing:"$ONLY_TESTING")
xcodebuild "${run_args[@]}"
