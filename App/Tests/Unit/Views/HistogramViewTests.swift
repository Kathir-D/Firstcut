// Owner: ui.

import Testing

@testable import Firstcut

@Suite("HistogramView")
@MainActor
struct HistogramViewTests {
    @Test("sharedPeak takes the maximum value across all channels")
    func sharedPeak() {
        #expect(HistogramView.sharedPeak([[0.1, 0.5], [0.2, 0.9]]) == 0.9)
        #expect(HistogramView.sharedPeak([[], []]) == 0.0001)
    }

    @Test("modeTitle labels the two modes")
    func modeTitle() {
        #expect(HistogramView.modeTitle(showsRGB: false) == "Brightness")
        #expect(HistogramView.modeTitle(showsRGB: true) == "RGB")
    }
}
