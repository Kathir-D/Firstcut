//! Owner: infra. Swift-specific UniFFI bindings generator, shipped inside the workspace so nobody
//! needs `cargo install uniffi_bindgen`. It emits the XCFramework-compatible modulemap that lets
//! Swift `import FirstcutCore` from the packaged xcframework. See scripts/build-core.sh.

fn main() {
    uniffi::uniffi_bindgen_swift();
}
