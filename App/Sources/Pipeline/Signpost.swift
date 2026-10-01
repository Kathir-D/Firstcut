// Owner: pipeline.
//
// `os_signpost` intervals, named after the rows of todo.md §7.3 and §7.5.
//
// ## Why this file
//
// Every performance number in the project is supposed to be a measurement, and the interactive ones —
// arrow key to a sharp photo, batch switch, 100% zoom — matter most to the user and are hardest to
// get. A stopwatch around a key handler measures the handler, not the frame the user sees.
// Instruments' signposts measure *between* two points on a timeline, so "keyDown → next presented
// frame" is expressible, and `xctrace` already aggregates it into p50/p95/p99.
//
// So the intervals here are named after the claims in todo.md rather than after the functions that
// happen to contain them. If a row in §7.3 has no interval, it cannot be measured without a stopwatch
// and a judgement call, and removing that is the point of this file.
//
// ## One signposter per interval
//
// `OSSignpostID` identity is per-`OSSignposter`, and `beginInterval` without an explicit id is
// **exclusive** — two overlapping intervals of the same name on the same signposter are ambiguous
// and Instruments merges them. The decode engine runs up to `maxConcurrent` drains at once, all
// emitting `decodeThumbnail`, so sharing one signposter would fuse four real decodes into one
// misleading span. A fresh signposter per interval costs a struct wrap around a static `os_log_t`
// and gives every interval its own id.
//
// ## Cost
//
// `OSSignposter` is a no-op unless something is tracing. The one rule is: **do not compute anything
// inside an interval that is only needed for the signpost**, because that work is not free even when
// tracing is off. The debug HUD's numbers come from counters the engine keeps anyway.

import Foundation
import os

/// One measurement point. Cheap to create, `Sendable`, and safe to pass into a decode thread.
public struct SignpostInterval: Sendable {
    private let signposter: OSSignposter
    private let name: StaticString
    private let state: OSSignpostIntervalState

    /// Begin an interval. The returned value ends it on `end`, or on deinit if the caller forgets —
    /// which is the safe direction to fail in: an unclosed interval is *visible* in Instruments, where a
    /// crash in a decode callback is not recoverable at all.
    public static func begin(_ name: StaticString) -> SignpostInterval {
        let signposter = OSSignposter(subsystem: subsystem, category: category)
        return SignpostInterval(
            signposter: signposter, name: name, state: signposter.beginInterval(name))
    }

    /// A named point rather than an interval: "this happened", with no duration.
    public static func event(_ name: StaticString) {
        OSSignposter(subsystem: subsystem, category: category).emitEvent(name)
    }

    public func end() {
        signposter.endInterval(name, state)
    }

    /// One log for the app, so a trace of `firstcut` has every interval on one timeline.
    private static let subsystem = "com.kathird.firstcut"
    private static let category = "pipeline"
}

/// The interval names, as constants so a rename is a compile error at every use site.
///
/// Kept next to what they measure: the §7.3 row, and the span the interval is supposed to cover. The
/// two must not drift, because a signpost that measures the wrong span produces a number that looks
/// fine and is not.
public enum Signposts {
    // §7.5 "Open → first photo": the open panel returning to the first commit with an image on screen.
    // Ends in the view layer, which is the only place that can see a presented frame.
    public static let openToFirstPhoto: StaticString = "openToFirstPhoto"

    // §7.3 "Arrow key → sharp photo": `perform(_:)` to the next presented frame.
    public static let keyToFrame: StaticString = "keyToFrame"

    // §7.3 "Batch switch": the same, for ⌘→.
    public static let batchToFrame: StaticString = "batchToFrame"

    // §7.3 "100% zoom": the click to the full-resolution bitmap on screen.
    public static let zoomToSharp: StaticString = "zoomToSharp"

    // The decode engine's own work, split by tier so a slow T2 and a slow T3 cannot hide in one
    // average. Display decodes emit `decodeDisplay` whichever path they took, and `decodeFromBytes`
    // when it was the byte-range one — the §7.5 claim is that the range is cheaper, and that is only a
    // measurement if the two are distinguishable in a trace.
    public static let decodeThumbnail: StaticString = "decodeThumbnail"
    public static let decodeDisplay: StaticString = "decodeDisplay"
    public static let decodeFromBytes: StaticString = "decodeFromBytes"

    // The queue, which is where a prefetch that is not keeping up actually shows up.
    public static let setFocus: StaticString = "setFocus"
    public static let evictions: StaticString = "evictions"
}

/// The four rows of todo.md §7.3 that only a **presented frame** can close, and what closes them.
///
/// They are grouped here rather than in `AppModel` because the distinction they turn on is a
/// property of the measurement, not of the model: a `PresentedFrame.standIn` is a 256 px thumbnail,
/// so a span that says "arrow key → *sharp* photo" must not end on one, while a span that says
/// "folder open → a photograph on screen" legitimately can.
public enum FrameSpan {
    public enum Accepts: Equatable, Sendable {
        /// Only the display decode. For `keyToFrame`, `batchToFrame` and `zoomToSharp`, whose rows are
        /// written in terms of the sharp photograph. A stand-in closing one of these would report a fast
        /// frame for a soft picture, which is the lie §7.3 exists to prevent.
        case displayOnly

        /// Either frame. For `openToFirstPhoto`: the row is a photograph being *there*, and the first
        /// paint is frequently a thumbnail while the display decode is still running.
        case anyFrame
    }
}
