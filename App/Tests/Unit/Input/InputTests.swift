// Owner: app-logic.
import AppKit
import Foundation
import Testing

@testable import Firstcut

// MARK: - Command

@Suite("Command")
struct CommandTests {
    @Test("Every catalog entry round-trips through its id and argument")
    func catalogRoundTrips() {
        for entry in CommandCatalog.all {
            let command = entry.command
            let rebuilt = Command(id: command.id, argument: command.argument)
            #expect(rebuilt == command, "\(command.id) did not survive a round trip")
        }
    }

    @Test("Catalog ids are unique per command+argument")
    func catalogIdsUnique() {
        var seen: Set<String> = []
        for entry in CommandCatalog.all {
            #expect(seen.insert(entry.id).inserted, "duplicate catalog id \(entry.id)")
        }
    }

    @Test("An unknown id is rejected instead of crashing")
    func unknownId() {
        #expect(Command(id: "nope.nothing") == nil)
    }

    @Test("Arguments are clamped to their legal range")
    func argumentClamping() {
        #expect(Command(id: "rate.stars", argument: 99) == .setStars(5))
        #expect(Command(id: "rate.stars", argument: -3) == .setStars(0))
        #expect(Command(id: "rate.starsAndAdvance", argument: nil) == .setStarsAndAdvance(1))
        #expect(Command(id: "view.compare", argument: 9) == .showCompare(4))
        #expect(Command(id: "label", argument: 6) == .setLabel(.red))
        #expect(Command(id: "label", argument: 2) == .setLabel(nil))  // not one of 6…9
    }

    @Test("Zoom has no default key, by design (task.md §2, §9.2, §10)")
    func zoomHasNoDefaultKey() throws {
        var store = KeymapStore(directory: temporaryDirectory())
        _ = try store.load()
        for command in [Command.toggleZoomLock, .toggleZoom(at: nil), .magnify(by: 1.1, at: nil)] {
            #expect(store.defaults.isBound(command) == false, "\(command.id) must not be bound by default")
        }
    }

    @Test("Only navigation and rating repeat when a key is held")
    func repeatability() {
        #expect(Command.photoNext.isRepeatable)
        #expect(Command.setStars(3).isRepeatable)
        #expect(Command.toggleKeep.isRepeatable)
        #expect(Command.finishCull.isRepeatable == false)
        #expect(Command.undo.isRepeatable == false)
        #expect(Command.openFolder.isRepeatable == false)
        #expect(Command.toggleFullScreen.isRepeatable == false)
    }

    @Test("Rating commands are the ones that change a photo")
    func ratingCommands() {
        #expect(Command.setStars(1).isRatingChange)
        #expect(Command.togglePickFlag.isRatingChange)
        #expect(Command.setLabel(.red).isRatingChange)
        #expect(Command.photoNext.isRatingChange == false)
        #expect(Command.undo.isRatingChange == false)
    }
}

// MARK: - Chords

@Suite("KeyChord")
struct KeyChordTests {
    @Test("Every key maps to a unique ANSI key code and back")
    func keyCodes() throws {
        for key in Key.allCases {
            let code = try #require(key.keyCode, "\(key.rawValue) has no key code")
            #expect(Key(keyCode: code) == key)
        }
    }

    @Test("Letters sit on their QWERTY positions")
    func ansiLetters() {
        #expect(Key.a.keyCode == 0)
        #expect(Key.z.keyCode == 6)
        #expect(Key.leftArrow.keyCode == 123)
        #expect(Key.keypadEnter.keyCode == 76)
    }

    @Test("Compact form parses the way menus print it")
    func compactParsing() {
        #expect(KeyChord(compact: "⇧⌘Z") == KeyChord(.z, [.shift, .command]))
        #expect(KeyChord(compact: "⌘←") == KeyChord(.leftArrow, [.command]))
        #expect(KeyChord(compact: "P") == KeyChord(.p))
        #expect(KeyChord(compact: "⇪") == KeyChord(.capsLock, [.capsLock]))
        #expect(KeyChord(compact: "`") == KeyChord(.backtick))
        #expect(KeyChord(compact: "nonsense") == nil)
    }

    @Test("Modifier glyphs follow the Apple menu order")
    func symbols() {
        #expect(KeyChord(.z, [.control, .option, .shift, .command]).description == "⌃⌥⇧⌘Z")
        #expect(KeyChord(.return, [.command]).description == "⌘↩")
    }

