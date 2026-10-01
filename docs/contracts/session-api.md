# Contract: Session API (Rust core → Swift)

- **Owner:** core-store
- **Consumers:** app-logic (primary), qa
- **Version:** v0.2 (draft; frozen as v1.0 at the end of wave 1)

The single object Swift talks to for everything that isn't pixels. It wraps core-meta's scan,
core-batch's batching, and core-store's persistence.

## Types

```rust
pub enum Flag { None, Pick, Reject }
pub enum ColorLabel { Red, Yellow, Green, Blue, Purple }

pub struct Rating {
    pub stars: u8,                   // 0..=5 (stars mode)
    pub flag: Flag,
    pub label: Option<ColorLabel>,
    pub keep: bool,                  // keep mode
}

pub struct Cursor { pub batch: BatchId, pub photo: PhotoId }

pub struct Change { pub id: u64, pub photo: PhotoId, pub batch: BatchId, pub batch_index: u32,
                    pub before: Rating, pub after: Rating }

pub struct SessionSnapshot {
    pub folder: String,
    pub photos: Vec<PhotoMeta>,         // in capture order
    pub batches: Vec<Batch>,
    pub ratings: HashMap<PhotoId, Rating>,
    pub visited: HashSet<BatchId>,
    pub cursor: Option<Cursor>,
    pub last_photo_in_batch: HashMap<BatchId, PhotoId>,
}

pub enum UnkeptAction { MarkRejectedInXmp, MoveToSubfolder(String), MoveToTrash, DeletePermanently, Nothing }
pub enum KeptAction { None, CopyTo(String), MoveTo(String), SplitByTier(String), SplitByStars(String), WriteList(String) }
pub struct FinishOptions { pub unkept: UnkeptAction, pub kept: KeptAction, pub rating_mode: RatingMode }
pub struct FileOp { pub kind: FileOpKind, pub from: String, pub to: Option<String> }  // one per physical file (RAW, companions, .xmp)
pub struct FinishPlan { pub ops: Vec<FileOp>, pub bytes_to_copy: u64, pub warnings: Vec<String> }
pub struct FinishReport { pub done: u32, pub failed: Vec<(String, String)>, pub undoable: bool }
```

Also part of this contract, because the UI needs them and they cross the FFI boundary:

```rust
pub enum RatingMode { Stars, KeepNotKeep }
pub enum Tier { Keep, Good, Maybe, Unrated, Rejected }   // §6.1, drives the Finish summary

pub enum FileOpKind { Move, Copy, Trash, Delete, MarkRejected, WriteList }
impl FileOpKind { fn is_undoable(&self) -> bool; }      // false only for Delete

/// How a folder's session database was found. Swift shows a note when it is `Moved`.
pub enum MatchKind { Created, Exact, Moved { from: String } }

/// Settings that change what lands in the sidecars (§6.2). Persisted with the session.
pub struct XmpMapping {
    pub keep_rating: i64,                  // default 5
    pub keep_label: Option<ColorLabel>,    // keep as a colour label instead of a number
    pub not_keep_rating: Option<i64>,      // None = remove the rating; Some(-1) = rejected
    pub not_keep_label: Option<ColorLabel>,
}
```

## Session object

