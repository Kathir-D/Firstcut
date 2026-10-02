// Owner: ui.
//
// The Settings window (todo.md §9.8), a native SwiftUI `Settings` scene: ⌘, opens it, and it has one
// tab per group. Every control edits `AppModel.settings` through `updateSettings`, the one place
// that persists a change and reacts to it, so nothing here can leave the model and the file
// disagreeing.
//
// Only settings the app actually honors are shown. A switch that does nothing is worse than no
// switch, so the ones the code does not read yet (the debug HUD, DNG writes) are not offered; DNG
// writes are excluded on principle, because Firstcut never modifies an original (todo.md §11).
//
// "Exact RAW" used to be on that list. It is not any more: `DecodeEngine.decodeExactRaw` develops
// the sensor data through `CIRAWFilter` and `setFocus` points the tier at the current photograph
// (`ImageProvider.updateExactRaw`), so the toggle has an effect.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    let model: AppModel

    var body: some View {
        TabView {
            GeneralSettingsTab(model: model)
                .tabItem { Label("General", systemImage: "gearshape") }
            KeyboardSettingsTab(model: model)
                .tabItem { Label("Keyboard", systemImage: "keyboard") }
            ViewerSettingsTab(model: model)
                .tabItem { Label("Viewer", systemImage: "photo") }
            MetadataSettingsTab(model: model)
                .tabItem { Label("Metadata", systemImage: "tag") }
            PerformanceSettingsTab(model: model)
                .tabItem { Label("Performance", systemImage: "speedometer") }
        }
        .frame(width: 620, height: 520)
        .preferredColorScheme(.dark)
    }
}

/// A binding into `AppSettings` that writes through `updateSettings`.
///
/// `@MainActor` so the closures it builds may touch the model, which is main-actor state; the key
/// path is captured once as `nonisolated(unsafe)` because key paths are not `Sendable` and this one
/// is never used off the main actor.
@MainActor
private func settingBinding<T: Equatable>(
    _ model: AppModel, _ keyPath: WritableKeyPath<AppSettings, T>
) -> Binding<T> {
    nonisolated(unsafe) let path = keyPath
    return Binding(
        get: { model.settings[keyPath: path] },
        set: { value in model.updateSettings { $0[keyPath: path] = value } })
}

// MARK: - General

private struct GeneralSettingsTab: View {
    let model: AppModel

