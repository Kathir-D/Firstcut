// Owner: qa.
//
// The independent half of the ground-truth review. todo.md §12 says the batching review is "done by
// looking at the photos", and senior-dev's REV-55 says ground truth that the same agent produces
// and scores cannot certify itself. This file is qa's half of that separation:
//
//   * **core-batch** produces boundaries with `batch()` and scores itself against
//     `tests/fixtures/ground-truth/<game>.json`.
//   * **qa** derives the *candidate* boundaries here, from the committed exiftool fixtures only —
//     capture-time gaps, EXIF orientation changes, ShutterCount jumps — with no access to the
//     batcher's output, and then looks at the photos for each one.
//
// The two lists are compared later. A boundary this file flags and core-batch joins is a missed
// split; a boundary core-batch splits and this file does not flag is either a legitimate visual
// call or a false positive, and the photos decide which.
//
// Everything here is deterministic: same fixtures in, same sample out, byte for byte, so the
// audit is reproducible and senior-dev can check which boundaries were sampled.

import Foundation

/// How qa sorts a gap between two consecutive photos, from the committed fixtures alone.
public enum GapClass: String, Sendable, CaseIterable {
    /// Faster than any burst the cameras produce in this set (fastest observed: 40 ms, Game1JENKS).
    /// Not a boundary on its own.
    case intraBurst
    /// 0.5 s … 2 s. The ambiguous zone: the shutter may have been released and pressed again during
    /// the same play. This is where visual refinement has to decide, and where every single
    /// boundary must be checked by eye (todo.md §12).
    case ambiguous
    /// > 2 s. The shutter was released for a while; a boundary without looking.
    case hard
}

/// Why a boundary was sampled. A boundary can be sampled for several reasons at once.
public struct SampleReason: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }

    public static let ambiguousGap = SampleReason(rawValue: 1 << 0)
    public static let orientationChange = SampleReason(rawValue: 1 << 1)
    public static let shutterCountJump = SampleReason(rawValue: 1 << 2)
    public static let fastTail = SampleReason(rawValue: 1 << 3)
    public static let deterministicSample = SampleReason(rawValue: 1 << 4)
}

public struct Gap: Sendable {
    public let beforeIndex: Int
    public let afterIndex: Int
    public let beforeName: String
    public let afterName: String
    public let seconds: Double
    public let gapClass: GapClass
    public let reasons: SampleReason

    public var reasonText: String {
        var parts: [String] = []
        if reasons.contains(.ambiguousGap) { parts.append("ambiguous-gap") }
        if reasons.contains(.orientationChange) { parts.append("orientation") }
        if reasons.contains(.shutterCountJump) { parts.append("shuttercount-jump") }
        if reasons.contains(.fastTail) { parts.append("fast-tail") }
        if reasons.contains(.deterministicSample) { parts.append("sample") }
        return parts.joined(separator: ",")
    }

    public var identifier: String { "\(beforeName)|\(afterName)" }

    /// One-line caption for the contact sheet, so a verdict can never be recorded against the
    /// wrong picture: it names both frames, the gap, and why the boundary was sampled.
    public var title: String {
        "\(beforeName)→\(afterName)  Δt \(String(format: "%.3f", seconds))s  [\(reasonText)]"
    }
}

public struct GameAuditPlan: Sendable {
    public let game: Game
    public let photoCount: Int
    /// Capture-ordered file names, so a boundary can be turned into "show me these frames" without
    /// re-deriving the ordering.
    public let photoNames: [String]
    public let gaps: [Gap]
    /// Every boundary that must be looked at: the whole ambiguous zone, plus every orientation
    /// change and ShutterCount jump, plus the fast tail. Nothing here is a random sample.
    public let mandatory: [Gap]
    /// A deterministic ≥ 20% sample of the remaining intra-burst gaps, so "look at the photos" is
    /// not limited to the cases the timestamp already flagged as suspicious.
    public let sampled: [Gap]

    public var allReviewed: [Gap] { mandatory + sampled }
}

public enum BoundarySampler {
    /// todo.md §5.3's ambiguous band. A gap below this is inside a burst at any frame rate present in
    /// the test set; a gap above is a released shutter. Everything between needs eyes.
    public static let ambiguousRange: ClosedRange<Double> = 0.5...2.0

    /// §12's floor: "at least 20% of boundaries per game". Applied to the boundaries we did not
    /// already have to look at, so the total coverage is well above the floor.
    public static let sampleFraction = 0.25

