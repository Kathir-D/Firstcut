# Changelog

All notable changes to Firstcut are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/).

## [unreleased — v2]

The v2 rewrite lives on the `v2` branch ([docs/v2-plan.md](docs/v2-plan.md)): the core Rust
logic is a one-way ladder of domain crates (`crates/*`, `apps/firstcut-cli`), the Swift/Xcode
app tree is gone in favour of the native egui app (`apps/firstcut`), and the never-crash lint
sweep (§4.5) is enforced across the workspace.

### Added and changed in v2

- Native app on macOS, Windows and Linux (D1), with one bundled Inter family so glyphs and
  snapshots match across the three OSes (D10).
- Burst batching, loupe, grid, 2/3/4-up compare, AF and clipping overlays, histogram, info
  panel, both rating modes, Finish Cull with a typed delete gate and undo, Settings, keymap
  recorder/import/export, recent shoots, live folder watching and sidecar import.
- Exact RAW sensor develop through rawler on every OS, with LibRaw as the macOS/Linux fallback
  (D11). Progressive display keeps the loupe on the embedded preview until the develop lands
  (D12); the warm 24 MP develop is 0.343 s on the M1 Pro, first pixels 0.057–0.068 s. A held
  arrow key paints the camera's 6–8 ms `PRVW` navigation preview, the sharp display is
  prefetched 32 ahead/12 behind (four workers measure 54–59 frames/s, so a 20 Hz hold has
  ~3× headroom), and the exact develop waits for a 250 ms settle (D13). Scrolling neither
  blanks nor starves the visible frame.
- Native folder picker via `rfd`, including camera-card roots with multiple rolls asking
  “Choose a Shoot”.
- `cargo xtask package`: macOS `.app` + zip + cask stamp, Windows zip, Linux AppImage + `.deb`,
  SHA-256 sidecars, and a tag-driven 3-OS release workflow. Signing is optional.
- `firstcut --control <port>` JSON-lines control channel and `firstcut-cli mcp` over stdio;
  both documented in `docs/control-protocol.md` and `docs/mcp.md`.
- The Rust core is covered by workspace tests plus opt-in real-photo tests; the 3-OS CI matrix
  runs `cargo xtask ci` and the window smoke on every push.

## [0.1.0] - unreleased

The first release.

### Added

- Open a folder of photos, split it into bursts automatically, and cull batch by batch from the
  keyboard, with Lightroom-style shortcuts that can all be remapped.
- Burst batching from capture time (10 ms resolution), shutter count, orientation, focal length,
  exposure, and a perceptual hash of each frame, computed in the background and used to settle the
  boundaries that timing alone cannot decide. Single frames a few seconds apart are grouped.
- Two rating modes: Stars, and Keep / Not keep with green and red rings.
- Loupe, Grid and Compare views (2-, 3- and 4-up, with synchronized zoom). Pinch or click to zoom to
  100%, drag or scroll to pan, and zoom lock to compare a burst on the same detail.
- Autofocus point overlay, highlight and shadow clipping overlay, histogram, and a Lightroom-style
  info panel whose fields you can choose.
- Ratings are saved to a session database as you go and mirrored to `.xmp` sidecars that Lightroom
  and Capture One read. Undo and redo, resume where you left off, and import of ratings already in
  sidecars when a folder is opened for the first time.
- Finish Cull: move, trash, delete or mark the photos you did not keep, and copy, move or split the
  ones you did, with a dry-run preview, a typed confirmation for permanent deletion, and Undo Finish.
- A Settings window (General, Keyboard, Viewer, Metadata, Performance) with a shortcut editor.
- A Welcome window with recent folders and how far through each one you got.
- The folder is watched: new files appear and deleted files drop out while you cull.
- Metadata for JPEG, HEIC, PNG, TIFF and the TIFF-based RAW formats, and never skipping a photo that
  macOS can open.
- Homebrew cask and GitHub release, ad-hoc signed.
- Every RAW format listed on raw.pixls.us is opened, including Canon CRW, Minolta MRW, Olympus ORI
  and older Kodak, Phase One, Sinar, Casio and ARRI files. `firstcut verify <folder>` compares the
  metadata reader with `exiftool` file by file.
