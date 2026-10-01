// Owner: ui.
//
// The Finish Cull sheet (todo.md §9.7). It is a thin view over the model's state machine: it draws
// whatever `state.finishStage` is and turns every button into a `FinishAction`. It never advances
// the stage itself, so what is on screen is always what the model believes.
//
//   summary → options → dry run → (typed confirmation for a permanent delete) → executing → report
//
// Nothing touches the disk before the dry run has been shown and the user has pressed "Finish", and
// the dry run is built by the same code that executes it, so the list the user reads is the truth.

import AppKit
import SwiftUI

struct FinishSheet: View {
    let state: any CullViewState

    /// Typed confirmation for the one action that cannot be undone (todo.md §9.7).
    @State private var deleteConfirmation = ""

    private static let confirmationWord = "DELETE"

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            switch state.finishStage {
            case .hidden:
                EmptyView()
            case .confirm:
                ConfirmStage(state: state)
            case .summary(let summary):
                SummaryStage(summary: summary, state: state)
            case .options(let summary, let settings):
                OptionsStage(summary: summary, settings: settings, state: state)
            case .dryRun(_, let settings, let plan):
                DryRunStage(
                    settings: settings, plan: plan, state: state, confirmation: $deleteConfirmation,
                    word: Self.confirmationWord)
            case .executing:
                ExecutingStage()
            case .report(_, let report):
                ReportStage(report: report, state: state)
            case .failed(_, let message):
                FailedStage(message: message, state: state)
            }
        }
        .padding(24)
        .frame(width: 560)
        .frame(minHeight: 320)
        .onChange(of: state.finishStage.isVisible) { _, visible in
            if !visible { deleteConfirmation = "" }
        }
    }
}

// MARK: - Shared pieces

private struct SheetHeader: View {
    let title: String
    var subtitle: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.title2.weight(.semibold))
            if let subtitle {
                Text(subtitle).font(.callout).foregroundStyle(.secondary)
            }
        }
        .padding(.bottom, 16)
        .accessibilityAddTraits(.isHeader)
    }
}

private struct Warning: View {
    let text: String
    var isSevere = false

    var body: some View {
        Label(text, systemImage: isSevere ? "exclamationmark.octagon.fill" : "exclamationmark.triangle.fill")
            .font(.callout)
            .foregroundStyle(isSevere ? Color.red : Color.orange)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 2)
    }
}

private struct ButtonRow<Leading: View, Trailing: View>: View {
    @ViewBuilder var leading: () -> Leading
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack {
            leading()
            Spacer()
            trailing()
        }
        .padding(.top, 20)
    }
}

// MARK: - Stages

/// The question Settings → General asks before any of this starts (todo.md §9.7). It is a stage
/// rather than an alert so the sheet stays the only place a Finish decision is made, and so Escape
/// still cancels the whole thing rather than dismissing one layer of it.
private struct ConfirmStage: View {
    let state: any CullViewState

    var body: some View {
        SheetHeader(
            title: "Finish Cull",
            subtitle: "Nothing happens until the next screen says what will happen to every file")

        Text(
            "You will see what is kept, what is not, and a dry-run list of every file operation "
                + "before anything touches the disk. Permanent deletes in that list cannot be undone."
        )
        .font(.callout)
        .foregroundStyle(.secondary)

        ButtonRow {
            Button("Cancel", role: .cancel) { state.finish(.cancel) }
                .keyboardShortcut(.cancelAction)
        } trailing: {
            Button("Continue") { state.finish(.confirm) }
                .keyboardShortcut(.defaultAction)
        }
    }
}

private struct SummaryStage: View {
    let summary: FinishSummary
    let state: any CullViewState

    var body: some View {
        SheetHeader(
            title: "Finish Cull",
            subtitle: "\(summary.totalPhotos) photos in \(summary.batchCount) batches")

        Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 6) {
            ForEach(Tier.allCases, id: \.self) { tier in
                GridRow {
                    HStack(spacing: 8) {
                        Circle().fill(color(for: tier)).frame(width: 8, height: 8)
                        Text(tier.title)
                    }
                    Text("\(summary[tier])")
                        .monospacedDigit()
                        .gridColumnAlignment(.trailing)
                        .fontWeight(tier == .keep ? .semibold : .regular)
                }
            }
        }
        .accessibilityElement(children: .contain)

        if summary.unvisitedBatches > 0 {
            Warning(
                text:
                    "\(summary.unvisitedBatches) batch\(summary.unvisitedBatches == 1 ? " was" : "es were") never opened. Photos in them count as not kept."
            )
            .padding(.top, 12)
        }

