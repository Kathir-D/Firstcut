// Owner: ui.
//
// Stands in for core-store's rating-mode mapping (`map_rating` / `tier(mode)` in session-api.md,
// REV-31 and REV-69), which is the single pure function both modes are defined by. It lives with
// the preview model, not in the view types, because a view that derived a tier from
// `stars`/`keep` would disagree with the finish summary the moment the real mapping lands.
//
// DELETE at the swap (REQ-ui-1): call app-logic's `AppModel` instead.

import Foundation

enum RatingTiers {
  /// task.md §6.1 in stars mode. Reject wins over stars, as §6.1 specifies.
  static func tier(for rating: Rating, mode: RatingMode) -> CullTier {
    switch mode {
    case .stars:
      if rating.flag == .reject { return .rejected }
      if rating.stars >= 4 { return .keep }
      if rating.stars == 3 { return .good }
      if rating.stars >= 1 { return .maybe }
      return .unrated
    case .keep:
      // Every photo starts as not-keep, so an untouched photo is `.unrated` and only an explicit
      // rejection or a reject flag shows as `.rejected`.
      if rating.flag == .reject { return .rejected }
      if rating.keep { return .keep }
      return .unrated
    }
  }

  static func isKeep(_ rating: Rating) -> Bool {
    rating.keep || rating.stars >= 4
  }
}