    public static func plan(for game: Game) throws -> GameAuditPlan {
        let records = try Fixtures.exifToolRecords(for: game)
        let gaps = try gaps(for: game, records: records)
        let mandatoryIndices = Set(
            gaps.enumerated().compactMap { index, gap in
                (gap.gapClass == .ambiguous
                    || gap.reasons.contains(.orientationChange)
                    || gap.reasons.contains(.shutterCountJump)
                    || gap.reasons.contains(.fastTail)) ? index : nil
            })
        let mandatory = gaps.enumerated().filter { mandatoryIndices.contains($0.offset) }.map(\.element)

        let candidates = gaps.enumerated().filter { !mandatoryIndices.contains($0.offset) }.map(\.element)
        let wanted = max(1, Int((Double(candidates.count) * sampleFraction).rounded()))
        let sampled = deterministicSample(candidates, count: wanted).map {
            Gap(
                beforeIndex: $0.beforeIndex, afterIndex: $0.afterIndex,
                beforeName: $0.beforeName, afterName: $0.afterName,
                seconds: $0.seconds, gapClass: $0.gapClass,
                reasons: $0.reasons.union(.deterministicSample))
        }

        return GameAuditPlan(
            game: game,
            photoCount: records.count,
            photoNames: records.map(\.fileName),
            gaps: gaps,
            mandatory: mandatory,
            sampled: sampled
        )
    }

    /// Every gap between consecutive photos in capture order, classified.
    public static func gaps(for game: Game, records: [ExifToolRecord]) throws -> [Gap] {
        let times = try records.map { record in
            guard let instant = record.captureUnixMicroseconds else {
                throw FixtureError.fixtureMissing(
                    "\(game.rawValue)/\(record.fileName) has no parseable capture time")
            }
            return instant
        }
        // Strict ordering is what makes a time-based candidate list sound. If the fixture were not
        // strictly ordered, every "gap" below could be negative and the whole audit would be
        // meaningless, so this fails loudly instead of quietly.
        precondition(
            zip(times, times.dropFirst()).allSatisfy { $1 > $0 },
            "\(game.rawValue) fixture is not strictly ordered by capture time"
        )

        return zip(records, records.dropFirst()).enumerated().map { index, pair in
            let (before, after) = pair
            let seconds = Double(times[index + 1] - times[index]) / 1_000_000
            var reasons: SampleReason = []
            if ambiguousRange.contains(seconds) { reasons.insert(.ambiguousGap) }
            if let b = before.orientation, let a = after.orientation, b != a {
                reasons.insert(.orientationChange)
            }
            if let b = before.shutterCount, let a = after.shutterCount, a > b + 1 {
                reasons.insert(.shutterCountJump)
            }
            if before.fileNumberTail >= 6117, before.fileNumberTail <= 6164,
                game == Game.ambiguousTailGame
            {
                reasons.insert(.fastTail)
            }
            let gapClass: GapClass =
                seconds < ambiguousRange.lowerBound
                ? .intraBurst
                : seconds <= ambiguousRange.upperBound
                    ? .ambiguous
                    : .hard
            return Gap(
                beforeIndex: index, afterIndex: index + 1,
                beforeName: before.fileName, afterName: after.fileName,
                seconds: seconds, gapClass: gapClass, reasons: reasons)
        }
    }

    /// SplitMix64. Not security, not speed — just a sample that is identical on every machine and
    /// in every run, so an audit can be re-run and compared. `SystemRandomNumberGenerator` would
    /// make the audit unreproducible and therefore uncheckable.
    static func deterministicSample(_ items: [Gap], count: Int) -> [Gap] {
        guard count > 0, !items.isEmpty else { return [] }
        var state: UInt64 = 0x9E37_79B9_7F4A_7C15
        func next() -> UInt64 {
            state &+= 0x9E37_79B9_7F4A_7C15
            var z = state
            z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
            z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
            return z ^ (z >> 31)
        }
        // Reservoir sampling in one pass: O(count) memory, and every item is equally likely.
        var reservoir: [Gap] = []
        var seen: UInt64 = 0
        for item in items {
            seen += 1
            if reservoir.count < count {
                reservoir.append(item)
            } else {
                let slot = Int(next() % seen)
                if slot < count { reservoir[slot] = item }
            }
        }
        var ordered = reservoir
        ordered.sort(by: { a, b in a.beforeIndex < b.beforeIndex })
        return ordered
    }
}

extension ExifToolRecord {
    /// The numeric part of `IMG_6164.CR3` → 6164. Used to detect the fast tail of Game1JENKS
    /// (todo.md §3 and §5.4 name `IMG_6117`–`IMG_6164` explicitly).
    public var fileNumberTail: Int {
        let digits = fileName.dropFirst("IMG_".count).prefix(while: \.isNumber)
        return Int(digits) ?? 0
    }
}
