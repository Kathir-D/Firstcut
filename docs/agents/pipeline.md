# Agent: pipeline

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Make navigation never wait. Own decoding, caching, prefetch scheduling, the memory budget, and the viewer layer that puts pixels on screen.

## End goal (definition of done for this agent)

Holding → through an entire test game (key repeat) produces **zero** cache misses in the previous/current/next batch, with no memory growth; every §7.3 target is met and recorded; image quality rules in §7.2 hold (native pixel scale, no re-encoding); pinch and click-to-100% zoom work in the viewer layer.

**Done means (and senior-dev has signed it off in `docs/review.md`):** qa's stress test shows 0 focus misses on all four games; every §7.3 row has a measured value that meets the target.

## Owns (only you edit these)

`App/Sources/Pipeline/`, `App/Sources/Render/` (including `PhotoViewerLayerView`), `App/Tests/Unit/Pipeline/`

## Does NOT own

What's selected or which batch comes next (app-logic tells you through `setFocus`), chrome around the viewer (ui), batching (core-batch).

## task.md sections to read

§3 Measured facts, §7 Image pipeline & performance (all), §9.2 Main viewer (zoom, overlays, histogram)

## Contracts

- **Owns:** [pipeline-api.md](../contracts/pipeline-api.md)
- **Consumes:** photo-meta.md (`preview` byte range, orientation), batching.md (VisualSig algorithm), app-model.md (`ViewerState`)
- **Provides to others:** `ImageProvider`, `PhotoViewerLayerView`, VisualSig for every photo, `PipelineStats`

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

- [ ] **Wave 1**: freeze pipeline-api.md at v1.0 with app-logic and ui.
- [ ] **Wave 1**: standalone spike (no Rust needed; list files with `FileManager`): decode embedded previews at viewport size with ImageIO (DCT scaling), measure parallel throughput on 8–10 P-cores, IOSurface upload, display swap time. Record numbers in your status and task.md §3.
- [ ] **Wave 1**: quality benchmark: embedded preview vs `CIRAWFilter` on high-ISO night frames at fit and 100%; recommend defaults (task.md §7.2).
- [ ] **Wave 2**: tiered cache T0–T4, priority scheduler with instant re-prioritization and cancellation, memory budget + pressure handling, whole-shoot thumbnails at `.utility` + VisualSig computation → `Session.submit_visual_sigs`.
- [ ] **Wave 2**: `PhotoViewerLayerView` with IOSurface-backed layers, Retina-exact sizing, resize re-decode.
- [ ] **Wave 3**: pinch zoom (anchored), click → 100% at that spot / click → fit, drag + two-finger scroll panning, click-vs-drag separation, spring animation; zoom lock prefetching T3 neighbors; exact-RAW T4; AF overlay (orientation-mapped), clipping mask, histogram (vImage/Metal).
- [ ] **Wave 4**: tune to every §7.3 target; signposts; debug HUD data.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "pipeline" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/pipeline.md (your charter + status),
docs/review.md (fix every open finding under "pipeline" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree (../Firstcut-wt/pipeline, branch agent/pipeline) and only on the paths you own.
Then pick the next unchecked deliverable, do it, then update your Live status (answer REV findings in
Incoming requests), tick task.md boxes you own,
commit, and push. Ask other agents for anything you need through requests, never by editing their files.
```

---

## Live status

_Last updated: — (not started)_

### Current focus

Not started. Waiting for the owner's go-ahead.

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
