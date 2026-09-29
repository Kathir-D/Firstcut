// Owner: ui.
//
// **Not an implementation.** A forwarding shim, and the reason it is not deleted is that
// `PreviewCullViewState` still needs a tier for its synthetic photos.
//
// Until this merge there were two rating-mode mappings in the tree — this one and app-logic's
// `RatingRules` — and they disagreed. `tier(for:mode:)` here returned `.keep` for any photo with
// `keep == true` **or** `stars >= 4`, in either mode; `RatingRules.tier(of:mode:)` respects the
// mode. A keep stored with 0 stars read as Keep in stars mode from one and Unrated from the other,
// which is the exact failure REV-31 and REV-69 exist to prevent: the filmstrip, the info panel and
// the Finish summary must be reading one function or they will eventually show three different
// answers for one photo.
//
// `core/firstcut-core/src/store/rating.rs` (`display_rating` / `map_rating` / `tier`) is the
// single source of truth, and it is also where the *write* path goes: `display_rating` returns a
// view, not a rating you may store, because in keep mode it synthesises `keep: true` for a 4–5 star
// photo. Nothing in Swift ever calls `display_rating` to decide what to write; that is
// `map_rating`, and it lives on the Rust side of the bridge
// (`App/Sources/Session/CoreSessionBackend.swift`).
//
// Every function here is a call through. When the views bind to `AppModel` — which supplies `tier`
// and `isKeep` as values, so no view computes one — this file is deleted with
// `PreviewCullViewState.swift`.

import Foundation

enum RatingTiers {
  /// app-logic's single implementation, in both modes.
  static func tier(for rating: Rating, mode: RatingMode) -> Tier {
    RatingRules.tier(of: rating, mode: mode)
  }

  /// Keep, for the given mode. The one a view should use.
  static func isKeep(_ rating: Rating, mode: RatingMode) -> Bool {
    RatingRules.isKeep(rating, mode: mode)
  }

  /// Keep in *either* mode: "the Finish step will keep this", which is what a filmstrip ring and a
  /// summary count both mean, and what this function was reaching for by hand before.
  ///
  /// Derived from the same mapping as the two above rather than re-tested as `keep || stars >= 4`,
  /// so a third definition of "is a keep" cannot exist in this tree.
  static func isKeep(_ rating: Rating) -> Bool {
    isKeep(rating, mode: .stars) || isKeep(rating, mode: .keep)
  }
}