```rust
impl Session {
    pub fn open(folder: String, listener: Box<dyn SessionListener>) -> Result<Session>;  // scan + order + provisional batches + restore DB/XMP
    pub fn snapshot(&self) -> SessionSnapshot;
    pub fn folder(&self) -> &Path;                       // canonical shoot folder
    pub fn matched(&self) -> MatchKind;                  // new, same place, or re-matched after a move
    pub fn rating_mode(&self) -> RatingMode;
    pub fn set_rating_mode(&self, mode: RatingMode);     // existing data is preserved and mapped

    pub fn submit_visual_sigs(&self, sigs: Vec<(PhotoId, VisualSig)>);   // from pipeline; may re-batch unvisited batches → listener.batches_changed

    pub fn set_rating(&self, photo: PhotoId, rating: Rating) -> Change;   // DB now, XMP debounced ≤ 1 s
    pub fn undo(&self) -> Option<Change>;   // the transition made: before = undone value, after = restored
    pub fn redo(&self) -> Option<Change>;   // same shape; a new rating clears the redo stack
                                            // (Swift's SessionBackend.undo returns the change undone, so
                                            //  UniFFICoreSession swaps the pair)

    pub fn set_cursor(&self, cursor: Cursor);
    pub fn mark_visited(&self, batch: BatchId);

    pub fn plan_finish(&self, opts: FinishOptions) -> FinishPlan;        // dry run
    pub fn execute_finish(&self, plan: FinishPlan, progress: Box<dyn Progress>) -> FinishReport; // cancellable
    pub fn undo_finish(&self) -> Result<FinishReport>;

    pub fn flush(&self);                                                 // on quit and on batch change
}

pub trait SessionListener: Send + Sync {
    fn batches_changed(&self, batches: Vec<Batch>);   // only unvisited batches can differ
    fn files_changed(&self);                          // FSEvents: files added/removed
    fn xmp_error(&self, photo: PhotoId, message: String);
    /// The folder moved since the last time this shoot was opened; `from` is where it was.
    fn session_moved(&self, from: String) {}
}

pub trait Progress: Send {
    fn report(&self, done: u32, total: u32, current: &str);
    /// Polled between operations; returning true stops the run cleanly and still reports what
    /// was already done.
    fn is_cancelled(&self) -> bool;
}
```

## Where the state lives

- **Session database**: one SQLite file per shoot in
  `~/Library/Application Support/Firstcut/Sessions/<hash>.sqlite`, in WAL mode. The name is derived
  from the volume identity, the canonical folder path and a fingerprint of the folder's file names
  and sizes, so:
  - the same folder always finds the same database;
  - a **moved or renamed folder** is re-matched by fingerprint (its old path no longer exists) and
    the session comes with it — `Session::matched()` is then `Moved`;
  - a folder whose files **changed while the app was closed** (culled in Finder, a second card
    added, rejects moved into `_Not kept/` by Finish) keeps its session: an earlier database for the
    same path is re-homed when at least one of its photos, and at least a tenth of the smaller of
    the two sets, is still in the folder (matched by file name and size, in any subfolder). A file
    renamed while the app was closed keeps its rating (inode, or shutter count + size);
  - a **reshoot in the same folder** (same path, the old files gone and new ones in) gets its own
    database and the old one is left untouched;
  - a **copy** of a shoot is never confused for a move, because the original is still there.
- **XMP sidecars**: `<basename>.xmp` next to each photo, Lightroom's naming. The database is the
  source of truth for the app; the sidecar is the mirror other tools read.
- **Originals are never modified.** Nothing in the session path opens a photo for writing; the
  Finish step only moves, copies, trashes or deletes whole groups.

## Guarantees

- The "only rate in the current batch" rule is enforced by **app-logic**, not here. The API accepts
  any photo.
- `set_rating` returns in < 1 ms: one synchronous WAL transaction, and the sidecar write is queued
  for a background thread. Repeated changes to one photo collapse into a single sidecar write.
- A crash loses **zero** database writes and **zero** sidecar writes in practice: anything the
  debounce had not written yet is still marked pending in the database and is re-queued on the next
  open. The bound is one debounce window (≤ 1 s) of work repeated, never lost.
- Every rating change is undoable, including across batches: `Change` carries the batch index it was
  made in, so `undo()` can tell app-logic where to navigate before it reverts anything (§6.3).
- `undo_finish` reverses every operation of the last Finish run that is reversible. Permanent
  deletes are not reversible, and `FinishPlan::is_undoable()` / `FinishReport.undoable` say so
  before and after the fact.
- The Finish plan never overwrites: a destination that already exists, or that another photo in the
  same plan claims, gets a `-2`, `-3`, … suffix before the extension, applied to the whole group.
- Opening a folder with sidecars but no database imports the ratings from the sidecars.

## Errors

Anything that can fail returns `Result<SessionError, _>`, never a panic and never a silent no-op:

| Variant | Meaning | What the app should do |
| --- | --- | --- |
| `FolderNotFound` / `NotAFolder` | the path is not a photo folder | tell the user, do nothing |
| `NewerSchema` | the session database was written by a newer Firstcut | refuse to open, suggest updating |
| `NotXmp` | a `.xmp` file is not an XMP packet | report through `xmp_error`, never overwrite it |
| `Io` | sidecar or database could not be written | report and carry on; the rating is safe in the database |
| `BatchError` | core-batch failed | show the error; the session stays closed |

## Mock

