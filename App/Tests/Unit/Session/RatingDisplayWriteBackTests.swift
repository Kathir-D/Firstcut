// Owner: app-logic.
//
// REV-69, and the data-loss hazard behind it.
//
// `core/firstcut-core/src/store/rating.rs` is explicit about the trap:
//
//     /// **The returned value is a view, not a rating you may store.** In keep mode it promotes
//     /// 4–5 stars to `keep: true` (KEEP_STARS), which is a statement about the *other* mode, so
//     /// writing this back would persist a keep the user never gave.
//
// Concretely, a 4-star photo in keep mode *displays* as `Rating(stars: 4, keep: true)`. Persist
// that projection and the photo silently gains a keep the user never gave, and in stars mode the
// same thing in reverse clears a keep. So the tests below hold the whole stack — a real `AppModel`
// over a real `CoreSessionBackend` over a scripted core — and check that **nothing reaches storage
// except a change the user made**.
//
// The other half of the hazard is on the read side: a view that recomputed `tier` or `isKeep` from
// `stars`/`keep` would show something the finish summary disagrees with. `RatingTiers` is now a
// forwarding shim over `RatingRules` and `ModelCullViewState` copies the model's values, and
// `theViewNeverDerivesItsOwnTier` below is what holds that in place.

import Foundation
import Testing

@testable import Firstcut

@Suite("A displayed rating is never written back to storage (REV-69)")
@MainActor
struct RatingDisplayWriteBackTests {
    /// Every rating shape the app can produce, and what each of them looks like in each mode.
    private static let states: [Rating] = [
        Rating(),
        Rating(stars: 1),
        Rating(stars: 3),
        Rating(stars: 4),
        Rating(stars: 5),
        Rating(stars: 0, keep: true),  // a keep written by another tool, with no stars
        Rating(stars: 2, flag: .reject),
        Rating(stars: 4, flag: .pick, label: .blue, keep: true),
    ]

    private static func makeModel() -> (AppModel, ScriptedCoreSession, CoreSessionBackend) {
        let photos = FixturePhotos.syntheticPhotos(count: 8, burstSize: 4)
        let core = ScriptedCoreSession(
            SessionData(folder: "/tmp/shoot", photos: photos, batches: FixturePhotos.batches(for: photos)))
        let backend = CoreSessionBackend(core: core, initial: core.snapshot())
        return (AppModel(.testing(backend: backend)), core, backend)
    }

    @Test("Switching the rating mode writes nothing: a changed display is not a change")
    func modeSwitchNeverWrites() throws {
        let (model, core, backend) = Self.makeModel()
        model.open(backend, folderName: "Shoot")
        // One photo per state, so each one is inspected in isolation.
        for (index, rating) in Self.states.enumerated() {
            _ = try core.setRating(photo: model.allPhotos[index].id, rating)
        }
        let before = core.data.ratings

        model.updateSettings { $0.general.ratingMode = .keep }
        model.updateSettings { $0.general.ratingMode = .stars }

        // `RatingRules.synchronized` is the one thing allowed to change a rating, and it is
        // deliberately a no-op for a state that is already consistent. Nothing the *display* does
        // may reach storage.
        let changed = before.keys.filter { before[$0] != core.data.ratings[$0] }
        #expect(changed.isEmpty, "a mode switch changed \(changed.count) stored ratings")
    }

    @Test("Every rating maps to the same tier in the view, the model and the session")
    func theViewNeverDerivesItsOwnTier() {
        let (model, _, backend) = Self.makeModel()
        model.open(backend, folderName: "Shoot")
        let state = ModelCullViewState(model: model, images: PreviewImageSource(seed: 1))

        for mode in [RatingMode.stars, .keep] {
            model.updateSettings { $0.general.ratingMode = mode }
            state.send(.setRating(stars: 4))
            let modelPhoto = try! #require(model.currentPhoto)
            let viewPhoto = try! #require(state.currentPhoto)
            // The view's `tier` is the model's, not a second opinion.
            #expect(viewPhoto.tier == modelPhoto.tier)
            #expect(viewPhoto.isKeep == modelPhoto.isKeep)
            #expect(viewPhoto.rating == modelPhoto.rating)
            // And the one mapping, not two.
            #expect(viewPhoto.tier == RatingTiers.tier(for: modelPhoto.rating, mode: mode))
        }
    }

    @Test("RatingTiers is the same function as RatingRules, not a second implementation")
    func ratingTiersForwards() {
        for rating in Self.states {
            for mode in [RatingMode.stars, .keep] {
                #expect(RatingTiers.tier(for: rating, mode: mode) == RatingRules.tier(of: rating, mode: mode))
                #expect(
                    RatingTiers.isKeep(rating, mode: mode) == RatingRules.isKeep(rating, mode: mode))
            }
            // "Is a keep in either mode" is derived from that mapping, not re-tested by hand.
            #expect(
                RatingTiers.isKeep(rating)
                    == (RatingRules.isKeep(rating, mode: .stars) || RatingRules.isKeep(rating, mode: .keep)))
        }
    }

    @Test("A 4-star photo shows as a keep in keep mode and its stored stars are still four")
    func theCrossModePromotionIsDisplayOnly() {
        let (model, core, backend) = Self.makeModel()
        model.open(backend, folderName: "Shoot")
        model.perform(.setStars(4))
        #expect(core.data.ratings[model.currentPhoto!.id]?.stars == 4)

        model.updateSettings { $0.general.ratingMode = .keep }
        // In keep mode the photo is a keep — 4 stars is a keep in both modes (rating.rs,
        // `a_starred_photo_shows_as_a_keep_in_keep_mode`).
        #expect(model.currentPhoto?.isKeep == true)
        #expect(model.currentPhoto?.tier == .keep)
        // …and the stored stars were not rewritten to mean something else.
        #expect(core.data.ratings[model.currentPhoto!.id]?.stars == 4)
    }

    @Test("Only a change the user made reaches storage")
    func onlyUserChangesAreWritten() throws {
        let (model, core, backend) = Self.makeModel()
        model.open(backend, folderName: "Shoot")
        #expect(core.data.ratings.isEmpty)

        // Navigation, view changes, panel toggles, focus: none of them touch a rating.
        model.perform(.photoNext)
        model.perform(.showGrid)
        model.perform(.toggleInfoPanel)
        model.perform(.toggleHUD)
        model.updateSettings { $0.viewer.backgroundGray = 0.2 }
        #expect(core.data.ratings.isEmpty, "navigation and view changes are not rating changes")

        // One keystroke, one write — on whatever photo is now selected.
        let id = try #require(model.currentPhoto?.id)
        model.perform(.setStars(3))
        #expect(core.data.ratings[id]?.stars == 3)
    }
}
