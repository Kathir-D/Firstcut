// Owner: settings.
//
// The settings model had no tests at all, which is why a divide-by-zero in `formatShutter` sat in
// the 344 lines that draw every number in the info panel: `Int(infinity)` traps, and a CR3 whose
// ExposureTime numerator is zero is a legal file.
//
// These are the ones with teeth — a value in, a string or a value out, with the boundaries the
// arithmetic actually has.

import Foundation
import Testing

@testable import Firstcut

@Suite("Settings: info panel formatting")
struct InfoFieldFormattingTests {

    /// The crash. `seconds == 0` evaluated `1 / 0` and then `Int(inf)`, which is a trap, not a NaN —
    /// Swift's `Double`→`Int` conversion has no infinity case and the process dies. The core reaches
    /// it because it only rejects a zero *denominator*, so a numerator of zero becomes `Some(0.0)`.
    @Test("A zero exposure time does not trap the info panel")
    func zeroExposureTimeIsSafe() {
        #expect(InfoField.formatShutter(0) == "—")
        #expect(InfoField.formatShutter(-1) == "—", "and neither does a nonsense one")
        #expect(InfoField.formatShutter(.nan) == "—")
        #expect(InfoField.formatShutter(.infinity) == "—")
    }

    /// The other side of the boundary, because the fix could have been "always return —".
    @Test("Shutter speeds are the ones a photographer reads")
    func shutterSpeedsAreReadable() {
        #expect(InfoField.formatShutter(1.3) == "1.3 s")
        #expect(InfoField.formatShutter(1.0) == "1.0 s")
        #expect(InfoField.formatShutter(0.5) == "1/2 s")
        #expect(InfoField.formatShutter(1.0 / 2000) == "1/2000 s")
        // The rounding is `.rounded()`, so a shutter just over 1/60 reads as 1/60 and not 1/59.
        #expect(InfoField.formatShutter(1.0 / 61) == "1/61 s")
    }
}

@Suite("Settings: the keep mapping round trip")
struct KeepMappingTests {

    /// `KeepMapping` has a hand-written `Codable`, and `stars5` exists for one reason only: so a
    /// settings file written under the old rule still reads. Nothing asserted that it did, so the
    /// one migration the decoder carries could have rotted unnoticed — and a `DecodingError` here
    /// does not fail loudly, it resets *every* metadata setting the user had.
    @Test("Every keep mapping survives a save and load")
    func keepMappingSurvivesARoundTrip() throws {
        let cases: [KeepMapping] = [
            .stars1, .stars5, .rating(3),
            .colorLabel(.red), .colorLabel(.yellow), .colorLabel(.green),
        ]
        for mapping in cases {
            let directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("FirstcutSettings-\(UUID().uuidString)", isDirectory: true)
            defer { try? FileManager.default.removeItem(at: directory) }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

            let store = SettingsStore(directory: directory)
            var settings = AppSettings()
            settings.metadata.keepMapping = mapping
            try store.save(settings)

            #expect(
                store.load().metadata.keepMapping == mapping,
                "\(mapping) did not survive the round trip")
        }
    }

    /// And an unknown value must not take the whole file down with it.
    @Test("An unrecognised keep mapping falls back rather than throwing")
    func anUnknownKeepMappingFallsBack() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutSettings-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try #"{ "general": {}, "metadata": { "keepMapping": { "type": "nonsense" } } }"#
            .write(
                to: directory.appendingPathComponent("settings.json"), atomically: true,
                encoding: .utf8)

        let loaded = SettingsStore(directory: directory).load()
        #expect(
            loaded.metadata.keepMapping == .stars1,
            "a bad value must not reset every other setting the user had")
    }

    /// The Settings → Metadata picker reads `stars == 4 ? .fourStars : .fiveStars`, so with the
    /// model's default of one star the control *displays* "5 stars" and the first touch writes 5 —
    /// silently reverting the decision that a keep is one star, and recreating the collision with a
    /// 5-star rating that `keepDisplayStars` exists to prevent.
    @Test("The model default is one star, not five")
    func theModelDefaultIsOneStar() {
        #expect(AppSettings().metadata.keepMapping == .stars1)
    }
}

@Suite("Settings: the folder name default")
struct FinishFolderDefaultTests {

    /// `.with(folder: "")` substitutes `_Not kept`, which is the whole point of that method. The
    /// Settings → General binding assigned `moveToSubfolder(text)` directly and so bypassed it: pick
    /// "Move to a subfolder", leave the field empty, and the Finish sheet then blocks on an empty
    /// required field instead of using the default.
    @Test("Clearing the subfolder name falls back to the default")
    func clearingTheSubfolderNameFallsBack() {
        #expect(UnkeptAction.moveToSubfolder("Rejects").with(folder: "") == .moveToSubfolder("_Not kept"))
        #expect(UnkeptAction.moveToSubfolder("Rejects").with(folder: "Maybe") == .moveToSubfolder("Maybe"))
        // The kept side substitutes per action: a list gets a filename, a plain copy or move does not.
        #expect(KeptAction.writeList("kept.txt").with(folder: "") == .writeList("kept.txt"))
        #expect(KeptAction.splitByStars("By Stars").with(folder: "") == .splitByStars(""))
    }
}