- A RAW that macOS cannot decode is shown from the JPEG the camera stored inside it.
- Optional **Reject Whole Batch** (`⇧X`), off by default: flags every frame of a burst as rejected
  in one step, which one undo takes back. Turn it on in Settings → General.

### Fixed

- The "Exact RAW" develop was rendered into sRGB, which clipped the saturated colours a sports frame
  is full of (jerseys, signage, LED boards; up to 7% of a frame) and put them ΔE 3–4.5 away from
  what Preview and Photos show for the same file. It now renders in Display P3 and matches them.
  Wide-gamut photos (P3 HEIC, Adobe RGB TIFF) also keep their gamut, and the histogram and clipping
  overlay read them in their own colour space.

Defects found in a full audit of `core/` and `App/`, grouped by what they would have cost a user.
None of these had a reported symptom, which is why they are worth writing down.

**Could have lost a photograph**

- A cross-volume Finish move copied the file, compared a byte count read from the page cache, and
  unlinked the original — a power loss in between left a truncated copy and no original. The copy is
  now on the device before the source is removed, and the destination is claimed with `create_new`
  instead of being checked with `exists()` (`fs::copy` truncates, so a file restored by a sync client
  in that window was destroyed).
- A `rel_path` from the session database reached a file operation through `folder.join()` unchecked.
  `join` replaces the base on an absolute component, so a row reading `../../../Thesis.pdf` became a
  delete outside the shoot folder. Every path that reaches a file operation is now checked to land
  under the shoot.
- Renaming a session database checkpointed nothing, so a failure between moving the database and
  moving its write-ahead log left a database whose recent transactions were stranded in the old log.
- Marking a photograph "not keep" deleted the colour label the user had put on it, because an absent
  label means *remove the attribute*. The app still had it; Lightroom's copy lost it.
- An `xmp:Rating="0"` written by another tool imported as a real rating, giving every never-rated
  frame in an imported shoot a row and moving the Finish counts.
- The sidecar's temporary file was opened with `File::create`, which follows a symlink, at a name
  predictable from the process id. It is now claimed with `create_new`.

**Could have stopped a folder opening**

- One unreadable entry during a scan failed the whole scan instead of skipping one photograph.
- A 64-bit box size was added to a file offset without a check, and a 17-digit EXIF year overflowed
  the calendar arithmetic.
- The SubIFD list and the IFD entry count were both read from the file and unbounded, and an AF record
  claiming 32,767 points cost ~10⁹ comparisons on a scan thread.
- A filename containing `\` had it rewritten to `/`, so `a\b.JPG` and `a/b.JPG` became one photograph.
- A `\` or an unreadable file also had no fallback to the file's own timestamp, so a body with no
  clock produced no capture time at all.

**Wrong information shown**

- A decode whose file changed while it ran kept its in-flight slot, and after a handful of those the
  engine refused every job for the rest of the session — including the idle wait every test and the
  progress bar use.
- Evicting a stale thumbnail also evicted the same photograph's display bitmap and its 92 MB RAW
  develop, so the RAW tier re-developed on every arrow key.
- A display decode scheduled by the histogram lost the EXIF orientation, so a rotated photograph was
  cached and drawn sideways.
- ⇧1–⇧5 moved the cursor twice per press with auto-advance on, so every second photograph was
  skipped and silently left unrated.
- The Settings → Viewer clipping thresholds were read through an existential and always came back
  as the defaults, so the sliders moved nothing.
- The typed `DELETE` confirmation survived pressing Back and changing the destination, leaving the
  Finish button armed for a list the user had not looked at.
- A capture time that came from the file system rather than the camera still produced a hard split.
- Boundary precision could be reported as 100% while a false positive was counted, because the
  denominator was taken before the shoot's first frame was filtered out.
- An ambiguous boundary was reported to the CLI as settled when it had scored below the split
  threshold — exactly the boundary a human is being asked to check.

**Release**

- The release workflow ran neither the Swift tests nor the lint, and CI is skipped for tags, so a tag
  push could publish an artefact whose tests failed. Both now run before the app is built.
- CI and release had no timeout. The documented failure mode of a build on this repo is a hang, not a
  failure, so a hung job held the queue for GitHub's six-hour default.

### Known limitations

See [Known limitations](README.md#known-limitations) in the README.
