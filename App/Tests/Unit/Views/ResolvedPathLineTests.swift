import Foundation
import Testing

@testable import Firstcut

@Suite("Resolved path line")
struct ResolvedPathLineTests {
    private let base = URL(fileURLWithPath: "/Volumes/Shoot/Game1", isDirectory: true)

    @Test("A relative name is taken from the shoot folder")
    func relativeUsesTheShootFolder() {
        #expect(ResolvedPathLine.resolved("Rejects", base: base) == "/Volumes/Shoot/Game1/Rejects")
    }

    @Test("An absolute path is shown as typed")
    func absoluteIsKept() {
        #expect(ResolvedPathLine.resolved("/Users/me/Out", base: base) == "/Users/me/Out")
    }

    @Test("Empty text or a relative name with no shoot folder shows nothing")
    func nothingToShow() {
        #expect(ResolvedPathLine.resolved("  ", base: base) == nil)
        #expect(ResolvedPathLine.resolved("Rejects", base: nil) == nil)
    }
}
