//! Which folder a session belongs to, and which file in the sessions directory holds it.
//!
//! task.md §11: the session database is "keyed by volume UUID + folder path + a fingerprint of
//! file names/sizes, so a moved folder can be re-matched". This module produces those three
//! pieces and the database file name derived from them.
//!
//! * **Volume identity** — `VolumeUUID` from `diskutil` when the folder is on a mounted volume we
//!   can ask about, and always the `statfs` filesystem id as a fallback. Two cards holding the
//!   same shoot must not share a session, and a session must not follow a copy of the folder.
//! * **Path** — canonical, so `/Users/me/Shoots/Game1` and `/Users/me/Shoots/./Game1` agree.
//! * **Fingerprint** — FNV-1a 64 over the sorted `(relative path, size)` of every image file in the
//!   folder. Sidecars, `.DS_Store` and other non-image files are excluded on purpose: Firstcut
//!   writes XMP itself, so a folder whose ratings changed must keep the same fingerprint. A
//!   different fingerprint means a different shoot, and therefore a different database file.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::error::{Result, StoreError};

/// Extensions treated as photo files by [`fingerprint`]. Mirrors `FileKind` in
/// docs/contracts/photo-meta.md; core-meta owns the parser's list, and a difference here only
/// changes which files take part in the hash, never whether a folder matches itself.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    // RAW
    "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "gpr", "heic", "heif", "iiq",
    "jpeg", "jpg", "kdc", "mef", "mos", "nef", "nrw", "orf", "pef", "png", "raf", "rw2", "rwl",
    "sr2", "srf", "srw", "tif", "tiff", "x3f",
];

/// FNV-1a, 64 bit. Stable across runs, platforms and Firstcut versions, and dependency-free: this
/// hash identifies folders, it does not defend against anything.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn fnv1a64_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a64(bytes))
}

/// Identity of the volume a folder lives on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeIdentity {
    /// `VolumeUUID` as reported by `diskutil`, when it could be read.
    pub uuid: Option<String>,
    /// `statfs` filesystem id, always available on macOS. Used when there is no UUID.
    pub fsid: Option<String>,
    /// Mount point, e.g. `/Volumes/FlyCaddy`. Handy for telling the user which card a session
    /// belongs to.
    pub mount_point: Option<PathBuf>,
}

impl VolumeIdentity {
    /// The single value the session row is keyed on.
    pub fn key(&self) -> String {
        match (&self.uuid, &self.fsid) {
            (Some(uuid), _) => format!("uuid:{uuid}"),
            (None, Some(fsid)) => format!("fsid:{fsid}"),
            (None, None) => "unknown".to_string(),
        }
    }
}

fn volume_cache() -> &'static Mutex<HashMap<PathBuf, VolumeIdentity>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, VolumeIdentity>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reads the volume identity of `path`.
///
/// `statfs` gives the mount point and filesystem id in one syscall. `diskutil` is only consulted
/// for a real mount point (an external card or disk), which keeps the common internal case free of
/// a subprocess. Results are cached per process: a session is opened once, but tests and the Finish
/// step ask repeatedly.
pub fn volume_identity(path: &Path) -> VolumeIdentity {
    if let Ok(Some(cached)) = volume_cache().lock().map(|cache| cache.get(path).cloned()) {
        return cached;
    }
    let identity = probe_volume(path);
    if let Ok(mut cache) = volume_cache().lock() {
        cache.insert(path.to_path_buf(), identity.clone());
    }
    identity
}

fn probe_volume(path: &Path) -> VolumeIdentity {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return VolumeIdentity::default();
    };
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut stats) } != 0 {
        return VolumeIdentity::default();
    }

    let mount_point = mount_point_of(&stats);

    let fsid = Some(format!(
        "{:08x}{:08x}",
        u32::from_le_bytes(fsid_bytes(&stats.f_fsid)[0..4].try_into().unwrap()),
        u32::from_le_bytes(fsid_bytes(&stats.f_fsid)[4..8].try_into().unwrap()),
    ));

    VolumeIdentity {
        uuid: mount_point.as_deref().and_then(volume_uuid),
        fsid,
        mount_point,
    }
}

