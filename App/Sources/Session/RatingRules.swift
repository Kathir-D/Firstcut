// Owner: app-logic.
//
// Rating rules for both modes (task.md §6). Pure functions, no state, no UI: this is the whole
// "what does a rating mean" answer in one file, which is why ui (visuals) and core-store (storage)
// can both work from it without asking.
//
// ## The two-field model
//
// `Rating` (session-api.md) carries **both** `stars` and `keep`, and Firstcut keeps them consistent
// rather than treating one as a translation of the other:
//
// * **Stars mode** — `stars` is what the user sets with 0…5. `keep` follows: `keep = stars >= threshold`
//   (default threshold 4, because §6.1 counts 5 *or* 4 as a full keep).
// * **Keep mode** — `keep` is the tier. The 1…5 keys still work and set stars, which keeps `keep`
//   in sync the same way, so a Keep is always visible as 5 stars and never contradicts the ring.
// * **Toggle keep** flips `keep` and leaves `stars` alone, so a 3-star "good" survives a visit in
//   keep mode.
//
// Consequence: switching modes mid-session needs no bulk migration at all — both fields are always
// present and consistent. The only lazy part is the §6 "keep ↔ 5 stars" mapping, applied when a
// photo is *touched* in a mode where its stars don't express its keep (a keep with 0 stars becomes
// 5 stars the first time you rate anything about it).
//
// The reject flag is mode-independent and always wins: an X is an explicit "no" in both modes.

import Foundation

public enum RatingRules {
    /// A photo is a full keep at this many stars or more (§6.1: 5 or 4).
    public static let defaultKeepThreshold = 4

    /// True when the photo counts as kept, for the given mode.
    public static func isKeep(_ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Bool {
        switch mode {
        case .stars: rating.stars >= UInt8(keepThreshold)
        case .keep: rating.keep
        }
    }

    /// The tier a photo sits in (app-model.md). `flag.pick` never changes the tier; it is stored
    /// for Lightroom parity and shown as a badge.
    public static func tier(of rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Tier {
        if rating.flag == .reject { return .rejected }
        switch mode {
        case .keep:
            return rating.keep ? .keep : .unrated
        case .stars:
            // §6.1: 5 or 4 is a full keep, and the boundary is the setting, not a constant, so a
            // photographer who wants only 5-star keeps can ask for it.
            let stars = Int(rating.stars)
            if stars >= keepThreshold { return .keep }
            return switch stars {
            case 3: .good
            case 2, 1: .maybe
            default: .unrated
            }
        }
    }

    /// Every field that says "this photo has been looked at", i.e. what Finish considers reviewed.
    public static func isReviewed(_ rating: Rating) -> Bool {
        rating.stars > 0 || rating.flag != .none || rating.label != nil || rating.keep
    }

    /// Applies a change to an existing rating and re-establishes the stars ↔ keep invariant.
    ///
    /// Every rating change in the app goes through here, so the invariant can't drift, and a change
    /// to one field can never silently clear another.
    public static func applying(
        to base: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold,
        _ change: (inout Rating) -> Void
    ) -> Rating {
        var rating = base
        change(&rating)
        return synchronized(rating, mode: mode, keepThreshold: keepThreshold)
    }

    /// Restores the invariant after a mutation: in stars mode `keep` follows `stars`; in keep mode a
    /// keep with no stars is recorded as 5 so the two never contradict (the §6 "keep ↔ 5 stars"
    /// mapping).
    public static func synchronized(
        _ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold
    ) -> Rating {
        var result = rating
        switch mode {
        case .stars:
            result.keep = Int(result.stars) >= keepThreshold
        case .keep:
            if result.keep, result.stars == 0 { result.stars = 5 }
        }
        return result
    }

    /// Star count for a keep, used when a keep is created from the flag key in keep mode.
    public static let keepStars: UInt8 = 5

    /// Tiers in the order the summary sheet lists them.
    public static let summaryOrder: [Tier] = [.keep, .good, .maybe, .unrated, .rejected]
}
