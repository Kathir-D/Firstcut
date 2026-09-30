# UI reference

- **Owner:** ui
- Finder gallery-view reference screenshots and side-by-side check notes.

## Files

| File | What |
| --- | --- |
| `finder-gallery-macos27.png` | Finder gallery view, the layout reference (todo.md §9) |
| `firstcut-wave1-loupe-macos27.png` | Firstcut wave 1, loupe + filmstrip, mock model |
| `firstcut-wave1-info-panel-macos27.png` | Firstcut wave 1, with the info inspector and the progress HUD |

All captured on the dev machine: **macOS 27.0, Xcode 27, Apple M1 Pro**, 2026-09-29, dark mode.

The Finder reference was taken on a folder of 16 Canon R8 CR3 files from
`~/Documents/testing/Game1JENKS` (symlinked into a scratch folder, nothing copied into the repo), so
the two screenshots show the same kind of photo at a similar size.

## Side-by-side notes (wave 1)

Checked, matches Finder:

- Unified toolbar, window title hidden, dark appearance forced app-wide.
- `‹ ›` navigation in a rounded glass capsule on the left; Finder puts the same capsule before the
  folder name.
- Title centred in the toolbar (`Batch 12 of 148 — IMG_3294.CR3`); Finder centres the folder name.
- View switcher, then inspector toggle and a primary action on the right, in Finder's order.
- Large photo over a neutral dark gray field, filmstrip along the bottom, no visible window
  chrome at the edges.
- Right-hand inspector over the full window height, with the item name as its header.

Deliberately different, and why:

- **No sidebar and no path bar.** One folder = one session (§1); the folder path lives in the info
  panel instead of a permanent bar.
- **Filmstrip is the current batch only** (todo.md §9.3), so it is short, not the whole folder.
  Finder's is every item in the folder.
- **Thumbnails keep their own aspect ratio** and the selected frame sits on a rounded gray plate.
  Finder's gallery filmstrip uses uniform boxes, because Finder's item icons are a fixed size for
  every file type.
- **The viewer area is empty until pipeline's `PhotoViewerLayerView` lands.** The placeholder shows
  the file name, dimensions and camera so the layout can be judged before decodes exist.

Still to check in later waves:

- Rounded corners on the large image itself (§9.2) — belongs to the space around pipeline's layer.
- Finder's full-screen mode: chrome hides and the filmstrip auto-hides at the bottom edge (§9.1).
- macOS 15 fallback with `NSVisualEffectView` materials. **Not captured**: this machine runs
  macOS 27, so a 15 VM is needed before the visual check can be called done.
- Reduce Transparency and Reduce Motion, and the Liquid Glass fidelity pass (wave 4).
