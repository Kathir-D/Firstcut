// Owner: app-logic.
//
// Loading, layering and persisting keymaps (todo.md §10, §9.8 Keyboard).
//
// Two files, one effective map:
//   * `App/Resources/DefaultKeymap.json` — shipped, read-only, Lightroom Classic defaults.
//   * `~/Library/Application Support/Firstcut/keymap.json` — the user's overrides. Holds only the
//     commands they changed, and is written atomically so a crash can't leave a half-written file
//     that silently drops every shortcut.

import Foundation

public struct KeymapStore: Sendable {
    public static let defaultsResourceName = "DefaultKeymap"
    public static let userFileName = "keymap.json"

    public let directory: URL
    private let bundle: Bundle

    public private(set) var defaults: Keymap
    public private(set) var overrides: Keymap

    /// Defaults with the user's overrides applied. This is what the router and the menus use.
    public var effective: Keymap { overrides.layered(over: defaults) }

    public var userFileURL: URL { directory.appendingPathComponent(Self.userFileName) }

    /// - Parameters:
    ///   - directory: where `keymap.json` lives. Injectable so tests never touch the real one.
    ///   - bundle: where `DefaultKeymap.json` is looked up. Injectable for the same reason.
    public init(directory: URL? = nil, bundle: Bundle = .main) {
        self.directory = directory ?? Self.defaultDirectory
        self.bundle = bundle
        self.defaults = Keymap()
        self.overrides = Keymap()
    }

    public static var defaultDirectory: URL {
        let base =
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return base.appendingPathComponent("Firstcut", isDirectory: true)
    }

    public enum LoadError: Error, CustomStringConvertible {
        case defaultsNotFound([String])
        case userKeymapUnreadable(URL, String)

        public var description: String {
            switch self {
            case .defaultsNotFound(let tried):
                "DefaultKeymap.json not found. Looked in: \(tried.joined(separator: ", "))"
            case .userKeymapUnreadable(let url, let reason):
                "Could not read \(url.lastPathComponent): \(reason)"
            }
        }
    }

    /// Reads both files. A missing or corrupt *user* file is not fatal — the app falls back to the
    /// defaults, because a broken preferences file must never stop someone culling a shoot.
    /// A missing *defaults* file is a build error and throws.
    @discardableResult
    public mutating func load() throws -> Bool {
        defaults = try loadDefaults()
        overrides = loadOverrides()
        return !overrides.bindings.isEmpty
    }

    private mutating func loadDefaults() throws -> Keymap {
        var tried: [String] = []
        for candidate in Self.defaultsURLCandidates(bundle: bundle) {
            tried.append(candidate.path)
            guard let data = try? Data(contentsOf: candidate) else { continue }
            return try Keymap(json: data)
        }
        throw LoadError.defaultsNotFound(tried)
    }

    private mutating func loadOverrides() -> Keymap {
        guard let data = try? Data(contentsOf: userFileURL) else { return Keymap() }
        return (try? Keymap(json: data)) ?? Keymap()
    }

    /// `Bundle.main` in the app, the test bundle in unit tests, and the source tree when running
    /// from a checkout — so `AppModel.preview` and the tests never need a built .app.
    static func defaultsURLCandidates(bundle: Bundle) -> [URL] {
        var urls: [URL] = []
        if let url = bundle.url(forResource: defaultsResourceName, withExtension: "json") {
            urls.append(url)
        }
        if let url = Bundle.main.url(forResource: defaultsResourceName, withExtension: "json") {
            urls.append(url)
        }
        // App/Resources/DefaultKeymap.json relative to this source file.
        let source = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // Input/
            .deletingLastPathComponent()  // Sources/
            .deletingLastPathComponent()  // App/
            .appendingPathComponent("Resources")
            .appendingPathComponent("\(defaultsResourceName).json")
        urls.append(source)
        return urls
    }

    // MARK: - Persistence

    public func save() throws {
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true)
        let data = try overrides.encoded()
        try data.write(to: userFileURL, options: .atomic)
    }

    public mutating func setOverrides(_ overrides: Keymap) {
        self.overrides = overrides
    }

    /// Throws away every user override.
    public mutating func resetToDefaults() {
        overrides = Keymap()
    }

    public mutating func importKeymap(from url: URL) throws {
        let data = try Data(contentsOf: url)
        overrides = try Keymap(json: data)
        try save()
    }

    public func exportKeymap(to url: URL) throws {
        try effective.encoded().write(to: url, options: .atomic)
    }

    /// Conflicts in the map the app actually runs.
    public func conflicts() -> [ChordConflict] { effective.conflicts() }
}
