// Owner: pipeline.
//
// The signpost layer. These are small tests on purpose — the layer exists so that a claim in
// todo.md §7.3 can be measured with Instruments rather than a stopwatch, and the only way that stops
// quietly rotting is if its two properties are asserted: the names are the ones the docs name, and an
// interval actually closes.

import Foundation
import Testing

@testable import Firstcut

@Suite("Signposts (todo.md §7.3)")
struct SignpostTests {
  @Test("An interval opens and closes without trapping")
  func anIntervalRoundTrips() {
    // Not interesting on its own. It is here because a signpost that is never closed is an
    // unclosed span in every trace that includes it, and the failure mode of forgetting to call
    // `end()` is silence.
    let interval = SignpostInterval.begin(Signposts.decodeThumbnail)
    interval.end()
  }

  @Test("An interval closed by deinit is not a crash")
  func anUnclosedIntervalDoesNotTrap() {
    // The safety net for a decode path that returns early: a missing end is visible in Instruments,
    // which is survivable, and a trap in a decode callback is not.
    _ = SignpostInterval.begin(Signposts.decodeDisplay)
  }

  @Test("An event emits without an interval")
  func anEventEmits() {
    SignpostInterval.event(Signposts.keyToFrame)
  }

  @Test("Every interval is named after a row of todo.md §7.3, and the names are distinct")
  func theNamesAreTheDocumentsNames() {
    // These strings are what a person greps for in a captured trace after reading the docs. If one
    // is renamed in the docs and not here, or here and not in the docs, the measurement is gone.
    let names = [
      Signposts.openToFirstPhoto, Signposts.keyToFrame, Signposts.batchToFrame,
      Signposts.zoomToSharp, Signposts.decodeThumbnail, Signposts.decodeDisplay,
      Signposts.decodeFromBytes, Signposts.setFocus, Signposts.evictions,
    ].map { $0.description }

    for name in names {
      #expect(!name.isEmpty)
      #expect(
        name.allSatisfy { $0.isLetter || $0.isNumber },
        "\(name) should be a plain identifier, so a trace filter matches it")
    }
    #expect(Set(names).count == names.count, "two intervals share a name: \(names)")

    // The names the docs actually use.
    #expect(names.contains("keyToFrame"))
    #expect(names.contains("decodeDisplay"))
    // The byte-range path must be distinguishable from the container read, or the §7.5 claim that
    // it is cheaper is not measurable.
    // `StaticString` is not `Equatable`, so compare the descriptions — which is also what a trace
    // filter does.
    #expect(Signposts.decodeFromBytes.description != Signposts.decodeDisplay.description)
  }
}
