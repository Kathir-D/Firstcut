import Foundation
import Testing

@testable import Firstcut

@Suite("Recent folders (todo.md §9.6)")
struct RecentFoldersTests {
  private func entry(_ name: String, total: Int = 100, rated: Int = 0) -> RecentFolder {
    RecentFolder(
      path: "/Volumes/Card/\(name)", name: name, openedAt: Date(timeIntervalSince1970: 1_000),
      totalPhotos: total, ratedPhotos: rated)
  }

  @Test("The newest folder comes first and a reopened one moves up instead of repeating")
  func newestFirstNoDuplicates() {
    var list: [RecentFolder] = []
    list = RecentFoldersStore.recording(entry("A"), in: list)
    list = RecentFoldersStore.recording(entry("B"), in: list)
    list = RecentFoldersStore.recording(entry("A", rated: 40), in: list)
    #expect(list.map(\.name) == ["A", "B"])
    #expect(list[0].ratedPhotos == 40, "the reopened entry carries the latest progress")
  }

  @Test("The list is capped")
  func capped() {
    var list: [RecentFolder] = []
    for index in 0..<(RecentFoldersStore.limit + 5) {
      list = RecentFoldersStore.recording(entry("Game\(index)"), in: list)
    }
    #expect(list.count == RecentFoldersStore.limit)
    #expect(list.first?.name == "Game\(RecentFoldersStore.limit + 4)")
  }

  @Test("Progress is a fraction of the shoot, and an empty folder is zero rather than NaN")
  func fraction() {
    #expect(entry("A", total: 200, rated: 50).fractionRated == 0.25)
    #expect(entry("B", total: 0, rated: 0).fractionRated == 0)
    #expect(entry("C", total: 10, rated: 99).fractionRated == 1)
  }

  @Test("The list survives a relaunch, and a corrupt file is an empty list, never a crash")
  func persistence() throws {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent("firstcut-recents-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = RecentFoldersStore(directory: directory)
    #expect(store.load().isEmpty)

    store.save([entry("A", rated: 3), entry("B")])
    #expect(RecentFoldersStore(directory: directory).load().map(\.name) == ["A", "B"])

    try Data("not json".utf8).write(to: store.fileURL)
    #expect(store.load().isEmpty)
  }
}