        Text(
            "Only the Keep tier is kept. Originals are never modified until you confirm on the next screens."
        )
        .font(.footnote)
        .foregroundStyle(.secondary)
        .padding(.top, 12)

        ButtonRow {
            Button("Cancel", role: .cancel) { state.finish(.cancel) }
                .keyboardShortcut(.cancelAction)
        } trailing: {
            Button("Choose What Happens Next") { state.finish(.showOptions) }
                .keyboardShortcut(.defaultAction)
        }
    }

    private func color(for tier: Tier) -> Color {
        switch tier {
        case .keep: Appearance.keepGreen
        case .good: .yellow
        case .maybe: .orange
        case .unrated: .gray
        case .rejected: Appearance.rejectRed
        }
    }
}

private struct OptionsStage: View {
    let summary: FinishSummary
    let settings: FinishSettings
    let state: any CullViewState

    var body: some View {
        SheetHeader(
            title: "What should happen?",
            subtitle: "\(summary.unkeptCount) not kept · \(summary.keptCount) kept")

        Text("Photos that were not kept").font(.headline).padding(.bottom, 6)
        Picker("Not kept", selection: unkeptKind) {
            ForEach(UnkeptAction.allCases, id: \.self) { action in
                Text(action.title).tag(action)
            }
        }
        .pickerStyle(.radioGroup)
        .labelsHidden()

        if settings.unkept.folderName != nil {
            TextField("Subfolder name", text: unkeptFolder)
                .textFieldStyle(.roundedBorder)
                .padding(.top, 6)
        }
        if settings.unkept.isDestructive {
            Warning(text: "Deleting permanently cannot be undone.", isSevere: true).padding(.top, 6)
        }

        Divider().padding(.vertical, 14)

        Text("Photos that were kept").font(.headline).padding(.bottom, 6)
        Picker("Kept", selection: keptKind) {
            ForEach(KeptAction.allCases, id: \.self) { action in
                Text(action.title).tag(action)
            }
        }
        .pickerStyle(.radioGroup)
        .labelsHidden()

        if settings.kept.folderName != nil {
            TextField(keptFieldLabel, text: keptFolder)
                .textFieldStyle(.roundedBorder)
                .padding(.top, 6)
        }
        if case .writeList = settings.kept {
            TextField("List file name", text: keptFolder)
                .textFieldStyle(.roundedBorder)
                .padding(.top, 6)
        }

        ButtonRow {
            Button("Back") { state.finish(.back) }
        } trailing: {
            Button("Preview") { state.finish(.preview) }
                .keyboardShortcut(.defaultAction)
                .disabled(!isValid)
        }
    }

    // The pickers select by *kind*, so switching kind keeps the text the user already typed.

    private var unkeptKind: Binding<UnkeptAction> {
        Binding(
            get: { settings.unkept.pickerTag },
            set: { newKind in
                state.finish(.setUnkept(newKind.with(folder: settings.unkept.folderName ?? "")))
            })
    }

    private var unkeptFolder: Binding<String> {
        Binding(
            get: { settings.unkept.folderName ?? "" },
            set: { state.finish(.setUnkept(.moveToSubfolder($0))) })
    }

    private var keptKind: Binding<KeptAction> {
        Binding(
            get: { settings.kept.pickerTag },
            set: { newKind in state.finish(.setKept(newKind.with(folder: settings.kept.text))) })
    }

    private var keptFolder: Binding<String> {
        Binding(
            get: { settings.kept.folderName ?? settings.kept.text },
            set: { state.finish(.setKept(settings.kept.with(folder: $0))) })
    }

    private var keptFieldLabel: String {
        switch settings.kept {
        case .copyTo, .moveTo: "Destination folder"
        default: "Folder to create the tier subfolders in"
        }
    }

    /// A folder name is required wherever the action needs one.
    private var isValid: Bool {
        let unkeptOK =
            settings.unkept.folderName.map { !$0.trimmingCharacters(in: .whitespaces).isEmpty } ?? true
        let keptOK: Bool
        if case .none = settings.kept {
            keptOK = true
        } else {
            keptOK = !settings.kept.text.trimmingCharacters(in: .whitespaces).isEmpty
        }
        return unkeptOK && keptOK
    }
}

private struct DryRunStage: View {
    let settings: FinishSettings
    let plan: FinishPlanData
    let state: any CullViewState
    @Binding var confirmation: String
    let word: String

    /// The model decided this at the dry-run stage, where the plan is known
    /// (`AppModel.runFinishDryRun`); the sheet only draws the gate. The warning about
    /// irreversibility above is deliberately *not* gated: it is a fact about the run, not a
    /// confirmation preference.
    private var needsTypedConfirmation: Bool { settings.requiresTypedConfirmation }
    private var isConfirmed: Bool { !needsTypedConfirmation || confirmation == word }