/// The mount point, from `statfs`. Only macOS's `statfs` carries `f_mntonname`; elsewhere (the
/// Linux containers this is developed in) there is no mount point and the fsid fallback is used.
#[cfg(target_os = "macos")]
fn mount_point_of(stats: &libc::statfs) -> Option<PathBuf> {
    // `f_mntonname` is a fixed-size C buffer, not a pointer, in `libc`.
    let name: Vec<u8> = stats
        .f_mntonname
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    match String::from_utf8(name) {
        Ok(name) if !name.is_empty() => Some(PathBuf::from(name)),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
fn mount_point_of(_stats: &libc::statfs) -> Option<PathBuf> {
    None
}

/// `libc` keeps `fsid_t`'s field private, but it is a plain `[i32; 2]`, so the eight bytes are read
/// directly. This is the fallback identity, used only when `diskutil` cannot be run.
fn fsid_bytes(fsid: &libc::fsid_t) -> [u8; 8] {
    debug_assert_eq!(
        std::mem::size_of::<libc::fsid_t>(),
        8,
        "libc changed the layout of fsid_t"
    );
    let mut out = [0u8; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(
            fsid as *const libc::fsid_t as *const u8,
            out.as_mut_ptr(),
            out.len(),
        );
    }
    out
}

/// `diskutil info -plist <mount point>` → `VolumeUUID`.
///
/// Parsed with a scan rather than a plist crate: the answer is a single `<key>VolumeUUID</key>`
/// followed by a `<string>`, and a dependency that can fail must not be able to fail here.
fn volume_uuid(mount_point: &Path) -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/diskutil")
        .arg("info")
        .arg("-plist")
        .arg(mount_point)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_plist_string(&String::from_utf8_lossy(&output.stdout), "VolumeUUID")
}

/// Pulls `<string>` out of a property list for one key. Returns `None` for an error dict, a
/// missing key or anything unexpected.
pub fn parse_plist_string(plist: &str, key: &str) -> Option<String> {
    if plist.contains("<key>Error</key>") {
        return None;
    }
    let key_tag = format!("<key>{key}</key>");
    let after = plist.split(&key_tag).nth(1)?;
    let open = after.find("<string>")? + "<string>".len();
    let close = after[open..].find("</string>")? + open;
    let value = after[open..close].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The names-and-sizes hash of a folder, plus the numbers behind it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub hash: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Hashes `(relative path, size)` for every image file in `folder`, recursively.
///
/// Skips dot-files, `._` AppleDouble files and anything that is not an image, so writing XMP
/// sidecars or having Finder drop a `.DS_Store` in the folder cannot invalidate a session.
pub fn fingerprint(folder: &Path) -> Result<Fingerprint> {
    Ok(fingerprint_of(image_files(folder)?))
}

/// `(relative path, size)` for every image file in `folder`, recursively, sorted; the files the
/// fingerprint is taken over.
pub fn image_files(folder: &Path) -> Result<Vec<(String, u64)>> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    let mut stack = vec![folder.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read = std::fs::read_dir(&dir)
            .map_err(|err| StoreError::io(format!("reading folder {}", dir.display()), err))?;
        for entry in read {
            let entry = entry
                .map_err(|err| StoreError::io(format!("reading folder {}", dir.display()), err))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                continue;
            }
            let rel = path
                .strip_prefix(folder)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            entries.push((rel, meta.len()));
        }
    }

    // Sorted so the hash does not depend on directory iteration order.
    entries.sort();
    Ok(entries)
}

