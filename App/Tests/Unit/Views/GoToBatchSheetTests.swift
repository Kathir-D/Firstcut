import Testing

@testable import Firstcut

@MainActor
@Suite("Go to Batch sheet")
struct GoToBatchSheetTests {
    @Test("A typed number is read, spaces ignored")
    func numberParsing() {
        #expect(GoToBatchSheet.number(from: " 12 ") == 12)
        #expect(GoToBatchSheet.number(from: "abc") == nil)
        #expect(GoToBatchSheet.number(from: "") == nil)
    }

    @Test("Messages: none for a real batch, a hint otherwise")
    func messages() {
        #expect(GoToBatchSheet.message(for: "3", batchCount: 5) == nil)
        #expect(GoToBatchSheet.message(for: "6", batchCount: 5) == "There are 5 batches.")
        #expect(GoToBatchSheet.message(for: "0", batchCount: 5) == "There are 5 batches.")
        #expect(GoToBatchSheet.message(for: "x", batchCount: 5) == "Type a batch number.")
    }
}
