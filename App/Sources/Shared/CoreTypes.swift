// Owner: infra + app-logic.
//
// The app's core types. **Most of what used to be hand-mirrored here is now the generated
// `FirstcutCore` type under the plain name** — see `CoreTypeAliases.swift`, which is where the
// `Ffi*` prefix stops. The types that stayed are the ones the generated type cannot express or is
// the wrong shape for (docs/contracts/session-api.md, "Swift types"): the app's own vocabulary, or
// a record it needs richer than the wire type. Don't add a mirror of a generated type here; add the
// alias.
//
// This file is part of the conversion layer (docs/contracts/build.md), which is why it imports
// `FirstcutCore`: the app types below are built *on* the generated ones (`Rating.flag` is
// `FfiFlag`), and a default value like `flag: Flag = .none` names a generated case. Nothing else in
// `App/Sources` outside the conversion layer may import it.

import FirstcutCore
import Foundation

public typealias PhotoID = UInt64
public typealias BatchID = UInt64

/// A perceptual signature for a photograph: a 64-bit difference hash plus a 48-bin colour
/// histogram (16 each for R, G, B). The core's `batch()` compares these to decide the ambiguous
/// boundaries timing alone cannot (todo.md §5.2). **Still an app type**: Rust packs the histogram
/// as a fixed 48-byte array and Swift wants `[UInt8]`, so the one conversion left in
/// `CoreTypeMapping` is not a no-op.
public struct VisualSig: Sendable, Equatable {
    public var dhash: UInt64
    public var hist: [UInt8]  // 48 values: 16 bins each for R, G, B

    public init(dhash: UInt64, hist: [UInt8]) {
        self.dhash = dhash
        self.hist = hist
    }
}

/// The app's rating-mode vocabulary. **Not** the generated `FfiRatingMode`: Rust spells the keep
/// mode `KeepNotKeep`, Swift spells it `keep`, and a case name cannot be aliased. The database
/// string is `"keep"` on both sides, so this is vocabulary only (see `CoreTypeMapping`).
public enum RatingMode: String, Sendable, Codable { case stars, keep }

/// The app's rating. The four fields are the generated `FfiRating`'s, but this stays a struct: the
/// app writes `Rating()` and `Rating(stars: 4)` in dozens of places and the generated initializer has
/// no defaults, and a defaulted one cannot be added in an extension (delegating to the generated
/// initializer of the same signature is infinite recursion; assigning the stored properties before
/// `self.init` is an error). `CoreTypeMapping` therefore still converts, and the conversion is four
/// assignments. `flag` and `label` are the generated types under their plain names.
public struct Rating: Sendable, Equatable {
    public var stars: UInt8 = 0
    public var flag: Flag = .none
    public var label: ColorLabel? = nil
    public var keep: Bool = false
    public init(stars: UInt8 = 0, flag: Flag = .none, label: ColorLabel? = nil, keep: Bool = false) {
        self.stars = stars
        self.flag = flag
        self.label = label
        self.keep = keep
    }

    /// Nothing set. "The user has not said anything about this photo" is a different question from
    /// "rated zero stars", and the cull rules ask it.
    public var isNeutral: Bool { self == Rating() }
}

/// The colour labels Lightroom shows (todo.md §6.3). An app type rather than the generated
/// `FfiColorLabel` because `Settings.keepMapping` persists one (a keep can be a colour label), and
/// `Codable` cannot be retroactively conformed onto an imported type without the compiler warning
/// and `swift-format lint` disagreeing about it (see `CoreTypeAliases.swift`). The strings are the
/// wire's strings, so a settings file written by any version decodes.
public enum ColorLabel: String, Sendable, Codable {
    case red
    case yellow
    case green
    case blue
    case purple
}
