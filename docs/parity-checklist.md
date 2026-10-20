# v2 parity checklist — the merge gate, mirrored (docs/v2-plan.md §7)

> Status, honest as of 2026-10-20, at the end of the M1 vertical slice. Each row says where
> its work happens (or landed), and — once a row is true — the named test that proves it.
> **Struck-through = proven by a named test** (§7's rule); a row without ✔ still needs the
> work or the owner's eyes. A status number (Mn) is the milestone whose work owns the row:
> M1 = vertical slice, M2 = pipeline, M3 = screens, M4 = finish flow, M5 = hardening,
> M6 = packaging + docs, then the merge gate.

Rows, in §7's order:

| # | Row | Status | Evidence / where |
| --- | --- | --- | --- |
| 1 | Welcome: recents, resume (auto-recognizes a shoot's folder, restores cursor) | — M3 | Recents.json format already documented (photo-meta semantics); slice shows plain welcome (`CullScreen::show_welcome`) |
| 2 | Open folder: scan + first-photo fast path | ✔ scan, fast path M2 | `engine` session tests (scan/batch/open errors), `firstcut-cli run` chain test (`a_chain_opens_rates_and_reports_state`); decode priority window is M2 (`DecodeBank` stand-in in the app, inflight 4 / cache 64) |
| 3 | Loupe: pinch/scroll zoom, click zoom, pan, zoom lock, double-click batch jump | ✔ part (ui leg) | `firstcut-ui/src/cull.rs`: scroll zoom 0.2..=12, drag pan, double-click reset; zoom lock + batch-jump dbl-click are M3 (`zoom_lock` setting already parses in the engine) |
| 4 | AF point overlay; clipping overlay (250/255, 5/255 settings wired) | — M3 | Both thresholds already in `settings.rs`; AF is parsed by `firstcut-meta` |
| 5 | Filmstrip; grid | ✔ part (strip) / M3 (grid) | strip proven: `firstcut` kittest snapshot `culling.png` + `culling-after-rating.png` |
| 6 | 2/3/4-up compare with synced zoom | — M3 | |
| 7 | Info panel with histogram (orientation-safe) | — M3 | |
| 8 | Progress HUD with current rating | — M2 | |
| 9 | Both rating modes (stars / keep), undo/redo | ✔ partial | Both modes already read/settings (`rateModeKeep` etc.), `keep.set`/`stars` through `registry::dispatch`, undo/redo `edit.undo`/`edit.redo` (engine tests ported); UI buttons for mode switching are M3 |
| 10 | Finish Cull flow: summary → options → dry run → typed DELETE → report → undo | — M4 | |
| 11 | Settings: General, Keyboard (recorder/import/export), Viewer, Metadata, Performance | ✔ part | Settings: `settings.rs` reads the real v1 file (`engine`'s `the_real_v1_file_every_section_loads`) — all 5 sections parse × shapes checked; the keyboard and the app's settings screens are M3 |
| 12 | Menus + shortcut cheat-sheet generated from the registry | — M3 | `engine.commands` already is the generated source (`REGISTRY`) |
| 13 | Live folder watching + reconcile; flush on quit | — M2 | (`notify`-based reconcile is the M2 row) |
| 14 | Rename reconciliation; sidecar import rules | — M4 (fileops carries the rules) | Rules from v1 `fileops` ported; no screen uses them yet |
| 15 | Reject Whole Batch (opt-in, off by default) | part | model rejects a batch (`batch.reject` in the registry) — the full flow (dialog, undo grouping) is M3 |
| 16 | Embedded-JPEG fallback for RAW the backend cannot decode | ✔ | `firstcut-decode`'s `decode_display`; a JPEG photo displays from the file itself (`a_jpeg_photo_displays_from_the_file_itself`, `firstcut-meta`) |
| 17 | Every format raw.pixls.us lists opens (metadata side unchanged; decode per §4.2) | — M2 | metadata-only parity is the existing `firstcut-meta` readers (CR3/CRW/MRW/X3FWarning/RAF/TIFF) — real-file sweep lands with Exact RAW. **Found in the M1 showcase run** (`docs/showcase/m1-screenshot.sh`, Game1JENKS): decoded pixels ignore EXIF orientation — loupe shows the frame sideways; the rotate belongs to the M2 decode-tier work (row 8's histogram expects it too) |
| 18 | §5 rows measured | — M2+ | `bench` exists for order/batch; decode rows come with M2 |
| 19 | Owner items (carried from todo §0.2): boundary ground truth, Exact RAW default, Game1JENKS pause decision, sidecar round-trip check, visual sign-off, Homebrew secret, tag v2.0.0 | **owner** | unchanged, do not synthesise; see todo.md |

## Surfaces (what M1 proved end-to-end)

- **UI**: window opens a real folder, filmstrip, loupe, keys 0–5/arrows/⌘⇧Z; only via
  `registry::dispatch` — `apps/firstcut/tests/snapshots/*.png`, `culling_ui.rs`.
- **CLI**: `firstcut-cli run` chains — `d638cd6`, `run.rs` tests.
- **Control channel**: `firstcut --control <port>` — `engine.execute`/`engine.state`/
  `engine.commands`/`ui.widgets`/`ui.clickWidget`/`ui.screenshot`, JSON lines on localhost —
  live session proved (`27e520b`); protocol doc lands at the M6 docs leg.
- **MCP**: `firstcut-cli mcp` — `tools/list` generated from the registry, `tools/call`
  dispatches, `isError` on bad params — `34ff0ad`.
- **Headless snapshot**: the two kittest PNGsdouble as the first screenshots; the README image
  leg (agent-driven showcase scripts) lands at the docs leg.

> Merge gate statement (docs/v2-plan.md §7): the owner strikes through rows 1–18 before
> fast-forwarding `main`; no row may move to done without a test name or the owner's word.
