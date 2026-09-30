// Owner: app-logic.
import Foundation
import Testing

@testable import Firstcut

// MARK: - Rating rules (task.md §6)

@Suite("Rating rules")
struct RatingRulesTests {
    @Test("Stars mode tiers (task.md §6.1)")
    func starTiers() {
        func tier(_ stars: UInt8, reject: Bool = false) -> Tier {
            var rating = Rating()
            rating.stars = stars
            rating.flag = reject ? .reject : .none
            return RatingRules.tier(of: rating, mode: .stars)
        }
        #expect(tier(5) == .keep)
        #expect(tier(4) == .keep)
        #expect(tier(3) == .good)
        #expect(tier(2) == .maybe)
        #expect(tier(1) == .maybe)
        #expect(tier(0) == .unrated)
        #expect(tier(0, reject: true) == .rejected)
        #expect(tier(5, reject: true) == .rejected)  // an explicit reject wins
    }

    @Test("Keep mode has two tiers only (task.md §6.2)")
    func keepTiers() {
        #expect(RatingRules.tier(of: Rating(keep: true), mode: .keep) == .keep)
        #expect(RatingRules.tier(of: Rating(), mode: .keep) == .unrated)
        #expect(RatingRules.tier(of: Rating(stars: 5, keep: true), mode: .keep) == .keep)
    }

    @Test("A pick flag never changes the tier")
    func pickFlagIsNeutral() {
        #expect(RatingRules.tier(of: Rating(flag: .pick), mode: .stars) == .unrated)
        #expect(RatingRules.tier(of: Rating(flag: .pick), mode: .keep) == .unrated)
    }

    @Test("Stars mode writes stars and leaves `keep` alone")
    func starsDoNotTouchKeep() {
        // These two used to assert the opposite — that stars mode drives `keep` — which is the bug
        // that wrote a 4-star photo's sidecar as `xmp:Rating="0"`. Storing the derived field made
        // `Session::set_rating` map a *mode the user is not in* to XMP, so the rating disappeared on
        // export to Lightroom. The derived answer is computed on the way out by `isKeep` / `tier`.
        let rated = RatingRules.applying(to: Rating(), mode: .stars) { $0.stars = 4 }
        #expect(rated.stars == 4)
        #expect(rated.keep == false, "the stored keep field belongs to keep mode only")
        // And the *displayed* answer is still a keep, which is what the user sees.
        #expect(RatingRules.isKeep(rated, mode: .keep, keepThreshold: 4))
        #expect(RatingRules.tier(of: rated, mode: .keep, keepThreshold: 4) == .keep)
    }

    @Test("A keep in keep mode is stored as `keep`, not as 5 stars")
    func keepIsStoredAsKeep() {
        // Also the reverse of the old rule. `stars` is the stars-mode field; writing 5 into it from a
        // keep-mode keystroke made a keep-mode photo show 5 stars when the user switched back.
        let kept = RatingRules.applying(to: Rating(), mode: .keep) { $0.keep = true }
        #expect(kept.keep)
        #expect(kept.stars == 0, "the stars field belongs to stars mode only")
        // Displayed in stars mode, a keep is the 5 stars it means (task.md §6).
        #expect(RatingRules.displayStars(kept, mode: .stars) == 5)
    }

    @Test("Toggling keep leaves existing stars alone")
    func toggleKeepsStars() {
        var rating = Rating()
        rating.stars = 3
        rating = RatingRules.synchronized(rating, mode: .keep)
        rating.keep = true  // as `toggleKeep` does
        #expect(rating.stars == 3)  // no longer contradicts, so nothing to remap
        #expect(rating.keep)
    }

    @Test("The keep threshold is configurable")
    func threshold() {
        var rating = Rating()
        rating.stars = 4
        #expect(RatingRules.isKeep(rating, mode: .stars, keepThreshold: 4))
        #expect(RatingRules.isKeep(rating, mode: .stars, keepThreshold: 5) == false)
    }

    @Test("Reviewed means the photo carries any rating signal")
    func reviewed() {
        #expect(RatingRules.isReviewed(Rating()) == false)
        #expect(RatingRules.isReviewed(Rating(stars: 1)))
        #expect(RatingRules.isReviewed(Rating(flag: .reject)))
        #expect(RatingRules.isReviewed(Rating(label: .red)))
        #expect(RatingRules.isReviewed(Rating(keep: true)))
    }
}

// MARK: - Navigation