    @Test("Modifier sets survive JSON as names")
    func modifierCoding() throws {
        let chord = KeyChord(.leftArrow, [.command, .shift])
        let data = try JSONEncoder().encode(chord)
        let text = String(decoding: data, as: UTF8.self)
        #expect(text.contains("\"command\""))
        #expect(text.contains("\"shift\""))
        #expect(try JSONDecoder().decode(KeyChord.self, from: data) == chord)
    }

    @Test("A key press becomes the chord it means")
    func fromEvent() throws {
        let event = try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [.shift], timestamp: 0, windowNumber: 0,
                context: nil, characters: "P", charactersIgnoringModifiers: "p", isARepeat: false, keyCode: 35))
        #expect(KeyChord(event: event) == KeyChord(.p, [.shift]))
    }

    @Test("A Caps Lock flags change reads as the caps chord")
    func fromFlagsEvent() throws {
        let event = try #require(
            NSEvent.keyEvent(
                with: .flagsChanged, location: .zero, modifierFlags: [.capsLock], timestamp: 0, windowNumber: 0,
                context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 57))
        #expect(KeyChord(event: event) == KeyChord(.capsLock, [.capsLock]))
    }
}

// MARK: - Keymap

@Suite("Keymap")
struct KeymapTests {
    static func defaults() throws -> Keymap {
        var store = KeymapStore(directory: temporaryDirectory())
        _ = try store.load()
        return store.defaults
    }

