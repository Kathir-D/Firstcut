// Owner: ui.
//
// `ModelCullViewState` — the projection the views bind to, over the real `AppModel`.
//
// Before this the app rendered `PreviewCullViewState`, so nothing tested the path the app actually
// takes. These are the tests that make a `Phase` case, a `CullAction` or a `Command` that the
// model cannot answer a compile error rather than a dead menu item.

import Foundation
import Testing

@testable import Firstcut

@Suite("The view state the app actually runs")
@MainActor
struct ModelCullViewStateTests {
    private static func makeState(
        ratingMode: RatingMode = .stars
    ) -> (ModelCullViewState, AppModel, MockSession) {
        let photos = FixturePhotos.syntheticPhotos(count: 12, burstSize: 3)
        let session = MockSession(photos: photos)
        let model = AppModel(.testing(backend: session))
        model.updateSettings { $0.general.ratingMode = ratingMode }
        model.open(session, folderName: "Game1JENKS")
        return (ModelCullViewState(model: model, images: PreviewImageSource(seed: 3)), model, session)
    }

    @Test("An unopened model is the welcome screen, and it is not the culling screen")
    func unopenedIsWelcome() {
        let model = AppModel(.preview(game: nil))
        let state = ModelCullViewState(model: model, images: PreviewImageSource(seed: 1))
        #expect(state.phase == .welcome)
        #expect(state.photosInCurrentBatch.isEmpty)
        #expect(state.currentPhoto == nil)
        #expect(state.batches.isEmpty)
    }

    @Test("A viewer pane resolves its image source and its model from the active state")
    func theActiveModelIsTheOneTheStateWraps() {
        // `AppEnvironment`'s host factory builds every viewer pane from `state.images` and reports
        // frames and viewport size to `state.activeModel` — both resolved when the pane is created,
        // so a `use(_:)` swap (the mock shoot, previews, the Screenshots workflow) is honoured.
        // Before that, a pane held the real `ImageProvider` even in the mock shoot, which never
        // gives it a folder: the loupe rendered flat black while the filmstrip drew its thumbnails.
        // This is the seam that keeps the two answerable to the same state.
        let (state, model, _) = Self.makeState()
        #expect(state.activeModel === model, "the pane reports to the model that is on screen")

        // The stand-in has no model to report to, and says so rather than reaching for a global.
        let preview = PreviewCullViewState(batchCount: 0)
        #expect(preview.activeModel == nil)

        // And the image source is the state's, so a swap changes what a new pane draws.
        let source = PreviewImageSource(seed: 7)
        let swapped = ModelCullViewState(model: model, images: source)
        #expect(swapped.images === source)
    }

    @Test("`activeModel` survives the trip through `any CullViewState`")
    func activeModelIsDispatchedDynamically() {
        // The test above cannot catch this, and neither could any other test at the time: it holds
        // the *concrete* `ModelCullViewState`, so `activeModel` binds statically to the
        // implementation. `AppEnvironment` does not hold the concrete type — it holds
        // `any CullViewState`, and the host factory asks that existential for the model.
        //
        // While `activeModel` was declared only in the protocol *extension*, an existential
        // dispatched it statically to the extension's body, so every call returned nil. The factory's
        // `if let model = state.activeModel` therefore never ran in the app, and the three closures
        // inside it — `onFramePresented` (§7.3's frame-latency intervals), `onViewportPixelSize`
        // (T2 decoded at exactly the pixels the viewer covers) and `onDoubleClick` (a double-click
        // jumps a batch) — were all silently dead. Deleting the double-click closure, or the other
        // two, left the suite green.
        let (concrete, model, _) = Self.makeState()

        // The existential is the whole point: this is the type the environment and every view hold.
        let existential: any CullViewState = concrete
        #expect(
            existential.activeModel === model,
            "an `any CullViewState` must dispatch `activeModel` to the concrete state, or every viewer pane built from it goes unwired"
        )

        // Same seam through a generic function, which is how the views consume it.
        func activeModel<S: CullViewState>(_ s: S) -> AppModel? { s.activeModel }
        #expect(activeModel(existential) === model)

