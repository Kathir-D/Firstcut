//! Owner: core-meta. See docs/contracts/ for this module's contract.
//!
//! The folder walk itself lives in [`crate::meta`], next to the types it produces, so there is one
//! copy of the pairing rules rather than two that drift. This module is the name the rest of the
//! core and the contract reach for: `scan::scan_folder`.
//!
//! The split is deliberate. `meta` is "what a photo is", `scan` is "how you find the photos in a
//! folder" — and a caller that only wants to walk a folder should not have to know that the
//! metadata types live next door.

pub use crate::meta::{
    FileKind, PhotoMeta, RawFormat, ScanError, ScanResult, Skipped, TimeSource, group_key,
    scan_folder,
};

// `docs/contracts/photo-meta.md` also specifies `meta_from_imageio`, for the files the scanner
// cannot read and the pipeline has to hand back from ImageIO. It is not implemented yet: the
// `ImageIoProps` shape is agreed with the pipeline, and inventing it here would only give two
// spellings of the same record to reconcile later.