    @Test("The shipped keymap is the Lightroom Classic table (task.md §10)")
    func shippedDefaults() throws {
        let keymap = try Self.defaults()
        let expected: [Command: KeyChord] = [
            .photoPrevious: KeyChord(.leftArrow),
            .photoNext: KeyChord(.rightArrow),
            .batchPrevious: KeyChord(.leftArrow, [.command]),
            .batchNext: KeyChord(.rightArrow, [.command]),
            .setStars(0): KeyChord(.zero),
            .setStars(1): KeyChord(.one),
            .setStars(5): KeyChord(.five),
            .setStarsAndAdvance(1): KeyChord(.one, [.shift]),
            .setStarsAndAdvance(5): KeyChord(.five, [.shift]),
            .rejectFlag: KeyChord(.x),
            .unflag: KeyChord(.u),
            .toggleFlag: KeyChord(.backtick),
            .setLabel(.red): KeyChord(.six),
            .setLabel(.blue): KeyChord(.nine),
            .toggleAutoAdvance: KeyChord(.capsLock, [.capsLock]),
            .showLoupe: KeyChord(.e),
            .showGrid: KeyChord(.g),
            .showCompare(2): KeyChord(.c),
            .toggleInfoPanel: KeyChord(.i),
            .toggleClippingOverlay: KeyChord(.j),
            .toggleAFOverlay: KeyChord(.a),
            .toggleHUD: KeyChord(.h),
            .undo: KeyChord(.z, [.command]),
            .redo: KeyChord(.z, [.command, .shift]),
            .openFolder: KeyChord(.o, [.command]),
            .finishCull: KeyChord(.return, [.command]),
            .toggleFullScreen: KeyChord(.f, [.control, .command]),
        ]
        for (command, chord) in expected {
            #expect(
                keymap.primaryChord(for: command, mode: .stars) == chord,
                "\(command.id) is not bound to \(chord) by default")
        }
    }

    @Test("Batch navigation also answers to [ and ]")
    func alternateBatchChords() throws {
        let keymap = try Self.defaults()
        #expect(keymap.chords(for: Command.batchNext.id).contains(KeyChord(.rightBracket)))
        #expect(keymap.chords(for: Command.batchPrevious.id).contains(KeyChord(.leftBracket)))
    }

    @Test("P is the pick flag in Stars mode and the keep toggle in Keep mode (task.md §6)")
    func modeSpecificBinding() throws {
        let keymap = try Self.defaults()
        let p = KeyChord(.p)
        #expect(keymap.command(for: p, mode: .stars) == .togglePickFlag)
        #expect(keymap.command(for: p, mode: .keep) == .toggleKeep)
    }

    @Test("A binding that only exists in one mode is a conflict only within that mode")
    func modeScopedConflicts() throws {
        let keymap = try Self.defaults()
        // P is deliberately bound in both modes, but the two bindings never overlap.
        let pConflicts = keymap.conflicts().filter { $0.chord == KeyChord(.p) }
        #expect(pConflicts.isEmpty)
    }

    @Test("A chord claimed twice in the same mode is reported")
    func conflictDetection() {
        var keymap = Keymap()
        // Two entries on one chord, as a hand-edited user keymap can easily produce.
        keymap.addAlternate(KeyChord(.k), to: .setStars(4))
        keymap.addAlternate(KeyChord(.k), to: .toggleHUD)
        let conflicts = keymap.conflicts()
        #expect(conflicts.count == 1)
        #expect(conflicts.first?.chord == KeyChord(.k))
        #expect(conflicts.first?.challenger.id == "hud.toggle")
    }

    @Test("Refusing a conflicting rebind leaves the map untouched")
    func refuseConflict() {
        var keymap = Keymap()
        keymap.bind(KeyChord(.k), to: .setStars(4))
        let result = keymap.bind(KeyChord(.k), to: .toggleHUD, resolveConflict: false)
        #expect(result != nil)
        #expect(keymap.command(for: KeyChord(.k), mode: .stars) == .setStars(4))
    }

    @Test("Accepting a conflicting rebind moves the chord")
    func acceptConflict() {
        var keymap = Keymap()
        keymap.bind(KeyChord(.k), to: .setStars(4))
        keymap.bind(KeyChord(.k), to: .toggleHUD)
        #expect(keymap.command(for: KeyChord(.k), mode: .stars) == .toggleHUD)
        #expect(keymap.conflicts().isEmpty)
    }

    @Test("A user override replaces the default chord and the default's whole command set")
    func layering() throws {
        let base = try Self.defaults()
        var overrides = Keymap()
        overrides.bind(KeyChord(.rightBracket), to: .photoNext)
        let layered = overrides.layered(over: base)
        #expect(layered.command(for: KeyChord(.rightArrow), mode: .stars) == nil)
        #expect(layered.command(for: KeyChord(.rightBracket), mode: .stars) == .photoNext)
        // Commands the user never mentioned keep their defaults.
        #expect(layered.command(for: KeyChord(.leftArrow), mode: .stars) == .photoPrevious)
    }

    @Test("Resetting a command brings its default back")
    func resetCommand() throws {
        let base = try Self.defaults()
        var overrides = Keymap()
        overrides.bind(KeyChord(.rightBracket), to: .photoNext)
        overrides.reset(.photoNext, to: base)
        #expect(overrides.primaryChord(for: .photoNext, mode: .stars) == KeyChord(.rightArrow))
    }

    @Test("Caps Lock held while pressing a key still presses that key")
    func capsLockIsNotIntent() throws {
        let keymap = try Self.defaults()
        #expect(keymap.command(for: KeyChord(.p, [.capsLock]), mode: .stars) == .togglePickFlag)
    }

    @Test("A keymap survives a JSON round trip")
    func jsonRoundTrip() throws {
        let original = try Self.defaults()
        let data = try original.encoded()
        let restored = try Keymap(json: data)
        #expect(restored == original)
    }

    @Test("Every binding in the shipped keymap names a command this build knows")
    func noDanglingBindings() throws {
        let keymap = try Self.defaults()
        for binding in keymap.bindings {
            #expect(binding.resolved != nil, "\(binding.id) is not a known command")
        }
    }

    @Test("A user keymap file is written, read back and layered")
    func storeRoundTrip() throws {
        let directory = temporaryDirectory()
        var store = KeymapStore(directory: directory)
        _ = try store.load()
        var overrides = Keymap()
        overrides.bind(KeyChord(.j, [.command]), to: .openFolder)
        store.setOverrides(overrides)
        try store.save()

        var reloaded = KeymapStore(directory: directory)
        let hadOverrides = try reloaded.load()
        #expect(hadOverrides)
        #expect(reloaded.effective.command(for: KeyChord(.j, [.command]), mode: .stars) == .openFolder)
    }

    @Test("Export produces something importable")
    func exportImport() throws {
        let directory = temporaryDirectory()
        var store = KeymapStore(directory: directory)
        _ = try store.load()
        let exported = directory.appendingPathComponent("export.json")
        try store.exportKeymap(to: exported)

        var other = KeymapStore(directory: directory.appendingPathComponent("imported"))
        _ = try other.load()
        try other.importKeymap(from: exported)
        #expect(other.effective.bindings.count == store.effective.bindings.count)
    }

    @Test("A corrupt user keymap falls back to the defaults instead of failing to launch")
    func corruptUserKeymap() throws {
        let directory = temporaryDirectory()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("{ not json".utf8).write(
            to: directory.appendingPathComponent(KeymapStore.userFileName))
        var store = KeymapStore(directory: directory)
        let hadOverrides = try store.load()
        #expect(hadOverrides == false)
        #expect(store.effective.bindings.isEmpty == false)
    }
}

