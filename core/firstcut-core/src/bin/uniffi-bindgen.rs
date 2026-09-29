//! Owner: infra. Thin wrapper so the UniFFI bindings generator ships inside the workspace and
//! nobody needs `cargo install uniffi_bindgen`. scripts/build-core.sh calls this.
//!
//! It is the same entry point as the `uniffi-bindgen` binary of the `uniffi` crate; we re-export it
//! so the version can never drift from the `uniffi` dependency of the core.

fn main() {
    uniffi::uniffi_bindgen_main();
}
