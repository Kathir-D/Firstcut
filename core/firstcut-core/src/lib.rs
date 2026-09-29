//! Firstcut core library. Owner of this file: infra (module wiring only).
//!
//! Every agent's module is declared here up front so agents never need to edit this file
//! to add their own code. See docs/contracts/build.md for who owns which module.

// core-meta
pub mod formats;
pub mod meta;
pub mod scan;

// core-batch
pub mod batch;
pub mod order;

// core-store
pub mod fileops;
pub mod session;
pub mod store;
pub mod xmp;

// infra (UniFFI exports)
pub mod ffi;

// Generates the scaffolding that the Swift bindings in App/Generated/ call into.
// Must stay at the crate root; the rest of ffi.rs is the only place exports are added.
uniffi::setup_scaffolding!();
