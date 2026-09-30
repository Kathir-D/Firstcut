//! Finish Cull: deciding what happens to every file, and then doing it (task.md §9.7).
//!
//! Split in two halves on purpose:
//!
//! * **[`plan_finish`]** is pure: it reads the ratings, works out where every file would go, and
//!   returns a [`FinishPlan`]. It touches the filesystem only to look (sizes, what already exists,
//!   free space), never to change anything, so the app can show the dry-run preview the UI asks for
//!   and be sure the preview is the truth.
//! * **execution** walks the same plan, one [`FileOp`] at a time, logs each one in the session
//!   database, and can walk them backwards to undo. It is written in wave 3, once `Session` exists;
//!   the plan it consumes is already final, which is what keeps the undo log honest.
//!
//! The rules the plan has to honour, all from task.md §9.7:
//!
//! * A photo is a **group**: RAW + paired JPEG/HEIF + `.xmp` sidecar always travel together.
//! * **Never overwrite.** A destination that already exists gets a numeric suffix, and every member
//!   of the group gets the same one, so a group never comes apart.
//! * **Check free space** before copying, and say so in the plan rather than failing halfway.
//! * A **permanent delete is not undoable**, and the plan says so before anything happens.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::store::rating::{Rating, RatingMode, Tier};
use crate::store::records::PhotoRow;

/// What to do with the photos that were not kept (task.md §9.7).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum UnkeptAction {
    /// Leave the files, but write `xmp:Rating="-1"` so Lightroom shows them as rejected.
    MarkRejectedInXmp,
    /// Move them into a subfolder next to the originals.
    MoveToSubfolder(String),
    /// Move them to the Trash, where Finder can recover them.
    MoveToTrash,
    /// Delete them for good. Not undoable.
    DeletePermanently,
    /// Do nothing.
    #[default]
    Nothing,
}

/// What to do with the photos that were kept.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum KeptAction {
    #[default]
    None,
    CopyTo(String),
    MoveTo(String),
    /// One subfolder per tier, named `5 Keep`, `3 Good`, `1 Maybe` (task.md §9.7).
    SplitByTier(String),
    /// One subfolder per star count, named `5`, `4`, `3`, …
    SplitByStars(String),
    /// Write a plain text list of the kept file names.
    WriteList(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishOptions {
    pub unkept: UnkeptAction,
    pub kept: KeptAction,
    pub rating_mode: RatingMode,
    /// Stars that make a keep in stars mode (4 or 5, Settings → General). The session fills it in
    /// from its own setting, like `rating_mode`, so the plan agrees with the filmstrip.
    pub keep_stars: u8,
}

impl Default for FinishOptions {
    fn default() -> Self {
        Self {
            unkept: UnkeptAction::default(),
            kept: KeptAction::default(),
            rating_mode: RatingMode::default(),
            keep_stars: crate::store::rating::KEEP_STARS,
        }
    }
}

/// One physical file operation. The whole group is a run of these, and each one is logged
/// separately so undo can put each file back where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOpKind {
    Move,
    Copy,
    Trash,
    Delete,
    /// Write `xmp:Rating="-1"` into a sidecar. Not a file move, but undoable.
    MarkRejected,
    /// Write the list of kept files.
    WriteList,
}

impl FileOpKind {
    /// Whether [`Session::undo_finish`](crate::session::Session::undo_finish) can reverse this.
    /// A permanent delete cannot: the bytes are gone.
    pub fn is_undoable(&self) -> bool {
        !matches!(self, FileOpKind::Delete)
    }

    /// The string stored in `file_ops.kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            FileOpKind::Move => "move",
            FileOpKind::Copy => "copy",
            FileOpKind::Trash => "trash",
            FileOpKind::Delete => "delete",
            FileOpKind::MarkRejected => "mark_rejected",
            FileOpKind::WriteList => "write_list",
        }
    }

    pub fn parse(kind: &str) -> Option<FileOpKind> {
        Some(match kind {
            "move" => FileOpKind::Move,
            "copy" => FileOpKind::Copy,
            "trash" => FileOpKind::Trash,
            "delete" => FileOpKind::Delete,
            "mark_rejected" => FileOpKind::MarkRejected,
            "write_list" => FileOpKind::WriteList,
            _ => return None,
        })
    }
}

impl fmt::Display for FileOpKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOp {
    pub kind: FileOpKind,
    /// Absolute path, as it is now.
    pub from: String,
    /// Absolute destination, or `None` for operations that have no destination.
    pub to: Option<String>,
}

impl FileOp {
    pub fn new(kind: FileOpKind, from: impl Into<String>, to: Option<String>) -> FileOp {
        FileOp {
            kind,
            from: from.into(),
            to,
        }
    }
}

/// A dry run: everything that would happen, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinishPlan {
    pub ops: Vec<FileOp>,
    /// Bytes that would be copied. Moves and deletes are free.
    pub bytes_to_copy: u64,
    /// Things the user should know before agreeing: no free space, an unvisited batch, a
    /// destination that does not exist yet, a permanent delete.
    pub warnings: Vec<String>,
}

impl FinishPlan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// False as soon as the plan contains a permanent delete, which cannot be undone.
    pub fn is_undoable(&self) -> bool {
        self.ops.iter().all(|op| op.kind.is_undoable())
    }

    pub fn count_of(&self, kind: FileOpKind) -> usize {
        self.ops.iter().filter(|op| op.kind == kind).count()
    }

    /// The one-line-per-operation list the Finish sheet shows before anything is executed.
    pub fn preview_lines(&self) -> Vec<String> {
        self.ops
            .iter()
            .map(|op| match &op.to {
                Some(to) => format!("{} {} → {}", op.kind, file_name(&op.from), file_name(to)),
                None => format!("{} {}", op.kind, file_name(&op.from)),
            })
            .collect()
    }

    /// One summary line per tier, for the totals at the top of the Finish sheet.
    pub fn summary_lines(&self, counts: &HashMap<Tier, usize>) -> Vec<String> {
        Tier::ALL
            .iter()
            .map(|tier| format!("{}: {}", tier, counts.get(tier).copied().unwrap_or(0)))
            .collect()
    }
}

/// What an execution did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinishReport {
    pub done: u32,
    /// `(path, reason)` for each file that could not be done.
    pub failed: Vec<(String, String)>,
    /// False once a permanent delete has run, whatever else happened.
    pub undoable: bool,
}

