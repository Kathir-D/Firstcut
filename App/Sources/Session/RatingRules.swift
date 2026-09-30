// Owner: app-logic.
//
// Rating rules for both modes (todo.md §6). Pure functions, no state, no UI: this is the whole
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
    /// Exactly "the tier is Keep", as the core's `Rating::is_kept` decides it for Finish: a rejected
    /// photo is never a keep, whatever its stars.
    public static func isKeep(_ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Bool {
        tier(of: rating, mode: mode, keepThreshold: keepThreshold) == .keep
    }

    /// The tier a photo sits in (app-model.md). `flag.pick` never changes the tier; it is stored
    /// for Lightroom parity and shown as a badge.
    public static func tier(of rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Tier {
        if rating.flag == .reject { return .rejected }
        switch mode {
        case .keep:
            // A 4–5 star photo is a keep in *both* modes, so switching to keep mode must not make
            // the user's keeps vanish. That promotion is a **display** rule: the stored `keep` stays
            // false and the stored `stars` stay four. See `synchronized` for why the write-back is
            // the bug.
            return rating.keep || Int(rating.stars) >= keepThreshold ? .keep : .unrated
        case .stars:
            // §6.1: 5 or 4 is a full keep, and the boundary is the setting, not a constant, so a
            // photographer who wants only 5-star keeps can ask for it. The tier follows the stars
            // *shown*: a keep made in keep mode (stored with 0 stars) is the 5 stars it displays as,
            // and a Keep, as the core's `Rating::tier` counts it for Finish.
            let stars = Int(displayStars(rating, mode: mode, keepThreshold: keepThreshold))
            if stars >= keepThreshold { return .keep }
            if stars >= 3 { return .good }
            if stars >= 1 { return .maybe }
            return .unrated
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

    /// Restores the invariant after a mutation.
    ///
    /// **Only one direction, and it is the one the storage model uses.** In *stars* mode a rating
    /// change writes `stars` and leaves `keep` alone; in *keep* mode a keep is stored as `keep` and
    /// `stars` stays 0.
    ///
    /// The earlier version ran the rule the other way as well — in stars mode it set
    /// `keep = stars >= keepThreshold`. That looks harmless and is not: the display already
    /// promotes 4–5 stars to a keep in keep mode (`display_rating`, one implementation of which lives
    /// in Rust as `store::rating::display_rating`), so writing `keep` back from `stars` stored a mode
    /// the user was not in, and then `Session::set_rating` mapped *that* to XMP. The visible symptom
    /// was a 4-star photo whose sidecar said `xmp:Rating="0"` — Lightroom reads that as unrated, so
    /// the rating vanished on export. It is the write-back hazard REV-31 and REV-69 are about, and it
    /// was reachable from a keystroke.
    ///
    /// The tier a photo is *displayed* in is computed from the rating on the way out
    /// (`tier(of:mode:keepThreshold:)`); nothing needs a second field kept in step.
    public static func synchronized(
        _ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold
    ) -> Rating {
        rating
    }

    /// The star count to **display** for a rating, in the given mode.
    ///
    /// A keep stored with `stars == 0` would read as Unrated in stars mode, so it shows as the 5
    /// stars it means (todo.md §6: "a keep ↔ 5 stars"). This is the display half of that rule;
    /// `synchronized` is deliberately the *storage* half, and it does not write it back.
    public static func displayStars(
        _ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold
    ) -> UInt8 {
        switch mode {
        case .stars: rating.stars == 0 && rating.keep ? keepStars : rating.stars
        case .keep: rating.stars
        }
    }

    /// Star count for a keep, used when a keep is created from the flag key in keep mode.
    public static let keepStars: UInt8 = 5

    /// Tiers in the order the summary sheet lists them.
    public static let summaryOrder: [Tier] = [.keep, .good, .maybe, .unrated, .rejected]
}
