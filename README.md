<div align="center">

<img src="assets/branding/firstcut-512.png" alt="Firstcut app icon" width="128" height="128">

# Firstcut

**A fast, burst-aware photo culler, native on macOS, Windows and Linux.**

Open one folder, let Firstcut order it into bursts, and cull with the keyboard. The
pixels you need are already on screen; a full-resolution sensor render follows when
you ask for it.

![Platform](https://img.shields.io/badge/platform-macOS%2012%2B%20%7C%20Windows%2010%2B%20%7C%20Linux-black)
![Rust](https://img.shields.io/badge/Rust-1.99-000000?logo=rust)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)

</div>

> [!IMPORTANT]
> v2 is the pure-Rust rewrite. It lives on the `v2` branch and in
> [draft PR #6](https://github.com/Kathir-D/Firstcut/pull/6); `main` still carries v1.
> The merge gate is in [docs/v2-plan.md](docs/v2-plan.md) §7 and its status is tracked in
> [docs/parity-checklist.md](docs/parity-checklist.md).

## What works

| Area | Status |
| --- | --- |
| Open a shoot, scan it, split it into bursts | Works; boundary ground truth is still an owner check |
| Loupe, grid, 2/3/4-up compare, synced zoom | Works |
| AF points, clipping overlays, histogram, info panel | Works; thresholds come from Settings |
| Stars and Keep modes, undo/redo, Reject Whole Batch (opt-in) | Works; Reject Whole Batch only flags unrated frames |
| Finish Cull: summary → options → dry run → typed `DELETE` → report → undo | Works |
| Settings, keymap recorder/import/export, recent shoots, live folder watching | Works |
| Embedded-JPEG display path | Works on every OS |
| Exact RAW sensor develop | rawler on all three OSes; LibRaw fallback on macOS/Linux |
| CLI (`run`, `bench`, `verify`, `mcp`) and the control channel | Works; see [docs/control-protocol.md](docs/control-protocol.md) and [docs/mcp.md](docs/mcp.md) |
| Packaging: macOS `.app`/zip + cask, Windows zip, Linux AppImage + `.deb` | `cargo xtask package`; signing is optional |
| Owner sign-off: batching ground truth, preview-vs-RAW default, visual pass | Pending |

## Performance (measured, Apple M1 Pro)

| Path | Measured |
| --- | --- |
| First photo after opening a 708-file shoot | 0.24–0.29 s |
| Held arrow, navigation preview (`PRVW`) | 0.006–0.008 s |
| Next photo, embedded display preview | 0.057–0.068 s |
| Display throughput, 2048 px (eight workers) | 89.4 frames/s |
| Display throughput, 1350 px laptop fit (PRVW path) | 638 frames/s |
| Display throughput, 3240 px 4K fit (full preview) | 82.4 frames/s |
| Display throughput, 4320 px 5K fit (full preview) | 74.9 frames/s |
| Display throughput, 5076 px 6K fit (full preview) | 62.4 frames/s |
| 1:1 zoom re-decode (embedded full preview) | 0.068 s |
| Exact RAW, first pixels (display preview) | 0.057–0.068 s |
| Exact RAW, full sensor develop | 0.257 s (decode 0.12 + demosaic 0.094 + pack 0.024 + orientation 0.015) |

The exact develop is 0.257 s because a 24 MP CRX decode alone is ~0.12 s on this CPU and
PPG demosaic another ~0.094 s; the 0.119 s target is below both. A held arrow key is not decode-latency bound: the
loupe paints the 6–8 ms navigation preview every repeat, the display tier is prefetched
32 ahead/12 behind (plus batch heads), and eight workers sustain 89.4 frames/s at the
2048 px baseline (82.4 at the 3240 px 4K fit) against a 20 Hz frame budget, so after a
short lead-in the sharp frame is already waiting. The sensor frame swaps in when the cursor
rests. The handoff is zero-copy: the decoder allocates its RGBA pixels 4-byte aligned, the
decode worker adopts them as egui's `Color32` without a per-pixel conversion, and a fitted texture
shares the decoder's `Arc` rather than cloning it — keeping 4.1 ms of conversion and 0.6 ms
of copy off the paint thread per 3240×2160 frame.

Display work is sized by what the pane can actually show, not by a fixed long edge. A 3:2
frame on a 16:9 4K pane is 3240 px wide, not 3840, and a 1440×900 1x laptop pane asks for
1350 px — small enough that the camera's 1620 px `PRVW` already covers it, so the full
6000×4000 entropy decode is skipped and eight workers measure 638 frames/s. A 2× zoom
asks for 2700 px, not the whole sensor. The GPU texture cache is capped at 768 MB so a long
4K hold cannot keep every frame ever shown resident. Details and the owner options are in
[docs/v2-plan.md](docs/v2-plan.md) §5.1 (decision D13).

## Build and run

The toolchain is pinned by `rust-toolchain.toml` (Rust 1.99.0).

```sh
cargo run -p firstcut                 # the desktop app
cargo run -p firstcut -- --smoke      # paint a few frames and exit
cargo run -p firstcut-cli -- --help   # CLI
cargo test --workspace                # tests (photos are opt-in, see below)
cargo xtask ci                        # the full local gate
```

Platform prerequisites:

- **macOS**: Xcode Command Line Tools. LibRaw is vendored and built by `rsraw`.
- **Windows**: Rust's MSVC toolchain and the Visual Studio C++ build tools. The exact
  RAW path is pure Rust (rawler), so a Windows build has no LibRaw dependency.
- **Linux**: a Vulkan driver (or run with `FIRSTCUT_WGPU_BACKEND=gl`), plus
  `libxkbcommon`, `libwayland-client`, `libgl` and `libvulkan` for a packaged build.

Real-photo tests are opt-in and never enter CI:

```sh
FIRSTCUT_TEST_PHOTOS=$HOME/Documents/testing FIRSTCUT_ALLOW_PHOTO_TESTS=1 \
  cargo test --workspace -- --include-ignored
```

## Packaging

```sh
cargo xtask package
```

This builds the release app for the host OS and writes the installer plus a `.sha256`
sidecar to `target/package`. See [packaging/README.md](packaging/README.md) for the
artifact names, the Linux glibc floor and the optional signing variables.

## Keyboard

The shipped map and every cheat-sheet row are generated from
`crates/firstcut-engine/resources/DefaultKeymap.json`; Settings → Keyboard records a new
chord and writes `keymap.json`. The common ones: arrows move, `0`–`5` rate, `P` picks,
`X` rejects, `G` grid, `C` compare, `Z` toggles zoom, `H` HUD, `I` info, `J` clipping,
`⌘,` Settings, `⌘↩` Finish.

## Documentation

- [docs/v2-plan.md](docs/v2-plan.md) — the contract: architecture, decisions, milestones, gates
- [docs/parity-checklist.md](docs/parity-checklist.md) — the merge gate, row by row
- [docs/control-protocol.md](docs/control-protocol.md) — drive the real window over localhost
- [docs/mcp.md](docs/mcp.md) — the command registry as MCP tools
- [packaging/README.md](packaging/README.md) — artifacts, checksums, signing
- [assets/ATTRIBUTION.md](assets/ATTRIBUTION.md) — fonts and icons

## License

GPL-3.0-only. Third-party notices live with the assets and in the packaged `LICENSE.txt`.