impl FinishReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Builds the dry run. See the module docs for the rules it follows.
///
/// `unvisited_batches` is how many batches the user never looked at; a non-zero count becomes a
/// warning, because finishing a shoot with unseen bursts is usually a mistake (task.md §9.7).
pub fn plan_finish(
    folder: &Path,
    photos: &[PhotoRow],
    ratings: &HashMap<u64, Rating>,
    options: &FinishOptions,
    unvisited_batches: usize,
) -> FinishPlan {
    let mut plan = FinishPlan::default();
    let mut reserved: HashMap<PathBuf, ()> = HashMap::new();

    if unvisited_batches > 0 {
        plan.warnings.push(format!(
            "{unvisited_batches} batch{} never opened",
            if unvisited_batches == 1 {
                " was"
            } else {
                "es were"
            }
        ));
    }
    if options.unkept == UnkeptAction::DeletePermanently {
        plan.warnings
            .push("Deleting permanently cannot be undone".to_string());
    }

    let mut kept_names: Vec<String> = Vec::new();
    let mut copy_bytes: u64 = 0;
    let mut copy_destinations: Vec<PathBuf> = Vec::new();

    for photo in photos {
        let rating = ratings.get(&photo.id).copied().unwrap_or_default();
        let kept = rating.is_kept_at(options.rating_mode, options.keep_stars);
        let files = group_files(folder, photo);

        if kept {
            match &options.kept {
                KeptAction::None => {}
                KeptAction::CopyTo(root) | KeptAction::MoveTo(root) => {
                    let root = resolve(folder, root);
                    let destination = unique_destination(&root, &photo.rel_path, &mut reserved);
                    if !root.exists() {
                        plan.warnings.push(format!(
                            "{} does not exist yet, it will be created",
                            root.display()
                        ));
                    }
                    // A move to another volume is a copy as far as free space goes.
                    let copies = matches!(options.kept, KeptAction::CopyTo(_));
                    if copies || crosses_volume(folder, &destination) {
                        copy_bytes += files.iter().map(|file| file.size).sum::<u64>();
                        copy_destinations.push(destination.clone());
                    }
                    let kind = if copies {
                        FileOpKind::Copy
                    } else {
                        FileOpKind::Move
                    };
                    plan.ops.extend(destination_ops(kind, &files, &destination));
                    kept_names.push(file_name(&destination.to_string_lossy()));
                }
                KeptAction::SplitByTier(root) => {
                    let subfolder = Tier::Keep.split_dir(options.rating_mode);
                    let destination =
                        split_destination(folder, root, &subfolder, &photo.rel_path, &mut reserved);
                    if crosses_volume(folder, &destination) {
                        copy_bytes += files.iter().map(|file| file.size).sum::<u64>();
                        copy_destinations.push(destination.clone());
                    }
                    plan.ops
                        .extend(destination_ops(FileOpKind::Move, &files, &destination));
                    kept_names.push(file_name(&destination.to_string_lossy()));
                }
                KeptAction::SplitByStars(root) => {
                    let subfolder = rating.stars.to_string();
                    let destination =
                        split_destination(folder, root, &subfolder, &photo.rel_path, &mut reserved);
                    if crosses_volume(folder, &destination) {
                        copy_bytes += files.iter().map(|file| file.size).sum::<u64>();
                        copy_destinations.push(destination.clone());
                    }
                    plan.ops
                        .extend(destination_ops(FileOpKind::Move, &files, &destination));
                    kept_names.push(file_name(&destination.to_string_lossy()));
                }
                KeptAction::WriteList(_) => {
                    kept_names.push(file_name(&photo.rel_path));
                }
            }
        } else {
            match &options.unkept {
                UnkeptAction::Nothing => {}
                UnkeptAction::MoveToSubfolder(root) => {
                    let root = resolve(folder, root);
                    let destination = unique_destination(&root, &photo.rel_path, &mut reserved);
                    if !root.exists() {
                        plan.warnings.push(format!(
                            "{} does not exist yet, it will be created",
                            root.display()
                        ));
                    }
                    plan.ops
                        .extend(destination_ops(FileOpKind::Move, &files, &destination));
                }
                UnkeptAction::MoveToTrash => {
                    for file in &files {
                        plan.ops.push(FileOp::new(
                            FileOpKind::Trash,
                            file.path.to_string_lossy().into_owned(),
                            None,
                        ));
                    }
                }
                UnkeptAction::DeletePermanently => {
                    for file in &files {
                        plan.ops.push(FileOp::new(
                            FileOpKind::Delete,
                            file.path.to_string_lossy().into_owned(),
                            None,
                        ));
                    }
                }
                UnkeptAction::MarkRejectedInXmp => {
                    // The sidecar is the file being changed, so that is what the op points at.
                    let sidecar = PathBuf::from(format!("{}.xmp", photo.rel_path));
                    plan.ops.push(FileOp::new(
                        FileOpKind::MarkRejected,
                        resolve(folder, &sidecar.to_string_lossy())
                            .to_string_lossy()
                            .into_owned(),
                        None,
                    ));
                }
            }
        }
    }

    if let KeptAction::WriteList(path) = &options.kept {
        let destination = resolve(folder, path);
        plan.ops.push(FileOp::new(
            FileOpKind::WriteList,
            file_name(&destination.to_string_lossy()),
            Some(destination.to_string_lossy().into_owned()),
        ));
        plan.warnings.push(format!(
            "Kept list: {} ({} photos)",
            destination.display(),
            kept_names.len()
        ));
    }

    plan.bytes_to_copy = copy_bytes;
    if copy_bytes > 0 {
        check_free_space(&copy_destinations, copy_bytes, &mut plan.warnings);
    }
    plan
}

/// One file of a photo group, with its size on disk.
#[derive(Clone, Debug)]
struct GroupFile {
    path: PathBuf,
    size: u64,
}

/// The RAW, its companions and the sidecar — everything that must travel together.
///
/// A sidecar Firstcut has written is included even if it was not there when the folder was
/// scanned, because a group that left its rating behind would look rejected in Lightroom.
fn group_files(folder: &Path, photo: &PhotoRow) -> Vec<GroupFile> {
    let mut relatives = vec![photo.rel_path.clone()];
    relatives.extend(photo.companions.iter().cloned());

    // A sidecar Firstcut has written is part of the group even if it was not there when the
    // folder was scanned: a group that left its rating behind would look unrated in Lightroom.
    let sidecar = format!("{}.xmp", photo.rel_path);
    if !relatives.contains(&sidecar) {
        relatives.push(sidecar);
    }

    relatives
        .into_iter()
        .filter_map(|relative| {
            let path = resolve(folder, &relative);
            let metadata = std::fs::metadata(&path).ok()?;
            metadata.is_file().then_some(GroupFile {
                path,
                size: metadata.len(),
            })
        })
        .collect()
}

/// One op per file of the group, all sharing the destination's base name.
///
/// The group is renamed as a group: the destination decides the base name, and every member keeps
/// its own extension, so `IMG_0001.CR3`, `IMG_0001.JPG` and `IMG_0001.CR3.xmp` become
/// `IMG_0001-2.CR3`, `IMG_0001-2.JPG` and `IMG_0001-2.CR3.xmp` and never come apart.
fn destination_ops(kind: FileOpKind, files: &[GroupFile], destination: &Path) -> Vec<FileOp> {
    let (Some(directory), Some(stem)) = (destination.parent(), destination.file_stem()) else {
        return Vec::new();
    };

    let mut ops = Vec::with_capacity(files.len());
    for file in files {
        let name = match file.path.extension() {
            Some(extension) => {
                format!("{}.{}", stem.to_string_lossy(), extension.to_string_lossy())
            }
            None => stem.to_string_lossy().into_owned(),
        };
        let to = directory.join(&name);
        // A file that is already where it would be put is not an operation.
        if to == file.path {
            continue;
        }
        ops.push(FileOp::new(
            kind.clone(),
            file.path.to_string_lossy().into_owned(),
            Some(to.to_string_lossy().into_owned()),
        ));
    }
    ops
}

fn split_destination(
    folder: &Path,
    root: &str,
    subfolder: &str,
    rel_path: &str,
    reserved: &mut HashMap<PathBuf, ()>,
) -> PathBuf {
    let directory = resolve(folder, &format!("{root}/{subfolder}"));
    unique_destination(&directory, rel_path, reserved)
}

/// The destination for one photo, never overwriting anything.
///
/// Two cases: a file with that name is already there, or another photo in this same plan is going
/// to be written there. Both get the same treatment — `name-2`, `name-3`, … before the extension —
/// so a group never comes apart and nothing is overwritten.
fn unique_destination(
    directory: &Path,
    rel_path: &str,
    reserved: &mut HashMap<PathBuf, ()>,
) -> PathBuf {
    let name = Path::new(rel_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel_path.to_string());
    let mut candidate = directory.join(&name);
    let mut counter = 1;
    while candidate.exists() || reserved.contains_key(&candidate) {
        counter += 1;
        candidate = directory.join(suffixed(&name, counter));
    }
    reserved.insert(candidate.clone(), ());
    candidate
}

/// `IMG_0001.CR3` + 2 → `IMG_0001-2.CR3`, the way Capture One and Lightroom do it.
pub fn suffixed(name: &str, counter: usize) -> String {
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) => {
            format!(
                "{}-{counter}.{}",
                stem.to_string_lossy(),
                extension.to_string_lossy()
            )
        }
        _ => format!("{name}-{counter}"),
    }
}

/// Relative destinations are taken from the shoot folder, absolute ones as they are.
fn resolve(folder: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        folder.join(path)
    }
}

/// Adds a warning when the copies would not fit. `sampled` is any path on the destination volume.
fn check_free_space(sampled: &[PathBuf], bytes: u64, warnings: &mut Vec<String>) {
    let Some(probe) = sampled.first() else { return };
    let Some(available) = free_space(probe) else {
        return;
    };
    if bytes > available {
        warnings.push(format!(
            "Not enough space at the destination: {} needed, {} available",
            human_bytes(bytes),
            human_bytes(available)
        ));
    }
}

