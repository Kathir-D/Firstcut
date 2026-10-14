import Foundation
import Testing

@testable import Firstcut

@Suite("PathInput: typed folder paths")
struct PathInputTests {
    @Test("expand trims and resolves a leading ~")
    func expand() {
        #expect(PathInput.expand("~/x").hasPrefix(NSHomeDirectory()))
        #expect(PathInput.expand("~").hasPrefix(NSHomeDirectory()))
        #expect(PathInput.expand("\n  /some/path  \n").contains("/some/path"))
        #expect(PathInput.expand("   ") == "")
    }

    @Test("check classifies a folder, a file, a missing path, and empty text")
    func check() {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("PathInput-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }

        let check = PathInput.check(folder.path)
        #expect(check == .folder(URL(fileURLWithPath: folder.path, isDirectory: true)))

        let file = folder.appendingPathComponent("note.txt")
        try? FileManager.default.createFile(atPath: file.path, contents: nil)
        #expect(PathInput.check(file.path) == .notAFolder)

        #expect(PathInput.check("/no/such/path-\(UUID().uuidString)") == .notFound)
        #expect(PathInput.check("   ") == .empty)
    }

    @Test("message returns the right strings, or nil when the check is fine")
    func message() {
        #expect(PathInput.message(for: .notFound) == "No folder at that path.")
        #expect(PathInput.message(for: .notAFolder) == "That is a file, not a folder.")
        #expect(PathInput.message(for: .empty) == nil)
        #expect(PathInput.message(for: .folder(URL(fileURLWithPath: "/"))) == nil)
    }

    @Test("suggestions list matching folders under the parent, sorted, excluding files")
    func suggestions() {
        let temp = FileManager.default.temporaryDirectory
            .appendingPathComponent("PathInput-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: temp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temp) }

        for name in ["Alpha", "Albatross", "Beta"] {
            try? FileManager.default.createDirectory(
                at: temp.appendingPathComponent(name, isDirectory: true),
                withIntermediateDirectories: true
            )
        }
        try? FileManager.default.createFile(
            atPath: temp.appendingPathComponent("note.txt").path,
            contents: nil
        )

        let expected = ["\(temp.path)/Alpha", "\(temp.path)/Albatross"].sorted()
        let actual = PathInput.suggestions(for: "\(temp.path)/Al")
        #expect(actual == expected)
        #expect(!actual.contains(where: { $0.hasSuffix("/Beta") }))
        #expect(!actual.contains(where: { $0.hasSuffix("/note.txt") }))
    }
}
