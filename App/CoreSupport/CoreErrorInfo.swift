// Compiled into the FirstcutCore target, not the app (see project.yml).
//
// UniFFI generates `FirstcutError` as an *internal* type of the bindings module, so app code can
// catch it but cannot switch on its cases: `case .NoPhotographs` does not compile outside
// FirstcutCore. This file is the one place that can, so it converts the Rust error into a public
// value the app can actually match.
//
// The alternative -- letting every error surface as `String(reflecting:)`, which is what the
// generated `errorDescription` does -- would put `Io(message: "…")` in front of a user, and would
// make "you picked a file, not a folder" indistinguishable from "the drive went away". Both matter,
// because one is a user mistake with an obvious fix and the other is data the user cares about.
//
// The messages themselves are written here, in the app's language, not in Rust: Rust says *what
// happened* (`.NoPhotographs`), the app decides what to *say*.

import Foundation

/// A matchable description of a Rust core error.
public struct CoreErrorInfo: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case notAFolder
        case noPhotographs
        case store
        case io
        case unknown
    }

    public let kind: Kind
    /// Whatever detail Rust carried, if any. Empty for the cases that have none.
    public let message: String

    public init(kind: Kind, message: String = "") {
        self.kind = kind
        self.message = message
    }
}

extension Error {
    /// The Rust error behind this one, if it came from the core. `nil` for an error thrown by app
    /// code, so callers can tell "the core said no" from "we had a bug".
    public var asCoreError: CoreErrorInfo? {
        guard let error = self as? FirstcutError else { return nil }
        switch error {
        case .NotAFolder: return CoreErrorInfo(kind: .notAFolder)
        case .NoPhotographs: return CoreErrorInfo(kind: .noPhotographs)
        case .Store(let message): return CoreErrorInfo(kind: .store, message: message)
        case .Io(let message): return CoreErrorInfo(kind: .io, message: message)
        }
    }
}
