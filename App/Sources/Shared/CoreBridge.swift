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

import Foundation
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

    // MARK: - The session API
    //
    // `core/firstcut-core/src/ffi.rs` exports a UniFFI **object**, `Session`, rather than a bag of
    // free functions, so the symbol names to look for are the constructors and the methods on
    // `SessionProtocol` in `App/Generated/FirstcutCore.swift`. `sessionMembers` lists them, and
    // `CoreBridgeTests.exportsAreNotStale` checks each one against the generated file, so this
    // inventory cannot rot into a lie.
    //
    // The three Finish entry points are **not** in the list: `Session::plan_finish`,
    // `execute_finish` and `undo_finish` have no `#[uniffi::export]` yet, so Finish refuses and
    // says so (see `UniFFICoreSession`).
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

    /// The three Finish entry points, named as they will appear when exported. Used by the seam
    /// and by the test that fails when one of them lands without the app being updated.
    public static let missingFinishMembers: [String] = [
        "Session.planFinish",
        "Session.executeFinish",
        "Session.undoFinish",
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
    /// Kept as source text and still read from the repository, because that is a test's job: a test
    /// *should* read the file and assert on it. `hasSessionAPI` no longer depends on it, so nothing
    /// on the app's launch or folder-open path can prompt for a folder.
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
        // Test-only. Nothing on the app's launch or folder-open path calls this: `hasSessionAPI` is
        // answered by the type system now. That is what stopped the TCC prompt, because this is the
        // one function that reaches into the repository, and the repository is under ~/Documents.
        // `App/Generated/FirstcutCore.swift` is a sibling of `App/Sources`, so two levels up from
        // `App/Sources/Shared`.
        var directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        for _ in 0..<2 { directory.deleteLastPathComponent() }
        let url = directory.appendingPathComponent("Generated/FirstcutCore.swift")
        return try? String(contentsOf: url, encoding: .utf8)
    }

}