    var body: some View {
        Form {
            Section("Rating") {
                Picker("Rating mode", selection: settingBinding(model, \.general.ratingMode)) {
                    Text("Stars").tag(RatingMode.stars)
                    Text("Keep / Not keep").tag(RatingMode.keep)
                }
                Text(ratingModeHint)
                    .font(.footnote).foregroundStyle(.secondary)
                if model.settings.general.ratingMode == .stars {
                    Picker(
                        "A star rating counts as a keep from",
                        selection: settingBinding(model, \.general.keepThreshold)
                    ) {
                        Text("5 stars").tag(5)
                        Text("4 stars").tag(4)
                    }
                }
                Toggle(
                    "Move to the next photo after rating", isOn: settingBinding(model, \.general.autoAdvance))
            }

            Section("Navigation") {
                Picker(
                    "At the end of a batch",
                    selection: settingBinding(model, \.general.arrowBehaviorAtBatchEnd)
                ) {
                    ForEach(ArrowBehavior.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                Picker(
                    "When entering a batch", selection: settingBinding(model, \.general.enteringBatchBehavior)
                ) {
                    ForEach(EnteringBatchBehavior.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }

            Section("Confirmations") {
                confirmations
            }

            Section("Finish Cull defaults") {
                Picker("Photos not kept", selection: unkeptKind) {
                    ForEach(UnkeptAction.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                if model.settings.general.finishUnkept.folderName != nil {
                    TextField("Subfolder name", text: unkeptFolder)
                }
                Picker("Photos kept", selection: keptKind) {
                    ForEach(KeptAction.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                if case .none = model.settings.general.finishKept {
                } else {
                    TextField("Folder or file name", text: keptText)
                }
            }
        }
        .formStyle(.grouped)
        .padding(.horizontal, 8)
    }

    private var ratingModeHint: String {
        guard model.settings.general.ratingMode == .stars else {
            return "Every photo starts as Not keep. One key keeps it."
        }
        return model.settings.general.keepThreshold >= 5
            ? "1–5 stars per photo. Only 5 stars counts as a keep."
            : "1–5 stars per photo. 4 and 5 stars count as a keep."
    }

    private var unkeptKind: Binding<UnkeptAction> {
        Binding(
            get: { model.settings.general.finishUnkept.pickerTag },
            set: { kind in
                model.updateSettings {
                    $0.general.finishUnkept = kind.with(folder: $0.general.finishUnkept.folderName ?? "")
                }
            })
    }

    /// Shown because the model honours both of them (`AppModel.runFinishDryRun` for the typed word;
    /// the extra step for `confirmBeforeFinish` is the sheet's `askToFinish` state).
    private var confirmations: some View {
        Group {
            Toggle(
                "Confirm before Finish", isOn: settingBinding(model, \.general.confirmBeforeFinish))
            Toggle(
                "Ask for the typed word before deleting permanently",
                isOn: settingBinding(model, \.general.confirmPermanentDelete))
            Text(
                "Permanent delete cannot be undone, so the typed word (DELETE) is the last stop before it. "
                    + "Turning that off does not make the run reversible."
            )
            .font(.footnote).foregroundStyle(.secondary)
        }
    }

    private var unkeptFolder: Binding<String> {
        Binding(
            get: { model.settings.general.finishUnkept.folderName ?? "" },
            set: { text in model.updateSettings { $0.general.finishUnkept = .moveToSubfolder(text) } })
    }

    private var keptKind: Binding<KeptAction> {
        Binding(
            get: { model.settings.general.finishKept.pickerTag },
            set: { kind in
                model.updateSettings { $0.general.finishKept = kind.with(folder: $0.general.finishKept.text) }
            })
    }

    private var keptText: Binding<String> {
        Binding(
            get: { model.settings.general.finishKept.text },
            set: { text in
                model.updateSettings { $0.general.finishKept = $0.general.finishKept.with(folder: text) }
            })
    }
}

// MARK: - Viewer

private struct ViewerSettingsTab: View {
    let model: AppModel

    var body: some View {
        Form {
            Section("Photo") {
                LabeledContent("Background") {
                    Slider(value: settingBinding(model, \.viewer.backgroundGray), in: 0...1)
                        .frame(width: 220)
                }
                Toggle(
                    "Keep the zoom when moving between photos", isOn: settingBinding(model, \.viewer.zoomLock)
                )
                Toggle("Show autofocus points", isOn: settingBinding(model, \.viewer.afOverlay))
                Toggle("Show the progress HUD", isOn: settingBinding(model, \.viewer.hudVisible))
            }

            Section("Exact RAW") {
                // Shown because the pipeline honours it (todo.md §0.3): this is the one decode that
                // develops the sensor data instead of reading the camera's embedded preview. The
                // cost is the reason for the wording -- measured on a Canon R8 CR3, a full-resolution
                // develop is 0.299 s against 0.089 s for the preview, and it is redone for each
                // photograph you move to. It is off by default for that reason, not because it is
                // unfinished.
                Toggle(
                    "Develop the sensor data, not the embedded preview",
                    isOn: settingBinding(model, \.viewer.exactRaw))
                Text(
                    "Slower per photograph, and only for RAW files. Off means the picture you see is "
                        + "the preview the camera wrote into the file."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Section("Clipping overlay") {
                // Shown because the pipeline honours them (todo.md §0.3): these reach
                // `ClippingMask` through the viewer's presentation. The ranges stop short of the
                // ends on purpose -- 1.0 highlights every white pixel and 0.0 shadows every black
                // one, which is a picture painted entirely red or blue rather than an overlay.
                LabeledContent("Highlights") {
                    Slider(value: settingBinding(model, \.viewer.clippingHighlightThreshold), in: 0.5...1)
                        .frame(width: 220)
                }
                LabeledContent("Shadows") {
                    Slider(value: settingBinding(model, \.viewer.clippingShadowThreshold), in: 0...0.5)
                        .frame(width: 220)
                }
                Text("A channel at or past the point is painted; J toggles the overlay.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section("Info panel fields") {
                ForEach(InfoField.allCases) { field in
                    Toggle(field.title, isOn: fieldBinding(field))
                }
            }
        }
        .formStyle(.grouped)
        .padding(.horizontal, 8)
    }

    private func fieldBinding(_ field: InfoField) -> Binding<Bool> {
        Binding(
            get: { model.settings.viewer.infoFields.contains(field) },
            set: { shown in
                model.updateSettings {
                    if shown {
                        $0.viewer.infoFields.insert(field)
                    } else {
                        $0.viewer.infoFields.remove(field)
                    }
                }
            })
    }
}

// MARK: - Metadata

private struct MetadataSettingsTab: View {
    let model: AppModel

    private enum KeepKind: Hashable {
        case fiveStars, fourStars
        case label(ColorLabel)
    }

    var body: some View {
        Form {
            Section("XMP sidecars") {
                Toggle(
                    "Write ratings to .xmp sidecar files", isOn: settingBinding(model, \.metadata.writeXmp))
                Toggle(
                    "Also for JPEG, HEIF and other non-RAW photos",
                    isOn: settingBinding(model, \.metadata.writeSidecarsForJpegs)
                )
                .disabled(!model.settings.metadata.writeXmp)
                Text(
                    "Lightroom and Capture One read these sidecars. Firstcut writes only the sidecar, never the photo itself, and keeps anything else already in an existing one."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }

            Section("Keep / Not keep mode") {
                Picker("A keep is written as", selection: keepKind) {
                    Text("5 stars").tag(KeepKind.fiveStars)
                    Text("4 stars").tag(KeepKind.fourStars)
                    ForEach([ColorLabel.green, .blue, .yellow, .red, .purple], id: \.self) { label in
                        Text("Color label: \(label.titleKey)").tag(KeepKind.label(label))
                    }
                }
                Text(
                    "Lightroom does not read pick flags from XMP, so a keep has to be a rating or a label to survive an import."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }
            .disabled(!model.settings.metadata.writeXmp)
        }
        .formStyle(.grouped)
        .padding(.horizontal, 8)
    }

    private var keepKind: Binding<KeepKind> {
        Binding(
            get: {
                switch model.settings.metadata.keepMapping {
                case .rating(let stars): stars == 4 ? .fourStars : .fiveStars
                case .colorLabel(let label): .label(label)
                }
            },
            set: { kind in
                model.updateSettings {
                    switch kind {
                    case .fiveStars: $0.metadata.keepMapping = .rating(5)
                    case .fourStars: $0.metadata.keepMapping = .rating(4)
                    case .label(let label): $0.metadata.keepMapping = .colorLabel(label)
                    }
                }
            })
    }
}

// MARK: - Performance

private struct PerformanceSettingsTab: View {
    let model: AppModel

    private var budgetGB: String {
        let bytes = Double(model.settings.memoryBudgetBytes)
        return String(format: "%.1f GB", bytes / 1_073_741_824)
    }

    /// "auto (4)" or the number, so the stepper shows what auto *resolves to* rather than a bare 0.
    private var decodeThreadsLabel: String {
        let configured = model.settings.performance.decodeThreads
        return configured < 1 ? "auto (\(ImageProvider.resolvedDecodeThreads(configured)))" : "\(configured)"
    }

    var body: some View {
        Form {
            Section("Memory") {
                LabeledContent("Photo cache") {
                    HStack {
                        Slider(
                            value: settingBinding(model, \.performance.memoryBudgetFraction), in: 0.1...0.7,
                            step: 0.05
                        )
                        .frame(width: 180)
                        Text(
                            "\(Int((model.settings.performance.memoryBudgetFraction * 100).rounded()))% · \(budgetGB)"
                        )
                        .monospacedDigit()
                        .frame(width: 96, alignment: .trailing)
                    }
                }
                Text(
                    "How much of this Mac's memory Firstcut may use to keep decoded photos ready. Applies the next time Firstcut starts."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }

            Section("Debug") {
                Toggle("Debug HUD", isOn: settingBinding(model, \.performance.debugHUD))
                Text(
                    "Shows focus misses, decode queue depth, cache hits, memory by tier and the byte-range "
                        + "counters. todo.md §7.1 requires focus misses to stay 0; this is where you watch it."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }

            Section("Decoding") {
                Stepper(
                    "Decode threads: \(decodeThreadsLabel)",
                    value: settingBinding(model, \.performance.decodeThreads), in: 0...12)
                Text(
                    "How many photos may be decoded at once. Auto is the measured knee (4 on an 8-core Mac: four "
                        + "and eight threads decode the same number of photos a second), so a lower number makes "
                        + "the machine feel lighter and a higher one gets a burst of focus ahead sooner. "
                        + "Applies the next time Firstcut starts."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }

            Section("Look-ahead") {
                Stepper(
                    "Batches kept ready on each side: \(model.settings.performance.lookAheadBatches)",
                    value: settingBinding(model, \.performance.lookAheadBatches), in: 1...6)
                Text(
                    "Photos in the previous, current and next batch are always decoded before you can reach them. More batches use more memory."
                )
                .font(.footnote).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding(.horizontal, 8)
    }
}

// MARK: - Keyboard

private struct KeyboardSettingsTab: View {
    let model: AppModel
    @State private var recording: CommandCatalogEntry?
    @State private var message: String?

    var body: some View {
        VStack(spacing: 0) {
            List {
                ForEach(MenuSection.allCases, id: \.self) { section in
                    let entries = CommandCatalog.entries.filter { $0.menu == section }
                    if !entries.isEmpty {
                        Section(section.title) {
                            ForEach(entries) { entry in row(entry) }
                        }
                    }
                }
            }
            .listStyle(.inset(alternatesRowBackgrounds: true))

            Divider()
            HStack {
                if let message {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout).foregroundStyle(.orange)
                }
                Spacer()
                Button("Import…") { importKeymap() }
                Button("Export…") { exportKeymap() }
                Button("Reset to Lightroom Defaults") {
                    model.resetKeymap()
                    message = nil
                }
            }
            .padding(12)
        }
        // Closing Settings (or switching tab) mid-recording must hand the keys back to the main
        // window; otherwise the key router would stay muted.
        .onDisappear { if recording != nil { stopRecording() } }
    }

    private func row(_ entry: CommandCatalogEntry) -> some View {
        let chords = model.keymap.chords(for: entry.commandID, argument: entry.argument)
        let isRecording = recording?.id == entry.id
        return HStack {
            Text(entry.title)
            Spacer()
            if isRecording {
                KeyRecorder(
                    onRecord: { chord in commit(chord, to: entry) },
                    onCancel: { stopRecording() }
                )
                .frame(width: 150, height: 24)
            } else {
                Text(chords.isEmpty ? "—" : chords.map(\.description).joined(separator: "  "))
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(chords.isEmpty ? .secondary : .primary)
                Button("Record") { startRecording(entry) }
                    .controlSize(.small)
                if !chords.isEmpty {
                    Button {
                        model.updateKeymap { $0.unbind(entry.command) }
                    } label: {
                        Image(systemName: "xmark.circle")
                    }
                    .buttonStyle(.borderless)
                    .help("Remove this shortcut")
                    .accessibilityLabel("Remove shortcut for \(entry.title)")
                }
            }
        }
    }

    private func startRecording(_ entry: CommandCatalogEntry) {
        message = nil
        recording = entry
        // While a shortcut is being recorded the key router must not run the keys as commands.
        model.isTextEditing = true
    }

    private func stopRecording() {
        recording = nil
        model.isTextEditing = false
    }

    private func commit(_ chord: KeyChord, to entry: CommandCatalogEntry) {
        defer { stopRecording() }
        // A chord already used by a different command is refused with the name of the command that
        // holds it, rather than silently stealing the shortcut (todo.md §9.8: conflict detection).
        var conflict: String?
        model.updateKeymap { keymap in
            if keymap.bind(chord, to: entry.command, resolveConflict: false) != nil {
                let holder = keymap.binding(for: chord, mode: model.ratingMode)
                conflict =
                    CommandCatalog.entries.first {
                        $0.commandID == holder?.command && $0.argument == holder?.argument
                    }?
                    .title ?? holder?.command
            }
        }
        message = conflict.map { "\(chord.description) is already used by \($0). Remove it there first." }
    }

    private func exportKeymap() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Firstcut Keymap.json"
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do { try model.exportKeymap(to: url) } catch {
            message = "Could not export: \(error.localizedDescription)"
        }
    }

    private func importKeymap() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try model.importKeymap(from: url)
            message = nil
        } catch {
            message = "Could not import: \(error.localizedDescription)"
        }
    }
}

/// Records one key chord: click "Record", press the keys. Escape cancels.
private struct KeyRecorder: NSViewRepresentable {
    var onRecord: (KeyChord) -> Void
    var onCancel: () -> Void

    func makeNSView(context: Context) -> RecorderView {
        let view = RecorderView()
        view.onRecord = onRecord
        view.onCancel = onCancel
        DispatchQueue.main.async { view.window?.makeFirstResponder(view) }
        return view
    }

    func updateNSView(_ view: RecorderView, context: Context) {
        view.onRecord = onRecord
        view.onCancel = onCancel
    }

    final class RecorderView: NSView {
        var onRecord: ((KeyChord) -> Void)?
        var onCancel: (() -> Void)?

        override var acceptsFirstResponder: Bool { true }

        override func draw(_ dirtyRect: NSRect) {
            NSColor.controlAccentColor.withAlphaComponent(0.25).setFill()
            let path = NSBezierPath(roundedRect: bounds, xRadius: 5, yRadius: 5)
            path.fill()
            let text = "Press a key…" as NSString
            let attributes: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.labelColor,
            ]
            let size = text.size(withAttributes: attributes)
            text.draw(
                at: NSPoint(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2),
                withAttributes: attributes)
        }

        override func keyDown(with event: NSEvent) {
            if event.keyCode == 53 {  // escape
                onCancel?()
                return
            }
            guard let chord = KeyChord(event: event) else { return }
            onRecord?(chord)
        }

        override func resignFirstResponder() -> Bool {
            onCancel?()
            return true
        }
    }
}