@MainActor
@Suite("AppModel navigation")
struct AppModelNavigationTests {
    /// 4 batches of 3 photos: enough to test every edge without a real shoot.
    static func makeModel(photoCount: Int = 12) -> (AppModel, MockSession) {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: photoCount, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "Test")
        return (model, session)
    }

    @Test("Opening a session starts on the first photo of the first batch")
    func openStartsAtTheBeginning() throws {
        let (model, _) = Self.makeModel()
        #expect(model.phase == .culling)
        #expect(model.currentBatchIndex == 0)
        #expect(model.currentPhotoIndex == 0)
        #expect(model.currentPhoto?.fileName == "IMG_0001.CR3")
        #expect(model.batches.count == 4)
    }

    @Test("A folder that fails to open leaves the shoot that was open on screen")
    func failedOpenKeepsTheCurrentShoot() throws {
        let (model, _) = Self.makeModel()
        model.perform(.photoNext)
        model.open(folder: URL(fileURLWithPath: "/nonexistent-\(UUID().uuidString)"))
        #expect(model.lastError != nil)
        #expect(model.phase == .culling)
        #expect(model.allPhotos.count == 12)
        #expect(model.currentPhotoIndex == 1)
    }

    @Test("→ walks the batch and stops at the end by default (task.md §9.4)")
    func arrowStopsAtBatchEnd() throws {
        let (model, _) = Self.makeModel()
        model.perform(.photoNext)
        #expect(model.currentPhotoIndex == 1)
        model.perform(.photoNext)
        #expect(model.currentPhotoIndex == 2)
        model.perform(.photoNext)
        #expect(model.currentBatchIndex == 0)  // stopped, did not roll over
        #expect(model.currentPhotoIndex == 2)
    }

    @Test("At the last photo, → crosses into the next batch when the setting says so")
    func arrowContinues() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.arrowBehaviorAtBatchEnd = .continueIntoNextBatch }
        model.perform(.photoNext)
        model.perform(.photoNext)
        model.perform(.photoNext)
        #expect(model.currentBatchIndex == 1)
        #expect(model.currentPhotoIndex == 0)  // the first photo of the next batch
    }

    @Test("Stepping back off the first photo lands on the last of the previous batch")
    func backwardCrosses() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.arrowBehaviorAtBatchEnd = .continueIntoNextBatch }
        model.perform(.photoPrevious)
        #expect(model.currentBatchIndex == 0)
        // On to the second batch, then back off its front edge.
        model.moveBatch(by: 1)
        model.perform(.photoPrevious)
        #expect(model.currentBatchIndex == 0)
        #expect(model.currentPhotoIndex == 2)
    }

    @Test("Batch navigation stops at the ends of the shoot")
    func batchEdges() throws {
        let (model, _) = Self.makeModel()
        #expect(model.moveBatch(by: -1) == false)
        model.moveBatch(by: 1)
        model.moveBatch(by: 1)
        model.moveBatch(by: 1)
        #expect(model.currentBatchIndex == 3)
        #expect(model.moveBatch(by: 1) == false)
        #expect(model.currentBatchIndex == 3)
    }

    @Test("Entering a batch selects its first photo by default")
    func enteringFirstPhoto() throws {
        let (model, _) = Self.makeModel()
        model.moveBatch(by: 1)
        #expect(model.currentPhotoIndex == 0)
    }

    @Test("Entering a batch can resume the last photo viewed (task.md §11)")
    func enteringLastViewed() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.enteringBatchBehavior = .firstPhoto }
        model.moveBatch(by: 1)
        model.perform(.photoNext)
        model.perform(.photoNext)
        #expect(model.currentPhotoIndex == 2)

        // Leaving and coming back to a batch resumes where the user was in it.
        model.updateSettings { $0.general.enteringBatchBehavior = .lastViewed }
        model.moveBatch(by: 1)
        #expect(model.currentBatchIndex == 2)
        model.moveBatch(by: -1)
        #expect(model.currentBatchIndex == 1)
        #expect(model.currentPhotoIndex == 2)
    }

    @Test("Entering a batch it has never seen starts at the first photo")
    func enteringUnvisitedBatch() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.enteringBatchBehavior = .lastViewed }
        model.moveBatch(by: 2)
        #expect(model.currentPhotoIndex == 0)
    }

    @Test("Batches the user has been in are marked visited")
    func visitedTracking() throws {
        let (model, session) = Self.makeModel()
        #expect(model.isVisited(0))
        #expect(model.isVisited(1) == false)
        model.moveBatch(by: 1)
        #expect(model.isVisited(1))
        #expect(session.isVisited(model.batches[1].id))
        // Batch 0 was visited on open, batch 1 just now.
        #expect(model.progress.unvisitedBatches == 2)
    }

    @Test("The cursor is pushed to the session so a resume lands in the same place")
    func cursorTracking() throws {
        let (model, session) = Self.makeModel()
        model.perform(.photoNext)
        let last = try #require(session.cursorHistory.last)
        #expect(last.photo == model.currentPhoto?.id)
        #expect(last.batch == model.currentBatch?.id)
    }

    @Test("Changing batch flushes the debounced XMP queue (task.md §6.3)")
    func flushOnBatchChange() throws {
        let (model, session) = Self.makeModel()
        let before = session.flushCount
        model.moveBatch(by: 1)
        #expect(session.flushCount == before + 1)
        model.perform(.photoNext)
        #expect(session.flushCount == before + 1)  // same batch: nothing to flush
    }

    @Test("Every navigation re-reports the focus window to the pipeline (task.md §7.3)")
    func focusOnEveryNavigation() throws {
        let images = MockImageProvider()
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session, images: images))
        model.open(session)

        let initial = try #require(images.lastFocus)
        #expect(initial.currentPhoto == model.currentPhoto?.id)
        #expect(initial.windows.count == 3)  // look-ahead defaults to 2
        #expect(initial.guaranteedWindows.count == 3)  // previous, current, next

        model.perform(.photoNext)
        #expect(images.lastFocus?.currentPhoto == model.currentPhoto?.id)

        model.moveBatch(by: 1)
        let afterBatch = try #require(images.lastFocus)
        #expect(afterBatch.windows.contains { $0.batchID == model.currentBatch?.id })
        #expect(afterBatch.windows.contains { $0.batchID == model.batches[0].id })
        #expect(afterBatch.windows.contains { $0.batchID == model.batches[2].id })
        #expect(images.focusCount == 3)  // open + two navigations
    }

    @Test("The focus window covers the whole current batch in capture order")
    func focusCarriesEveryPhotoInTheBatch() throws {
        let images = MockImageProvider()
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session, images: images))
        model.open(session)
        let focus = try #require(images.lastFocus)
        let window = try #require(focus.windows.first { $0.batchID == model.currentBatch?.id })
        #expect(window.photoIDs == model.currentBatch?.photoIDs)
    }

    @Test("The viewport size reaches the pipeline so T2 decodes at the right size")
    func viewportSize() throws {
        let images = MockImageProvider()
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 6, burstSize: 3))
        let model = AppModel(.testing(backend: session, images: images))
        model.open(session)
        #expect(images.lastFocus?.viewportPixelSize == .zero)
        model.setViewportPixelSize(CGSize(width: 2880, height: 1800))
        #expect(images.lastFocus?.viewportPixelSize == CGSize(width: 2880, height: 1800))
    }

    @Test("Progress counts batches, photos and tiers")
    func progress() throws {
        let (model, _) = Self.makeModel()
        model.perform(.photoNext)
        let progress = model.progress
        #expect(progress.batchNumber == 1)
        #expect(progress.batchCount == 4)
        #expect(progress.photoNumber == 2)
        #expect(progress.photoCount == 3)
        #expect(progress.photosLeftInBatch == 1)
        #expect(progress.batchTitle == "Batch 1 of 4")
        #expect(progress.totalPhotos == 12)
        #expect(progress[.unrated] == 12)
    }

    @Test("View and panel commands only change their own state")
    func viewCommands() throws {
        let (model, _) = Self.makeModel()
        model.perform(.showGrid)
        #expect(model.viewMode == .grid)
        model.perform(.showCompare(3))
        #expect(model.viewMode == .compare(3))
        model.perform(.showLoupe)
        #expect(model.viewMode == .loupe)
        #expect(model.infoPanelVisible == false)
        model.perform(.toggleInfoPanel)
        #expect(model.infoPanelVisible)
        model.perform(.toggleHUD)
        #expect(model.hudVisible == false)
        model.perform(.toggleAFOverlay)
        #expect(model.afOverlay)
        model.perform(.toggleClippingOverlay)
        #expect(model.clippingOverlay)
    }

    @Test("Click zooms to 100% at that spot, clicking again returns to fit (task.md §9.2)")
    func clickToZoom() throws {
        let (model, _) = Self.makeModel()
        #expect(model.viewer.zoomed == false)
        model.perform(.toggleZoom(at: NormalizedPoint(x: 0.25, y: 0.75)))
        #expect(model.viewer.zoomed)
        #expect(model.viewer.anchor == NormalizedPoint(x: 0.25, y: 0.75))
        model.perform(.toggleZoom(at: nil))
        #expect(model.viewer.zoomed == false)
        #expect(model.viewer.anchor == nil)
    }

    @Test("Pinch in zooms, pinch out returns to fit, and zoom lock is its own toggle")
    func pinchAndLock() throws {
        let (model, _) = Self.makeModel()
        model.perform(.magnify(by: 1.2, at: NormalizedPoint(x: 0.5, y: 0.5)))
        #expect(model.viewer.zoomed)
        model.perform(.magnify(by: 0.8, at: nil))
        #expect(model.viewer.zoomed == false)
        model.perform(.toggleZoomLock)
        #expect(model.viewer.zoomLock)
    }

    @Test("photos(inBatch:) is the current batch in capture order")
    func photosInBatch() throws {
        let (model, _) = Self.makeModel()
        let photos = model.photos(inBatch: 1)
        #expect(photos.count == 3)
        #expect(photos.map(\.fileName) == ["IMG_0004.CR3", "IMG_0005.CR3", "IMG_0006.CR3"])
        #expect(model.photos(inBatch: 99).isEmpty)
    }
}