fn fingerprint_of(entries: Vec<(String, u64)>) -> Fingerprint {
    let mut bytes: Vec<u8> = Vec::with_capacity(entries.len() * 32);
    let mut total_bytes = 0u64;
    for (rel, size) in &entries {
        bytes.extend_from_slice(rel.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.push(0);
        total_bytes += size;
    }

    Fingerprint {
        hash: fnv1a64_hex(&bytes),
        file_count: entries.len(),
        total_bytes,
    }
}

/// Everything that identifies "the shoot in this folder".
#[derive(Clone, Debug)]
pub struct FolderIdentity {
    /// Canonical absolute path of the folder.
    pub folder: PathBuf,
    pub volume: VolumeIdentity,
    pub fingerprint: Fingerprint,
}

impl FolderIdentity {
    /// Reads everything needed to find or create this folder's session database.
    ///
    /// This walks the folder's files, so it is the one call in `Session::open` that touches every
    /// file. It reads no file contents.
    pub fn detect(folder: &Path) -> Result<FolderIdentity> {
        let canonical = std::fs::canonicalize(folder).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                StoreError::FolderNotFound(folder.to_path_buf())
            } else {
                StoreError::io(format!("opening folder {}", folder.display()), err)
            }
        })?;
        if !canonical.is_dir() {
            return Err(StoreError::NotAFolder(canonical));
        }
        Ok(FolderIdentity {
            volume: volume_identity(&canonical),
            fingerprint: fingerprint(&canonical)?,
            folder: canonical,
        })
    }

    /// Last path component, for the UI.
    pub fn folder_name(&self) -> String {
        self.folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.folder.to_string_lossy().into_owned())
    }

    /// The database file name: a hash of volume + path, plus the short fingerprint so a folder
    /// that has been re-shot (same path, different files) gets its own session instead of
    /// silently taking over the old one.
    pub fn db_file_name(&self) -> String {
        let fp_short: String = self.fingerprint.hash.chars().take(8).collect();
        format!("{}{fp_short}.sqlite", self.db_file_prefix())
    }

    /// The part of [`Self::db_file_name`] that depends only on the volume and the path: every
    /// session ever opened for this folder starts with it.
    pub fn db_file_prefix(&self) -> String {
        let mut key = self.volume.key().into_bytes();
        key.push(0);
        key.extend_from_slice(self.folder.to_string_lossy().as_bytes());
        let path_hash = fnv1a64(&key);
        format!("{path_hash:016x}-")
    }

    pub fn db_path(&self, sessions_dir: &Path) -> PathBuf {
        sessions_dir.join(self.db_file_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(folder: &Path, name: &str, contents: &[u8]) {
        fs::write(folder.join(name), contents).unwrap();
    }

    #[test]
    fn fnv1a64_is_stable() {
        // Locked in so a future refactor cannot silently change every session file name.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64_hex(b"Firstcut"), "b58025ebb7b939e7");
    }

    #[test]
    fn fingerprint_ignores_sidecars_and_junk() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        write(folder, "IMG_0001.CR3", &[0u8; 10]);
        write(folder, "IMG_0002.CR3", &[0u8; 20]);
        write(folder, ".DS_Store", b"junk");
        write(folder, "._IMG_0001.CR3", b"junk");
        write(folder, "IMG_0001.xmp", b"<x/>");
        write(folder, "notes.txt", b"hello");

        let before = fingerprint(folder).unwrap();
        assert_eq!(before.file_count, 2);
        assert_eq!(before.total_bytes, 30);

        // What Firstcut itself does: write sidecars.
        write(folder, "IMG_0001.xmp", b"<x:xmpmeta>rating</x:xmpmeta>");
        write(folder, "IMG_0002.xmp", b"<x:xmpmeta/>");
        assert_eq!(
            fingerprint(folder).unwrap(),
            before,
            "XMP writes must not change the identity"
        );
    }

    #[test]
    fn fingerprint_changes_with_names_and_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        write(folder, "IMG_0001.CR3", &[0u8; 10]);
        let before = fingerprint(folder).unwrap();

        write(folder, "IMG_0002.CR3", &[0u8; 20]);
        let added = fingerprint(folder).unwrap();
        assert_ne!(added.hash, before.hash);
        assert_eq!(added.file_count, 2);

        write(folder, "IMG_0001.CR3", &[0u8; 11]);
        assert_ne!(fingerprint(folder).unwrap().hash, before.hash);
    }

    #[test]
    fn fingerprint_is_order_independent_and_recursive() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path(), "IMG_0002.CR3", &[0u8; 20]);
        write(a.path(), "IMG_0001.CR3", &[0u8; 10]);
        fs::create_dir(a.path().join("sub")).unwrap();
        write(&a.path().join("sub"), "IMG_0003.CR3", &[0u8; 30]);

        fs::create_dir(b.path().join("sub")).unwrap();
        write(&b.path().join("sub"), "IMG_0003.CR3", &[0u8; 30]);
        write(b.path(), "IMG_0001.CR3", &[0u8; 10]);
        write(b.path(), "IMG_0002.CR3", &[0u8; 20]);

        assert_eq!(
            fingerprint(a.path()).unwrap(),
            fingerprint(b.path()).unwrap()
        );
    }

    #[test]
    fn volume_identity_has_a_filesystem_id() {
        let dir = tempfile::tempdir().unwrap();
        let volume = volume_identity(dir.path());
        assert!(
            volume.fsid.is_some(),
            "statfs should always give a filesystem id"
        );
        assert!(volume.key().starts_with("uuid:") || volume.key().starts_with("fsid:"));
    }

    #[test]
    fn plist_parsing_reads_the_uuid_and_rejects_errors() {
        let good = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>DeviceIdentifier</key>
	<string>disk3s5</string>
	<key>VolumeName</key>
	<string>Data</string>
	<key>VolumeUUID</key>
	<string>0EF1B2C3-4D5E-6F70-8192-A3B4C5D6E7F8</string>
</dict>
</plist>"#;
        assert_eq!(
            parse_plist_string(good, "VolumeUUID").as_deref(),
            Some("0EF1B2C3-4D5E-6F70-8192-A3B4C5D6E7F8")
        );
        assert_eq!(
            parse_plist_string(good, "VolumeName").as_deref(),
            Some("Data")
        );
        assert_eq!(parse_plist_string(good, "Nope"), None);

        let failure = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>Error</key>
	<true/>
	<key>ErrorMessage</key>
	<string>Could not find disk: /Users/me/Documents</string>
</dict>
</plist>"#;
        assert_eq!(parse_plist_string(failure, "VolumeUUID"), None);
    }

    #[test]
    fn db_file_name_separates_shoots_in_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        write(folder, "IMG_0001.CR3", &[0u8; 10]);
        let first = FolderIdentity::detect(folder).unwrap();

        write(folder, "IMG_0002.CR3", &[0u8; 10]);
        let second = FolderIdentity::detect(folder).unwrap();

        assert_eq!(first.folder, second.folder, "same path");
        assert_ne!(
            first.db_file_name(),
            second.db_file_name(),
            "different shoot, different db"
        );
        assert!(first.db_file_name().ends_with(".sqlite"));
        assert_eq!(first.db_file_name().len(), 16 + 1 + 8 + ".sqlite".len());
    }

    #[test]
    fn detect_rejects_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let err = FolderIdentity::detect(&dir.path().join("nope")).unwrap_err();
        assert!(matches!(err, StoreError::FolderNotFound(_)), "{err}");
    }

    #[test]
    fn detect_rejects_a_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "not-a-folder.txt", b"x");
        let err = FolderIdentity::detect(&dir.path().join("not-a-folder.txt")).unwrap_err();
        assert!(matches!(err, StoreError::NotAFolder(_)), "{err}");
    }

    #[test]
    fn folder_name_is_the_last_component() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "IMG_0001.CR3", &[0u8; 10]);
        let identity = FolderIdentity::detect(dir.path()).unwrap();
        assert_eq!(
            identity.folder_name(),
            dir.path().file_name().unwrap().to_string_lossy()
        );
    }
}
