// Owner: app-logic.
//
// The keymap: which chord runs which command, in which rating mode, with the user's overrides on
// top of `App/Resources/DefaultKeymap.json` (todo.md §10).
//
// Two things the plain "chord → command" table doesn't cover, both of which the contract needs:
//
// 1. **Mode-specific bindings.** `flag.pick` (P) and `keep.toggle` (P) are the same chord doing
//    different things in Stars and Keep mode (todo.md §6.1, §6.2). A binding can therefore declare
//    the modes it applies to; `nil` means all of them. A mode-specific binding wins over a
//    mode-agnostic one on the same chord.
// 2. **Alternate chords.** Batch navigation has both ⌘←/⌘→ and `[`/`]` (todo.md §10), so several
//    bindings may point at the same command. The first one is what menus display.

import Foundation

public struct KeyBinding: Hashable, Sendable, Identifiable {
    public var command: String
    public var argument: Int?
    public var chord: KeyChord
    /// nil = every rating mode.
    public var modes: Set<RatingMode>?

    public init(command: String, argument: Int? = nil, chord: KeyChord, modes: Set<RatingMode>? = nil) {
        self.command = command
        self.argument = argument
        self.chord = chord
        self.modes = modes
    }

    public init(_ command: Command, chord: KeyChord, modes: Set<RatingMode>? = nil) {
        self.init(
            command: command.id, argument: command.argument, chord: chord, modes: modes)
    }

    /// Resolved command, or nil when the id isn't known to this build (an old keymap file).
    public var resolved: Command? { Command(id: command, argument: argument) }

    public var id: String {
        if let argument { "\(command)(\(argument))" } else { command }
    }

    public func applies(to mode: RatingMode) -> Bool {
        guard let modes else { return true }
        return modes.contains(mode)
    }

    public var appliesToAllModes: Bool { modes == nil }

    enum CodingKeys: String, CodingKey {
        case command, argument, key, modifiers, modes
    }
}

// Flat wire format, exactly the shape the contract documents:
//   { "command": "rate.stars", "argument": 3, "key": "three", "modifiers": [] }
extension KeyBinding: Codable {
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        command = try c.decode(String.self, forKey: .command)
        argument = try c.decodeIfPresent(Int.self, forKey: .argument)
        chord = KeyChord(
            try c.decode(Key.self, forKey: .key),
            KeyModifiers(names: try c.decode([String].self, forKey: .modifiers)))
        modes = try c.decodeIfPresent(Set<RatingMode>.self, forKey: .modes)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(command, forKey: .command)
        try c.encodeIfPresent(argument, forKey: .argument)
        try c.encode(chord.key, forKey: .key)
        try c.encode(chord.modifiers, forKey: .modifiers)
        try c.encodeIfPresent(modes, forKey: .modes)
    }
}

public struct KeymapDocument: Hashable, Sendable, Codable {
    public var version: Int
    public var bindings: [KeyBinding]

    public init(version: Int = 1, bindings: [KeyBinding]) {
        self.version = version
        self.bindings = bindings
    }
}

public struct ChordConflict: Hashable, Sendable, Identifiable {
    public let chord: KeyChord
    public let bindings: [KeyBinding]
    public var id: String { chord.description }

    /// The binding that already owned the chord, i.e. the one a new binding would displace.
    public var incumbent: KeyBinding { bindings[0] }
    public var challenger: KeyBinding { bindings[1] }
}

public struct Keymap: Hashable, Sendable {
    /// Insertion order is menu/editor display order.
    public private(set) var bindings: [KeyBinding]

    private var byChord: [KeyChord: [Int]] = [:]

    public init(bindings: [KeyBinding] = []) {
        self.bindings = bindings
        reindex()
    }

    public static let empty = Keymap()

    private mutating func reindex() {
        var index: [KeyChord: [Int]] = [:]
        for (offset, binding) in bindings.enumerated() {
            index[binding.chord, default: []].append(offset)
        }
        byChord = index
    }

    // MARK: - Lookup

    /// Caps Lock and fn are *state*, not intent: ⇪ then P is still P. The only exception is the
    /// synthesized Caps Lock chord itself, which is a real key press (`autoAdvance.toggle`).
    public static func normalized(_ chord: KeyChord) -> KeyChord {
        guard chord.key != .capsLock, chord.key != .function else { return chord }
        var modifiers = chord.modifiers
        modifiers.remove(.capsLock)
        modifiers.remove(.function)
        return KeyChord(chord.key, modifiers)
    }

    /// The binding a chord runs in `mode`, or nil when unbound. Returns the argument alongside so
    /// `rate.stars` knows which star count to set.
    public func binding(for chord: KeyChord, mode: RatingMode) -> KeyBinding? {
        let candidates = byChord[Keymap.normalized(chord)] ?? []
        // Mode-specific first, then mode-agnostic; within a group, the earliest binding wins.
        return
            candidates
            .map { bindings[$0] }
            .first { $0.modes?.contains(mode) == true }
            ?? candidates
            .map { bindings[$0] }
            .first { $0.applies(to: mode) }
    }

    public func command(for chord: KeyChord, mode: RatingMode) -> Command? {
        binding(for: chord, mode: mode)?.resolved
    }

    /// Every chord bound to a command, in display order.
    public func chords(for command: String, argument: Int? = nil) -> [KeyChord] {
        bindings.filter { $0.command == command && $0.argument == argument }.map(\.chord)
    }

    /// What a menu item shows as its shortcut.
    public func primaryChord(for command: Command, mode: RatingMode) -> KeyChord? {
        bindings.first {
            $0.command == command.id && $0.argument == command.argument && $0.applies(to: mode)
        }?
        .chord
    }