// MARK: - Rating through the model

@MainActor
@Suite("AppModel rating")
struct AppModelRatingTests {
    static func makeModel() -> (AppModel, MockSession) {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "Test")
        return (model, session)
    }

    @Test("Stars mode: 3 stars is Good, 5 is Keep, 0 is Unrated (task.md §6.1)")
    func starRatings() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(3))
        #expect(model.currentPhoto?.tier == .good)
        #expect(model.currentPhoto?.rating.stars == 3)
        model.perform(.setStars(5))
        #expect(model.currentPhoto?.tier == .keep)
        #expect(model.currentPhoto?.isKeep == true)
        model.perform(.setStars(0))
        #expect(model.currentPhoto?.tier == .unrated)
    }

    @Test("Auto-advance is off by default, so rating does not move the cursor")
    func autoAdvanceOffByDefault() throws {
        let (model, _) = Self.makeModel()
        #expect(model.autoAdvance == false)
        model.perform(.setStars(5))
        #expect(model.currentPhotoIndex == 0)
    }

    @Test("With auto-advance on, rating steps to the next photo")
    func autoAdvance() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.autoAdvance = true }
        model.perform(.setStars(4))
        #expect(model.currentPhotoIndex == 1)
    }

    @Test("Auto-advance crosses into the next batch, otherwise it dies at every burst end")
    func autoAdvanceCrossesBatches() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.autoAdvance = true }
        model.perform(.setStars(1))
        model.perform(.setStars(1))
        #expect(model.currentBatchIndex == 0)
        model.perform(.setStars(1))
        #expect(model.currentBatchIndex == 1)
        #expect(model.currentPhotoIndex == 0)
    }

    @Test("Caps Lock flips auto-advance for the session without touching the setting")
    func capsLockTogglesAutoAdvance() throws {
        let (model, _) = Self.makeModel()
        #expect(model.autoAdvance == false)
        model.perform(.toggleAutoAdvance)
        #expect(model.autoAdvance)
        #expect(model.settings.general.autoAdvance == false)
        model.perform(.toggleAutoAdvance)
        #expect(model.autoAdvance == false)
    }

    @Test("⇧3 rates and always advances, auto-advance or not (Lightroom)")
    func starsAndAdvance() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStarsAndAdvance(3))
        #expect(model.currentPhotoIndex == 1)
        #expect(model.photos(inBatch: 0)[0].rating.stars == 3)
        #expect(model.currentPhoto?.rating.stars == 0)
    }

    @Test("⇧5 on the last photo of a batch still advances")
    func starsAndAdvanceAtBatchEnd() throws {
        let (model, _) = Self.makeModel()
        model.perform(.photoNext)
        model.perform(.photoNext)  // the last photo of the batch
        #expect(model.currentPhotoIndex == 2)
        model.perform(.setStarsAndAdvance(5))
        #expect(model.currentBatchIndex == 1)
        #expect(model.currentPhotoIndex == 0)
    }

    @Test("P is the pick flag in Stars mode and the keep toggle in Keep mode (task.md §6)")
    func pickAndKeep() throws {
        let (model, _) = Self.makeModel()
        model.perform(.togglePickFlag)
        #expect(model.currentPhoto?.rating.flag == .pick)
        #expect(model.currentPhoto?.isKeep == false)

        model.updateSettings { $0.general.ratingMode = .keep }
        model.perform(.toggleKeep)
        #expect(model.currentPhoto?.rating.keep == true)
        #expect(model.currentPhoto?.tier == .keep)
        // The §6 "keep ↔ 5 stars" mapping is a *display* rule. It used to be asserted against
        // `rating.stars`, i.e. the stored value — and storing it is what made `Session::set_rating`
        // write a keep-mode photo as 5 stars into XMP and a stars-mode photo as 0. Assert the
        // display, which is what the user sees, and assert the stored stars are untouched.
        #expect(RatingRules.displayStars(model.currentPhoto!.rating, mode: .stars) == 5)
        #expect(model.currentPhoto?.rating.stars == 0)
        model.perform(.toggleKeep)
        #expect(model.currentPhoto?.rating.keep == false)
        #expect(model.currentPhoto?.tier == .unrated)
    }

    @Test("X rejects, U clears the flag, ` toggles it")
    func flags() throws {
        let (model, _) = Self.makeModel()
        model.perform(.rejectFlag)
        #expect(model.currentPhoto?.rating.flag == .reject)
        #expect(model.currentPhoto?.tier == .rejected)
        model.perform(.unflag)
        #expect(try currentFlag(model) == .none)
        model.perform(.toggleFlag)
        #expect(try currentFlag(model) == .pick)
        model.perform(.toggleFlag)
        #expect(try currentFlag(model) == .none)
    }

    @Test("Colour labels 6–9 work in both modes (task.md §6.3)")
    func labels() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setLabel(.red))
        #expect(model.currentPhoto?.rating.label == .red)
        model.perform(.setLabel(.blue))
        #expect(model.currentPhoto?.rating.label == .blue)
        model.perform(.setLabel(nil))
        #expect(model.currentPhoto?.rating.label == nil)
    }

    @Test("A label doesn't change the tier but does count as reviewed")
    func labelIsNeutral() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setLabel(.green))
        #expect(model.currentPhoto?.tier == .unrated)
        #expect(model.progress.ratedPhotos == 1)
    }

    @Test("Rating only ever touches the current photo (task.md §6.3)")
    func onlyCurrentBatchIsRateable() throws {
        let (model, session) = Self.makeModel()
        model.perform(.setStars(5))
        let firstID = try #require(model.currentPhoto?.id)
        #expect(session.xmpWriteCount == 1)
        model.moveBatch(by: 1)  // flushes the pending XMP queue
        #expect(session.xmpWrites.isEmpty)
        let secondID = try #require(model.currentPhoto?.id)
        model.perform(.setStars(5))
        #expect(session.xmpWriteCount == 2)
        #expect(session.xmpWrites == [secondID])
        // The first batch is untouched.
        #expect(model.photos(inBatch: 0)[0].tier == .keep)
        #expect(model.photos(inBatch: 1)[0].tier == .keep)
    }

    @Test("Re-rating the same photo the same way is a no-op, not an undo step")
    func noOpRatings() throws {
        let (model, session) = Self.makeModel()
        model.perform(.setStars(4))
        model.perform(.setStars(4))
        #expect(session.xmpWrites.count == 1)
        model.perform(.undo)
        #expect(model.currentPhoto?.rating.stars == 0)
    }

    @Test("Ratings land in the session immediately and count towards the HUD")
    func ratingsAreCounted() throws {
        let (model, session) = Self.makeModel()
        model.perform(.setStars(5))
        #expect(session.rating(for: try #require(model.currentPhoto?.id)).stars == 5)
        #expect(model.progress[.keep] == 1)
        #expect(model.progress[.unrated] == 11)
        #expect(model.progress.ratedPhotos == 1)
        #expect(model.summary.keptCount == 1)
        #expect(model.summary.unkeptCount == 11)
    }

    @Test("Switching mode mid-session keeps the data and re-derives the tiers (task.md §6)")
    func modeSwitch() throws {
        let (model, _) = Self.makeModel()
        // Auto-advance, so these land on three different photos.
        model.updateSettings { $0.general.autoAdvance = true }
        model.perform(.setStars(5))
        model.perform(.setStars(2))
        model.perform(.setStars(3))
        model.updateSettings { $0.general.autoAdvance = false }
        model.updateSettings { $0.general.ratingMode = .keep }

        // 5 stars and 4+ are keeps; 3 stars was never a keep.
        #expect(model.photos(inBatch: 0)[0].isKeep)
        #expect(model.photos(inBatch: 0)[1].isKeep == false)
        #expect(model.photos(inBatch: 0)[2].isKeep == false)
        #expect(model.progress[.keep] == 1)
        #expect(model.progress[.unrated] == 11)
        #expect(model.summary.keptCount == 1)
    }

    @Test("Switching to keep mode and back leaves the stars untouched")
    func modeSwitchRoundTrip() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(3))
        model.updateSettings { $0.general.ratingMode = .keep }
        model.updateSettings { $0.general.ratingMode = .stars }
        #expect(model.currentPhoto?.rating.stars == 3)
        #expect(model.currentPhoto?.tier == .good)
    }

    @Test("A change to the keep threshold re-derives every tier")
    func keepThresholdChange() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(4))
        #expect(model.currentPhoto?.tier == .keep)
        model.updateSettings { $0.keepThreshold = 5 }
        #expect(model.currentPhoto?.tier == .good)  // 4 stars is not a keep any more
        #expect(model.currentPhoto?.isKeep == false)
        model.updateSettings { $0.keepThreshold = 4 }
        #expect(model.currentPhoto?.tier == .keep)
    }

    @Test("Nothing can be rated before a folder is open")
    func ratingWithoutSession() throws {
        let model = AppModel(.preview(game: nil))
        model.perform(.setStars(5))
        #expect(model.currentPhoto == nil)
        #expect(model.phase == .welcome)
    }
}