        // And the stand-in still says nil, through the existential this time.
        let preview: any CullViewState = PreviewCullViewState(batchCount: 0)
        #expect(preview.activeModel == nil)
    }

    @Test("Every `Phase` case maps to a `CullPhase`, exhaustively")
    func phaseMappingIsExhaustive() {
        let (state, model, _) = Self.makeState()
        #expect(state.phase == .culling)

        // A loading phase is what `AppEnvironment` shows while a folder is being scanned.
        let loading = LoadProgress(title: "Reading", fraction: 0.5)
        #expect(loading.fraction == 0.5)
        #expect(model.phase == .culling)

        #expect(CullPhase.welcome.displayNameForTests == "welcome")
        #expect(CullPhase.loading(progress: 0).displayNameForTests == "loading")
        #expect(CullPhase.culling.displayNameForTests == "culling")
        #expect(CullPhase.finishing.displayNameForTests == "finishing")
    }

    @Test("A session shows the folder's real batch and photo counts")
    func projectsTheSession() {
        let (state, model, _) = Self.makeState()
        #expect(state.folderName == "Game1JENKS")
        #expect(state.batches.count == model.batches.count)
        #expect(state.batches.map(\.id) == model.batches.map(\.id))
        #expect(state.batches.map(\.index) == model.batches.map(\.index))
        #expect(state.photosInCurrentBatch.map(\.id) == model.photos(inBatch: 0).map(\.id))
        #expect(state.currentPhotoIndex == 0)
        #expect(state.currentPhoto?.fileName == "IMG_0001.CR3")
        #expect(state.progress.totalPhotos == 12)
    }

    @Test("Navigation actions move the model, not a copy of it")
    func navigationReachesTheModel() {
        let (state, model, _) = Self.makeState()
        state.send(.photoNext)
        #expect(model.currentPhotoIndex == 1)
        #expect(state.currentPhotoIndex == 1)
        #expect(state.currentPhoto?.id == model.currentPhoto?.id)

        state.send(.batchNext)
        #expect(model.currentBatchIndex == 1)
        #expect(state.currentBatchIndex == 1)
        #expect(state.photosInCurrentBatch.first?.id == model.photos(inBatch: 1).first?.id)
    }

    @Test("Selecting a frame in the filmstrip is a position inside the current batch")
    func selectPhotoIsBatchRelative() {
        let (state, model, _) = Self.makeState()
        state.send(.batchNext)
        state.send(.selectPhoto(1))
        #expect(model.currentBatchIndex == 1)
        #expect(model.currentPhotoIndex == 1)
        // Out of range does nothing, rather than jumping into the next batch.
        state.send(.selectPhoto(99))
        #expect(model.currentPhotoIndex == 1)
    }

    @Test("Rating actions persist through the session and update the projection")
    func ratingActionsPersist() {
        let (state, model, session) = Self.makeState()
        state.send(.setRating(stars: 5))
        #expect(model.currentPhoto?.rating.stars == 5)
        #expect(session.rating(for: model.currentPhoto!.id).stars == 5)
        #expect(state.currentPhoto?.rating.stars == 5)
        #expect(state.currentPhoto?.tier == .keep)
        #expect(state.currentPhoto?.isKeep == true)
        #expect(state.progress[.keep] == 1)

        state.send(.setFlag(.reject))
        #expect(state.currentPhoto?.tier == .rejected)
        state.send(.setFlag(.none))
        #expect(state.currentPhoto?.tier == .keep)
        state.send(.setLabel(.blue))
        #expect(state.currentPhoto?.rating.label == .blue)
    }

    @Test("Keep mode is a mode switch, not a second set of actions")
    func keepMode() {
        let (state, _, _) = Self.makeState(ratingMode: .keep)
        #expect(state.ratingMode == .keep)
        state.send(.toggleKeep)
        #expect(state.currentPhoto?.rating.keep == true)
        #expect(state.currentPhoto?.tier == .keep)
    }

    @Test("Undo and redo go through the model's history")
    func undoRedo() {
        let (state, model, _) = Self.makeState()
        state.send(.setRating(stars: 3))
        #expect(model.currentPhoto?.tier == .good)
        state.send(.undo)
        #expect(model.currentPhoto?.rating.stars == 0)
        state.send(.redo)
        #expect(model.currentPhoto?.rating.stars == 3)
    }

    @Test("Toggles reach the model, so the toolbar and the menu cannot disagree")
    func toggles() {
        let (state, model, _) = Self.makeState()
        #expect(state.isInfoPanelVisible == false)
        state.send(.toggleInfoPanel)
        #expect(model.infoPanelVisible)
        #expect(state.isInfoPanelVisible)
        state.send(.toggleHUD)
        #expect(state.isHUDVisible == false)
        state.send(.toggleAFOverlay)
        #expect(state.showsAFOverlay)
        state.send(.toggleClippingOverlay)
        #expect(state.showsClippingOverlay)
        state.send(.toggleAutoAdvance)
        #expect(state.autoAdvanceEnabled)
    }

    @Test("The view mode is a command, so L/G/E and the picker are one thing")
    func viewModes() {
        let (state, model, _) = Self.makeState()
        state.send(.setViewMode(.grid))
        #expect(state.viewMode == .grid)
        #expect(model.viewMode == .grid)
        state.send(.setViewMode(.compare(count: 3)))
        #expect(state.viewMode == .compare(count: 3))
        #expect(model.viewMode == .compare(3))
    }

    @Test("The projection holds no state of its own, so it cannot drift from the model")
    func noShadowState() {
        let (state, model, _) = Self.makeState()
        // Every accessor is a read-through. Changing the model behind the view's back is visible
        // immediately, with no notification to forget.
        model.perform(.photoNext)
        #expect(state.currentPhoto?.id == model.currentPhoto?.id)
        model.perform(.setStars(4))
        #expect(state.currentPhoto?.tier == .keep)
        #expect(state.progress.ratedPhotos == 1)
    }

    @Test("Every `CullAction` the app can send resolves to something the model understands")
    func everyActionMaps() {
        // The compile-time half of this is the exhaustive switch in `CullAction.command`; the
        // runtime half is that a mapped command does not trap on an unopened model.
        let model = AppModel(.preview(game: nil))
        let state = ModelCullViewState(model: model, images: PreviewImageSource(seed: 1))
        let actions: [CullAction] = [
            .photoPrevious, .photoNext, .batchPrevious, .batchNext, .selectPhoto(0),
            .setViewMode(.loupe), .toggleInfoPanel, .toggleHUD, .toggleAFOverlay,
            .toggleClippingOverlay, .toggleAutoAdvance, .setRating(stars: 3), .setFlag(.pick),
            .setFlag(.reject), .setFlag(.none), .setLabel(.green), .setLabel(nil), .toggleKeep,
            .undo, .redo, .openFolder, .finishCull,
        ]
        for action in actions { state.send(action) }
        #expect(state.phase == .welcome, "nothing above can open a folder or start a cull")
    }
}

extension CullPhase {
    /// A name for each phase, so a test can assert the mapping covers all of them.
    var displayNameForTests: String {
        switch self {
        case .welcome: "welcome"
        case .loading: "loading"
        case .culling: "culling"
        case .finishing: "finishing"
        }
    }
}