    public func isBound(_ command: Command) -> Bool {
        bindings.contains { $0.command == command.id && $0.argument == command.argument }
    }

    // MARK: - Conflicts

    /// Chords claimed by two different commands that can be live at the same time.
    ///
    /// Only bindings whose mode sets intersect count, so Stars-mode `flag.pick` and Keep-mode
    /// `keep.toggle` on P are not a conflict.
    public func conflicts() -> [ChordConflict] {
        var grouped: [KeyChord: [KeyBinding]] = [:]
        for binding in bindings {
            grouped[binding.chord, default: []].append(binding)
        }
        var result: [ChordConflict] = []
        for (chord, group) in grouped {
            // Two entries for the *same* command on one chord is a duplicate, not a conflict.
            var distinct: [KeyBinding] = []
            for binding in group where !distinct.contains(where: { $0.id == binding.id }) {
                distinct.append(binding)
            }
            guard distinct.count > 1 else { continue }
            for first in 0..<distinct.count {
                for second in (first + 1)..<distinct.count
                where Keymap.modesOverlap(distinct[first].modes, distinct[second].modes) {
                    result.append(ChordConflict(chord: chord, bindings: [distinct[first], distinct[second]]))
                    break
                }
                if result.last?.chord == chord { break }
            }
        }
        return result.sorted { $0.chord.description < $1.chord.description }
    }

    private static func modesOverlap(_ a: Set<RatingMode>?, _ b: Set<RatingMode>?) -> Bool {
        switch (a, b) {
        case (nil, _), (_, nil): true
        case (let a?, let b?): !a.isDisjoint(with: b)
        }
    }

    // MARK: - Editing

    /// Binds `chord` to a command, replacing whatever held it. With `resolveConflict` false the
    /// change is refused and the current conflicts are returned instead — that's what the keymap
    /// editor uses to show a conflict warning instead of silently stealing a shortcut.
    @discardableResult
    public mutating func bind(
        _ chord: KeyChord, to command: Command, mode: RatingMode? = nil,
        resolveConflict: Bool = true
    ) -> [ChordConflict]? {
        let modes: Set<RatingMode>? = mode.map { [$0] }
        if !resolveConflict,
            let existing = binding(for: chord, mode: modes?.first ?? .stars),
            existing.id != commandKey(command)
        {
            return conflicts()
        }
        removeBindings(on: chord, overlapping: modes)
        appendBinding(KeyBinding(command, chord: chord, modes: modes))
        return nil
    }

    /// Gives `chord` a second binding instead of moving it — used for alternates like `[` for
    /// `batch.previous`.
    public mutating func addAlternate(_ chord: KeyChord, to command: Command) {
        appendBinding(KeyBinding(command, chord: chord))
    }

    public mutating func unbind(_ chord: KeyChord) {
        bindings.removeAll { $0.chord == chord }
        reindex()
    }

    public mutating func unbind(_ command: Command) {
        bindings.removeAll { $0.command == command.id && $0.argument == command.argument }
        reindex()
    }

    /// Drops a command's user overrides so the defaults apply again.
    public mutating func reset(_ command: Command, to defaults: Keymap) {
        bindings.removeAll { $0.command == command.id && $0.argument == command.argument }
        let restored = defaults.bindings.filter {
            $0.command == command.id && $0.argument == command.argument
        }
        bindings.insert(contentsOf: restored, at: min(bindings.count, insertionIndex(of: command)))
        reindex()
    }

    public mutating func resetAll(to defaults: Keymap) {
        self = defaults
    }

    private func commandKey(_ command: Command) -> String {
        if let argument = command.argument { "\(command.id)(\(argument))" } else { command.id }
    }

    private func insertionIndex(of command: Command) -> Int {
        let first = bindings.firstIndex { $0.command == command.id }
        return first ?? bindings.count
    }

    private mutating func appendBinding(_ binding: KeyBinding) {
        bindings.append(binding)
        reindex()
    }

    private mutating func removeBindings(on chord: KeyChord, overlapping modes: Set<RatingMode>?) {
        bindings.removeAll { existing in
            existing.chord == chord && Keymap.modesOverlap(existing.modes, modes)
        }
        reindex()
    }

    // MARK: - Layering

    /// The user's keymap layered over the shipped defaults.
    ///
    /// A user entry replaces the default on the same chord, *and* replaces every default chord of
    /// the same command: the keymap editor always writes a command's complete chord set, so
    /// "rebind batch navigation to only `]`" behaves the way a shortcut editor is expected to, and
    /// adding an alternate means writing the existing chords back out too.
    public func layered(over defaults: Keymap) -> Keymap {
        var result = defaults
        let overriddenChords = Set(bindings.map(\.chord))
        let overriddenCommands = Set(bindings.map(\.id))
        result.bindings.removeAll {
            overriddenChords.contains($0.chord) || overriddenCommands.contains($0.id)
        }
        result.bindings.append(contentsOf: bindings)
        result.reindex()
        return result
    }

    // MARK: - Documents

    public var document: KeymapDocument { KeymapDocument(version: 1, bindings: bindings) }

    public func encoded(prettyPrinted: Bool = true) throws -> Data {
        let encoder = JSONEncoder()
        if prettyPrinted {
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        }
        return try encoder.encode(document)
    }

    public init(document: KeymapDocument) {
        self.init(bindings: document.bindings)
    }

    public init(json: Data) throws {
        self.init(document: try JSONDecoder().decode(KeymapDocument.self, from: json))
    }
}