// MARK: - Undo

@MainActor
@Suite("AppModel undo")
struct AppModelUndoTests {
    static func makeModel() -> (AppModel, MockSession) {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "Test")
        return (model, session)
    }

    @Test("⌘Z reverts the last rating")
    func undoRating() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.perform(.undo)
        #expect(model.currentPhoto?.rating.stars == 0)
        #expect(model.currentPhoto?.tier == .unrated)
        model.perform(.redo)
        #expect(model.currentPhoto?.rating.stars == 5)
    }

    @Test("Undoing a change made in another batch goes there first (task.md §6.3)")
    func undoNavigatesToTheOtherBatch() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        let ratedID = try #require(model.currentPhoto?.id)
        model.moveBatch(by: 2)
        #expect(model.currentBatchIndex == 2)
        model.perform(.undo)
        #expect(model.currentBatchIndex == 0)  // navigated back to the change
        #expect(model.currentPhoto?.id == ratedID)
        #expect(model.currentPhoto?.rating.stars == 0)
    }

    @Test("A cross-batch undo only marks the batch it lands in")
    func undoVisitsTheOldBatch() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.moveBatch(by: 1)
        model.undo()
        #expect(model.isVisited(0))
    }

    @Test("Undo with nothing to undo does nothing")
    func undoOnEmptyHistory() throws {
        let (model, _) = Self.makeModel()
        model.perform(.undo)
        #expect(model.currentPhoto?.rating.stars == 0)
        #expect(model.currentBatchIndex == 0)
    }

    @Test("Undo and redo walk the whole history in order")
    func undoRedoSequence() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(3))
        model.perform(.setStars(5))
        model.perform(.setLabel(.red))
        model.perform(.undo)
        #expect(model.currentPhoto?.rating.label == nil)
        #expect(model.currentPhoto?.rating.stars == 5)
        model.perform(.undo)
        #expect(model.currentPhoto?.rating.stars == 3)
        model.perform(.redo)
        model.perform(.redo)
        #expect(model.currentPhoto?.rating.stars == 5)
        #expect(model.currentPhoto?.rating.label == .red)
    }
}

