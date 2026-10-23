# Roadmap

Firstcut v2 is the native Rust rewrite. The binding plan is
[docs/v2-plan.md](docs/v2-plan.md); this page is the short view.

## Now — finish v2 and merge

- **M5, cross-platform hardening** (in progress): Windows long paths, DPI and watcher;
  Linux Wayland/X11 and the `FIRSTCUT_WGPU_BACKEND=gl` escape hatch; macOS document-folder
  permission strings; native file dialogs; bundled Inter so all three OSes draw identical
  glyphs; card-root shoot chooser. Exact RAW is now rawler on every platform, with LibRaw
  as the macOS/Linux fallback.
- **M6, packaging and docs**: `cargo xtask package` (macOS `.app`/zip + cask stamp, Windows
  zip, Linux AppImage + `.deb`), SHA-256 sums, a tag-driven release workflow, README /
  roadmap / control protocol / MCP docs, and a refreshed parity checklist.
- **Merge gate**: `cargo xtask ci` and the 3-OS matrix green; the owner checks the
  boundary ground truth, the preview-vs-RAW default and the visual pass. Only the owner
  fast-forwards `main`.

## Next — after the merge

- A native CRX/GPU decoder, if the exact-RAW floor (CRX decode alone is ~0.12 s for 24 MP)
  is worth chasing below the current 0.40 s warm develop.
- Installer signing for Windows and notarisation for macOS, when certificates exist.
- Manual batch split/merge and importable presets (`.xmp`, `.lrtemplate`, `.costyle`).
- Sparkle-class desktop updates.

## Not planned

- Web/WASM. v2 is native-only (plan D4).
- A second metadata parser for formats the existing readers already cover.
- Bundled proprietary assets or SF Symbols (plan D10).