/// Bytes available to this user on the volume holding `path`, or `None` if it cannot be read.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    // The statvfs fields are u32 on macOS and u64 on Linux, so the widening is needed on one.
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    Some(u64::from(stats.f_bavail).saturating_mul(stats.f_frsize as u64))
}

/// `1.4 GB`, for warnings the user has to read at a glance.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Opens a file or folder in Finder.
pub fn reveal_in_finder(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// Opens a folder in Lightroom, if it is installed. Returns whether the request could be made;
/// Lightroom may still refuse a folder it does not know.
pub fn open_in_lightroom(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("/usr/bin/open")
        .arg("-a")
        .arg("Adobe Lightroom Classic")
        .arg(path)
        .spawn()
        .map(|_| ())
}

// -------------------------------------------------------------- execution

/// What one operation actually did. This is what the undo log records, so it is written down even
/// when the result is a failure: "this one did not happen" is the most useful thing to know when
/// undoing the other forty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutedOp {
    pub kind: FileOpKind,
    /// Where the file was before.
    pub src: String,
    /// Where it is now. `None` for deletes and for failures, where it did not move.
    pub dst: Option<String>,
    pub size_bytes: u64,
    /// `done` or `failed`, as stored in `file_ops.status`.
    pub status: &'static str,
    /// Why it failed, when it did. Never swallowed: the report shows every one (task.md §9.7).
    pub error: Option<String>,
}

impl ExecutedOp {
    fn failure(op: &FileOp, reason: impl Into<String>) -> Self {
        Self {
            kind: op.kind.clone(),
            src: op.from.clone(),
            dst: None,
            size_bytes: 0,
            status: "failed",
            error: Some(reason.into()),
        }
    }

    pub fn is_done(&self) -> bool {
        self.status == "done"
    }
}

/// Walks a plan, one operation at a time, and reports what happened.
///
/// The plan is already final, so this half makes no decisions about *what* to do -- only about
/// whether it is still safe to do it. The one decision it does make is the important one: if the
/// destination is now taken, the operation fails rather than overwriting somebody's file. A dry run
/// can be minutes old by the time it is agreed to, and "never overwrite" (task.md §9.7) has to hold
/// at the moment of the write, not the moment of the preview.
///
/// Nothing here is undoable on its own; call [`undo_ops`] with what comes back.
pub fn execute_ops(ops: &[FileOp]) -> Vec<ExecutedOp> {
    ops.iter().map(execute_op).collect()
}

/// A count and a list of what could not be done, for the report the UI shows when it finishes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionSummary {
    pub done: u32,
    pub failed: Vec<(String, String)>,
    /// False as soon as a permanent delete ran, whatever else happened (task.md §9.7).
    pub undoable: bool,
}

impl ExecutionSummary {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
}

pub fn summarize(executed: &[ExecutedOp]) -> ExecutionSummary {
    let mut summary = ExecutionSummary {
        undoable: true,
        ..Default::default()
    };
    for op in executed {
        if op.is_done() {
            summary.done += 1;
        } else if let Some(reason) = &op.error {
            summary.failed.push((op.src.clone(), reason.clone()));
        }
        if op.kind == FileOpKind::Delete && op.is_done() {
            summary.undoable = false;
        }
    }
    summary
}

/// One operation, done.
fn execute_op(op: &FileOp) -> ExecutedOp {
    let source = PathBuf::from(&op.from);
    let size = std::fs::metadata(&source)
        .map(|meta| meta.len())
        .unwrap_or(0);

    // A source that has gone is a failure, not a crash: files move underneath a cull app (Finder,
    // another program, the photographer), and one missing file must not abandon the other forty.
    if !source.exists() {
        return ExecutedOp::failure(op, "the file is no longer there");
    }

    match &op.kind {
        FileOpKind::MarkRejected => mark_rejected(op),
        FileOpKind::WriteList => write_list(op),
        FileOpKind::Move => transfer(op, &source, Move::Rename),
        FileOpKind::Copy => transfer(op, &source, Move::Copy),
        FileOpKind::Trash => trash(op, &source, trash_for(&source)),
        FileOpKind::Delete => match std::fs::remove_file(&source) {
            Ok(()) => ExecutedOp {
                kind: op.kind.clone(),
                src: op.from.clone(),
                dst: None,
                size_bytes: size,
                status: "done",
                error: None,
            },
            Err(error) => ExecutedOp::failure(op, error.to_string()),
        },
    }
}

enum Move {
    Rename,
    Copy,
}

fn transfer(op: &FileOp, source: &Path, how: Move) -> ExecutedOp {
    let Some(target) = op.to.as_ref().map(PathBuf::from) else {
        return ExecutedOp::failure(op, "the plan gave no destination for this file");
    };
    let size = std::fs::metadata(source)
        .map(|meta| meta.len())
        .unwrap_or(0);

    if target.exists() {
        // The safety rule, enforced at the last possible moment.
        return ExecutedOp::failure(
            op,
            format!(
                "{} already exists, so the file was left alone",
                target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| target.to_string_lossy().into_owned())
            ),
        );
    }

    if let Some(parent) = target.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return ExecutedOp::failure(op, format!("could not create the folder: {error}"));
    }

    let result = match how {
        Move::Rename => move_file(source, &target),
        Move::Copy => std::fs::copy(source, &target).map(|_| ()),
    };

    match result {
        Ok(()) => ExecutedOp {
            kind: op.kind.clone(),
            src: op.from.clone(),
            dst: Some(target.to_string_lossy().into_owned()),
            size_bytes: size,
            status: "done",
            error: None,
        },
        Err(error) => ExecutedOp::failure(op, error.to_string()),
    }
}

/// Moves a file into the Trash so Finder can recover it (task.md §9.7: "Trash is recoverable via
/// Finder").
///
/// The plan carries no destination for a trash op: which Trash, and which free name in it, can only
/// be decided now. The name actually used is what the log records, so undo finds the file.
fn trash(op: &FileOp, source: &Path, trash: std::io::Result<PathBuf>) -> ExecutedOp {
    let size = std::fs::metadata(source)
        .map(|meta| meta.len())
        .unwrap_or(0);
    match trash.and_then(|trash| trash_into(source, &trash)) {
        Ok(target) => ExecutedOp {
            kind: op.kind.clone(),
            src: op.from.clone(),
            dst: Some(target.to_string_lossy().into_owned()),
            size_bytes: size,
            status: "done",
            error: None,
        },
        Err(error) => ExecutedOp::failure(op, format!("could not move it to the Trash: {error}")),
    }
}

/// Moves `source` into the `trash` folder under a free name (`IMG_0001-2.CR3` when the name is
/// taken). Nothing is ever overwritten, here or anywhere else in this module.
fn trash_into(source: &Path, trash: &Path) -> std::io::Result<PathBuf> {
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let mut target = trash.join(&name);
    let mut counter = 1;
    while target.exists() {
        counter += 1;
        target = trash.join(suffixed(&name, counter));
    }
    move_file(source, &target)?;
    Ok(target)
}

/// Moves `source` to `target`, across volumes if it has to.
///
/// `rename` is atomic but only works within one file system; a shoot culled straight off a card or
/// an external SSD, with the kept files going to the internal disk, gets `EXDEV` for every file.
/// Then the file is copied, the copy's size checked, and only then is the source removed. A failed
/// copy removes its partial target and leaves the source where it was.
pub fn move_file(source: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::rename(source, target) {
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => copy_then_remove(source, target),
        other => other,
    }
}

fn copy_then_remove(source: &Path, target: &Path) -> std::io::Result<()> {
    if target.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "the destination already exists",
        ));
    }
    let expected = std::fs::metadata(source)?;
    let copied = std::fs::copy(source, target).and_then(|bytes| {
        if bytes != expected.len() {
            return Err(std::io::Error::other(format!(
                "copied {bytes} of {} bytes",
                expected.len()
            )));
        }
        // Keep the capture-era modification time; Finder and other catalogs sort by it.
        if let Ok(modified) = expected.modified() {
            let _ = std::fs::File::options()
                .write(true)
                .open(target)
                .and_then(|file| file.set_modified(modified));
        }
        Ok(())
    });
    if let Err(error) = copied {
        let _ = std::fs::remove_file(target);
        return Err(error);
    }
    std::fs::remove_file(source)
}

