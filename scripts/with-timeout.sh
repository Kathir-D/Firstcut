#!/bin/bash
# Run a command, kill it if it takes longer than a deadline, and report which.
#
# Why this exists: on this machine a build or a test run can block forever instead of failing.
# The usual cause is a TCC consent prompt — the repository lives under ~/Documents, the test host is
# a GUI app, and an ad-hoc signed build gets a new identity on every rebuild, so macOS re-asks for
# Documents access and waits for a click that nobody is there to give. Without a deadline that is an
# apparently frozen shell and no output to act on. With one it is a 120-line report saying exactly
# which step hung.
#
# Usage: scripts/with-timeout.sh <seconds> <command> [args...]
#   Exit status is the command's, or 124 if the deadline passed (the same convention GNU timeout
#   uses, so a caller can tell "failed" from "hung").
set -uo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $(basename "$0") <seconds> <command> [args...]" >&2
    exit 64
fi

seconds="$1"
shift

printf '==> %s (deadline %ss)\n' "$*" "$seconds" >&2

# A marker the watchdog touches, so the exit status can distinguish "the command failed" from
# "the watchdog stopped it" — SIGTERM surfaces as 143, and the caller needs 124.
marker="$(mktemp -t firstcut-timeout)"
rm -f "$marker"

"$@" &
pid=$!

# `wait` returns early on a signal, so a watchdog is what turns the deadline into a kill.
( sleep "$seconds"; touch "$marker" 2>/dev/null; kill -TERM "$pid" 2>/dev/null ) &
watchdog=$!
# The job-control "Terminated" line is noise here: the timeout is the whole point and it is
# reported properly below.
disown "$watchdog" 2>/dev/null

wait "$pid"
status=$?
kill "$watchdog" 2>/dev/null
wait "$watchdog" 2>/dev/null

if [ -e "$marker" ]; then
    rm -f "$marker"
    printf '\n!!! TIMED OUT after %ss: %s\n' "$seconds" "$*" >&2
    printf '    If this was a build or a test run, the usual cause is a TCC consent prompt\n' >&2
    printf '    for ~/Documents. Check for a pending dialog before re-running.\n' >&2
    exit 124
fi
rm -f "$marker"
exit "$status"
