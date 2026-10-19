//! LibRaw bindings backend for [`firstcut-decode`]: develops RAW sensor data through
//! LibRaw (approved C/C++ dependency, decision D3 in docs/v2-plan.md).
//!
//! THE SINGLE `unsafe` CARVE-OUT OF THE WORKSPACE — docs/v2-plan.md §4.5. The
//! workspace forbids `unsafe_code` everywhere; this crate's own `[lints.rust]`
//! table (in Cargo.toml) is the one permitted exception, and `cargo xtask layers`
//! fails CI if any other crate carries its own lints table. Unsafe code here must
//! stay confined to FFI bindings and be shadowed by checked, safe wrappers the
//! rest of the ladder calls.
//!
//! The LibRaw C++ sources themselves are vendored and wired up at M2
//! (docs/v2-plan.md §6); this crate exists now so the carve-out and the layer
//! exception are in place before the first bindings lines are written.
