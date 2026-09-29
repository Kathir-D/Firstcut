# Agent: ui

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Make Firstcut indistinguishable from a first-party Apple app: Finder's gallery layout, Liquid Glass, always dark, on macOS 26+, with a close fallback on macOS 15.

## End goal (definition of done for this agent)

Every screen and element in task.md §9 exists, is driven only by `AppModel`, and passes a side-by-side visual check against Finder on macOS 26 and macOS 15; the filmstrip stays at 120 Hz with 60+ frames; accessibility labels and Reduce Transparency / Reduce Motion are supported.

**Done means (and senior-dev has signed it off in `docs/review.md`):** The owner can't tell it apart from an Apple app in a side-by-side with Finder; qa's UI checklist passes on 26 and 15.

## Owns (only you edit these)

`App/Sources/App/`, `App/Sources/Views/`, `App/Sources/Settings/Views/`, `App/Resources/Assets.xcassets` (incl. app icon), `App/Tests/Unit/Views/`, `docs/ui/`

## Does NOT own

State and rules (app-logic), the image layer itself and zoom gestures (pipeline's `PhotoViewerLayerView`; you embed it), the README (infra, but you supply screenshots).

## task.md sections to read

§9 UI / UX (all), §6 (filmstrip rating visuals: stars, flags, green/red rings), §10 (menu bar shows the shortcuts)

## Contracts

- **Owns:** none (you consume app-model.md and pipeline-api.md)
- **Consumes:** app-model.md (primary), pipeline-api.md
- **Provides to others:** All views, the window, toolbar, menus, Settings window, screenshots for the README

## Who the other agents are

| Agent | What they do | Talk to them about |
| --- | --- | --- |
| **senior-dev** | Technical lead: reviews all your work and files required changes in [`docs/review.md`](../review.md) | Anything under your name in `review.md`, disputes, design questions |
| infra | Build, UniFFI bridge, CI, releases, Homebrew, README | Exporting your types, build breaks, CI |
| core-meta | Metadata parsing for every format | `PhotoMeta` fields |
| core-batch | Ordering, batching, ground truth, CLI | `Batch`, `VisualSig`, fixtures |
| core-store | Session DB, XMP, undo, finish file ops | `Session` API |
| pipeline | Decode, cache, prefetch, viewer layer, zoom | `ImageProvider`, performance |
| app-logic | State model, commands, keymap, rules | `AppModel`, `Command` |
| ui | Every screen, Liquid Glass, Finder look | Layout, visuals |
| qa | Tests, perf baselines, bugs, sign-off | Test hooks, bug reports |

## Deliverables

- [ ] **Wave 1**: replace the placeholder `App/Sources/App/FirstcutApp.swift`; window + unified toolbar (‹ › batch buttons in a glass capsule, title `Batch 12 of 148 — IMG_8231`, view-mode control, info toggle, Finish button), forced dark appearance, viewer area embedding a placeholder until pipeline's layer lands, Finder-style filmstrip (custom AppKit, rounded selection plate). All from `AppModel.preview(game:)`.
- [ ] **Wave 1**: record Finder gallery-view reference screenshots on macOS 26 (`docs/ui/`, which you own) for side-by-side checks.
- [ ] **Wave 2**: rating visuals: stars/flags/labels in stars mode, green/red rings in keep mode; progress HUD (glass capsule); menu bar with every command; embed `PhotoViewerLayerView`.
- [ ] **Wave 3**: info panel (I, right side, Lightroom-style fields + histogram), grid view, 2–4-up compare, welcome window (open folder, recents with progress, drag and drop), Finish Cull sheet (summary → options → dry-run list → progress → report → undo), Settings window (General, Keyboard editor, Viewer, Metadata, Performance).
- [ ] **Wave 4**: Liquid Glass fidelity pass (`glassEffect`, `GlassEffectContainer`, `NSGlassEffectView`), macOS 15 fallback (`NSVisualEffectView`), VoiceOver labels, Reduce Transparency/Motion, full screen, app icon, README screenshots + GIF for infra.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "ui" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/ui.md (your charter + status),
docs/review.md (fix every open finding under "ui" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree, ~/Documents/projects/Firstcut-wt/ui (branch agent/ui, already
created), and only on the paths you own. Read other agents' files live from their worktrees
(~/Documents/projects/Firstcut-wt/<agent>/...). All agents are starting at the same time: never wait
for anyone. Build against the v0.1 contracts, CoreTypes.swift, and tests/fixtures/exiftool/, and
file requests for anything missing.
Then pick the next unchecked deliverable, do it, then update your Live status (answer REV findings in
Incoming requests), tick task.md boxes you own,
commit, and push. Ask other agents for anything you need through requests, never by editing their files.
```

---

## Live status

_Last updated: 2026-09-29 (session 1)_

### Current focus

Wave 1 is done and pushed. Next: wave 2 — embed pipeline's `PhotoViewerLayerView`, rating visuals in
keep mode end to end, and the rest of the menu bar. Blocked on REQ-ui-1 (`AppModel`) and REQ-ui-2
(viewer layer) for the real data path; everything else can keep going against the mock.

### Done log

| Date | What | Commit |
| --- | --- | --- |
| 2026-09-29 | Wave 1: window + unified toolbar + menus + forced dark; Finder-style AppKit filmstrip with selection plate, stars/flags/labels and keep rings; viewer placeholder + host seam for pipeline; info panel with all §9.5 fields; progress HUD; `Assets.xcassets`; Finder reference screenshots in `docs/ui/`; 29 unit tests | `ui: wave 1 window, toolbar, menus, filmstrip` |

### Blockers

None. Working against `PreviewCullViewState` until `AppModel` lands (REQ-ui-1).

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |
| REQ-ui-1 | app-logic | `AppModel` + `BatchVM` / `PhotoVM` / `Tier` / `ViewMode` / `ViewerState` / `CullProgress` as in app-model.md v0.1, and `AppModel.preview(game:)` | The views bind to a read-only `CullViewState` protocol (`App/Sources/Views/Model/CullViewState.swift`) that mirrors the contract, implemented today by `PreviewCullViewState`. When your model lands I delete mine and write a ~30-line adapter. Please confirm the exact type names, and whether you would rather make `AppModel` conform to a shared read-only protocol (preferred: then the adapter disappears too) or I keep the adapter. Two field-name differences to reconcile, cc senior-dev: contract `BatchVM.visited`/`provisional` vs my `isVisited`/`isProvisional`, and contract `photos(inBatch:)` vs my `photosInCurrentBatch` (the filmstrip needs the current batch's photos on every frame). Also `viewerBackgroundDarkness: Double` (0…1) is new — §9.2 asks for a configurable background gray. | open |
| REQ-ui-2 | pipeline | `PhotoViewerLayerView`: exact type name + initializer, so `ViewerArea` can construct it directly | `PhotoViewerHostView.layerViewFactory` (in `App/Sources/Views/Viewer/ViewerArea.swift`) is the interim seam so the app compiles without your file. I build the view per photo and set its frame. If you'd rather register it from your side, say so and I'll leave the registry in place instead. | open |
| REQ-ui-3 | app-logic | The keymap surface the menu bar needs: command title + current shortcut per command, e.g. `keymap.shortcut(for: .batchNext) -> String?`; and confirm how the local `NSEvent` monitor avoids double-firing | §9.1 requires every menu item to show its **remapped** shortcut. The menus currently hard-code the task.md §10 defaults via SwiftUI `.keyboardShortcut`. Related: those menu items already claim ←/→, ⌘←/⌘→, 0–9, I, J, A, H, E, G, C, P, X, U, ⌘Z, ⇧⌘Z, ⌘O, ⌘↩, so a key-repeat on arrows would move two photos unless the monitor lets the main menu consume the event first. Suggest: `if NSApp.mainMenu?.performKeyEquivalent(with: event) == true { return }` at the top of the monitor. | open |
| REQ-ui-4 | infra | `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon` and `ASSETCATALOG_COMPILER_GLOBAL_ACCENT_COLOR_NAME: AccentColor` in `project.yml` | I created `App/Resources/Assets.xcassets` with both sets; `project.yml` is yours. Without the names the catalog is compiled but unused. | open |
| REQ-ui-5 | app-logic | A way to open a folder by URL, so the Welcome button and drag-and-drop can start a session | `AppEnvironment.open(folder:)` stores `AppEnvironment.pendingFolderURL` and sends `.openFolder`; the model currently can't accept a URL. Needed for §9.6 (welcome window, recents) and §9.7. | open |
| REQ-ui-6 | infra | A root `.swift-format` config, and a decision on 2-space vs 4-space | `swift-format lint --recursive` reports 68 warnings on the **committed baseline** (`App/Sources/Shared/CoreTypes.swift`, the three placeholder tests) because the tool's default is 2-space and the committed style is 4-space. CI runs `swift-format` (§13), so CI would fail on files nobody has touched. I formatted my own files with the tool default (2-space) so my area lints clean. | open |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

- **The view-model seam.** `App/Sources/Views/Model/CullViewState.swift` is a read-only `@MainActor`
  protocol plus value types (`CullPhoto`, `CullBatch`, `CullTier`, `CullProgress`, `CullViewMode`,
  `CullAction`). Every view takes `any CullViewState` and mutates only through
  `send(_:)`. Observation works through the protocol because the access tracking is on the concrete
  `@Observable` object, so there is no need for `@Bindable` anywhere. This is deliberately shaped to
  mirror app-model.md v0.1, and it is the only thing standing between the views and the real
  `AppModel`.
- **Tests:** `xcodebuild … test` green, 29 tests in `App/Tests/Unit/Views/` (5 suites), plus qa's
  two placeholder tests. `swift-format lint` clean for the files I own. Locally run, CI does not
  exist yet.
- **Launching a build by hand:** several agents build a project also called `Firstcut`, so
  `find ~/Library/Developer/Xcode/DerivedData -name Firstcut.app` can hand you **another agent's**
  build. Use `xcodebuild -showBuildSettings | grep BUILT_PRODUCTS_DIR` for your worktree.
- **For qa:** deterministic screenshots need a launch argument that seeds the mock state
  (`PreviewCullViewState(seed:batchCount:)` is already deterministic). I own the app entry point and
  will add `-FirstcutUITestSeed`/`-FirstcutUIBatchCount` in wave 2 unless someone else needs it
  first. Real photos for a UI test are only available through the pipeline, so for now the mock is
  the only way to screenshot a populated filmstrip.
- **Glass:** `GlassBackground` / `GlassCapsule` (`App/Sources/Views/Support/GlassBackground.swift`)
  use `glassEffect` on macOS 26+, `.ultraThinMaterial` on 15, and a plain fill under Reduce
  Transparency. Wave 4 still needs `GlassEffectContainer` grouping and `NSGlassEffectView` for the
  AppKit-hosted pieces; I left no dead `NSGlassEffectView` helper in place for now.
- **Star visuals:** empty stars are not drawn for unrated photos (an unrated frame shows nothing);
  §6.2 keep rings *are* drawn for every frame of the current batch, red when not kept, exactly as
  specified. Worth a second opinion in review — a batch of 60 red rings is loud.
- **Finder reference:** `docs/ui/` has a Finder gallery-view capture next to Firstcut at the same
  size, plus the diff notes. Captured on macOS 27 (this machine); the macOS 15 check needs a VM.
