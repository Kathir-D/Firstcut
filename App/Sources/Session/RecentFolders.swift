// Owner: app-logic.
//
// The folders the user opened recently, with how far through each one they got (task.md §9.6, the
// Welcome window: "recent sessions with progress (resume)"). The shoot itself resumes from its
// session database; this is only the list that gets the user back to it.
//
// Stored next to the settings, as a plain file, so a test can point it at a temporary directory and
// nothing depends on `UserDefaults` state left by an earlier run.

import Foundation

public struct RecentFolder: Codable, Hashable, Identifiable, Sendable {
    public var path: String
    public var name: String
    public var openedAt: Date
    public var totalPhotos: Int
    public var ratedPhotos: Int

    public var id: String { path }

    public init(path: String, name: String, openedAt: Date, totalPhotos: Int, ratedPhotos: Int) {
        self.path = path
        self.name = name
        self.openedAt = openedAt
        self.totalPhotos = totalPhotos
        self.ratedPhotos = ratedPhotos
    }

    /// 0…1, or 0 for an empty folder.
    public var fractionRated: Double {
        totalPhotos > 0 ? min(1, Double(ratedPhotos) / Double(totalPhotos)) : 0
    }

    public var url: URL { URL(fileURLWithPath: path, isDirectory: true) }
}

public struct RecentFoldersStore: Sendable {
    public static let fileName = "recents.json"
    public static let limit = 10

    public let directory: URL

    public init(directory: URL? = nil) {
        self.directory = directory ?? KeymapStore.defaultDirectory
    }

    public var fileURL: URL { directory.appendingPathComponent(Self.fileName) }

    /// Never throws: a missing or corrupt list is an empty list, because it must not stop a launch.
    public func load() -> [RecentFolder] {
        guard let data = try? Data(contentsOf: fileURL) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return (try? decoder.decode([RecentFolder].self, from: data)) ?? []
    }

    public func save(_ folders: [RecentFolder]) {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(folders).write(to: fileURL, options: .atomic)
        } catch {
            // A recents list that cannot be saved costs the user a shortcut, nothing more.
        }
    }

    /// `folder` moved to the front, replacing any earlier entry for the same path, capped at
    /// `limit`. Pure, so it can be tested without a disk.
    public static func recording(_ folder: RecentFolder, in list: [RecentFolder]) -> [RecentFolder] {
        var result = list.filter { $0.path != folder.path }
        result.insert(folder, at: 0)
        return Array(result.prefix(limit))
    }
}
