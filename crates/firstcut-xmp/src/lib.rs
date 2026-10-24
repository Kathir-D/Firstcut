//! XMP sidecars: reading them, writing ratings into them, the mapping and the writer thread.
//!
//! Ported verbatim from v1 `firstcut_core::xmp` (as module `xmp`) at the M1 port. Callers in the
//! ladder spell it `firstcut_xmp::xmp::…` — parallel to `firstcut_meta::meta::…` — so contract
//! wording only changes its prefix. The sidecar-path math (`sidecar_path`,
//! `legacy_sidecar_path`) is re-exported from `firstcut-types`, where it lives for the scanner.
//!
//! Firstcut never modifies a photo. Everything it knows about a photo's rating is written to
//! XMP sidecars, so the same ratings show up in Lightroom or darktable; this module owns that file
//! format. Owner: core-xmp (see docs/contracts/ for this module's contract).

pub mod xmp;