app-logic uses `MockSession` (Swift, conforming to the generated protocol) backed by
`tests/fixtures/exiftool/<game>.json` (and later `tests/fixtures/meta/<game>.json`) until the real
core is linked. Use the Swift types in `App/Sources/Shared/CoreTypes.swift`.

Most of those Swift types are field-for-field duplicates of a generated type and are being replaced by
`typealias`es of the generated ones (`build.md`, **The Rust ↔ Swift bridge (UniFFI)**). Six are
genuine app-model types and must **not** be aliased away, because each one says something the core
does not:

| Type | Why it stays |
| --- | --- |
| `SessionData` | the app's cached model state, and its `visited` is a `Set<BatchID>`; UniFFI has no `Set`, so `FfiSessionSnapshot.visited` is a plain array |
| `RatingChange` | the app's contract is "the change that was undone", so `undo()` swaps the generated `before`/`after`; the generated `FfiChange` also carries `batchIndex`, which the app type does not |
| `FinishSettings` | keeps `ratingMode` on purpose — the core plans with its own mode (REV-78), so the app deliberately does not send it |
| `FinishReportData` | carries an app-set `wasUndo`; the generated `FfiFinishReport`'s `nothingToUndo` is a different question and is not surfaced |
| `RatingMode` | app vocabulary `.keep` where the generated enum says `.keepNotKeep` (the raw value is `"keep"` in both) |
| `FileOpKind` | app case `.writeXmp` where the generated enum says `.markRejected` |

## Proposed changes

1. **What "split by tier" covers.** `KeptAction::SplitByTier` is listed under the *kept* photos, but
   todo.md §9.7's example folder names (`5 Keep`, `3 Good`, `1 Maybe`) only make sense if the split
   also spreads the photos that were not kept — and in stars mode only 4–5 stars are a "keep" (§6.1),
   so as written the split can only ever produce a `5 Keep` folder. The planner currently follows the
   contract as written (split applies to kept photos). **app-logic owns the flow rule**: either the
   action becomes global, or it is renamed. Tracked as `REQ-core-store-2`.
2. **`Progress` and cancellation granularity.** The draft above polls cancellation between
   operations, which means a cancelled run can stop between two files of the same group and leave
   that group split. Proposal: cancellation is checked between *groups*, so a group is never left
   half-moved. Needs app-logic's agreement for the sheet's wording ("stop after this photo").
3. **View state in the cursor.** `SessionSnapshot.cursor` covers batch and photo; the `view` column
   in the database also holds opaque UI state (which view, zoom lock). Whether that string is part
   of `Session` or stays entirely in app-logic's own storage is not decided.

## Changelog

- v0.1: initial draft.
- v0.2: matched the contract to the implemented wave-1 storage. Added `RatingMode`, `Tier`,
  `FileOpKind`, `MatchKind`, `XmpMapping`, `Change.batch_index` (for §6.3 cross-batch undo),
  `Session::folder/matched/set_rating_mode`, `SessionListener::session_moved` (with a default
  implementation, so consumers are unaffected), the `Progress` trait, the session-location and
  identity rules, the error table, and the import-from-XMP behaviour. No existing signature changed.
- v0.3 (2026-09-30, after the merge): `Session::plan_finish`, `execute_finish` and `undo_finish` are
  implemented and exported over UniFFI (`FfiFinishOptions` carries no rating mode: the session plans
  with its own, so the preview cannot disagree with the filmstrip about what is kept, REV-78). Every
  operation is logged in `file_ops`, so Undo Finish works after a relaunch; a run that included a
  permanent delete refuses to undo (`SessionError::CannotUndo`). `Rating::tier` is now the *mapped*
  answer (what the filmstrip draws), so a 4-star photo in keep mode and a keep in stars mode are both
  kept. Added `Session::set_xmp_settings` (sidecars on/off, for non-RAW files, keep mapping),
  `import_ratings_from_sidecars` on a first open, and `compute_visual_sig` (the reference signature,
  REV-64). `rescan` on a folder that became empty returns an empty result rather than an error.
- v0.4: **Mock** now says which of the Swift types are genuine app-model types and must not be
  aliased away (`SessionData`, `RatingChange`, `FinishSettings`, `FinishReportData`, `RatingMode`,
  `FileOpKind`), each with the reason it stays. No signature changed; the rest are still on their way
  to being `typealias`es of the generated types.

