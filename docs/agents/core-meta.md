# Agent: core-meta

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [docs/README.md](../README.md#protocol-every-agent-follows-this).

## Mission

Read every photo's metadata fast and correctly, for every format in task.md §8, without ever reading whole files.

## End goal (definition of done for this agent)

`scan_folder()` returns a complete `PhotoMeta` for every file in all four test games in < 3 s total (M1 Pro), with every field matching exiftool on the Canon test set; every other format parses from its published spec with header-fixture unit tests and fails gracefully.

**Done means:** `verify` shows zero diffs on the test set; the benchmark is recorded; every format in §8 has a fixture test; the fuzzer runs 10 min with no panics.

## Owns (only you edit these)

`core/firstcut-core/src/scan/`, `src/meta/`, `src/formats/`, `tests/fixtures/headers/`, unit tests in those modules

## Does NOT own

Ordering and batching (core-batch), persistence (core-store), any Swift code.

## task.md sections to read

§3 Measured facts, §7.4 Metadata scan, §8 File format support, §5.1 (which fields ordering needs)

## Contracts

- **Owns:** [photo-meta.md](../contracts/photo-meta.md)
- **Consumes:** build.md
- **Provides to others:** `PhotoMeta`, `scan_folder()`, `meta_from_imageio()`, and a `firstcut verify <folder>` subcommand (request it from core-batch, who owns the CLI crate, or ask to add the file yourself)

## Who the other agents are

| Agent | What they do | Talk to them about |
| --- | --- | --- |
| infra | Build, UniFFI bridge, CI, releases, Homebrew, README | Exporting your types, build breaks, CI |
| core-meta | Metadata parsing for every format | `PhotoMeta` fields |
| core-batch | Ordering, batching, ground truth, CLI | `Batch`, `VisualSig`, fixtures |
| core-store | Session DB, XMP, undo, finish file ops | `Session` API |
| pipeline | Decode, cache, prefetch, viewer layer, zoom | `ImageProvider`, performance |
| app-logic | State model, commands, keymap, rules | `AppModel`, `Command` |
| ui | Every screen, Liquid Glass, Finder look | Layout, visuals |
| qa | Tests, perf baselines, bugs, sign-off | Test hooks, bug reports |

## Deliverables

- [ ] **Wave 1**: freeze photo-meta.md at v1.0 (confirm fields with core-batch, core-store, pipeline, app-logic).
- [ ] **Wave 1**: parallel directory scan (RAW/JPEG/HEIF pairing, `.xmp` detection), CR3 parser: ISO-BMFF boxes, CMT1–CMT4 TIFF IFDs, Canon MakerNote (CameraSettings, ShotInfo, FileNumber, ShutterCount, AFInfo2, SerialNumber, drive/shutter mode), embedded full-size JPEG preview offset/length/dimensions. `pread` headers only.
- [ ] **Wave 2**: `firstcut verify <folder>`: field-by-field diff against `exiftool -j`; zero diffs on all 2,880 test files. Benchmark `< 2 ms/file`, `< 3 s` per game; record in your status.
- [ ] **Wave 2**: CR2 (TIFF + Canon MakerNote) and CRW (CIFF).
- [ ] **Wave 3**: Sony ARW/SR2/SRF (incl. sequence/shot number and AF), then NEF/NRW, RAF, RW2, ORF, PEF, DNG, RWL, 3FR/FFF, IIQ, SRW, DCR/KDC/ERF/MEF/MOS, GPR, X3F (embedded JPEG), JPEG/HEIF/TIFF/PNG EXIF. Each gets a hand-built header byte fixture + unit test. Everything returns `warnings`/`skipped` rather than panicking.
- [ ] **Wave 3**: `meta_from_imageio()` fallback path.
- [ ] **Wave 4**: fuzz the parsers (`cargo fuzz`) with truncated/corrupt headers.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "core-meta" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: docs/README.md, docs/agents/core-meta.md (your charter + status), docs/contracts/build.md,
the contracts listed under "Contracts" in your file, the task.md sections listed in your file, and the
"Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree (../Firstcut-wt/core-meta, branch agent/core-meta) and only on the paths you own.
Pick the next unchecked deliverable, do it, then update your Live status, tick task.md boxes you own,
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

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

(Anything others should know: gotchas, measurements, decisions made inside your area.)