/// The Trash for `source`'s volume, where Finder would put it: `~/.Trash` on the home volume,
/// `<volume>/.Trashes/<uid>` on any other (a card, an external disk), so trashing a file is a
/// rename and "Put Back" works. Falls back to `~/.Trash` (a cross-volume move) when the volume's
/// trash cannot be used, e.g. a read-only or foreign file system.
fn trash_for(source: &Path) -> std::io::Result<PathBuf> {
    let home_trash = trash_dir()?;
    let Some(root) = volume_root(source) else {
        return Ok(home_trash);
    };
    if volume_root(&home_trash).as_deref() == Some(root.as_path()) {
        return Ok(home_trash);
    }
    let volume_trash = root.join(".Trashes");
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let mine = volume_trash.join(uid.to_string());
    if mine.is_dir() {
        return Ok(mine);
    }
    let created = (|| {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !volume_trash.is_dir() {
            // Finder's permissions: anyone may add a folder, no one may list or remove others'.
            std::fs::DirBuilder::new()
                .mode(0o1333)
                .create(&volume_trash)?;
            std::fs::set_permissions(&volume_trash, std::fs::Permissions::from_mode(0o1333))?;
        }
        std::fs::DirBuilder::new().mode(0o700).create(&mine)
    })();
    Ok(if created.is_ok() { mine } else { home_trash })
}

/// True when `destination` (which may not exist yet) is on a different volume from `folder`, so
/// moving there copies the bytes.
fn crosses_volume(folder: &Path, destination: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let device = |path: &Path| {
        path.ancestors()
            .find_map(|ancestor| std::fs::metadata(ancestor).ok())
            .map(|meta| meta.dev())
    };
    match (device(folder), device(destination)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

/// The mount point of the file system `path` is on: the highest ancestor on the same device.
fn volume_root(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let path = std::fs::canonicalize(path).ok()?;
    let device = std::fs::metadata(&path).ok()?.dev();
    let mut root = path.clone();
    for ancestor in path.ancestors().skip(1) {
        match std::fs::metadata(ancestor) {
            Ok(meta) if meta.dev() == device => root = ancestor.to_path_buf(),
            _ => break,
        }
    }
    Some(root)
}

fn trash_dir() -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("no HOME, so there is no Trash"))?;
    let trash = home.join(".Trash");
    if !trash.is_dir() {
        std::fs::create_dir_all(&trash)?;
    }
    Ok(trash)
}

/// `xmp:Rating="-1"` on the sidecar next to the file, so Lightroom shows the photo as rejected.
///
/// The sidecar is named after the RAW (`<basename>.xmp`), never after the JPEG: that is the
/// convention Lightroom reads, and a rejection the other catalog does not see is not a rejection.
fn mark_rejected(op: &FileOp) -> ExecutedOp {
    let source = PathBuf::from(&op.from);
    let sidecar = crate::xmp::sidecar_path(&source.to_string_lossy());
    let mapping = crate::xmp::XmpMapping::default();
    let rejected = crate::store::Rating {
        stars: 0,
        flag: crate::store::Flag::Reject,
        label: None,
        keep: false,
    };
    let written = crate::xmp::write_rating(
        &sidecar,
        rejected,
        crate::store::RatingMode::Stars,
        &mapping,
    );
    let size = std::fs::metadata(&sidecar)
        .map(|meta| meta.len())
        .unwrap_or(0);
    match written {
        Ok(_) => ExecutedOp {
            kind: op.kind.clone(),
            src: sidecar.to_string_lossy().into_owned(),
            dst: Some(sidecar.to_string_lossy().into_owned()),
            size_bytes: size,
            status: "done",
            error: None,
        },
        Err(error) => ExecutedOp::failure(op, error.to_string()),
    }
}

/// Writes the kept-files list (task.md §9.7: "a text/CSV list of kept file names").
///
/// The plan carries the *list file* as `from` and the lines to write as `to`, which is the only
/// shape available: a `FileOp` is a pair of paths, and inventing a third field would mean the undo
/// log could not describe the operation it has to reverse.
fn write_list(op: &FileOp) -> ExecutedOp {
    let Some(body) = op.to.as_deref() else {
        return ExecutedOp::failure(op, "the plan gave no list to write");
    };
    let target = PathBuf::from(&op.from);
    if let Some(parent) = target.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return ExecutedOp::failure(op, format!("could not create the folder: {error}"));
    }
    let lines: String = body
        .lines()
        .map(|line| format!("{}\n", line.trim_end()))
        .collect::<Vec<_>>()
        .join("");
    match std::fs::write(&target, lines) {
        Ok(()) => ExecutedOp {
            kind: op.kind.clone(),
            src: op.from.clone(),
            dst: Some(target.to_string_lossy().into_owned()),
            size_bytes: std::fs::metadata(&target)
                .map(|meta| meta.len())
                .unwrap_or(0),
            status: "done",
            error: None,
        },
        Err(error) => ExecutedOp::failure(op, error.to_string()),
    }
}

/// Walks a run backwards, putting every file back where it came from.
///
/// Only the operations that actually happened are reversed, and only those with somewhere to go:
/// a copy is undone by deleting the copy, a move by moving the file back. A permanent delete cannot
/// be reversed, and the caller is told so by `summary.undoable` *before* asking, which is why
/// [`FinishPlan::is_undoable`] and the UI both check it first.
pub fn undo_ops(executed: &[ExecutedOp]) -> Vec<ExecutedOp> {
    executed
        .iter()
        .rev()
        .filter(|op| op.is_done() && op.kind.is_undoable() && op.dst.is_some())
        .map(|op| {
            let target = PathBuf::from(op.dst.as_deref().unwrap_or_default());
            let source = PathBuf::from(&op.src);
            let undone = match op.kind {
                // Something new at the original path (a re-import, a file the user made) is never
                // overwritten to put ours back.
                FileOpKind::Move | FileOpKind::Trash if source.exists() => {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("{} is already there again", op.src),
                    ))
                }
                FileOpKind::Move => move_file(&target, &source),
                FileOpKind::Copy => std::fs::remove_file(&target),
                FileOpKind::Trash => move_file(&target, &source),
                // A sidecar write is reversed by putting the original bytes back, which is only
                // possible if something kept them. Rather than guess, this one reports that it
                // could not be undone, so the UI can say so out loud.
                FileOpKind::MarkRejected | FileOpKind::WriteList => {
                    return ExecutedOp {
                        kind: op.kind.clone(),
                        src: op.src.clone(),
                        dst: op.dst.clone(),
                        size_bytes: op.size_bytes,
                        status: "failed",
                        error: Some("this cannot be undone automatically".to_string()),
                    };
                }
                FileOpKind::Delete => return ExecutedOp::failure_unundoable(op),
            };
            match undone {
                Ok(()) => ExecutedOp {
                    kind: op.kind.clone(),
                    src: op.dst.clone().unwrap_or_default(),
                    dst: Some(op.src.clone()),
                    size_bytes: op.size_bytes,
                    status: "done",
                    error: None,
                },
                Err(error) => ExecutedOp {
                    kind: op.kind.clone(),
                    src: op.src.clone(),
                    dst: op.dst.clone(),
                    size_bytes: op.size_bytes,
                    status: "failed",
                    error: Some(error.to_string()),
                },
            }
        })
        .collect()
}

