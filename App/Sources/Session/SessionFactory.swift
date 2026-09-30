// Owner: app-logic.
//
// What the app opens a folder with.
//
// `SessionFactory` is the single decision point, and today it has only one answer worth having:
// `CoreSessionBackend` over `UniFFICoreSession` over the real generated
// `FirstcutCore.Session` (session-api.md §"Session object"). That means the app's folder open is a
// real `Session::open`: it scans with core-meta's CR3 parser, orders with core-order, batches with
// core-batch, and restores a real SQLite database in Application Support, with XMP sidecars
// debounced behind it.
//
// `FileSession` is the fallback for two cases and no more:
//   * the generated bindings do not declare `Session` (a stale `App/Generated`, or a core build
//     from before the exports landed) — `FirstcutCoreBridge.hasSessionAPI` reads the generated file
//     and says so; and
//   * tests that want a folder with no database anywhere near the developer's home directory, which
//     is what `sessionsDir` is for.
//
// What `FileSession` is **not** is the shipped path any more. It is a real scan with real EXIF
// read through ImageIO, and its ratings are in memory for the life of the process.

import Foundation

public enum SessionFactory {
    /// The shipped choice. One `if`, so the fallback is greppable rather than spread around.
    @MainActor
    public static func live() -> (URL) throws -> any SessionBackend {
        if FirstcutCoreBridge.hasSessionAPI {
            { url in try CoreSessionBackend.open(folder: url) }
        } else {
            { url in try FileSession.open(url) }
        }
    }

    /// The real Rust session, with the database somewhere the caller chooses. What the tests use.
    @MainActor
    public static func rust(sessionsDir: URL) -> (URL) throws -> any SessionBackend {
        { url in try CoreSessionBackend.open(folder: url, sessionsDir: sessionsDir.path) }
    }

    /// Always the file-system path, whatever the generated bindings say. Real files, real EXIF,
    /// no persistence — for tests and for the "the core is not here" case above.
    @MainActor
    public static func fileSystem() -> (URL) throws -> any SessionBackend {
        { url in try FileSession.open(url) }
    }

    /// Which of the two a caller is about to get. Shown in About and printed in a screenshot
    /// check, because "real photos" and "ratings that survive a quit" are different claims.
    public static var backendName: String {
        FirstcutCoreBridge.hasSessionAPI
            ? "Rust core \(FirstcutCoreBridge.coreVersion) (SQLite + XMP sidecars)"
            : "file system (no session database yet)"
    }

    /// Whether Finish can run: the Rust core exports the three Finish entry points, so this is true
    /// whenever the session is the real one.
    public static var canFinish: Bool { FirstcutCoreBridge.hasSessionAPI }
}

/// A session over a real folder, with no persistence behind it.
@MainActor
public enum FileSession {
    /// Sequential scan. For tests and small folders.
    public static func open(_ folder: URL) throws -> any SessionBackend {
        MockSession(data: try PhotoFolderScanner.scan(folder))
    }

    /// Concurrent scan with progress, for the app: 2 880 CR3 headers is seconds of work and the
    /// window needs to say so.
    public static func open(
        _ folder: URL, progress: @escaping @Sendable (Int, Int) -> Void
    ) async throws -> any SessionBackend {
        MockSession(data: try await PhotoFolderScanner.scan(folder, progress: progress))
    }
}
