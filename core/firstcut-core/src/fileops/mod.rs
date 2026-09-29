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

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FinishOptions {
    pub unkept: UnkeptAction,
    pub kept: KeptAction,
    pub rating_mode: RatingMode,
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
        let kept = rating.is_kept(options.rating_mode);
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
                    let kind = if matches!(options.kept, KeptAction::CopyTo(_)) {
                        copy_bytes += files.iter().map(|file| file.size).sum::<u64>();
                        copy_destinations.push(destination.clone());
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
                    plan.ops
                        .extend(destination_ops(FileOpKind::Move, &files, &destination));
                    kept_names.push(file_name(&destination.to_string_lossy()));
                }
                KeptAction::SplitByStars(root) => {
                    let subfolder = rating.stars.to_string();
                    let destination =
                        split_destination(folder, root, &subfolder, &photo.rel_path, &mut reserved);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::rating::{ColorLabel, Flag};
    use std::fs;

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