impl ExecutedOp {
    fn failure_unundoable(op: &ExecutedOp) -> ExecutedOp {
        ExecutedOp {
            kind: op.kind.clone(),
            src: op.src.clone(),
            dst: None,
            size_bytes: 0,
            status: "failed",
            error: Some("a deleted file cannot be brought back".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::rating::{ColorLabel, Flag, Tier};
    use std::fs;

    // ---------------------------------------------------------- execution

    /// The rule the whole module exists to protect: a dry run is a promise, and a file that
    /// appeared between the preview and the click must not be overwritten.
    #[test]
    fn a_move_never_overwrites_a_file_that_appeared_after_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("IMG_0001.CR3");
        let to = dir.path().join("kept").join("IMG_0001.CR3");
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::write(&from, b"the original").unwrap();
        // Something else claimed the destination after the plan was made.
        fs::write(&to, b"somebody else's file").unwrap();

        let done = execute_ops(&[FileOp::new(
            FileOpKind::Move,
            from.to_string_lossy().into_owned(),
            Some(to.to_string_lossy().into_owned()),
        )]);

        assert_eq!(done[0].status, "failed");
        assert!(done[0].error.as_deref().unwrap().contains("already exists"));
        assert_eq!(
            fs::read(&from).unwrap(),
            b"the original",
            "the source is untouched"
        );
        assert_eq!(
            fs::read(&to).unwrap(),
            b"somebody else's file",
            "the destination is untouched"
        );
    }

    #[test]
    fn a_move_takes_the_whole_group_and_undo_puts_every_file_back() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept");
        let sources: Vec<PathBuf> = ["IMG_0001.CR3", "IMG_0001.JPG", "IMG_0001.CR3.xmp"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                fs::write(&path, name.as_bytes()).unwrap();
                path
            })
            .collect();

        let plan: Vec<FileOp> = sources
            .iter()
            .map(|src| {
                let name = src.file_name().unwrap().to_string_lossy().into_owned();
                FileOp::new(
                    FileOpKind::Move,
                    src.to_string_lossy().into_owned(),
                    Some(kept.join(name).to_string_lossy().into_owned()),
                )
            })
            .collect();

        let done = execute_ops(&plan);
        assert_eq!(summarize(&done).done, 3);
        assert!(summarize(&done).is_clean(), "{:?}", summarize(&done).failed);
        for src in &sources {
            assert!(!src.exists(), "{} moved", src.display());
        }
        assert!(kept.join("IMG_0001.CR3").exists());
        assert!(
            kept.join("IMG_0001.JPG").exists(),
            "the JPEG travels with the RAW"
        );
        assert!(
            kept.join("IMG_0001.CR3.xmp").exists(),
            "so does the sidecar"
        );

        let undone = undo_ops(&done);
        assert_eq!(undone.len(), 3);
        for src in &sources {
            assert_eq!(
                fs::read(src).unwrap(),
                src.file_name().unwrap().as_encoded_bytes(),
                "{} came back",
                src.display()
            );
        }
    }

    /// A copy is undone by removing the copy, never by touching the original.
    #[test]
    fn undoing_a_copy_deletes_the_copy_and_leaves_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("IMG_0002.CR3");
        let to = dir.path().join("kept").join("IMG_0002.CR3");
        fs::write(&from, b"original bytes").unwrap();

        let done = execute_ops(&[FileOp::new(
            FileOpKind::Copy,
            from.to_string_lossy().into_owned(),
            Some(to.to_string_lossy().into_owned()),
        )]);
        assert_eq!(done[0].status, "done");
        assert_eq!(fs::read(&to).unwrap(), b"original bytes");

