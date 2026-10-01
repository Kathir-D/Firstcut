# Contract: PhotoMeta & scanning

- **Owner:** core-meta
- **Consumers:** core-batch, core-store, pipeline, app-logic, ui
- **Version:** v0.2 (draft; frozen as v1.0 at the end of wave 1)

## Types (Rust, exported to Swift through UniFFI)

```rust
/// Stable across runs: FNV-1a of the path relative to the session folder, folded into 63 bits so
/// the id round-trips through the store's `i64` column (batch/mod.rs:32, :84).
pub struct PhotoId(pub u64);

pub enum FileKind { Raw(RawFormat), Jpeg, Heif, Tiff, Png }
pub enum RawFormat { Cr3, Cr2, Crw, Arw, Sr2, Srf, Nef, Nrw, Raf, Rw2, Orf, Pef, Dng, Rwl,
                     ThreeFr, Fff, Iiq, Srw, Dcr, Kdc, Erf, Mef, Mos, Gpr, X3f }

pub struct CaptureTime {
    pub unix_ms: i64,              // DateTimeOriginal + SubSecTimeOriginal, converted to UTC using offset if present
    pub subsec_resolution_ms: u16, // 10 for Canon R8 (two sub-second digits); 1000 for zero or one;
                                    // 1 for three or more (meta/mod.rs:777)
    pub offset_minutes: Option<i16>,
    pub source: TimeSource,        // Exif | FileModified (fallback, must be flagged)
}

pub struct ByteRange { pub offset: u64, pub len: u64 }

pub struct EmbeddedPreview { pub range: ByteRange, pub width: u32, pub height: u32 } // JPEG bytes inside the file

pub struct AfPoint { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub in_focus: bool } // normalized 0..1 over the AF image rectangle
pub enum TimeSource { Exif, FileModified }

/// core-meta's AfInfo (meta/cr3.rs:115). core-batch's JSON fixture adapter carries the narrower
/// two-field shape (batch/fixture.rs:112); the two must not be confused.
pub struct AfInfo {
    pub area_mode: Option<String>,
    pub image_width: u32,          // the AF image rectangle the points are normalized over
    pub image_height: u32,
    pub points: Vec<AfPoint>,
    pub points_in_focus: Vec<u16>,
}

pub struct PhotoMeta {
    pub id: PhotoId,
    pub rel_path: String,           // primary file (RAW if a pair)
    pub companions: Vec<String>,    // paired JPEG/HEIF with the same base name + existing .xmp
    pub kind: FileKind,
    pub file_size: u64,
    pub capture_time: Option<CaptureTime>,
    pub shutter_count: Option<u64>,
    pub file_number: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub camera_serial: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub exposure_time_s: Option<f32>,
    pub f_number: Option<f32>,
    pub iso: Option<u32>,
    pub exposure_comp_ev: Option<f32>,
    pub metering_mode: Option<String>,
    pub drive_mode: Option<String>,
    pub shutter_mode: Option<String>,
    pub orientation: u8,            // EXIF 1..8
    pub width: u32,
    pub height: u32,                // sensor orientation, before applying orientation
    pub af: Option<AfInfo>,
    /// The 1620x1080 `PRVW` JPEG. NOT full resolution, and not what the loupe draws: it is 7% of the
    /// pixels of a 6000x4000 frame, so a viewer built on it silently caps at 1620 px
    /// (meta/mod.rs:117, meta/cr3.rs:150, todo.md §7.5). It is the cheap first-photo fast path.
    pub preview: Option<EmbeddedPreview>,
    /// The full-resolution JPEG (6000x4000 on an R8) from the CR3's first image track. **This is the
    /// display image**: the app decodes display bitmaps straight from this byte range, so ImageIO
    /// never parses the CR3 container (meta/mod.rs:120, meta/cr3.rs:153, todo.md §7.5). A CR3 carries
    /// three JPEGs — `THMB` 160x120, `PRVW` 1620x1080, and this one — and conflating them is the bug
    /// that was made twice (todo.md §7.5).
    pub full_preview: Option<EmbeddedPreview>,
    /// `st_dev` of the file, so a session can tell a rename from a reshoot (REV-68).
    pub device: Option<i64>,
    /// `st_ino`, which survives a rename inside a volume.
    pub ino: Option<i64>,
    pub warnings: Vec<String>,      // non-fatal parse issues
}

pub struct Skipped { pub rel_path: String, pub reason: String }
pub struct ScanResult { pub photos: Vec<PhotoMeta>, pub skipped: Vec<Skipped> }
```

`EmbeddedPreview` is `{ range: ByteRange, width: u32, height: u32 }` (`meta/cr3.rs:91`) — a byte
range inside the file, plus the dimensions, so nothing has to re-derive them. `core-meta`'s
`AfInfo` is **richer** than the two-field shape core-batch's fixture adapter uses: it also carries
`image_width` / `image_height` (the AF image rectangle, used to normalize the points) and
`points_in_focus` (`meta/cr3.rs:115`; the narrower `batch::fixture::AfInfo` is at
`batch/fixture.rs:112`).

## Functions

```rust
/// Parallel, reads headers only. Must never read whole files.
pub fn scan_folder(folder: &Path) -> Result<ScanResult, ScanError>;

/// One photograph from one header read, for the first-photo fast path (todo.md §7.3). The same
/// `meta_for` call the full scan makes, so it returns byte-for-byte what the scan will return for
/// that file. It is the first photograph *by name*, not by capture time — reading the time is what
/// the scan is for.
pub fn read_photo(folder: &Path, rel_path: &str) -> Option<PhotoMeta>;      // meta/mod.rs:484

/// The file `read_photo` would read, from a directory listing and no header read at all. `None` for
/// a folder with no photo files, so the caller can fall back to the normal open.
pub fn first_photo_name(folder: &Path) -> Option<String>;                  // meta/mod.rs:543
```

