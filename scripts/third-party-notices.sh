#!/bin/bash
#
# Regenerates THIRD-PARTY-NOTICES.md from the crates that are actually linked into the app
# (normal dependencies of firstcut-core, not dev or build tools). Run at each release, and whenever
# Cargo.lock changes; CI's `--check` mode fails when the committed file is stale.
#
# Usage: scripts/third-party-notices.sh [--check]

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

command -v python3 >/dev/null 2>&1 || die "python3 is required"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cargo metadata --format-version 1 --locked --manifest-path "$FIRSTCUT_ROOT/core/Cargo.toml" > "$TMP/meta.json"

python3 - "$TMP/meta.json" "$FIRSTCUT_ROOT/THIRD-PARTY-NOTICES.md" "$TMP/out.md" <<'PY'
import json, sys
meta, committed, out_path = sys.argv[1:4]
m = json.load(open(meta))
pk = {p['id']: p for p in m['packages']}
nodes = {n['id']: n for n in m['resolve']['nodes']}
root = next(i for i in pk if pk[i]['name'] == 'firstcut-core')
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]['deps']:
        if any(k['kind'] is None for k in d['dep_kinds']):
            stack.append(d['pkg'])
rows = sorted((pk[i]['name'], pk[i]['version'], pk[i].get('license') or 'UNKNOWN',
               pk[i].get('repository') or '') for i in seen if i != root)
unknown = [r[0] for r in rows if r[2] == 'UNKNOWN' or 'GPL' in r[2].replace('LGPL', '')]
if unknown:
    sys.exit(f"license needs a human look: {unknown}")
head = open(committed).read().split('| Crate |')[0]
lines = [head + '| Crate | Version | License | Source |', '| --- | --- | --- | --- |']
for name, ver, lic, repo in rows:
    lines.append(f"| `{name}` | {ver} | {lic} | {'<'+repo+'>' if repo else ''} |")
open(out_path, 'w').write('\n'.join(lines) + '\n')
PY

if [ "${1:-}" = "--check" ]; then
  diff -u "$FIRSTCUT_ROOT/THIRD-PARTY-NOTICES.md" "$TMP/out.md" >/dev/null \
    || die "THIRD-PARTY-NOTICES.md is stale; run scripts/third-party-notices.sh"
  log "THIRD-PARTY-NOTICES.md is current"
else
  cp "$TMP/out.md" "$FIRSTCUT_ROOT/THIRD-PARTY-NOTICES.md"
  log "wrote THIRD-PARTY-NOTICES.md"
fi
