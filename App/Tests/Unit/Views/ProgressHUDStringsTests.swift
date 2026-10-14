import Foundation
import Testing

@testable import Firstcut

@Suite("Progress HUD strings")
struct ProgressHUDStringsTests {
    @Test("The compact reject-batch button exposes an explicit accessibility label")
    func rejectBatchLabel() {
        // The button is now an icon-only control, so its action is announced via an accessibility
        // label rather than the button's on-screen text.
        #expect(ProgressHUD.rejectBatchAccessibilityLabel == "Reject batch")
    }

    @Test("The compact reject-batch button keeps its tooltip wording")
    func rejectBatchTooltip() {
        let tooltip = ProgressHUD.rejectBatchTooltip
        #expect(tooltip == "Reject every photo in this batch (⇧X). Undo puts them all back.")
    }
}
