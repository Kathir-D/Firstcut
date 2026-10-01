// Owner: ui.
//
// Unified toolbar (todo.md §9.1): ‹ › batch navigation in a glass capsule on the left, batch
// position + file name as the title, view-mode control, info toggle and Finish on the right.

import SwiftUI

struct FirstcutToolbar: ToolbarContent {
    let environment: AppEnvironment

    /// REV-75: the culling controls are not culled themselves. Outside `.culling`/`.finishing` the
    /// toolbar shows the folder name and Open Folder, because a "Batch 12 of 148" title and a Finish
    /// button on the welcome screen is the same defect one level up: chrome for a phase the window is
    /// not in.
    var body: some ToolbarContent {
        switch environment.state.phase {
        case .culling, .finishing:
            cullingItems
        case .welcome, .loading:
            idleItems
        }
    }

    @ToolbarContentBuilder
    private var cullingItems: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            BatchNavigationCapsule(environment: environment)
        }
        ToolbarItem(placement: .principal) {
            BatchTitle(environment: environment)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            ViewModeControl(environment: environment)
            InfoToggleButton(environment: environment)
            FinishCullButton(environment: environment)
        }
    }

    @ToolbarContentBuilder
    private var idleItems: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Text(environment.state.folderName)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Appearance.secondaryLabel)
                .accessibilityLabel("Folder \(environment.state.folderName)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                environment.send(.openFolder)
            } label: {
                Text("Open Folder…")
                    .font(.system(size: 12, weight: .medium))
            }
            .buttonStyle(.borderedProminent)
            .help("Open Folder… (⌘O)")
            .accessibilityLabel("Open folder")
        }
    }
}

struct BatchNavigationCapsule: View {
    let environment: AppEnvironment

    var body: some View {
        GlassCapsule {
            HStack(spacing: 0) {
                capsuleButton(
                    systemName: "chevron.left",
                    label: "Previous batch",
                    isEnabled: environment.state.currentBatchIndex > 0,
                    action: .batchPrevious
                )
                Divider().frame(height: 14).overlay(Appearance.separator)
                capsuleButton(
                    systemName: "chevron.right",
                    label: "Next batch",
                    isEnabled: environment.state.currentBatchIndex + 1 < environment.state.batches.count,
                    action: .batchNext
                )
            }
            .padding(.horizontal, 2)
            .padding(.vertical, 2)
        }
        .fixedSize()
    }

    private func capsuleButton(systemName: String, label: String, isEnabled: Bool, action: CullAction)
        -> some View
    {
        Button {
            environment.send(action)
        } label: {
            Image(systemName: systemName)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: 22, height: 20)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .foregroundStyle(isEnabled ? Appearance.primaryLabel : Appearance.tertiaryLabel)
        .help(label)
        .accessibilityLabel(label)
    }
}

struct BatchTitle: View {
    let environment: AppEnvironment

    var body: some View {
        HStack(spacing: 6) {
            Text(
                "Batch \(environment.state.currentBatchIndex + 1) of \(max(environment.state.batches.count, 1))"
            )
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(Appearance.primaryLabel)
            .monospacedDigit()
            if let name = environment.state.currentPhoto?.fileName {
                Text("—")
                    .font(.system(size: 13))
                    .foregroundStyle(Appearance.tertiaryLabel)
                Text(name)
                    .font(.system(size: 13))
                    .foregroundStyle(Appearance.secondaryLabel)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if isCurrentBatchProvisional {
                Circle()
                    .fill(Appearance.secondaryLabel)
                    .frame(width: 5, height: 5)
                    .help("This batch is provisional and may still be refined")
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(titleAccessibility)
    }

    private var isCurrentBatchProvisional: Bool {
        let index = environment.state.currentBatchIndex
        let batches = environment.state.batches
        return batches.indices.contains(index) && batches[index].isProvisional
    }

    private var titleAccessibility: String {
        let state = environment.state
        let name = state.currentPhoto?.fileName ?? "no photo"
        return "Batch \(state.currentBatchIndex + 1) of \(state.batches.count), \(name)"
    }
}

struct ViewModeControl: View {
    let environment: AppEnvironment

    var body: some View {
        Picker("", selection: viewModeBinding) {
            ForEach(CullViewMode.allCases) { mode in
                Image(systemName: symbol(for: mode))
                    .help(mode.displayName)
                    .tag(mode)
                    .accessibilityLabel(mode.displayName)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 104)
        .help("View mode")
    }

    private var viewModeBinding: Binding<CullViewMode> {
        Binding(
            get: { environment.state.viewMode },
            set: { environment.send(.setViewMode($0)) }
        )
    }

    private func symbol(for mode: CullViewMode) -> String {
        switch mode {
        case .loupe: "viewfinder"
        case .grid: "square.grid.2x2"
        case .compare: "rectangle.split.2x1"
        }
    }
}

struct InfoToggleButton: View {
    let environment: AppEnvironment

    var body: some View {
        Button {
            environment.send(.toggleInfoPanel)
        } label: {
            Image(systemName: "sidebar.trailing")
                .frame(width: 20, height: 18)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .foregroundStyle(
            environment.state.isInfoPanelVisible ? Color.accentColor : Appearance.secondaryLabel
        )
        .help("Info panel (I)")
        .accessibilityLabel("Info panel")
        .accessibilityValue(environment.state.isInfoPanelVisible ? "On" : "Off")
    }
}

struct FinishCullButton: View {
    let environment: AppEnvironment

    var body: some View {
        Button {
            environment.send(.finishCull)
        } label: {
            Text("Finish")
                .font(.system(size: 12, weight: .medium))
                .padding(.horizontal, 10)
                .padding(.vertical, 3)
        }
        .buttonStyle(.borderedProminent)
        .help("Finish cull (↩)")
        .accessibilityLabel("Finish cull")
    }
}
