// Owner: app-logic.
//
// Physical keys, modifier combinations and the "chord" that the keymap maps to commands
// (docs/contracts/app-model.md, task.md §10).
//
// `Key` raw values are ASCII identifiers so `keymap.json` is readable and diffable; `symbol` is
// what menus and the keymap editor show. `keyCode` is the ANSI (US) hardware code macOS reports in
// `NSEvent.keyCode`, which is what makes this a *position* mapping rather than a layout one — ⌘Z
// stays on the key labelled Z on a QWERTY layout, like every other Mac app.

import AppKit
import Foundation

public enum Key: String, Hashable, Sendable, CaseIterable, Codable {
    case a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x, y, z
    case zero, one, two, three, four, five, six, seven, eight, nine
    case leftArrow, rightArrow, upArrow, downArrow
    case escape, `return`, tab, space, `delete`, forwardDelete
    case home, end, pageUp, pageDown, keypadEnter
    case backtick, minus, equal, leftBracket, rightBracket, backslash
    case semicolon, quote, comma, period, slash
    case capsLock
    case function

    /// ANSI hardware code, or nil for keys that can't come from a keyDown event.
    public var keyCode: UInt16? {
        switch self {
        case .a: 0
        case .s: 1
        case .d: 2
        case .f: 3
        case .h: 4
        case .g: 5
        case .z: 6
        case .x: 7
        case .c: 8
        case .v: 9
        case .b: 11
        case .q: 12
        case .w: 13
        case .e: 14
        case .r: 15
        case .y: 16
        case .t: 17
        case .one: 18
        case .two: 19
        case .three: 20
        case .four: 21
        case .six: 22
        case .five: 23
        case .equal: 24
        case .nine: 25
        case .seven: 26
        case .minus: 27
        case .eight: 28
        case .zero: 29
        case .rightBracket: 30
        case .o: 31
        case .u: 32
        case .leftBracket: 33
        case .i: 34
        case .p: 35
        case .return: 36
        case .l: 37
        case .j: 38
        case .quote: 39
        case .k: 40
        case .semicolon: 41
        case .backslash: 42
        case .comma: 43
        case .slash: 44
        case .n: 45
        case .m: 46
        case .period: 47
        case .tab: 48
        case .space: 49
        case .backtick: 50
        case .delete: 51
        case .escape: 53
        case .capsLock: 57
        case .home: 115
        case .pageUp: 116
        case .forwardDelete: 117
        case .end: 119
        case .pageDown: 121
        case .leftArrow: 123
        case .rightArrow: 124
        case .downArrow: 125
        case .upArrow: 126
        case .function: 63
        case .keypadEnter: 76
        }
    }

    public init?(keyCode: UInt16) {
        guard let match = Key.allCases.first(where: { $0.keyCode == keyCode }) else { return nil }
        self = match
    }

    /// What the menu bar and the keymap editor print.
    public var symbol: String {
        switch self {
        case .leftArrow: "←"
        case .rightArrow: "→"
        case .upArrow: "↑"
        case .downArrow: "↓"
        case .escape: "esc"
        case .return: "↩"
        case .tab: "⇥"
        case .space: "space"
        case .delete: "⌫"
        case .forwardDelete: "⌦"
        case .home: "↖"
        case .end: "↘"
        case .pageUp: "⇞"
        case .pageDown: "⇟"
        case .capsLock: "⇪"
        case .function: "fn"
        case .keypadEnter: "⌤"
        case .backtick: "`"
        default:
            // Letters print as capitals, the way the menu bar does: ⇧⌘Z, not ⇧⌘z.
            rawValue.uppercased()
        }
    }
}

