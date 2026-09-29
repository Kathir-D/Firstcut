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

_Last updated: — (not started)_

### Current focus

Not started. Ready to start (bootstrap done, see task.md §0.7).

### Done log

| Date | What | Commit |
| --- | --- | --- |

### Blockers

None.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

(Anything others should know: gotchas, measurements, decisions made inside your area.)
