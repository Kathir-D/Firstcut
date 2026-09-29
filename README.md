<div align="center">

# Firstcut

**A hyper-fast, burst-aware photo culler for macOS.**

Open a folder of RAW files from a shoot, let Firstcut split it into bursts, and pick your keepers with
the keyboard — without ever waiting for a photo to load.

![Platform](https://img.shields.io/badge/platform-macOS%2015%2B-black?logo=apple)
![Apple Silicon](https://img.shields.io/badge/Apple%20Silicon-arm64-black)
![Swift](https://img.shields.io/badge/Swift-6-F05138?logo=swift&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-core-000000?logo=rust)
![Status](https://img.shields.io/badge/status-in%20development-orange)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)

</div>

> [!NOTE]
> Firstcut is in early development. There is no release to install yet — see the
> [roadmap](#roadmap) and [`task.md`](task.md) for the full plan and progress.

## Table of contents

- [Why Firstcut](#why-firstcut)
- [Features](#features)
- [How it works](#how-it-works)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Supported formats](#supported-formats)
- [Installation](#installation)
- [Architecture](#architecture)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

## Why Firstcut

A single football game can produce 1,500 RAW files, most of them bursts of 10–40 frames of the same
play. Culling that in a general-purpose photo manager means waiting: after a few dozen photos the
preview stops keeping up, and every frame shows up blurry until it finishes loading.

Firstcut does one job — deciding what to keep — and is built around two ideas:

1. **Never wait.** Every photo you can reach is already decoded and on the GPU before you get there.
2. **Cull by burst, not by file.** The shoot is split into batches (one burst or moment each), so you
   compare frames of the same play side by side and pick the sharpest ones.

Firstcut does **not** use AI to rate or reject photos. You make every call; it just gets out of the way.

## Features

- ⚡ **Instant navigation** — the previous, current, and next batch are fully decoded ahead of time,
  and the cache grows further when RAM allows. No lazy loading, no progressive blur.
- 🎞️ **Automatic burst batching** — built from capture time (to the hundredth of a second), shutter
  count, lens and exposure data, and a lightweight visual comparison. Works from slow continuous
  shooting to 40 fps bursts, and ignores file-name rollover (`IMG_9999` → `IMG_0001`).
- ⭐ **Two rating modes**
  - **Stars** — 5–4 keep, 3 good, 1–2 maybe; anything unrated is treated as not kept.
  - **Keep / Not keep** — everything starts as not kept; tap one key to keep a frame. Green and red
    rings in the filmstrip show your picks at a glance.
- 🖥️ **Feels like Finder** — large photo with a filmstrip underneath, Liquid Glass toolbar, native
  macOS look. Always dark, so it doesn't affect how you judge a photo.
- 🔍 **Focus checking** — pinch to zoom, or click any spot to jump to 100% right there (click again to
  go back). Zoom lock keeps that spot as you arrow through a burst, and an overlay shows the camera's
  autofocus point.
- ℹ️ **Lightroom-style info panel**, histogram, clipping warnings, grid, and 2–4-up compare views.
- ⌨️ **Lightroom keyboard shortcuts** out of the box, every one remappable.
- 🔁 **Undo everything and resume anytime** — your progress is saved as you go.
- 🤝 **Plays well with Lightroom and Capture One** — ratings are written to standard `.xmp` sidecars.
- 🧹 **Finish step** — once every batch is done, choose what happens to the photos you didn't keep:
  move them to Trash or a subfolder, mark them rejected, copy your keepers somewhere else, and more.
  Originals are never modified.

## How it works

1. **Open a folder** containing the whole shoot (`⌘O` or drag it onto the window).
2. Firstcut reads the metadata of every file in parallel and **groups the shoot into batches**.
3. **Cull batch by batch**: use `←` `→` to move through the frames of a burst, rate with `1`–`5` (or
   `P` in Keep mode), and move to the next batch with `⌘→` or the toolbar buttons.
4. When you're done, **Finish Cull** (`⌘↩`) shows a summary and asks what to do with the photos you
   didn't keep.

## Keyboard shortcuts

Defaults match Lightroom Classic. All of them can be changed in **Settings → Keyboard**. Zooming
uses the mouse or trackpad instead of a key: pinch, or click a spot to view it at 100%.

| Action | Shortcut |
| --- | --- |
| Previous / next photo | `←` / `→` |
| Previous / next batch | `⌘←` / `⌘→` |
| Rate 1–5 stars / clear | `1`–`5` / `0` |
| Keep (Keep mode) · Pick flag | `P` |
| Reject / unflag | `X` / `U` |
| Color labels | `6` `7` `8` `9` |
| Info panel | `I` |
| Grid / Loupe / Compare | `G` / `E` / `C` |
| Auto-advance | `Caps Lock` |
| Undo / redo | `⌘Z` / `⇧⌘Z` |
| Finish Cull | `⌘↩` |

## Supported formats

Canon is the first priority, then Sony. Every major RAW format is supported:

| Brand | Formats |
| --- | --- |
| Canon | `.CR3` (including C-RAW), `.CR2`, `.CRW` |
| Sony | `.ARW`, `.SR2`, `.SRF` |
| Nikon | `.NEF`, `.NRW` |
| Fujifilm | `.RAF` |
| Panasonic | `.RW2` |
| Olympus / OM System | `.ORF` |
| Pentax / Ricoh | `.PEF`, `.DNG` |
| Leica, Apple ProRAW, others | `.DNG`, `.RWL` |
| Hasselblad · Phase One | `.3FR`, `.FFF` · `.IIQ` |
| Samsung · Kodak · Epson · Mamiya · Leaf · GoPro | `.SRW` · `.DCR`, `.KDC` · `.ERF` · `.MEF` · `.MOS` · `.GPR` |
| Sigma | `.X3F` (embedded preview only) |
| Non-RAW | `.JPG`, `.HEIC`, `.HIF`, `.TIFF`, `.PNG` |

RAW + JPEG/HEIF pairs are treated as a single photo.

> [!WARNING]
> Firstcut has **only been tested with Canon cameras** (CR3, including C-RAW). The other formats are
> built to their published specs and should work, but haven't been tried on real files. If something
> doesn't look right with your camera, please [open an issue](https://github.com/Kathir-D/Firstcut/issues).

## Installation

> [!IMPORTANT]
> Requires a Mac with Apple Silicon running macOS 15 Sequoia or later. Liquid Glass styling needs
> macOS 26 Tahoe or later; macOS 15 gets the closest native equivalent.
>
> These instructions are for the first release (v0.1.0), which isn't out yet.

### Homebrew (recommended)

```sh
brew tap Kathir-D/tap
brew trust Kathir-D/tap
brew install --cask firstcut
```

Installs to `/Applications/Firstcut.app` and updates with `brew upgrade --cask firstcut`. **No
Gatekeeper approval needed** — see the note below.

`brew trust` is required: Homebrew 7 refuses to load casks from an untrusted tap.

> **Why a personal tap?** Homebrew's official cask repository only accepts apps that pass Gatekeeper.
> Firstcut is ad-hoc signed and not notarized (the project has no paid Apple Developer account), so
> it's distributed through its own tap instead. The cask clears the quarantine attribute after
> Homebrew has verified the download's SHA-256, so the app opens without a prompt.

### Direct download

```sh
curl -fLO https://github.com/Kathir-D/Firstcut/releases/download/v0.1.0/Firstcut-0.1.0.zip
unzip Firstcut-0.1.0.zip
sudo mv Firstcut.app /Applications/
open /Applications/Firstcut.app
```

`curl` doesn't set the quarantine attribute, so this usually opens without a prompt. If macOS does
ask, approve it once in **System Settings › Privacy & Security › Open Anyway**. A download through a
browser always needs that step.

### Build from source

**Requirements:** Xcode 26 or later, Rust (stable, via [rustup](https://rustup.rs)),
[XcodeGen](https://github.com/yonaskolb/XcodeGen).

```sh
git clone https://github.com/Kathir-D/Firstcut.git
cd Firstcut
scripts/build-app.sh
open dist/Firstcut.app
```

`scripts/build-app.sh` builds the Rust core, generates the Xcode project, builds Release, ad-hoc signs
the app, and leaves it at `dist/Firstcut.app`. To work in Xcode instead:

```sh
brew install xcodegen
scripts/build-core.sh     # Rust core + Swift bindings
xcodegen                  # generates Firstcut.xcodeproj
open Firstcut.xcodeproj   # run the Firstcut scheme
```

Run the Rust tests with `cargo test --manifest-path core/Cargo.toml`. Tests that need real photos
look in `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) and skip themselves if it's missing.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Interface | SwiftUI + AppKit | Window, toolbar, viewer, filmstrip, settings, Liquid Glass |
| Image pipeline | ImageIO, Core Image, Metal | Preview and RAW decoding, tiered cache, GPU display |
| Core | Rust (via UniFFI) | Metadata parsing, ordering, batching, SQLite session, XMP sidecars, file operations |

Details, performance targets, and design decisions are in [`task.md`](task.md).

## Roadmap

- [x] Project plan and repository
- [ ] Rust core: metadata scanning and burst batching
- [ ] Zero-wait image pipeline
- [ ] Finder-style interface with both rating modes
- [ ] Zoom, AF point overlay, info panel, compare, and grid
- [ ] Finish Cull flow and settings
- [ ] Every major RAW format
- [ ] Liquid Glass polish and macOS 15 fallback
- [ ] v0.1.0 release on GitHub and Homebrew

## Contributing

Issues and pull requests are welcome. Before starting on something big, please open an issue to
discuss it. Keep changes focused, run `cargo fmt`, `cargo clippy`, and the test suites before
submitting, and never commit photos — test images stay outside the repository.

## License

Firstcut is free software released under the [GNU General Public License v3.0](LICENSE). You can
use, study, change, and share it. If you distribute a modified version, you must release its source
code under the same license.