// MARK: - Re-batching

@MainActor
@Suite("Batches changing under the user")
struct BatchesChangedTests {
    static func makeModel() -> (AppModel, MockSession) {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 24, burstSize: 6))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "Test")
        return (model, session)
    }

    @Test("Refined boundaries never move the batch the user is in (task.md §5.4)")
    func currentBatchIsFrozen() throws {
        let (model, session) = Self.makeModel()
        let stayingID = try #require(model.currentBatch?.id)
        let stayingPhoto = try #require(model.currentPhoto?.id)
        let batchCountBefore = model.batches.count

        // Visual signatures that disagree with the timing, so unvisited batches re-batch.
        var sigs: [(PhotoID, VisualSig)] = []
        for index in 0..<24 {
            sigs.append((model.allPhotos[index].id, VisualSig(dhash: index < 6 ? 0 : ~0, hist: [])))
        }
        session.submitVisualSigs(sigs)

        #expect(model.currentBatch?.id == stayingID)
        #expect(model.currentPhoto?.id == stayingPhoto)
        #expect(model.batches.count >= batchCountBefore)  // only ever more boundaries
    }

    @Test("Re-batching later in the shoot keeps the cursor's photo selected")
    func laterRebatchKeepsPhoto() throws {
        let (model, session) = Self.makeModel()
        model.moveBatch(by: 2)
        let photoID = try #require(model.currentPhoto?.id)
        session.submitVisualSigs(
            (0..<24).map { (model.allPhotos[$0].id, VisualSig(dhash: $0 < 6 ? 0 : ~0, hist: [])) })
        #expect(model.currentPhoto?.id == photoID)
    }

    @Test("Batch indices and the cursor stay consistent after a re-batch")
    func indicesStayConsistent() throws {
        let (model, session) = Self.makeModel()
        session.submitVisualSigs(
            (0..<24).map { (model.allPhotos[$0].id, VisualSig(dhash: $0 < 6 ? 0 : ~0, hist: [])) })
        for (offset, batch) in model.batches.enumerated() {
            #expect(batch.index == offset)
            #expect(model.batchIndex(for: batch.id) == offset)
            #expect(batch.photoIDs.count == batch.range.count)
        // Only rebuilt batches stop being provisional; the batch the user is in is frozen and comes
        // back exactly as it was (batching.md).
        #expect(model.batches[0].provisional)
        #expect(model.batches.dropFirst().allSatisfy { !$0.provisional })
        }
        #expect(model.photos(inBatch: 0).map(\.id) == model.batches[0].photoIDs)
    }

    @Test("XMP import refreshes the tiers")
    func importedRatings() throws {
        let (model, _) = Self.makeModel()
        var keep = Rating()
        keep.stars = 5
        model.sessionDidImportRatings([try #require(model.currentPhoto?.id): keep])
        #expect(model.currentPhoto?.tier == .keep)
        #expect(model.progress[.keep] == 1)
    }

    @Test("An XMP failure is reported, not fatal")
    func xmpFailure() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.sessionDidFailWritingXMP(photo: try #require(model.currentPhoto?.id), message: "disk full")
        #expect(model.lastError != nil)
        #expect(model.currentPhoto?.rating.stars == 5)
    }
}

// MARK: - Finish

@MainActor
@Suite("Finish Cull flow")
struct FinishFlowTests {
    static func makeModel() -> (AppModel, MockSession) {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "Test")
        return (model, session)
    }

    @Test("The summary counts every tier and warns about unvisited batches (task.md §9.7)")
    func summary() throws {
        let (model, _) = Self.makeModel()
        model.updateSettings { $0.general.autoAdvance = true }
        model.perform(.setStars(5))
        model.perform(.setStars(3))
        model.updateSettings { $0.general.autoAdvance = false }
        model.moveBatch(by: 3)
        model.perform(.setStars(5))
        model.startFinish()

        guard case .summary(let summary) = model.finish else {
            Issue.record("expected the summary stage")
            return
        }
        #expect(summary[.keep] == 2)
        #expect(summary[.good] == 1)
        #expect(summary.unkeptCount == 10)
        #expect(summary.unvisitedBatches == 2)
        #expect(summary.isComplete == false)
        #expect(summary.totalPhotos == 12)
    }

    @Test("The flow runs summary → options → dry run → execute → report")
    func fullFlow() throws {
        let (model, session) = Self.makeModel()
        model.perform(.setStars(5))
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings { $0.unkept = .moveToSubfolder("_Not kept") }
        model.runFinishDryRun()

        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        #expect(plan.ops.isEmpty == false)
        // Every unkept photo moves, keeps stay put.
        #expect(plan.ops.count == 11)
        #expect(plan.ops.allSatisfy { $0.kind == .move })
        #expect(plan.ops.contains { $0.to?.hasSuffix("_Not kept/IMG_0002.CR3") == true })

        model.executeFinish()
        guard case .report(_, let report) = model.finish else {
            Issue.record("expected the report stage")
            return
        }
        #expect(report.done == 11)
        #expect(report.undoable)
        #expect(session.executedPlans.count == 1)
    }

    @Test("Nothing runs before a dry run has been shown")
    func noExecutionWithoutDryRun() throws {
        let (model, session) = Self.makeModel()
        model.startFinish()
        model.showFinishOptions()
        model.executeFinish()
        #expect(session.executedPlans.isEmpty)
    }

    @Test("Permanent delete says it can't be undone")
    func permanentDelete() throws {
        let (model, _) = Self.makeModel()
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings { $0.unkept = .deletePermanently }
        model.runFinishDryRun()
        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        #expect(plan.warnings.contains { $0.contains("cannot be undone") })
        model.executeFinish()
        guard case .report(_, let report) = model.finish else {
            Issue.record("expected the report stage")
            return
        }
        #expect(report.undoable == false)
    }

    @Test("Trash warns that Firstcut can't undo it, Finder can")
    func trashWarning() throws {
        let (model, _) = Self.makeModel()
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings { $0.unkept = .moveToTrash }
        model.runFinishDryRun()
        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        #expect(plan.ops.allSatisfy { $0.kind == .trash })
        #expect(plan.warnings.contains { $0.contains("Finder") })
    }

    @Test("Copying keeps needs free space, and the size is reported")
    func copyKeeps() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings {
            $0.unkept = .nothing
            $0.kept = .copyTo("/Volumes/Backup")
        }
        model.runFinishDryRun()
        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        #expect(plan.ops.count == 1)
        #expect(plan.ops.first?.kind == .copy)
        #expect(plan.bytesToCopy == 12_000_000)
    }

    @Test("Writing a list touches one file, not the photos")
    func writeList() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings {
            $0.unkept = .nothing
            $0.kept = .writeList("kept.txt")
        }
        model.runFinishDryRun()
        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        #expect(plan.ops.count == 1)
        #expect(plan.ops.first?.kind == .writeList)
        #expect(plan.ops.first?.from == "IMG_0001.CR3")
    }

    @Test("Finish can be undone while the moves are reversible")
    func undoFinish() throws {
        let (model, session) = Self.makeModel()
        model.startFinish()
        model.showFinishOptions()
        model.runFinishDryRun()
        model.executeFinish()
        model.undoFinish()
        #expect(session.undoneFinishes == 1)
    }

    @Test("Cancelling hides the sheet and touches nothing")
    func cancel() throws {
        let (model, session) = Self.makeModel()
        model.startFinish()
        model.cancelFinish()
        #expect(model.finish == .hidden)
        #expect(session.executedPlans.isEmpty)
    }

    @Test("The rating mode can't be faked: the plan uses the live one")
    func planFollowsMode() throws {
        let (model, _) = Self.makeModel()
        model.perform(.setStars(5))
        model.startFinish()
        model.showFinishOptions()
        model.updateFinishSettings { $0.ratingMode = .keep }
        #expect(model.settings.general.ratingMode == .stars)
        model.runFinishDryRun()
        guard case .dryRun(_, _, let plan) = model.finish else {
            Issue.record("expected the dry run stage")
            return
        }
        // In stars mode the 5-star photo is a keep, so it is not among the 11 moved.
        #expect(plan.ops.count == 11)
    }

    @Test("Finish is only offered while culling")
    func finishNeedsASession() throws {
        let model = AppModel(.preview(game: nil))
        model.startFinish()
        #expect(model.finish == .hidden)
    }
}