public struct KeyModifiers: OptionSet, Hashable, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }

    public static let shift = KeyModifiers(rawValue: 1 << 0)
    public static let control = KeyModifiers(rawValue: 1 << 1)
    public static let option = KeyModifiers(rawValue: 1 << 2)
    public static let command = KeyModifiers(rawValue: 1 << 3)
    public static let capsLock = KeyModifiers(rawValue: 1 << 4)
    public static let function = KeyModifiers(rawValue: 1 << 5)

    public static let commandShift: KeyModifiers = [.command, .shift]
    public static let controlCommand: KeyModifiers = [.control, .command]

    /// The characters `NSEvent.modifierFlags` carries, in the order the keymap JSON uses.
    public var names: [String] {
        var names: [String] = []
        if contains(.control) { names.append("control") }
        if contains(.option) { names.append("option") }
        if contains(.shift) { names.append("shift") }
        if contains(.command) { names.append("command") }
        if contains(.capsLock) { names.append("capsLock") }
        if contains(.function) { names.append("function") }
        return names
    }

    public init(names: [String]) {
        var value: KeyModifiers = []
        for name in names {
            switch name {
            case "control", "ctrl": value.insert(.control)
            case "option", "alt": value.insert(.option)
            case "shift": value.insert(.shift)
            case "command", "cmd", "meta": value.insert(.command)
            case "capsLock", "caps": value.insert(.capsLock)
            case "function", "fn": value.insert(.function)
            default: break
            }
        }
        self = value
    }

    /// Glyph order follows the Apple menus: ⌃⌥⇧⌘ then the key.
    public var symbols: String {
        var s = ""
        if contains(.control) { s += "⌃" }
        if contains(.option) { s += "⌥" }
        if contains(.shift) { s += "⇧" }
        if contains(.command) { s += "⌘" }
        if contains(.capsLock) { s += "⇪" }
        if contains(.function) { s += "fn" }
        return s
    }

    public var eventModifierFlags: NSEvent.ModifierFlags {
        var flags: NSEvent.ModifierFlags = []
        if contains(.shift) { flags.insert(.shift) }
        if contains(.control) { flags.insert(.control) }
        if contains(.option) { flags.insert(.option) }
        if contains(.command) { flags.insert(.command) }
        if contains(.capsLock) { flags.insert(.capsLock) }
        if contains(.function) { flags.insert(.function) }
        return flags
    }

    public init(eventModifierFlags flags: NSEvent.ModifierFlags) {
        var value: KeyModifiers = []
        if flags.contains(.shift) { value.insert(.shift) }
        if flags.contains(.control) { value.insert(.control) }
        if flags.contains(.option) { value.insert(.option) }
        if flags.contains(.command) { value.insert(.command) }
        if flags.contains(.capsLock) { value.insert(.capsLock) }
        if flags.contains(.function) { value.insert(.function) }
        self = value
    }
}

extension KeyModifiers: Codable {
    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        self.init(names: try container.decode([String].self))
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(names)
    }
}

/// A key plus modifiers. Caps Lock is a *modifier* bit here as well as a key, so a binding can match
/// "P with Caps Lock held" and "Caps Lock on its own" without a second concept.
public struct KeyChord: Hashable, Sendable, Codable, CustomStringConvertible {
    public var key: Key
    public var modifiers: KeyModifiers

    public init(_ key: Key, _ modifiers: KeyModifiers = []) {
        self.key = key
        self.modifiers = modifiers
    }

    public var description: String { modifiers.symbols + key.symbol }

    /// The compact form used by tests, clipboard copies and imported keymaps: "⇧⌘Z", "⌘←", "⇪".
    public init?(compact: String) {
        // ⇪ is both the Caps Lock key and the Caps Lock modifier, so a bare one means the key.
        if compact == Key.capsLock.symbol { self.init(.capsLock, [.capsLock]); return }
        var modifiers: KeyModifiers = []
        var rest = Substring(compact)
        // Longest symbols first so ⌘ isn't confused with ⌃ and "fn" isn't read as "f".
        for (symbol, flag) in [
            ("⌃", KeyModifiers.control), ("⌥", KeyModifiers.option),
            ("⇧", KeyModifiers.shift), ("⌘", KeyModifiers.command), ("⇪", KeyModifiers.capsLock),
            ("fn", KeyModifiers.function),
        ] {
            while rest.hasPrefix(symbol) {
                modifiers.insert(flag)
                rest = rest.dropFirst(symbol.count)
            }
        }
        let name = String(rest)
        guard let key = Key.allCases.first(where: { $0.rawValue == name || $0.symbol == name }),
            !name.isEmpty
        else { return nil }
        self.init(key, modifiers)
    }

    public var compact: String { description }

    /// The chord a key press means. Used by the router and by the keymap editor's recorder
    /// (Settings → Keyboard), so "record a new key" is exactly what a press produces.
    public init?(event: NSEvent) {
        switch event.type {
        case .keyDown, .keyUp:
            guard let key = Key(keyCode: event.keyCode) else { return nil }
            self.init(key, KeyModifiers(eventModifierFlags: event.modifierFlags))
        case .flagsChanged:
            // Caps Lock and fn are toggles: only the transition to "on" is a press.
            guard event.modifierFlags.contains(.capsLock) else { return nil }
            self.init(.capsLock, [.capsLock])
        default:
            return nil
        }
    }
}
