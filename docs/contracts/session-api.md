# Contract: Session API (Rust core → Swift)

- **Owner:** core-store
- **Consumers:** app-logic (primary), qa
- **Version:** v0.1 (draft; frozen as v1.0 at the end of wave 1)

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

pub struct Change { pub id: u64, pub photo: PhotoId, pub batch: BatchId, pub before: Rating, pub after: Rating }

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

## Session object

```rust
impl Session {
    pub fn open(folder: String, listener: Box<dyn SessionListener>) -> Result<Session>;  // scan + order + provisional batches + restore DB/XMP
    pub fn snapshot(&self) -> SessionSnapshot;

    pub fn submit_visual_sigs(&self, sigs: Vec<(PhotoId, VisualSig)>);   // from pipeline; may re-batch unvisited batches → listener.batches_changed

    pub fn set_rating(&self, photo: PhotoId, rating: Rating) -> Change;   // DB now, XMP debounced ≤ 1 s
    pub fn undo(&self) -> Option<Change>;
    pub fn redo(&self) -> Option<Change>;

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
}
```

## Guarantees

- The "only rate in the current batch" rule is enforced by **app-logic**, not here. The API accepts
  any photo.
- `set_rating` returns in < 1 ms (the DB write is synchronous in WAL mode; XMP writes are async).
- A crash loses at most ~1 s of XMP writes and **zero** DB writes.
- Originals are never modified. Every file op moves the whole group (RAW + companions + .xmp).

## Mock

app-logic uses `MockSession` (Swift, conforming to the generated protocol) backed by
`tests/fixtures/meta/<game>.json` until the real core is linked.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
