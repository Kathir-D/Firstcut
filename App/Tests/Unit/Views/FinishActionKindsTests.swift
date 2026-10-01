import Foundation
import Testing

@testable import Firstcut

@Suite("Finish action kinds (shared by the Finish sheet and Settings)")
struct FinishActionKindsTests {
    @Test("A picker tag is found by kind, whatever folder name the value carries")
    func pickerTag() {
        let typed = UnkeptAction.moveToSubfolder("Rejects 2026")
        #expect(typed.pickerTag == UnkeptAction.allCases.first { $0.isSameKind(as: typed) })
        #expect(typed.isSameKind(as: .moveToSubfolder("anything")))
        #expect(!typed.isSameKind(as: .moveToTrash))
        #expect(UnkeptAction.deletePermanently.pickerTag == .deletePermanently)
    }

    @Test("Switching kind keeps the text the user typed, and an empty name gets a sensible default")
    func withFolder() {
        #expect(UnkeptAction.moveToSubfolder("x").with(folder: "Maybe") == .moveToSubfolder("Maybe"))
        #expect(UnkeptAction.moveToSubfolder("x").with(folder: "") == .moveToSubfolder("_Not kept"))
        #expect(UnkeptAction.moveToTrash.with(folder: "ignored") == .moveToTrash)

        #expect(KeptAction.none.with(folder: "ignored") == .none)
        #expect(KeptAction.copyTo("").with(folder: "/Volumes/Out") == .copyTo("/Volumes/Out"))
        #expect(KeptAction.splitByTier("").with(folder: "Sorted") == .splitByTier("Sorted"))
        #expect(KeptAction.writeList("").with(folder: "") == .writeList("kept.txt"))
    }

    @Test("`text` reads back whichever kind holds it")
    func text() {
        #expect(KeptAction.copyTo("/a").text == "/a")
        #expect(KeptAction.writeList("list.txt").text == "list.txt")
        #expect(KeptAction.none.text == "")
    }
}