`meta_from_imageio` — the escape hatch for a file the scanner cannot parse, where Swift passes
ImageIO's properties in — is **not implemented**. `scan/mod.rs:16-19` says so and deliberately leaves
the `ImageIoProps` shape unagreed rather than shipping two spellings of the same record. In the
meantime every non-CR3 format goes through the generic reader, which **never fails** on a file it
does not understand: the photo appears, ordered by file time, with the reason in `warnings`
(`meta/mod.rs:686-691`).

## Guarantees

- `scan_folder` on 1,500 Canon R8 CR3 files takes < 3 s on an M1 Pro (internal SSD). Measured
  **0.535–0.587 s cold, 0.054–0.056 s warm** scaled to 1,500 photos on the real test games —
  0.35–0.39 ms per photo cold, so it is I/O-bound and linear (`firstcut bench --folder`, release
  build; `docs/qa/perf-baselines.md`).
- `photos` and `skipped` are both **sorted by `rel_path`**, so two scans of the same folder produce
  the same ids in the same sequence and a session database written from one scan matches the next
  (REV-17, `meta/mod.rs:466`). Ordering *into* batches is still core-batch's job; this sort is only
  about reproducibility.
- Pairing: files that share a **group key** form one `PhotoMeta`, where the key is the relative path
  with the file's extension removed — `IMG_0001.CR3` and `IMG_0001.JPG` group, `Sat.1/IMG_0001.CR3`
  and `Sun.1/IMG_0001.CR3` do not (`group_key`, `meta/mod.rs:280`). Only the extension is stripped,
  never a dot inside a folder name, because card-shot folders really are called `Sat.1`. RAW is
  primary; JPEG/HEIF go into `companions`, alongside both sidecar spellings (`<base>.xmp` and the
  legacy `<base>.<ext>.xmp`) when they exist (`meta/mod.rs:399-418`).
- A file that can't be parsed shows up in `skipped` with a reason. It never panics and never aborts
  the scan. So does a **second RAW in one group**: it is neither a companion nor a photo, and
  dropping it quietly is exactly what "never skip a file macOS can read" forbids
  (`meta/mod.rs:425-438`).
- `.xmp` files are never photos, `.`-files and `._` AppleDouble sidecars are never photos, and any
  subfolder containing the `.firstcut-finished` marker Finish Cull drops in is skipped, so photos the
  user just moved away do not reappear in the next scan (`meta/mod.rs:192, 622-648`).
- Scan errors are `FolderNotFound` / `NotAFolder` / `Io`, never a panic and never an empty shoot:
  a missing folder is an error, and a genuinely empty folder scans to nothing
  (`ScanError`, `meta/mod.rs:155`; tests at `:1140-1160`). An empty `ScanResult` after a folder that
  became empty is returned as a result, not an error.

## Fixtures for consumers

- **Available now:** `tests/fixtures/exiftool/<game>.json`: raw exiftool output for all 2,880 test
  photos (exiftool tag names, numeric values for size/focal length/exposure/ISO/orientation/dimensions),
  sorted by capture time. Swift mocks decode these into `PhotoMeta`
  (`App/Sources/Session/FixturePhotos.swift:280-308`).
- **Also available now:** `tests/fixtures/meta/<game>.json`: a `Vec<PhotoMeta>` in camelCase JSON with
  this struct's field names, generated by `firstcut dump-meta --from-exiftool` and committed for all
  four games. core-batch's CI regression tests run entirely on these, so they need none of the 42 GB
  of RAW files (`core/firstcut-core/tests/batching.rs:3, 31`), as do the Swift integration and
  performance suites.

**Swift types.** Use `App/Sources/Shared/CoreTypeAliases.swift`, not `CoreTypes.swift`. Most of what
used to be hand-mirrored in `CoreTypes.swift` is now the *generated* `Ffi*` type under its plain name
— `PhotoMeta`, `AfInfo`, `RawFormat`, `FileKind`, `TimeSource`, `CaptureTime`, `ByteRange`,
`EmbeddedPreview`, `AfPoint`, `Tier` — so `PhotoMeta` **is** `FfiPhotoMeta` (`CoreTypeAliases.swift:35-52`).
The app types that stayed, and the reason each one stays, are listed in
[session-api.md](session-api.md) §Mock; `CoreTypes.swift` keeps only `PhotoID`/`BatchID`, `VisualSig`,
`RatingMode`, `Rating` and `ColorLabel`.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
- v0.2 (2026-10-01): matched the contract to `core/firstcut-core/src/meta/`. **`PhotoMeta` gained
  `full_preview`**, the full-resolution JPEG byte range from the CR3's first image track, and it is
  the display image — while `preview` is the **1620×1080 `PRVW`, not full size**, which is the mistake
  todo.md §7.5 records as made twice; both are now spelled out in the struct. Added `device` / `ino`
  (rename vs reshoot, REV-68). Corrected `AfInfo` to core-meta's five-field shape and added the
  `read_photo` / `first_photo_name` functions behind the first-photo fast path; noted that
  `meta_from_imageio` is still unimplemented and that non-CR3 formats go through the never-failing
  generic reader. Corrected the guarantees: `photos` **is** sorted by `rel_path` (REV-17), a second RAW
  in a group is reported in `skipped` rather than dropped, `.xmp` / AppleDouble / `.firstcut-finished`
  folders are not photos, and the scan numbers are now the measured ones. Fixtures: both
  `exiftool/` and `meta/` sets exist and are committed. **Swift types**: replaced "use the Swift types
  in `CoreTypes.swift`" with the `CoreTypeAliases.swift` wording session-api.md uses — most app types
  are the generated `Ffi*` types under plain names.
