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
        tier(of: rating, mode: mode, keepThreshold: keepThreshold) == .keep
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
            // A 4- or 5-star photo is a keep here too: §6.1 counts both as a full keep, and the
            // Finish step keeps the Keep tier, so reading it as "not keep" would be a lie the
            // user discovers when their keeps get trashed.
            return (rating.keep || Int(rating.stars) >= keepThreshold) ? .keep : .unrated
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
    /// **In stars mode** `keep` follows `stars` (`keep = stars >= threshold`).
    ///
    /// **In keep mode** the two are one decision, so `stars` is *derived from* `keep` in both
    /// directions, not only when setting it:
    ///   - `keep == true`  → `stars = 5`, the §6 "keep ↔ 5 stars" mapping;
    ///   - `keep == false` → `stars = 0`.
    ///
    /// The second direction is not a detail. An earlier version only set the stars when keep was
    /// switched **on**, so toggling keep off left `stars == 5` behind. That produced a stored
    /// rating the user had explicitly un-kept, and since `tier` correctly reads 5 stars as a keep
    /// (which it must, or a 5-star photo would look Unrated in keep mode and Finish would trash a
    /// kept photo), the photo stayed a Keep in the UI *after the user removed it*. Turning a keep
    /// off has to clear the stars it invented.
    ///
    /// A photo that already carried stars from stars mode is left alone when its keep matches what
    /// those stars mean, so visiting it in keep mode does not destroy a 3-star "good".
    public static func synchronized(
        _ rating: Rating, mode: RatingMode, keepThreshold: Int = defaultKeepThreshold
    ) -> Rating {
        var result = rating
        switch mode {
        case .stars:
            result.keep = Int(result.stars) >= keepThreshold
        case .keep:
            if result.keep {
                // Only write the 5 stars if the current stars do not already mean "keep".
                if Int(result.stars) < keepThreshold { result.stars = keepStars }
            } else {
                // The user removed the keep, so the stars that stood for it must go too.
                if Int(result.stars) >= keepThreshold { result.stars = 0 }
            }
        }
        return result
    }

    /// Star count for a keep, used when a keep is created from the flag key in keep mode.
    public static let keepStars: UInt8 = 5

    /// Tiers in the order the summary sheet lists them.
    public static let summaryOrder: [Tier] = [.keep, .good, .maybe, .unrated, .rejected]
}
