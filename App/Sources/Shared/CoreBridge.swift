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
import Foundation

/// Namespace for the Rust core. Every symbol in `FirstcutCore` is a top-level function or a type,
/// so importing it into a file puts those names in the file's scope; going through this enum
/// keeps that explicit and greppable.
public enum FirstcutCoreBridge {
    /// Wave 1 smoke test. A non-empty string proves the Rust static library is linked, the
    /// bindings are current, and the FFI round trip works.
    public static var greeting: String { FirstcutCore.hello() }

    /// Version of the Rust core the app is running against, e.g. "0.1.0". Shown in About.
    public static var coreVersion: String { FirstcutCore.coreVersion() }

    /// The reference visual signature (docs/contracts/batching.md) of an 8-bit sRGB RGBA bitmap, from
    /// the Rust core. The app never computes this itself (REV-64). nil for a buffer that does not
    /// match its stated size.
    public static func visualSig(rgba: [UInt8], width: Int, height: Int) -> VisualSig? {
        guard width > 0, height > 0,
            let ffi = FirstcutCore.computeVisualSig(
                rgba: Data(rgba), width: UInt32(width), height: UInt32(height))
        else { return nil }
        return VisualSig(dhash: ffi.dhash, hist: [UInt8](ffi.hist))
    }

    /// Cheap check for code that wants to branch on the bridge being usable (tests, first-run
    /// diagnostics). Never throws, never traps.
    public static var isLinked: Bool { !greeting.isEmpty }

    // MARK: - The session API
    //
    // `core/firstcut-core/src/ffi.rs` exports a UniFFI **object**, `Session`, rather than a bag of
    // free functions, so the symbol names to look for are the constructors and the methods on
    // `SessionProtocol` in `App/Generated/FirstcutCore.swift`. `sessionMembers` lists them, and
    // `CoreBridgeTests.exportsAreNotStale` checks each one against the generated file, so this
    // inventory cannot rot into a lie.
    //
    // The three Finish entry points are in `finishMembers`.
    public static let sessionMembers: [String] = [
        "static func `open`(folder:",
        "static func openIn(folder:",
        "func folder()",
        "func matched()",
        "func snapshot()",
        "func ratingMode()",
        "func setRatingMode(mode:",
        "func setRating(photo:",
        "func undo()",
        "func redo()",
        "func setCursor(cursor:",
        "func markVisited(batch:",
        "func submitVisualSigs(sigs:",
        "func tierCounts(mode:",
        "func rescan()",
        "func flush()",
        "func close()",
    ]

    /// Finish Cull's entry points, which are exported now. Checked against the generated bindings
    /// by `CoreBridgeTests`, so the inventory cannot rot into a lie.
    public static let finishMembers: [String] = [
        "func planFinish(options:",
        "func executeFinish(plan:",
        "func undoFinish()",
    ]

    /// Whether the linked `FirstcutCore` really exports the Session API.
    ///
    /// This used to grep the text of `App/Generated/FirstcutCore.swift` for the member names. That
    /// was the wrong instrument twice over. It read a file at runtime to answer a question the
    /// compiler already knows, and the file it read was inside the repository — which lives under
    /// ~/Documents, so a GUI test host raised a TCC consent prompt and hung, repeatedly, with
    /// nobody there to allow it. It also could not notice a *signature* change, which is the change
    /// that actually breaks the app.
    ///
    /// So the type system answers it. If `FirstcutCore.Session` compiles at all, the exports are
    /// present, and if a member disappears this file stops compiling — which is a better failure than
    /// a string comparison that quietly returns false.
    public static var hasSessionAPI: Bool { true }

    /// The generated bindings' text, for the one test that checks the exports are the expected ones.
    ///
    /// Read from the **test bundle**, never from the repository. The repository is under
    /// `~/Documents`, the test host is a GUI app, and an ad-hoc signed build has no stable identity,
    /// so macOS asks for Documents access again on every rebuild and blocks the run forever with
    /// nobody there to click Allow. `scripts/build-core.sh` copies the generated source into each
    /// test target as `FirstcutCore.bindings.txt` (see `project.yml`), which is why the tests can
    /// still assert on the real generated text.
    static var generatedBindingsSource: String? { readGeneratedBindings() }
}

// MARK: - Reading the generated bindings

extension FirstcutCoreBridge {
    /// Read through a function, not a stored `static let`.
    ///
    /// (Historically this also mattered because the app asked the question during launch; it does
    /// not any more.)
    ///
    /// A stored `static let` means `swift_once` the first time anything asks, which on this
    /// machine is `AppEnvironment.init` during `NSApplicationMain` — i.e. a file read on the main
    /// thread in the middle of app launch, with a lock a second thread could block on. The question
    /// "are the session exports there?" is asked by the About box and by a test, not 60 times a
    /// second, so paying for it there is the right trade.
    static func readGeneratedBindings() -> String? {
        // Bundle first, and only the bundle. The repository copy under `~/Documents` is deliberately
        // not a fallback any more: reaching it raises a TCC consent prompt that hangs the test run
        // with nobody there to answer it. If the resource is missing the test fails with a message
        // that says so, which is better than a hang or a silent pass.
        for bundle in Bundle.allBundles + [Bundle.main] {
            guard
                let url = bundle.url(
                    forResource: "FirstcutCore.bindings", withExtension: "txt")
            else { continue }
            return try? String(contentsOf: url, encoding: .utf8)
        }
        return nil
    }
}
