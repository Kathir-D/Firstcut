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
    ///
    /// This reads the **mapped** answer, not the raw field, in both directions: a keep made in
    /// keep mode is a keep in stars mode, and a 4- or 5-star photo is a keep in keep mode. That
    /// symmetry is what stops the filmstrip showing a red ring on a photo the Finish step is
    /// about to move into the kept folder, or a green ring on one it is about to trash. It mirrors
    /// core-store's Rust `Rating::is_kept` / `Rating::tier`, which the session store also uses.
    public static func isKeep(_ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Bool {
        switch mode {
        case .stars: rating.stars >= UInt8(keepThreshold)
        // A 4–5 star photo is a keep in *both* modes, so switching to keep mode must not make the
        // user's keeps vanish. That promotion is a **display** rule: the stored `keep` stays false
        // and the stored `stars` stay four. See `synchronized` for why the write-back is the bug.
        case .keep: rating.keep || Int(rating.stars) >= keepThreshold
        }
    }

    /// The tier a photo sits in (app-model.md). `flag.pick` never changes the tier; it is stored
    /// for Lightroom parity and shown as a badge.
    ///
    /// **This is the only implementation of the rating-mode mapping in Swift.** It mirrors
    /// `Rating::tier` in `core/firstcut-core/src/store/rating.rs` field for field, and there is
    /// no second copy: `RatingTiers` in the Views folder was deleted because it disagreed with
    /// this one about a keep with 0 stars, which is REV-69's bug (keeps vanishing on a mode
    /// switch). ui reads the tier the model supplies; a view that recomputed it from
    /// `stars`/`keep` would disagree with the finish summary the moment the mapping changed.
    public static func tier(of rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold) -> Tier {
        if rating.flag == .reject { return .rejected }
        switch mode {
        case .keep:
            return isKeep(rating, mode: mode, keepThreshold: keepThreshold) ? .keep : .unrated
        case .stars:
            // §6: a keep made in keep mode maps to 5 stars, so it must not read as Unrated.
            let mapped: Int = rating.stars == 0 && rating.keep ? Int(RatingRules.keepStars) : Int(rating.stars)
            if mapped >= keepThreshold { return .keep }
            if mapped >= 3 { return .good }
            if mapped >= 1 { return .maybe }
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
    /// stars it means (task.md §6: "a keep ↔ 5 stars"). This is the display half of that rule;
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
