// Owner: infra.
//
// The single place Swift code reaches the Rust core.
//
// `FirstcutCore` is the module generated from the Rust `#[uniffi::export]`s in
// `core/firstcut-core/src/ffi.rs` (see docs/contracts/build.md). It is built by
// `scripts/build-core.sh` into `App/Generated/`, and every new export appears here as a plain
// function or a type in that module.
//
// Why this file exists:
// - One import site. If the bridge ever moves (a different generator, a real module instead of a
//   static framework), only this file changes.
// - A place to hang Swift-side conveniences and to fail loudly on a contract mismatch instead of
//   letting every call site deal with it.
// - Exports should be reached through here, not by importing `FirstcutCore` all over the app.
//
// Note for agents: the types in `CoreTypes.swift` (same directory) are hand-written stand-ins for
// the generated ones. infra swaps them when the real exports land -- see the wave 2 row in
// docs/agents/infra.md. Until then, call Rust through here and mock the rest with CoreTypes.

import FirstcutCore

/// Namespace for the Rust core. Every symbol in `FirstcutCore` is a top-level function or a type,
/// so importing it into a file puts those names in the file's scope; going through this enum
/// keeps that explicit and greppable.
public enum FirstcutCoreBridge {
    /// Wave 1 smoke test. A non-empty string proves the Rust static library is linked, the
    /// bindings are current, and the FFI round trip works.
    public static var greeting: String { FirstcutCore.hello() }

    /// Version of the Rust core the app is running against, e.g. "0.1.0". Shown in About.
    public static var coreVersion: String { FirstcutCore.coreVersion() }

    /// Cheap check for code that wants to branch on the bridge being usable (tests, first-run
    /// diagnostics). Never throws, never traps.
    public static var isLinked: Bool { !greeting.isEmpty }
}