    var body: some View {
        SheetHeader(
            title: plan.ops.isEmpty ? "Nothing to do" : "Review before finishing",
            subtitle: plan.ops.isEmpty
                ? "Finish would not change any file."
                : "\(plan.opCount) file operation\(plan.opCount == 1 ? "" : "s"), copying \(plan.bytesToCopyDescription)"
        )

        ForEach(plan.warnings, id: \.self) { warning in
            Warning(text: warning, isSevere: warning.localizedCaseInsensitiveContains("cannot be undone"))
        }

        if !plan.ops.isEmpty {
            List(Array(plan.ops.prefix(500).enumerated()), id: \.offset) { _, op in
                HStack(spacing: 8) {
                    Text(op.kind.rawValue).font(.caption.monospaced()).foregroundStyle(.secondary)
                        .frame(width: 64, alignment: .leading)
                    Text(line(for: op)).font(.callout).lineLimit(1).truncationMode(.middle)
                }
            }
            .frame(height: 200)
            .listStyle(.bordered)
            .accessibilityLabel("File operations")
            if plan.ops.count > 500 {
                Text("… and \(plan.ops.count - 500) more").font(.footnote).foregroundStyle(.secondary)
            }
        }

        if needsTypedConfirmation {
            VStack(alignment: .leading, spacing: 6) {
                Text("Type \(word) to confirm. This cannot be undone.").font(.callout)
                TextField(word, text: $confirmation).textFieldStyle(.roundedBorder)
            }
            .padding(.top, 12)
        }

        ButtonRow {
            Button("Back") { state.finish(.back) }
        } trailing: {
            HStack {
                Button("Cancel", role: .cancel) { state.finish(.cancel) }
                    .keyboardShortcut(.cancelAction)
                Button("Finish") { state.finish(.execute) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(plan.ops.isEmpty || !isConfirmed)
            }
        }
    }

    private func line(for op: FileOp) -> String {
        let from = URL(fileURLWithPath: op.from).lastPathComponent
        guard let to = op.to else { return from }
        return
            "\(from) → \(URL(fileURLWithPath: to).deletingLastPathComponent().lastPathComponent)/\(URL(fileURLWithPath: to).lastPathComponent)"
    }
}

private struct ExecutingStage: View {
    var body: some View {
        SheetHeader(title: "Finishing…", subtitle: "Moving your photos. Please keep Firstcut open.")
        ProgressView().progressViewStyle(.linear)
            .padding(.vertical, 24)
    }
}

private struct ReportStage: View {
    let report: FinishReportData
    let state: any CullViewState

    var body: some View {
        SheetHeader(
            title: report.wasUndo ? "Finish undone" : "Finished",
            subtitle: report.wasUndo
                ? "\(report.done) file operation\(report.done == 1 ? "" : "s") reversed."
                : "\(report.done) file operation\(report.done == 1 ? "" : "s") done.")

        if report.failed.isEmpty {
            Label("Everything went through.", systemImage: "checkmark.circle.fill")
                .foregroundStyle(Appearance.keepGreen)
        } else {
            Warning(
                text:
                    "\(report.failed.count) file\(report.failed.count == 1 ? "" : "s") could not be handled and \(report.failed.count == 1 ? "was" : "were") left alone:"
            )
            List(Array(report.failed.enumerated()), id: \.offset) { _, failure in
                VStack(alignment: .leading, spacing: 2) {
                    Text(URL(fileURLWithPath: failure.path).lastPathComponent).font(.callout)
                    Text(failure.reason).font(.caption).foregroundStyle(.secondary)
                }
            }
            .frame(height: 140)
            .listStyle(.bordered)
        }

        if !report.wasUndo && !report.undoable {
            Warning(text: "This run included a permanent delete, so it cannot be undone.")
                .padding(.top, 8)
        }

        ButtonRow {
            HStack {
                if !report.wasUndo && report.undoable {
                    Button("Undo Finish") { state.finish(.undo) }
                }
                if let folder = state.folderURL {
                    Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([folder]) }
                }
            }
        } trailing: {
            Button("Done") { state.finish(.cancel) }
                .keyboardShortcut(.defaultAction)
        }
    }
}

private struct FailedStage: View {
    let message: String
    let state: any CullViewState

    var body: some View {
        SheetHeader(title: "Finish stopped")
        Warning(text: message, isSevere: true)
        ButtonRow {
            EmptyView()
        } trailing: {
            Button("Close") { state.finish(.cancel) }.keyboardShortcut(.defaultAction)
        }
    }
}
