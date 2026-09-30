// Owner: app-logic.
//
// The single key router (todo.md §10): one `NSEvent` local monitor for the whole app, so no view
// can swallow a shortcut. Keys, menus, toolbar buttons and gestures all end up in the same
// `AppModel.perform(_:)`, which is why the UI is optional for every test.
//
// Three behaviours worth calling out, all required by the spec:
//
// * **Smooth key repeat.** A held → must fire `photo.next` on *every* repeat event; nothing is
//   coalesced or deferred, so the frame changes at key-repeat rate with no dropped events. Repeat is
//   gated per command (`Command.isRepeatable`) so ⌘↩ can't open the Finish sheet thirty times.
// * **Caps Lock.** macOS reports Caps Lock as a modifier change, not a key press. The router
//   synthesizes one chord on the off→on transition, which is Lightroom's auto-advance toggle, and
//   ignores the release.
// * **Text input wins.** While a text field is focused (the keymap recorder, the Finish folder name)
//   every key is passed through untouched.

import AppKit
import Foundation

@MainActor public protocol KeyRouterSource: AnyObject {
    var keymap: Keymap { get }
    var ratingMode: RatingMode { get }
    /// True while a text input owns the keyboard.
    var isTextEditing: Bool { get }
    func perform(_ command: Command)
}

@MainActor public final class KeyRouter {
    public weak var source: (any KeyRouterSource)?

    /// Whether keys should be routed right now. The app narrows this to "the culling window is the
    /// key window and no text field is being typed in", so a key pressed in a sheet, the Settings
    /// window or a text field is never also read as a rating. Defaults to always, which is what the
    /// tests drive.
    public var isActive: () -> Bool = { true }

    private var monitor: Any?
    private var capsLockOn = false

    public init(source: any KeyRouterSource) {
        self.source = source
    }

    // The app owns the router for the process lifetime, so there is no deinit: `NSEvent.removeMonitor`
    // is main-actor only and a deinit can't be isolated. `stop()` is the shutdown path.

    // MARK: - Installing

    public func start() {
        guard monitor == nil else { return }
        // `NSEvent` is main-actor isolated, so the block is main-actor too: no hop is needed, and
        // returning the event consumes it while nil lets it continue to the responder chain.
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) {
            [weak self] event in
            self?.handle(event) == true ? event : nil
        }
    }

    public func stop() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
    }

    /// Installs the monitor for the lifetime of the object. The app entry point (ui) calls this.
    public func install() {
        start()
    }

    // MARK: - Routing

    /// - Returns: true when the event was consumed.
    @discardableResult
    public func handle(_ event: NSEvent) -> Bool {
        guard let source, !source.isTextEditing, isActive() else { return false }

        switch event.type {
        case .flagsChanged:
            return handleFlagsChanged(event, source: source)
        case .keyDown:
            return handleKeyDown(event, source: source)
        default:
            return false
        }
    }

    private func handleFlagsChanged(_ event: NSEvent, source: any KeyRouterSource) -> Bool {
        let on = event.modifierFlags.contains(.capsLock)
        defer { capsLockOn = on }
        guard on, !capsLockOn else { return false }  // fire on the transition only
        return dispatch(KeyChord(.capsLock, [.capsLock]), isRepeat: false, source: source)
    }

    private func handleKeyDown(_ event: NSEvent, source: any KeyRouterSource) -> Bool {
        guard let key = Key(keyCode: event.keyCode) else { return false }
        let chord = KeyChord(key, KeyRouter.modifiers(from: event.modifierFlags))
        return dispatch(chord, isRepeat: event.isARepeat, source: source)
    }

    @discardableResult
    private func dispatch(_ chord: KeyChord, isRepeat: Bool, source: any KeyRouterSource) -> Bool {
        guard let binding = source.keymap.binding(for: chord, mode: source.ratingMode) else {
            return false
        }
        guard let command = binding.resolved else { return false }
        if isRepeat, !command.isRepeatable { return true }  // swallowed: the first press did the work
        source.perform(command)
        return true
    }

    public static func modifiers(from flags: NSEvent.ModifierFlags) -> KeyModifiers {
        var modifiers: KeyModifiers = []
        if flags.contains(.shift) { modifiers.insert(.shift) }
        if flags.contains(.control) { modifiers.insert(.control) }
        if flags.contains(.option) { modifiers.insert(.option) }
        if flags.contains(.command) { modifiers.insert(.command) }
        if flags.contains(.capsLock) { modifiers.insert(.capsLock) }
        if flags.contains(.function) { modifiers.insert(.function) }
        return modifiers
    }
}
