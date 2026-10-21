# v2 parity checklist — the merge gate, mirrored (docs/v2-plan.md §7)

> Status, honest as of 2026-10-21, M2 closed and M3 (screens) starting. Rows that moved carry their test names. Each row says where
> its work happens (or landed), and — once a row is true — the named test that proves it.
> **Struck-through = proven by a named test** (§7's rule); a row without ✔ still needs the
> work or the owner's eyes. A status number (Mn) is the milestone whose work owns the row:
> M1 = vertical slice, M2 = pipeline, M3 = screens, M4 = finish flow, M5 = hardening,
> M6 = packaging + docs, then the merge gate.

Rows, in §7's order:

| # | Row | Status | Evidence / where |
| --- | --- | --- | --- |
| 1 | Welcome: recents, resume (auto-recognizes a shoot's folder, restores cursor) | — M3 | Recents.json format already documented (photo-meta semantics); slice shows plain welcome (`CullScreen::show_welcome`) |
| 2 | Open folder: scan + first-photo fast path | ✔ | `engine` session tests (scan/batch/open errors), `firstcut-cli run` chain test (`a_chain_opens_rates_and_reports_state`). The priority window/cancellation shipped in `firstcut-engine::pipeline` (`a_discarded_decode_releases_its_slot`, `a_request_is_not_a_cache_hit_until_pixels_land`, `eviction_never_crosses_tiers_or_the_focus_window`). The fast path is `Model::begin_open` (placeholder header + preview, `Phase::Loading`) with the full scan on the app's thread (`finish_open`); `the_fast_path_shows_one_photo_before_the_scan_installs_the_catalog` proves the handoff |
| 3 | Loupe: pinch/scroll zoom, click zoom, pan, zoom lock, double-click batch jump | ✔ | `firstcut-ui/src/cull.rs`: scroll zoom 0.2..=12, drag pan, click-to-1:1 anchored on the clicked point (`native_zoom`, `zoom_anchor` tests), double-click jumps a batch in the half clicked (forward/back via `batch.jump`), and a move resets zoom/pan unless zoom lock is on |
| 4 | AF point overlay; clipping overlay (250/255, 5/255 settings wired) | — M3 | Both thresholds already in `settings.rs`; AF is parsed by `firstcut-meta` |
| 5 | Filmstrip; grid | ✔ | Strip: `culling.png` + `culling-after-rating.png`. Grid: G / Escape between screens, wrapping contact sheet of every photo with name and stars, accent on the cursor, click selects (batch-walk through `registry`), double-click opens the loupe — `culling-grid.png` |
| 6 | 2/3/4-up compare with synced zoom | — M3 | |
| 7 | Info panel with histogram (orientation-safe) | ✔ | `firstcut-decode` computes it from the **oriented** frame: `the_histogram_is_computed_from_the_oriented_frame`; the app carries it on `Frame` and `FrameImages.histogram`. The panel (`CullScreen::info_panel`, toggled with I) shows the file name, exposure/camera/lens/dimensions and paints the 64-bin luma area plus RGB traces — `culling-info-panel.png` |
| 8 | Progress HUD with current rating | ✔ | `firstcut-ui`'s floating HUD (`CullScreen::hud`): batch and photo position, shoot-wide keep/good/maybe/unrated (and rejected when non-zero), and the current photo's rating in the active mode plus flag and label. Toggled with H (view state, like v1's `hudVisible`). `the_hud_counts_from_one_and_words_the_active_mode`, `the_tier_line_shows_rejected_only_when_something_was_rejected`, and both kittest snapshots |
| 9 | Both rating modes (stars / keep), undo/redo | ✔ | The toolbar's Stars / Keep control dispatches `settings.ratingMode`; P is mode-aware (keep toggle in Keep mode, pick flag in Stars mode), X rejects; `keep.set`/`rate.stars`/undo/redo already went through `registry::dispatch` with engine tests. `culling-keep-mode.png` drives the real toolbar click and P key |
| 10 | Finish Cull flow: summary → options → dry run → typed DELETE → report → undo | — M4 | |
| 11 | Settings: General, Keyboard (recorder/import/export), Viewer, Metadata, Performance | ✔ part | Settings: `settings.rs` reads the real v1 file (`engine`'s `the_real_v1_file_every_section_loads`) — all 5 sections parse × shapes checked; the keyboard and the app's settings screens are M3 |
| 12 | Menus + shortcut cheat-sheet generated from the registry | — M3 | `engine.commands` already is the generated source (`REGISTRY`) |
| 13 | Live folder watching + reconcile; flush on quit | ✔ | `firstcut-engine/src/watch.rs::a_file_landing_in_the_folder_reports_a_change`; `model::a_rescan_keeps_the_cursor_and_bumps_the_catalog_revision`; app `on_exit` calls `Model::close_session` (flush + checkpoint), and `session.rs`'s debounce/flush tests cover the queue |
| 14 | Rename reconciliation; sidecar import rules | — M4 (fileops carries the rules) | Rules from v1 `fileops` ported; no screen uses them yet |
| 15 | Reject Whole Batch (opt-in, off by default) | part | model rejects a batch (`batch.reject` in the registry) — the full flow (dialog, undo grouping) is M3 |
| 16 | Embedded-JPEG fallback for RAW the backend cannot decode | ✔ | `firstcut-decode`'s `decode_display`; a JPEG photo displays from the file itself (`a_jpeg_photo_displays_from_the_file_itself`, `firstcut-meta`) |
| 17 | Every format raw.pixls.us lists opens (metadata side unchanged; decode per §4.2) | ✔ CR3 corpus sweep / other brands metadata-only | **The M1 showcase defect — sideways loupe — is fixed in the decode seam**: `every_exif_orientation_maps_the_corners_the_way_the_spec_says`, `decode_display_applies_the_exif_orientation_exactly_once`, and the LibRaw backend develops with `user_flip = 0` then calls the same seam. The real corpus now passes `every_sampled_photo_displays_in_display_orientation` (12 sampled photos) and `a_real_raw_develops_in_sensor_orientation_and_is_rotated_once` (orientation 8 → 3999×5999 portrait). Other brands have metadata readers but no decode samples in the corpus |
| 18 | §5 rows measured | ✔ measured / exact-RAW gate over | `firstcut-cli bench --decode --folder <game>`; numbers in `docs/v2-plan.md` §5.1. First photo 0.118–0.334 s ✔, arrow 0.070–0.082 s ✔, zoom decode 0.195 s (lower bound), **Exact RAW 1.03–1.05 s vs the 0.119 s gate** — LibRaw is CPU while v1's 0.079 s was Core Image; owner decision recorded in §5.1 |
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