        undo_ops(&done);
        assert!(!to.exists(), "the copy is gone");
        assert_eq!(
            fs::read(&from).unwrap(),
            b"original bytes",
            "the original is untouched"
        );
    }

    /// A file that vanished mid-cull fails that one operation and no other.
    #[test]
    fn one_missing_file_does_not_abandon_the_rest_of_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("IMG_0002.CR3");
        fs::write(&present, b"here").unwrap();
        let kept = dir.path().join("kept");

        let done = execute_ops(&[
            FileOp::new(
                FileOpKind::Move,
                dir.path().join("GONE.CR3").to_string_lossy().into_owned(),
                Some(kept.join("GONE.CR3").to_string_lossy().into_owned()),
            ),
            FileOp::new(
                FileOpKind::Move,
                present.to_string_lossy().into_owned(),
                Some(kept.join("IMG_0002.CR3").to_string_lossy().into_owned()),
            ),
        ]);

        assert_eq!(done[0].status, "failed");
        assert!(
            done[0]
                .error
                .as_deref()
                .unwrap()
                .contains("no longer there")
        );
        assert_eq!(done[1].status, "done", "the second file still moved");
        let summary = summarize(&done);
        assert_eq!(summary.done, 1);
        assert_eq!(summary.failed.len(), 1);
    }

    /// Delete is the one operation that cannot be taken back, and the summary has to say so.
    #[test]
    fn a_permanent_delete_is_reported_as_not_undoable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("IMG_0003.CR3");
        fs::write(&file, b"gone for good").unwrap();

        let done = execute_ops(&[FileOp::new(
            FileOpKind::Delete,
            file.to_string_lossy().into_owned(),
            None,
        )]);
        assert_eq!(done[0].status, "done");
        assert!(!file.exists());
        assert!(
            !summarize(&done).undoable,
            "a delete makes the whole run not undoable"
        );

        let undone = undo_ops(&done);
        assert!(undone.is_empty(), "nothing is even attempted");
    }

    /// Undo of a *mixed* run reverses the reversible parts and leaves the delete alone.
    #[test]
    fn undo_skips_the_delete_and_reverses_the_moves_around_it() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept");
        let moving = dir.path().join("IMG_0004.CR3");
        let deleting = dir.path().join("IMG_0005.CR3");
        fs::write(&moving, b"moved").unwrap();
        fs::write(&deleting, b"deleted").unwrap();

        let done = execute_ops(&[
            FileOp::new(
                FileOpKind::Move,
                moving.to_string_lossy().into_owned(),
                Some(kept.join("IMG_0004.CR3").to_string_lossy().into_owned()),
            ),
            FileOp::new(
                FileOpKind::Delete,
                deleting.to_string_lossy().into_owned(),
                None,
            ),
        ]);
        assert!(!summarize(&done).undoable);

        let undone = undo_ops(&done);
        assert_eq!(undone.len(), 1, "only the move is reversible");
        assert_eq!(fs::read(&moving).unwrap(), b"moved");
    }

    /// A rejected photo must be marked on the sidecar Lightroom reads, without touching the RAW.
    #[test]
    fn mark_rejected_writes_the_sidecar_and_never_the_raw() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("IMG_0006.CR3");
        fs::write(&raw, b"the raw bytes, exactly").unwrap();

        let done = execute_ops(&[FileOp::new(
            FileOpKind::MarkRejected,
            raw.to_string_lossy().into_owned(),
            None,
        )]);

        assert_eq!(done[0].status, "done", "{:?}", done[0].error);
        assert_eq!(
            fs::read(&raw).unwrap(),
            b"the raw bytes, exactly",
            "the RAW is never rewritten"
        );
        let sidecar = dir.path().join("IMG_0006.CR3.xmp");
        assert!(sidecar.exists(), "the sidecar is next to the RAW");
        let text = fs::read_to_string(&sidecar).unwrap();
        assert!(
            text.contains("-1"),
            "xmp:Rating=\"-1\" is what Lightroom reads as rejected: {text}"
        );
    }

    #[test]
    fn a_move_into_a_folder_that_does_not_exist_yet_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("IMG_0007.CR3");
        fs::write(&from, b"x").unwrap();
        let nested = dir.path().join("kept").join("5 Keep");

        let done = execute_ops(&[FileOp::new(
            FileOpKind::Move,
            from.to_string_lossy(),
            Some(nested.join("IMG_0007.CR3").to_string_lossy().into_owned()),
        )]);

        assert_eq!(done[0].status, "done", "{:?}", done[0].error);
        assert!(nested.join("IMG_0007.CR3").exists());
    }

    /// IMG_0001 is a RAW with a paired JPEG and a sidecar already on disk, left unrated.
    /// IMG_0002 is a plain RAW, and the one that gets kept.
    struct Shoot {
        dir: tempfile::TempDir,
    }

    impl Shoot {
        fn new() -> Shoot {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("IMG_0001.CR3"), vec![0u8; 1000]).unwrap();
            fs::write(dir.path().join("IMG_0001.JPG"), vec![0u8; 100]).unwrap();
            fs::write(dir.path().join("IMG_0001.CR3.xmp"), b"<x/>").unwrap();
            fs::write(dir.path().join("IMG_0002.CR3"), vec![0u8; 2000]).unwrap();
            fs::create_dir(dir.path().join("_Not kept")).unwrap();
            Shoot { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn photos(&self) -> Vec<PhotoRow> {
            vec![
                photo(
                    1,
                    "IMG_0001.CR3",
                    1000,
                    vec!["IMG_0001.JPG", "IMG_0001.CR3.xmp"],
                ),
                photo(2, "IMG_0002.CR3", 2000, vec![]),
            ]
        }

        /// The default shoot: the second photo is a 5-star keep.
        fn ratings(&self) -> HashMap<u64, Rating> {
            HashMap::from([(2, Rating::stars(5))])
        }

        fn plan(&self, options: FinishOptions) -> FinishPlan {
            plan_finish(self.path(), &self.photos(), &self.ratings(), &options, 0)
        }
    }

    fn photo(id: u64, rel_path: &str, file_size: u64, companions: Vec<&str>) -> PhotoRow {
        PhotoRow {
            id,
            rel_path: rel_path.to_string(),
            group_key: rel_path.trim_end_matches(".CR3").to_string(),
            companions: companions.into_iter().map(str::to_string).collect(),
            file_size,
            mtime_ms: None,
            device: None,
            ino: None,
            meta_json: None,
            ordinal: Some(id as i64 - 1),
            first_seen_at_ms: 0,
            last_seen_at_ms: 0,
            present: true,
        }
    }

    fn options(unkept: UnkeptAction, kept: KeptAction) -> FinishOptions {
        FinishOptions {
            unkept,
            kept,
            rating_mode: RatingMode::Stars,
            ..Default::default()
        }
    }

    fn file_names(plan: &FinishPlan, needle: &str) -> Vec<String> {
        plan.ops
            .iter()
            .filter(|op| op.from.contains(needle))
            .map(|op| {
                Path::new(&op.from)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    fn destination_folders(plan: &FinishPlan) -> Vec<String> {
        let mut folders: Vec<String> = plan
            .ops
            .iter()
            .filter_map(|op| op.to.as_deref())
            .map(|path| {
                Path::new(path)
                    .parent()
                    .and_then(Path::file_name)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        folders.sort();
        folders.dedup();
        folders
    }

    /// REV-78, at the level where a photo is actually lost. The Finish planner must decide from
    /// the **mapped** rating, the same one the filmstrip draws, and not from the raw `keep` field.
    ///
    /// The reported failure was: a 4-star photo in keep mode showed a green Keep ring and was then
    /// scheduled for the trash. Unrated photos go to `_Not kept`, so "trashed" here means the plan
    /// contains a move for a photo the UI promised would be kept.
    #[test]
    fn a_photo_the_ui_shows_as_kept_is_never_moved_to_not_kept() {
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            for stars in 0..=Rating::MAX_STARS {
                for keep in [false, true] {
                    let shoot = Shoot::new();
                    // Rate the second photo (the one with no companion) and leave the first alone.
                    let rating = Rating::new(stars, Flag::None, None, keep);
                    let ratings = HashMap::from([(2u64, rating)]);

                    let plan = plan_finish(
                        shoot.path(),
                        &shoot.photos(),
                        &ratings,
                        &FinishOptions {
                            unkept: UnkeptAction::MoveToSubfolder("_Not kept".into()),
                            kept: KeptAction::None,
                            rating_mode: mode,
                            ..Default::default()
                        },
                        0,
                    );

                    // What the filmstrip would draw.
                    let shown_as_keep =
                        crate::store::rating::display_tier(&rating, mode) == Tier::Keep;
                    let discarded = file_names(&plan, "IMG_0002");

                    if shown_as_keep {
                        assert!(
                            discarded.is_empty(),
                            "{stars} stars keep={keep} in {mode} draws a Keep ring but Finish \
                             would discard it: {discarded:?}"
                        );
                    } else {
                        assert!(
                            !discarded.is_empty(),
                            "{stars} stars keep={keep} in {mode} is not shown as kept, so Finish \
                             should dispose of it, but produced no operations at all"
                        );
                    }
                }
            }
        }
    }

    /// The same rule for a **permanent delete**, which is the one option that cannot be undone. A
    /// false positive here destroys the photograph, so it gets its own test rather than being
    /// folded into the move test above.
    #[test]
    fn a_photo_the_ui_shows_as_kept_is_never_permanently_deleted() {
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            for stars in 4..=Rating::MAX_STARS {
                let shoot = Shoot::new();
                let rating = Rating::stars(stars);
                assert_eq!(
                    crate::store::rating::display_tier(&rating, mode),
                    Tier::Keep,
                    "{stars} stars in {mode} must read as a keep"
                );

                let plan = plan_finish(
                    shoot.path(),
                    &shoot.photos(),
                    &HashMap::from([(2u64, rating)]),
                    &FinishOptions {
                        unkept: UnkeptAction::DeletePermanently,
                        kept: KeptAction::None,
                        rating_mode: mode,
                        ..Default::default()
                    },
                    0,
                );

                assert!(
                    plan.ops.is_empty() || file_names(&plan, "IMG_0002").is_empty(),
                    "{stars} stars in {mode} would be deleted even though the UI shows it as kept: \
                     {:?}",
                    plan.ops
                );
            }
        }
    }

    /// The Finish **summary** counts tiers, so the count the user approves and the set of files
    /// that are actually kept have to come from the same rule. A summary that says "12 kept" while
    /// the plan trashes 3 of them is the same bug wearing a different hat.
    #[test]
    fn the_kept_count_agrees_with_the_files_kept() {
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            for stars in 0..=Rating::MAX_STARS {
                let shoot = Shoot::new();
                let rating = Rating::stars(stars);
                let kept_by_rating = rating.is_kept(mode);

                let plan = plan_finish(
                    shoot.path(),
                    &shoot.photos(),
                    &HashMap::from([(2u64, rating)]),
                    &FinishOptions {
                        unkept: UnkeptAction::DeletePermanently,
                        kept: KeptAction::None,
                        rating_mode: mode,
                        ..Default::default()
                    },
                    0,
                );
                let actually_deleted = !file_names(&plan, "IMG_0002").is_empty();
                assert_eq!(
                    kept_by_rating, !actually_deleted,
                    "{stars} stars in {mode}: the rating says kept={kept_by_rating} but the plan \
                     deleted={actually_deleted}"
                );
            }
        }
    }

    #[test]
    fn doing_nothing_produces_no_operations() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(UnkeptAction::Nothing, KeptAction::None));
        assert!(plan.is_empty());
        assert_eq!(plan.bytes_to_copy, 0);
        assert!(plan.is_undoable());
    }

    #[test]
    fn a_group_moves_as_one() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(
            UnkeptAction::MoveToSubfolder("_Not kept".into()),
            KeptAction::None,
        ));

        // RAW, paired JPEG and the existing sidecar all travel together.
        let group = file_names(&plan, "IMG_0001");
        assert_eq!(group.len(), 3, "{group:?}");
        assert!(group.contains(&"IMG_0001.CR3".to_string()));
        assert!(group.contains(&"IMG_0001.JPG".to_string()));
        assert!(group.contains(&"IMG_0001.CR3.xmp".to_string()));

        for op in plan.ops.iter().filter(|op| op.from.contains("IMG_0001")) {
            let to = op.to.as_deref().expect("a move has a destination");
            assert!(to.contains("_Not kept"), "{to}");
            assert!(
                to.ends_with(&format!(
                    ".{}",
                    Path::new(&op.from).extension().unwrap().to_string_lossy()
                )),
                "{to} should keep the member's own extension"
            );
        }

        // The kept photo is not touched, and IMG_0002 has no sidecar on disk yet, so its group is
        // just the RAW.
        assert!(file_names(&plan, "IMG_0002").is_empty());
    }

    #[test]
    fn a_sidecar_written_after_the_scan_still_travels_with_the_group() {
        let shoot = Shoot::new();
        // Firstcut has now written a sidecar for the second photo as well.
        fs::write(shoot.path().join("IMG_0002.CR3.xmp"), b"<x:xmpmeta/>").unwrap();
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &HashMap::new(),
            &options(UnkeptAction::MoveToTrash, KeptAction::None),
            0,
        );
        let mut group = file_names(&plan, "IMG_0002");
        group.sort();
        assert_eq!(group, vec!["IMG_0002.CR3", "IMG_0002.CR3.xmp"]);
    }

    #[test]
    fn an_existing_destination_is_never_overwritten() {
        let shoot = Shoot::new();
        fs::write(shoot.path().join("_Not kept/IMG_0001.CR3"), b"mine").unwrap();

        let plan = shoot.plan(options(
            UnkeptAction::MoveToSubfolder("_Not kept".into()),
            KeptAction::None,
        ));
        let destinations: Vec<String> = plan
            .ops
            .iter()
            .filter(|op| op.from.ends_with("IMG_0001.CR3"))
            .filter_map(|op| op.to.clone())
            .collect();
        assert_eq!(destinations.len(), 1);
        assert!(
            destinations[0].ends_with("IMG_0001-2.CR3"),
            "{}",
            destinations[0]
        );
        // The whole group takes the same suffix, so it stays together.
        let jpeg = plan
            .ops
            .iter()
            .find(|op| op.from.ends_with("IMG_0001.JPG"))
            .and_then(|op| op.to.clone())
            .unwrap();
        assert!(jpeg.ends_with("IMG_0001-2.JPG"), "{jpeg}");
    }

    #[test]
    fn two_photos_with_the_same_name_do_not_collide() {
        let shoot = Shoot::new();
        // The same base name again, in a subfolder of the shoot: a second card.
        fs::create_dir(shoot.path().join("card2")).unwrap();
        fs::write(shoot.path().join("card2/IMG_0001.CR3"), vec![0u8; 10]).unwrap();
        let mut photos = shoot.photos();
        photos.push(photo(3, "card2/IMG_0001.CR3", 10, vec![]));

        let plan = plan_finish(
            shoot.path(),
            &photos,
            &HashMap::new(),
            &options(
                UnkeptAction::MoveToSubfolder("_Not kept".into()),
                KeptAction::None,
            ),
            0,
        );
        let destinations: Vec<String> = plan
            .ops
            .iter()
            .filter(|op| op.from.ends_with("card2/IMG_0001.CR3"))
            .filter_map(|op| op.to.clone())
            .collect();
        assert_eq!(destinations.len(), 1);
        assert!(
            destinations[0].ends_with("IMG_0001-2.CR3"),
            "the second one is suffixed: {}",
            destinations[0]
        );
        let first: Vec<String> = plan
            .ops
            .iter()
            .filter(|op| op.from.ends_with("IMG_0001.CR3") && !op.from.contains("card2"))
            .filter_map(|op| op.to.clone())
            .collect();
        assert!(first[0].ends_with("IMG_0001.CR3"), "{}", first[0]);
    }

    #[test]
    fn copying_reports_the_bytes_and_leaves_the_original_alone() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(
            UnkeptAction::Nothing,
            KeptAction::CopyTo("/tmp/firstcut-kept".into()),
        ));
        // Only the kept photo is copied, and only the file itself: it has no companions.
        assert_eq!(plan.bytes_to_copy, 2000);
        assert_eq!(plan.count_of(FileOpKind::Copy), 1);
        assert!(plan.ops[0].to.as_deref().unwrap().contains("firstcut-kept"));
        assert!(
            plan.warnings
                .iter()
                .all(|w| !w.contains("Not enough space"))
        );
    }

    #[test]
    fn a_group_is_copied_whole_sidecar_included() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::stars(4))]);
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &ratings,
            &options(
                UnkeptAction::Nothing,
                KeptAction::CopyTo("/tmp/firstcut-kept".into()),
            ),
            0,
        );
        assert_eq!(
            plan.bytes_to_copy,
            1000 + 100 + 4,
            "RAW, JPEG and the 4-byte sidecar"
        );
        assert_eq!(plan.count_of(FileOpKind::Copy), 3);
    }

    #[test]
    fn moving_is_free_and_deleting_is_not_undoable() {
        let shoot = Shoot::new();
        let moved = shoot.plan(options(
            UnkeptAction::MoveToSubfolder("_Not kept".into()),
            KeptAction::None,
        ));
        assert_eq!(moved.bytes_to_copy, 0);
        assert!(moved.is_undoable());

        let deleted = shoot.plan(options(UnkeptAction::DeletePermanently, KeptAction::None));
        assert_eq!(deleted.count_of(FileOpKind::Delete), 3, "the whole group");
        assert!(!deleted.is_undoable());
        assert!(
            deleted
                .warnings
                .iter()
                .any(|w| w.contains("cannot be undone"))
        );
    }

    #[test]
    fn trashing_produces_one_op_per_file_with_no_destination() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(UnkeptAction::MoveToTrash, KeptAction::None));
        assert_eq!(plan.count_of(FileOpKind::Trash), 3);
        assert!(
            plan.ops
                .iter()
                .all(|op| op.kind != FileOpKind::Trash || op.to.is_none())
        );
    }

    #[test]
    fn marking_rejected_touches_the_sidecar_only() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(UnkeptAction::MarkRejectedInXmp, KeptAction::None));
        assert_eq!(
            plan.count_of(FileOpKind::MarkRejected),
            1,
            "one per unkept photo; the 5-star keep is left alone"
        );
        for op in plan
            .ops
            .iter()
            .filter(|op| op.kind == FileOpKind::MarkRejected)
        {
            assert!(op.from.ends_with(".xmp"), "{}", op.from);
            assert!(op.to.is_none());
        }
        // No photo is moved or deleted.
        assert_eq!(plan.count_of(FileOpKind::Delete), 0);
        assert_eq!(plan.count_of(FileOpKind::Move), 0);
        assert!(plan.is_undoable());
    }

    #[test]
    fn splitting_kept_photos_by_stars_uses_the_star_count_as_the_folder() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::stars(4)), (2, Rating::stars(5))]);
        let photos = shoot.photos();

        let by_stars = plan_finish(
            shoot.path(),
            &photos,
            &ratings,
            &options(
                UnkeptAction::Nothing,
                KeptAction::SplitByStars("/tmp/kept".into()),
            ),
            0,
        );
        assert_eq!(destination_folders(&by_stars), vec!["4", "5"]);
    }

    #[test]
    fn splitting_by_tier_uses_the_documented_folder_name() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::stars(5))]);
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &ratings,
            &options(
                UnkeptAction::Nothing,
                KeptAction::SplitByTier("/tmp/kept".into()),
            ),
            0,
        );
        assert_eq!(destination_folders(&plan), vec!["5 Keep"]);
    }

    #[test]
    fn a_photo_outside_the_keep_tier_is_not_in_the_kept_split() {
        // Only 4 and 5 stars are a "keep" in stars mode (task.md §6.1), so a 3-star photo never
        // reaches a kept action. Whether "split into 5 Keep / 3 Good / 1 Maybe" should also spread
        // the unkept photos is a flow question for app-logic: see the proposal in
        // docs/contracts/session-api.md.
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::stars(5)), (2, Rating::stars(3))]);
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &ratings,
            &options(
                UnkeptAction::Nothing,
                KeptAction::SplitByTier("/tmp/kept".into()),
            ),
            0,
        );
        assert_eq!(destination_folders(&plan), vec!["5 Keep"]);
    }

    #[test]
    fn a_kept_list_is_written_once() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(
            UnkeptAction::Nothing,
            KeptAction::WriteList("kept.txt".into()),
        ));
        let lists: Vec<&FileOp> = plan
            .ops
            .iter()
            .filter(|op| op.kind == FileOpKind::WriteList)
            .collect();
        assert_eq!(lists.len(), 1);
        assert!(lists[0].to.as_deref().unwrap().ends_with("kept.txt"));
        assert_eq!(
            lists[0].from, "kept.txt",
            "the list is named by its file name"
        );
        assert!(
            plan.warnings.iter().any(|w| w.contains("1 photos")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn unvisited_batches_are_a_warning() {
        let shoot = Shoot::new();
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &shoot.ratings(),
            &options(UnkeptAction::Nothing, KeptAction::None),
            3,
        );
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("3 batches were never opened")),
            "{:?}",
            plan.warnings
        );
        let single = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &shoot.ratings(),
            &options(UnkeptAction::Nothing, KeptAction::None),
            1,
        );
        assert!(single.warnings.iter().any(|w| w.contains("1 batch was")));
    }

    #[test]
    fn a_missing_destination_folder_is_announced() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(
            UnkeptAction::MoveToSubfolder("_Not kept yet".into()),
            KeptAction::None,
        ));
        assert!(plan.warnings.iter().any(|w| w.contains("will be created")));
    }

    #[test]
    fn keep_mode_only_keeps_the_kept_ones() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::keep()), (2, Rating::keep_with(false))]);
        let mut opts = options(
            UnkeptAction::MoveToSubfolder("_Not kept".into()),
            KeptAction::None,
        );
        opts.rating_mode = RatingMode::KeepNotKeep;
        let plan = plan_finish(shoot.path(), &shoot.photos(), &ratings, &opts, 0);

        // The not-keep moves with its whole group; the keep stays where it is.
        assert_eq!(file_names(&plan, "IMG_0002"), vec!["IMG_0002.CR3"]);
        assert!(file_names(&plan, "IMG_0001").is_empty());
    }

    #[test]
    fn a_rejected_photo_is_treated_as_unkept() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([(1, Rating::new(5, Flag::Reject, None, false))]);
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &ratings,
            &options(
                UnkeptAction::MoveToSubfolder("_Not kept".into()),
                KeptAction::MoveTo("/tmp/kept".into()),
            ),
            0,
        );
        assert!(
            plan.ops
                .iter()
                .filter(|op| op.from.contains("IMG_0001"))
                .all(|op| op.to.as_deref().unwrap().contains("_Not kept")),
            "a reject is never a keep"
        );
        assert!(
            plan.ops
                .iter()
                .all(|op| !op.to.as_deref().unwrap_or_default().contains("/tmp/kept"))
        );
    }

    /// A second file system for the cross-volume tests, when this machine has one.
    fn other_volume() -> Option<tempfile::TempDir> {
        use std::os::unix::fs::MetadataExt;
        let shm = Path::new("/dev/shm");
        let here = tempfile::tempdir().ok()?;
        let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
        if !shm.is_dir() || dev(shm) == dev(here.path()) {
            return None;
        }
        tempfile::tempdir_in(shm).ok()
    }

    #[test]
    fn a_move_across_volumes_copies_then_removes() {
        let Some(other) = other_volume() else {
            eprintln!("no second file system here; skipped");
            return;
        };
        let here = tempfile::tempdir().unwrap();
        let source = here.path().join("IMG_0001.CR3");
        std::fs::write(&source, vec![7u8; 100_000]).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(old)
            .unwrap();

        let target = other.path().join("IMG_0001.CR3");
        move_file(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read(&target).unwrap(), vec![7u8; 100_000]);
        assert_eq!(std::fs::metadata(&target).unwrap().modified().unwrap(), old);

        // And back, which is what undo does.
        move_file(&target, &source).unwrap();
        assert!(source.exists() && !target.exists());
    }

    #[test]
    fn a_cross_volume_move_never_overwrites() {
        let here = tempfile::tempdir().unwrap();
        let source = here.path().join("a.jpg");
        let target = here.path().join("b.jpg");
        std::fs::write(&source, b"mine").unwrap();
        std::fs::write(&target, b"theirs").unwrap();
        assert!(copy_then_remove(&source, &target).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"mine");
        assert_eq!(std::fs::read(&target).unwrap(), b"theirs");
    }

    #[test]
    fn a_move_to_another_volume_counts_as_bytes_to_copy() {
        let Some(other) = other_volume() else {
            return;
        };
        let shoot = Shoot::new();
        let kept_root = other.path().join("Kept").to_string_lossy().into_owned();
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &HashMap::from([(1u64, Rating::stars(5))]),
            &FinishOptions {
                kept: KeptAction::MoveTo(kept_root),
                ..Default::default()
            },
            0,
        );
        assert!(plan.bytes_to_copy > 0, "{plan:?}");
        assert!(!crosses_volume(shoot.path(), &shoot.path().join("Kept")));
    }

    #[test]
    fn trashing_a_file_moves_it_and_undo_brings_it_back() {
        // The plan has no destination for a trash op (see the test above); executing one used to
        // fail with "the plan gave no destination", so Move to Trash never trashed anything.
        let shoot = tempfile::tempdir().unwrap();
        let trash_folder = tempfile::tempdir().unwrap();
        let source = shoot.path().join("IMG_0001.CR3");
        fs::write(&source, b"raw").unwrap();
        fs::write(trash_folder.path().join("IMG_0001.CR3"), b"older").unwrap();
        let op = FileOp::new(FileOpKind::Trash, source.to_string_lossy(), None);

        let done = trash(&op, &source, Ok(trash_folder.path().to_path_buf()));
        assert!(done.is_done(), "{:?}", done.error);
        assert!(!source.exists());
        let landed = trash_folder.path().join("IMG_0001-2.CR3");
        assert_eq!(done.dst.as_deref(), Some(&*landed.to_string_lossy()));
        assert_eq!(fs::read(&landed).unwrap(), b"raw");
        assert_eq!(
            fs::read(trash_folder.path().join("IMG_0001.CR3")).unwrap(),
            b"older",
            "the file already in the Trash is untouched"
        );

        let undone = undo_ops(&[done]);
        assert_eq!(undone[0].status, "done", "{:?}", undone[0].error);
        assert_eq!(fs::read(&source).unwrap(), b"raw");
    }

    #[test]
    fn the_volume_root_is_the_mount_point() {
        let Some(other) = other_volume() else {
            return;
        };
        let file = other.path().join("x.jpg");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(volume_root(&file).unwrap(), Path::new("/dev/shm"));
    }

    #[test]
    fn undo_does_not_overwrite_a_file_that_came_back() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("IMG_0001.JPG");
        let moved = dir.path().join("_Not kept").join("IMG_0001.JPG");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::write(&moved, b"ours").unwrap();
        std::fs::write(&original, b"new file").unwrap();
        let executed = ExecutedOp {
            kind: FileOpKind::Move,
            src: original.to_string_lossy().into_owned(),
            dst: Some(moved.to_string_lossy().into_owned()),
            size_bytes: 4,
            status: "done",
            error: None,
        };
        let undone = undo_ops(&[executed]);
        assert_eq!(undone[0].status, "failed");
        assert_eq!(std::fs::read(&original).unwrap(), b"new file");
        assert_eq!(std::fs::read(&moved).unwrap(), b"ours");
    }

    #[test]
    fn suffixed_names_follow_the_capture_one_convention() {
        assert_eq!(suffixed("IMG_0001.CR3", 2), "IMG_0001-2.CR3");
        assert_eq!(suffixed("IMG_0001", 3), "IMG_0001-3");
        assert_eq!(suffixed("a.b.c", 2), "a.b-2.c");
    }

    #[test]
    fn file_op_kinds_round_trip_through_the_database_spelling() {
        for kind in [
            FileOpKind::Move,
            FileOpKind::Copy,
            FileOpKind::Trash,
            FileOpKind::Delete,
            FileOpKind::MarkRejected,
            FileOpKind::WriteList,
        ] {
            assert_eq!(FileOpKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(FileOpKind::parse("nonsense"), None);
        assert!(!FileOpKind::Delete.is_undoable());
        assert!(FileOpKind::Trash.is_undoable());
    }

    #[test]
    fn the_preview_is_readable() {
        let shoot = Shoot::new();
        let plan = shoot.plan(options(UnkeptAction::MoveToTrash, KeptAction::None));
        let lines = plan.preview_lines();
        assert!(
            lines.iter().any(|line| line == "trash IMG_0001.CR3"),
            "{lines:?}"
        );

        let counts = HashMap::from([(Tier::Keep, 1), (Tier::Maybe, 1)]);
        assert_eq!(
            plan.summary_lines(&counts),
            vec![
                "Keep: 1",
                "Good: 0",
                "Maybe: 1",
                "Unrated: 0",
                "Rejected: 0"
            ]
        );
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1500), "1.5 KB");
        assert_eq!(human_bytes(42_000_000_000), "42.0 GB");
    }

    #[test]
    fn free_space_is_readable_for_a_real_folder() {
        let shoot = Shoot::new();
        let available = free_space(shoot.path()).expect("a real folder has free space");
        assert!(available > 0);
    }

    #[test]
    fn a_colour_label_does_not_make_a_photo_a_keep() {
        let shoot = Shoot::new();
        let ratings = HashMap::from([
            (1, Rating::stars(5)),
            (2, Rating::new(0, Flag::None, Some(ColorLabel::Red), false)),
        ]);
        let plan = plan_finish(
            shoot.path(),
            &shoot.photos(),
            &ratings,
            &options(UnkeptAction::MoveToTrash, KeptAction::None),
            0,
        );
        assert_eq!(file_names(&plan, "IMG_0002"), vec!["IMG_0002.CR3"]);
        assert!(file_names(&plan, "IMG_0001").is_empty());
    }
}