// MARK: - Fixtures

@Suite("Fixtures")
struct FixturePhotosTests {
    @Test("The exiftool dump of Game1JENKS decodes into 708 CR3 photos")
    func loadsRealFixture() throws {
        let photos = try FixturePhotos.loadPhotos(game: "Game1JENKS")
        #expect(photos.count == 708)
        #expect(photos.allSatisfy { $0.kind == .raw(.cr3) })
        #expect(photos.allSatisfy { $0.captureTime != nil })
        #expect(photos.allSatisfy { $0.cameraModel == "Canon EOS R8" })
        #expect(photos.allSatisfy { $0.shutterCount != nil })
    }

    @Test("Sub-second EXIF timestamps convert to UTC with the original offset")
    func exifDateParsing() throws {
        let capture = try #require(
            ExifDate.parse("2026:08:27 19:54:49.84-06:00", offsetTimeOriginal: "-06:00"))
        // 19:54:49.84 at -06:00 is 2026-08-28 01:54:49.840 UTC.
        #expect(capture.unixMs == 1_787_882_089_840)
        #expect(capture.offsetMinutes == -360)
        #expect(capture.subsecResolutionMs == 10)
    }

    @Test("A timestamp without a fraction or offset still parses")
    func exifDateWithoutFraction() throws {
        let capture = try #require(ExifDate.parse("2026:08:27 19:54:49", offsetTimeOriginal: nil))
        #expect(capture.unixMs == 1_787_860_489_000)
        #expect(capture.subsecResolutionMs == 1000)
    }

    @Test("Photo ids are stable across runs")
    func stableIDs() throws {
        #expect(FixturePhotos.stableID("IMG_0001.CR3") == FixturePhotos.stableID("IMG_0001.CR3"))
        #expect(FixturePhotos.stableID("IMG_0001.CR3") != FixturePhotos.stableID("IMG_0002.CR3"))
    }

    @Test("The mock batcher splits the real shoot into many plausible batches")
    func mockBatching() throws {
        let photos = try FixturePhotos.loadPhotos(game: "Game1JENKS")
        let batches = FixturePhotos.batches(for: photos)
        #expect(batches.count > 10)
        #expect(batches.count < photos.count)
        #expect(batches.flatMap(\.photoIds).count == photos.count)
        #expect(batches.allSatisfy { $0.provisional })
        // Deterministic: the same input always gives the same batches (task.md §5.3).
        #expect(FixturePhotos.batches(for: photos) == batches)
    }

    @Test("A fast burst stays together and a long pause splits it")
    func mockBatchingBoundaries() throws {
        let photos = FixturePhotos.syntheticPhotos(count: 24, burstSize: 12)
        let batches = FixturePhotos.batches(for: photos)
        #expect(batches.count == 2)
        #expect(batches[0].photoIds.count == 12)
    }
}

@MainActor
/// `model.currentPhoto?.rating.flag == .none` compares the *optional* against nil rather than the
/// enum case, so the flag has to be unwrapped first.
func currentFlag(_ model: AppModel) throws -> Flag {
    try #require(model.currentPhoto?.rating.flag)
}