// MARK: - Router

@MainActor
@Suite("KeyRouter")
struct KeyRouterTests {
    /// Records what the router asks for, so routing can be asserted without a model.
    @MainActor
    final class Spy: KeyRouterSource {
        var keymap: Keymap
        var ratingMode: RatingMode = .stars
        var isTextEditing = false
        var received: [(Command, Int?)] = []

        init(keymap: Keymap) { self.keymap = keymap }

        func perform(_ command: Command) {
            received.append((command, command.argument))
        }
    }

    static func keymap() throws -> Keymap {
        var store = KeymapStore(directory: temporaryDirectory())
        _ = try store.load()
        return store.effective
    }

    static func event(_ key: Key, _ modifiers: NSEvent.ModifierFlags = [], isRepeat: Bool = false) throws -> NSEvent {
        try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0, windowNumber: 0,
                context: nil, characters: key.symbol, charactersIgnoringModifiers: key.symbol.lowercased(),
                isARepeat: isRepeat, keyCode: key.keyCode!))
    }

    static func flagsEvent(_ flags: NSEvent.ModifierFlags) throws -> NSEvent {
        try #require(
            NSEvent.keyEvent(
                with: .flagsChanged, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: 0,
                context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false,
                keyCode: Key.capsLock.keyCode!))
    }

    @Test("→ runs photo.next")
    func arrowKey() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.rightArrow)))
        #expect(spy.received.map(\.0) == [.photoNext])
    }

    @Test("⌘→ runs batch.next, and [ does too")
    func batchKeys() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.rightArrow, [.command])))
        #expect(router.handle(try Self.event(.rightBracket)))
        #expect(spy.received.map(\.0) == [.batchNext, .batchNext])
    }

    @Test("3 carries the argument through as rate.stars(3)")
    func argumentRouting() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.three)))
        #expect(spy.received.first?.0 == .setStars(3))
        #expect(spy.received.first?.1 == 3)
    }

    @Test("P follows the rating mode")
    func modeAwareRouting() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.p)))
        spy.ratingMode = .keep
        #expect(router.handle(try Self.event(.p)))
        #expect(spy.received.map(\.0) == [.togglePickFlag, .toggleKeep])
    }

    @Test("A held arrow fires on every repeat, for the photo and batch commands")
    func smoothRepeat() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        for _ in 0..<5 { #expect(router.handle(try Self.event(.rightArrow, isRepeat: true))) }
        #expect(spy.received.count == 5)
    }

    @Test("Holding ⌘↩ fires Finish Cull once")
    func repeatDoesNotDuplicate() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.return, [.command])))
        #expect(router.handle(try Self.event(.return, [.command], isRepeat: true)))
        #expect(spy.received.map(\.0) == [.finishCull])
    }

    @Test("Caps Lock toggles auto-advance on the way down, not on release")
    func capsLock() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.flagsEvent([])) == false)  // already off
        #expect(router.handle(try Self.flagsEvent([.capsLock])))  // press
        #expect(router.handle(try Self.flagsEvent([.capsLock])) == false)  // held
        #expect(router.handle(try Self.flagsEvent([])) == false)  // release
        #expect(spy.received.map(\.0) == [.toggleAutoAdvance])
    }

    @Test("Keys pass through while a text field has focus")
    func textEditingWins() throws {
        let spy = Spy(keymap: try Self.keymap())
        spy.isTextEditing = true
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.rightArrow)) == false)
        #expect(spy.received.isEmpty)
    }

    @Test("An unbound key is not swallowed")
    func unboundKey() throws {
        let spy = Spy(keymap: try Self.keymap())
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.k)) == false)
    }

    @Test("A remapped chord routes to its new command")
    func remapped() throws {
        var keymap = try Self.keymap()
        keymap.bind(KeyChord(.k), to: .toggleInfoPanel)
        let spy = Spy(keymap: keymap)
        let router = KeyRouter(source: spy)
        #expect(router.handle(try Self.event(.k)))
        #expect(spy.received.map(\.0) == [.toggleInfoPanel])
    }
}

// MARK: - Helpers

/// A unique scratch directory per test, so nothing touches the real Application Support.
func temporaryDirectory(function: String = #function) -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("FirstcutTest-\(abs(function.hashValue))", isDirectory: true)
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}
