//! Shared vocabulary for the whole ladder (docs/v2-plan.md §4.1): ids and their hash,
//! the read-only `Photo` view ordering/batching read, the rating and sidecar naming
//! vocabulary. Pure data and pure path math — depends on nothing.
//!
//! What lives here and why (recorded at the M1 port):
//!
//! * [`ids`] — `PhotoId`, `BatchId`, `fnv1a64`. Minted by the scanner (`firstcut-meta`),
//!   keyed on by the batcher, stored by the catalog; putting them at the bottom of the
//!   ladder is what stops any two of them drifting.
//! * [`photo`] — the `Photo` view trait + `effective_time_ms`/`time_is_fallback`. Before
//!   the port they sat in v1 `batch::view` and were read by both the ordering/batching
//!   crate and `firstcut-meta` (which implements the trait for `PhotoMeta`); a shared
//!   definition is what breaks the order↔batch↔meta cycle.
//! * [`rating`] — moved out of v1 `store::rating` verbatim. `firstcut-xmp` and
//!   `firstcut-fileops` need it and neither may depend on the catalog, which is the
//!   bottom-safe home for it.
//! * [`sidecar`] — the `<basename>.xmp` sidecar-path computation (`sidecar_path`,
//!   `legacy_sidecar_path`), moved out of v1 `xmp::document`: seconds-pure path math that
//!   the scanner (`firstcut-meta`) also needs, and meta may not depend on xmp. The rest
//!   of the sidecar reading/writing stays in `firstcut-xmp`, which re-exports these.
//! * [`row`] — `PhotoRow`, a pure-data view of one photo's row. `firstcut-fileops` reads
//!   it and `firstcut-catalog` reads/writes it; fileops may not depend on the catalog
//!   (ladder), so the shared record lifts here.
//!
//! Keep names and APIs identical to v1 so ported code and tests need no redesign.

pub mod ids;
pub mod photo;
pub mod rating;
pub mod row;
pub mod sidecar;

pub use ids::{BatchId, PhotoId, batch_id, fnv1a64, fold_to_i64, photo_id};
pub use photo::{Photo, effective_time_ms, time_is_fallback};
pub use rating::{
    ColorLabel, Flag, KEEP_STARS, Rating, RatingMode, Tier, clamp_keep_stars, display_rating, display_rating_at, display_tier, map_rating,
};
pub use row::PhotoRow;
pub use sidecar::{legacy_sidecar_path, sidecar_path};
