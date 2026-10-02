// Owner: app-logic.
//
// How an `AppModel` gets its collaborators. Injected rather than constructed inside the model for
// three reasons: the fixtures can drive it before the Rust core exists, qa can hand it a scripted
// session, and the real UniFFI `Session` slots in without the model changing a line.

import Foundation

@MainActor
public struct Dependencies {
    /// nil = an empty session; the model starts in `.welcome` until one is opened.
    public var backend: (any SessionBackend)?
    public var images: any ImageProviding
    public var keymap: Keymap
    public var settings: AppSettings
    public var settingsStore: SettingsStore
    public var keymapStore: KeymapStore
    /// How `AppModel.open(folder:)` turns a folder into a session. The shipped app replaces this with
    /// core-store's `Session.open`; previews point it at a fixture.
    public var sessionFactory: (URL) throws -> any SessionBackend
    /// When set, `AppModel.open(folder:)` opens through this instead, off the main thread, showing
    /// the loading screen meanwhile. nil (the tests, previews) keeps the synchronous factory.
    public var asyncSessionFactory: (@MainActor (URL) async throws -> any SessionBackend)?
    /// Reads **one** photograph's metadata from a folder, without reading the rest — the core's
    /// `first_photo_name` + `read_photo`. nil wherever there are no real files to read (tests,
    /// previews), and the open path is then exactly as slow as it was.
    ///
    /// This is what todo.md §7.3's "folder open → first photo on screen < 1 s" is built on: a
    /// directory listing plus one header read, so a photograph is on screen while the other 2,879
    /// headers are still being read. The photo it names is the *first by file name*, not the first in
    /// capture order — capture time is what orders a shoot (§2) and reading it is what the scan is
    /// for — so it is a first frame, not the shoot's first frame. See `AppModel.showProvisional`.
    public var firstPhoto: (@Sendable (URL) -> PhotoMeta?)?

    public init(
        backend: (any SessionBackend)? = nil,
        images: any ImageProviding = MockImageProvider(),
        keymap: Keymap = .empty,
        settings: AppSettings = AppSettings(),
        settingsStore: SettingsStore = SettingsStore(),
        keymapStore: KeymapStore = KeymapStore(),
        sessionFactory: @escaping (URL) throws -> any SessionBackend = { url in
            MockSession(data: try MockSession.dataForFolder(url))
        },
        asyncSessionFactory: (@MainActor (URL) async throws -> any SessionBackend)? = nil,
        firstPhoto: (@Sendable (URL) -> PhotoMeta?)? = nil
    ) {
        self.backend = backend
        self.images = images
        self.keymap = keymap
        self.settings = settings
        self.settingsStore = settingsStore
        self.keymapStore = keymapStore
        self.sessionFactory = sessionFactory
        self.asyncSessionFactory = asyncSessionFactory
        self.firstPhoto = firstPhoto
    }

    /// Everything on disk: real Application Support, the keymap shipped in the bundle, the real
    /// `ImageProvider` decoding real CR3 previews, and the real Rust `Session` behind the session
    /// factory. This is the shipped app.
    ///
    /// The two mocks that used to be here are gone: `MockImageProvider` drew coloured rectangles,
    /// and the default `sessionFactory` opened a `MockSession`. Both are still available — via
    /// `.preview()` and `MockImageProvider` directly — because the tests need them, but nothing in
    /// the app's own path reaches a mock now.
    public static func live() -> Dependencies {
        let settingsStore = SettingsStore()
        var keymapStore = KeymapStore()
        _ = try? keymapStore.load()
        let settings = settingsStore.load()
        return Dependencies(
            backend: nil,
            images: ImageProvider(
                memoryBudgetBytes: settings.memoryBudgetBytes,
                maxConcurrentDecodes: settings.performance.decodeThreads),
            keymap: keymapStore.effective,
            settings: settings,
            settingsStore: settingsStore,
            keymapStore: keymapStore,
            sessionFactory: SessionFactory.live(),
            asyncSessionFactory: SessionFactory.liveAsync(),
            firstPhoto: CoreFirstPhoto.read)
    }

    /// Fixtures (or a synthetic shoot) and a scratch directory, so a preview can't write over the
    /// real settings or keymap.
    public static func preview(game: String? = "Game1JENKS", photoLimit: Int? = nil) -> Dependencies {
        let scratch = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutPreview-\(UUID().uuidString)", isDirectory: true)
        var keymapStore = KeymapStore(directory: scratch)
        _ = try? keymapStore.load()
        let data = previewData(game: game, photoLimit: photoLimit)
        return Dependencies(
            backend: MockSession(data: data),
            images: MockImageProvider(),
            keymap: keymapStore.effective,
            settings: AppSettings(),
            settingsStore: SettingsStore(directory: scratch),
            keymapStore: keymapStore)
    }

    /// A scratch directory for stores, so a test that builds `Dependencies` **itself** — rather
    /// than through `.testing` — still cannot read or write the real settings, keymap or recents.
    ///
    /// `SettingsStore()` and `KeymapStore()` both default to `~/Library/Application Support/
    /// Firstcut`, so a test that omits them shares state with the installed app: it loads whatever
    /// the last run left, and `AppModel.recordRecent` **saves back into it**. That happened — a unit
    /// test left three `Game1JENKS` entries in the real `recents.json`, and a later run failed
    /// because of them. Nothing in CI showed it because the runner's Application Support starts
    /// empty every time, so the leak is invisible until it is not.
    ///
    /// Every test that builds `Dependencies` should pass `settingsStore:` and `keymapStore:` from
    /// here. `.testing` and `.preview` already do.
    public static func scratchStore() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutTest-\(UUID().uuidString)", isDirectory: true)
    }

    /// A test-friendly variant: everything in memory except the directories.
    public static func testing(
        backend: any SessionBackend, images: any ImageProviding = MockImageProvider(),
        settings: AppSettings = AppSettings(), keymap: Keymap? = nil
    ) -> Dependencies {
        let scratch = scratchStore()
        var keymapStore = KeymapStore(directory: scratch)
        _ = try? keymapStore.load()
        return Dependencies(
            backend: backend,
            images: images,
            keymap: keymap ?? keymapStore.effective,
            settings: settings,
            settingsStore: SettingsStore(directory: scratch),
            keymapStore: keymapStore)
    }

    private static func previewData(game: String?, photoLimit: Int?) -> SessionData {
        if let game, let loaded = try? FixturePhotos.loadSessionData(game: game) {
            guard let limit = photoLimit, loaded.photos.count > limit else { return loaded }
            let trimmed = Array(loaded.photos.prefix(limit))
            return SessionData(
                folder: loaded.folder, photos: trimmed, batches: FixturePhotos.batches(for: trimmed))
        }
        return FixturePhotos.syntheticSessionData()
    }
}
