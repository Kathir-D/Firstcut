//! The metadata scanner: turning a folder on disk into `PhotoMeta`s (docs/contracts/photo-meta.md).
//!
//! Ported verbatim from v1 `firstcut_core` at the M1 port, with only imports re-pointed: [`meta`]
//! is what a photo is, [`scan`] is the folder walk (a re-export shim, kept so callers and the
//! contracts keep spelling it `scan::scan_folder`), [`formats`] ports as its own module. The ids
//! and the sidecar-path math this crate needs live in `firstcut-types`, below it in the ladder.
//!
//! Owner: core-meta. See docs/contracts/ for this module's contract.

pub mod formats;
pub mod meta;
pub mod scan;
