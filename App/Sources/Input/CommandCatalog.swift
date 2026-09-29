// Owner: app-logic.
//
// The catalog every UI surface enumerates: the menus (task.md §9.1), the keymap editor
// (Settings → Keyboard) and the conflict report. Keeping it next to `Command` is what guarantees a
// command can't exist without a title, a menu home and a defined default key.
//
// Titles are localizable from day one even though v0.1 ships English only (task.md §1 non-goals).

import Foundation

public enum CommandMenu: String, Hashable, Sendable, CaseIterable, Codable {
    case photo
    case view
    case file
    case edit

    public var title: String { CatalogText.localized(rawValue.capitalized) }
}

public struct CommandCatalogEntry: Hashable, Sendable, Identifiable {
    public let command: Command
    public let menu: CommandMenu
    public let title: String

    public var id: String {
        if let argument = command.argument { "\(command.id)(\(argument))" } else { command.id }
    }

    public var commandID: String { command.id }
    public var argument: Int? { command.argument }
}

public enum CommandCatalog {
    public static let all: [CommandCatalogEntry] = entries

    /// Every command in the order menus should show them.
    public static let entries: [CommandCatalogEntry] = {
        var list: [CommandCatalogEntry] = []

        // Photo: navigation + rating (task.md §6, §9.4, §10)
        list += [CommandCatalogEntry(command: .photoPrevious, menu: .photo, title: "Previous Photo")]
        list += [CommandCatalogEntry(command: .photoNext, menu: .photo, title: "Next Photo")]
        list += [CommandCatalogEntry(command: .batchPrevious, menu: .photo, title: "Previous Batch")]
        list += [CommandCatalogEntry(command: .batchNext, menu: .photo, title: "Next Batch")]
        for stars in 1...5 {
            let word = stars == 1 ? "Star" : "Stars"
            list.append(
                CommandCatalogEntry(
                    command: .setStars(stars), menu: .photo, title: CatalogText.rated(stars, word)))
        }
        list.append(
            CommandCatalogEntry(command: .setStars(0), menu: .photo, title: "Clear Stars"))
        for stars in 1...5 {
            let word = stars == 1 ? "Star" : "Stars"
            list.append(
                CommandCatalogEntry(
                    command: .setStarsAndAdvance(stars), menu: .photo,
                    title: CatalogText.rateAndAdvance(stars, word)))
        }
        list += [
            CommandCatalogEntry(command: .togglePickFlag, menu: .photo, title: "Pick Flag"),
            CommandCatalogEntry(command: .toggleKeep, menu: .photo, title: "Keep"),
            CommandCatalogEntry(command: .rejectFlag, menu: .photo, title: "Reject Flag"),
            CommandCatalogEntry(command: .unflag, menu: .photo, title: "Clear Flag"),
            CommandCatalogEntry(command: .toggleFlag, menu: .photo, title: "Toggle Flag"),
        ]
        for label in [ColorLabel.red, .yellow, .green, .blue] {
            list.append(
                CommandCatalogEntry(
                    command: .setLabel(label), menu: .photo,
                    title: CatalogText.colorLabel(label.titleKey)))
        }
        list.append(
            CommandCatalogEntry(
                command: .setLabel(nil), menu: .photo, title: "Clear Color Label"))
        list += [
            CommandCatalogEntry(
                command: .toggleAutoAdvance, menu: .photo, title: "Auto-Advance")
        ]

        // View (task.md §9.2, §9.3, §9.5, §9.6)
        list += [
            CommandCatalogEntry(command: .showLoupe, menu: .view, title: "Loupe"),
            CommandCatalogEntry(command: .showGrid, menu: .view, title: "Grid"),
            CommandCatalogEntry(command: .showCompare(2), menu: .view, title: "Compare 2-Up"),
            CommandCatalogEntry(command: .showCompare(3), menu: .view, title: "Compare 3-Up"),
            CommandCatalogEntry(command: .showCompare(4), menu: .view, title: "Compare 4-Up"),
            CommandCatalogEntry(command: .toggleInfoPanel, menu: .view, title: "Info Panel"),
            CommandCatalogEntry(command: .toggleClippingOverlay, menu: .view, title: "Clipping Overlay"),
            CommandCatalogEntry(command: .toggleAFOverlay, menu: .view, title: "AF Point Overlay"),
            CommandCatalogEntry(command: .toggleHUD, menu: .view, title: "Progress HUD"),
            // Zoom is gesture-only by design; the commands exist so menus and remapping stay complete.
            CommandCatalogEntry(command: .toggleZoomLock, menu: .view, title: "Zoom Lock"),
        ]

        // File
        list += [
            CommandCatalogEntry(command: .openFolder, menu: .file, title: "Open Folder…"),
            CommandCatalogEntry(command: .finishCull, menu: .file, title: "Finish Cull"),
        ]

        // Edit
        list += [
            CommandCatalogEntry(command: .undo, menu: .edit, title: "Undo"),
            CommandCatalogEntry(command: .redo, menu: .edit, title: "Redo"),
        ]

        return list
    }()

    public static func entry(for command: Command) -> CommandCatalogEntry? {
        all.first { $0.command == command }
    }

    /// Entries for one menu, in display order.
    public static func entries(in menu: CommandMenu) -> [CommandCatalogEntry] {
        all.filter { $0.menu == menu }
    }
}

extension ColorLabel {
    public var titleKey: String {
        switch self {
        case .red: "Red"
        case .yellow: "Yellow"
        case .green: "Green"
        case .blue: "Blue"
        case .purple: "Purple"
        }
    }
}

/// Localization helper. v0.1 is English only, so the key *is* the English text; adding a
/// Localizable.strings later changes nothing at the call sites (task.md §1 non-goals).
enum CatalogText {
    static func localized(_ value: String) -> String {
        NSLocalizedString(value, comment: "")
    }

    static func rated(_ stars: Int, _ word: String) -> String {
        let value = stars == 1 ? "1 \(word)" : "\(stars) \(word)"
        return localized(value)
    }

    static func rateAndAdvance(_ stars: Int, _ word: String) -> String {
        let value = stars == 1 ? "1 \(word) and Advance" : "\(stars) \(word) and Advance"
        return localized(value)
    }

    static func colorLabel(_ name: String) -> String { localized("\(name) Label") }
}
