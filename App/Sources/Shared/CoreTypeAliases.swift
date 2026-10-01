// Owner: app-logic + infra.
//
// One definition per type: these are the **generated** types, under the plain names the app uses.
//
// `App/Generated/FirstcutCore.swift` is generated from `core/firstcut-core/src/ffi.rs` and carries
// an `Ffi` prefix on every type (`FfiPhotoMeta`, `FfiRating`, …). This file is where the prefix
// stops for the types that are *field-for-field the same type*: the alias is not a second
// definition, it is the same definition under the name the rest of the app already uses, so nothing
// outside the conversion layer (docs/contracts/build.md) can tell the difference.
//
// What is deliberately **not** here, and why — docs/contracts/session-api.md records the full
// reasons; the short version is that each of these is a different type from the wire type, or the
// app vocabulary is deliberately not the wire vocabulary:
//
//   `Rating`       the app builds `Rating()` everywhere and the generated memberwise init has no
//                  defaults; a defaulted init in an extension is impossible here, because delegating
//                  to the generated init of the same signature is recursion and assigning the stored
//                  properties before `self.init` is an error. Four fields is a cheap mirror.
//   `RatingMode`   `.keep` (app) vs `.keepNotKeep` (wire) — a case name cannot be aliased
//   `PhotoMeta`    still an app type in this pass (converted in CoreTypeMapping), next steps alias it
//   `AfInfo`       the generated record is *richer* (imageWidth/imageHeight/pointsInFocus); the
//                  conversion drops that data today, and aliasing would make it addressable
//   `SessionData`  `visited` is a `Set<BatchID>` (UniFFI has no Set) and it is the cached model state
//   `RatingChange` the app swaps before/after for undo; the wire type also carries `batchIndex`
//   `FinishSettings`, `FinishReportData`, `FileOpKind`, `Tier`, `VisualSig`, `FileOp`,
//   `FinishPlanData` — see session-api.md
//
// Every alias below is field-for-field, case-for-case identical to its generated type, which is why
// there is no conversion left to write: `Flag(x)` *was* `FfiFlag(x)`.

import FirstcutCore

// MARK: - Photo metadata

public typealias PhotoMeta = FfiPhotoMeta
public typealias AfInfo = FfiAfInfo

public typealias RawFormat = FfiRawFormat
public typealias FileKind = FfiFileKind
public typealias TimeSource = FfiTimeSource
public typealias CaptureTime = FfiCaptureTime
public typealias ByteRange = FfiByteRange
public typealias EmbeddedPreview = FfiEmbeddedPreview
public typealias AfPoint = FfiAfPoint

// MARK: - Ratings

public typealias Flag = FfiFlag
public typealias ColorLabel = FfiColorLabel

/// The per-tier totals come from the core's single `display_tier` (rating.rs), so `Tier` is the
/// generated enum and the app only supplies the strings the sheets show.
public typealias Tier = FfiTier

// MARK: - Session state

public typealias Batch = FfiBatch
public typealias SessionCursor = FfiCursor
public typealias MatchKind = FfiMatchKind
public typealias FileOpFailure = FfiFileOpFailure
public typealias SkippedFile = FfiSkipped

// MARK: - Codable, because the mirrors were

// The hand-written mirrors were `Codable` and exactly one of them has to stay: `Settings.keepMapping`
// stores the **color label** a keep is written as (`SettingsModel.swift`), so `ColorLabel` has to
// decode from the same string earlier versions wrote (`"red"`), not from whatever a synthesized
// conformance for a raw-value-less enum would produce. Nothing else in the app is encoded — the only
// JSON Firstcut writes is `AppSettings` and the keymap — which is why `Rating`, `AfInfo` and
// `PhotoMeta` lost a `Codable` nobody was using.
//
// It is a retroactive conformance (the type belongs to the generated module, `Codable` to the
// standard library), hence `@retroactive` — which is also what the compiler asks for. Nothing else
// here needs to be `Codable`: the only JSON the app writes is `AppSettings` and the keymap, and
// `Rating`/`AfInfo`/`PhotoMeta` were declared `Codable` without anything ever encoding them.

extension FfiColorLabel: @retroactive Codable {
    public init(from decoder: any Decoder) throws {
        switch try decoder.singleValueContainer().decode(String.self) {
        case "red": self = .red
        case "yellow": self = .yellow
        case "green": self = .green
        case "blue": self = .blue
        case "purple": self = .purple
        case let other:
            throw DecodingError.dataCorrupted(
                DecodingError.Context(
                    codingPath: decoder.codingPath, debugDescription: "not a color label: \(other)"))
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .red: try container.encode("red")
        case .yellow: try container.encode("yellow")
        case .green: try container.encode("green")
        case .blue: try container.encode("blue")
        case .purple: try container.encode("purple")
        }
    }
}

// MARK: - What the app adds to the generated types

// `Identifiable` for the two records the SwiftUI lists iterate (`\.self` ids would work, but
// `Identifiable` is what `ForEach` and the diffing want, and the `id` fields are right there), and
// `CaseIterable` for the tier list. Retroactive conformances: both the type and the protocol belong
// to other modules, which is what the attribute is for.
extension FfiPhotoMeta: @retroactive Identifiable {}
extension FfiBatch: @retroactive Identifiable {}

extension FfiTier: @retroactive CaseIterable {
    /// Spelled out rather than synthesized: a conformance declared outside the file that declares
    /// the type cannot use synthesis, and the order is the one the sheets and the summary show.
    public static var allCases: [FfiTier] { [.keep, .good, .maybe, .unrated, .rejected] }

    public var title: String {
        switch self {
        case .keep: "Keep"
        case .good: "Good"
        case .maybe: "Maybe"
        case .unrated: "Unrated"
        case .rejected: "Rejected"
        }
    }

    /// Only the Keep tier is a keep; Good and Maybe are shown as keeps in the sense of "worth a
    /// second look", and the Finish summary's split-by-tier uses this to decide what goes where.
    public var countsAsKept: Bool { self == .keep }
}

extension FfiAfInfo {
    /// The two-argument form the previews and fixtures build: no sensor frame, no focus indices,
    /// which is what "unknown" looks like on both sides.
    public init(areaMode: String, points: [AfPoint]) {
        self.init(
            areaMode: areaMode, imageWidth: 0, imageHeight: 0, points: points, pointsInFocus: [])
    }
}
