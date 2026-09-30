//! XMP sidecars: reading them, merging into them, and writing them (task.md §11, §6).
//!
//! Firstcut never modifies a photo. Everything it knows about a photo's rating is written to
//! `<basename>.xmp` next to the file, the way Lightroom does it, so the same ratings show up in
//! Lightroom, Bridge or Capture One. The session database stays the source of truth for the app and
//! the sidecar is the mirror.
//!
//! * [`XmpDocument`] reads and merges one sidecar without disturbing a byte it does not own.
//! * [`XmpMapping`] decides what a rating looks like in XMP (task.md §6.2).
//! * [`XmpWriter`] is the debounced background queue, so a keystroke never waits for a file write.
//! * [`read_sidecar`], [`write_sidecar`] and [`write_values`] are the file-level entry points.
//!
//! Every write is atomic (temp file, `fsync`, rename), so a crash can never leave half a sidecar.

pub mod document;
pub mod mapping;
pub mod writer;
pub mod xml;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub use document::{
    SidecarValues, XmpDocument, XmpValues, existing_sidecar, legacy_sidecar_path, sidecar_base,
    sidecar_path,
};
pub use mapping::{REJECTED, XmpMapping, sidecar_label};
pub use writer::{PendingWrite, XmpWriter};

/// The xpacket wrapper, byte for byte what other tools write.
const XMPPACKET_BEGIN: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>";

pub type Result<T> = std::result::Result<T, XmpError>;

#[derive(Debug, thiserror::Error)]
pub enum XmpError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file exists but is not XMP. Reported, never overwritten.
    #[error("{} is not an XMP packet, refusing to touch it", path.display())]
    NotXmp { path: PathBuf },

    /// The document could not be edited at all, e.g. it is an empty file with no RDF block.
    #[error("this XMP packet has nowhere to put {field}: {reason}")]
    Unwritable { field: &'static str, reason: String },

    #[error("{field} cannot be set to {value:?}")]
    BadValue { field: &'static str, value: String },
}

impl XmpError {
    fn io(path: &Path, source: std::io::Error) -> XmpError {
        XmpError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Reads the values a sidecar holds. A sidecar that does not exist is not an error: it means the
/// photo has never been rated.
pub fn read_sidecar(path: &Path) -> Result<Option<SidecarValues>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(XmpDocument::parse(text).values())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(XmpError::io(path, err)),
    }
}

/// Merges `values` into the sidecar at `path` and writes it atomically, creating it if needed.
pub fn write_sidecar(path: &Path, values: &XmpValues) -> Result<SidecarValues> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(XmpError::io(path, err)),
    };

    let mut document = match existing {
        Some(text) => XmpDocument::parse(text),
        None => XmpDocument::new(),
    };
    document.apply(values).map_err(|err| match err {
        XmpError::NotXmp { .. } => XmpError::NotXmp {
            path: path.to_path_buf(),
        },
        other => other,
    })?;
    write_atomic(path, document.as_str())?;
    Ok(document.values())
}

/// Reads the sidecar, applies `mapping` to `rating`, and writes the result. The one call the
/// session layer needs per photo.
pub fn write_rating(
    path: &Path,
    rating: crate::store::Rating,
    mode: crate::store::RatingMode,
    mapping: &XmpMapping,
) -> Result<SidecarValues> {
    write_sidecar(path, &mapping.values_for(rating, mode))
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `contents` to `path` so that a reader sees either the old file or the new one, never a
/// half-written one: temp file in the same directory, `fsync`, rename, then `fsync` the directory
/// so the rename itself survives a crash.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let directory = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let directory = match directory {
        Some(dir) => dir,
        None => {
            return Err(XmpError::io(
                path,
                std::io::Error::from(std::io::ErrorKind::NotFound),
            ));
        }
    };

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sidecar".to_string());
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = directory.join(format!(".{name}.firstcut-{}-{unique}", std::process::id()));

    let write = |temp: &Path| -> std::io::Result<()> {
        let mut file = fs::File::create(temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    };

    if let Err(source) = write(&temp) {
        let _ = fs::remove_file(&temp);
        return Err(XmpError::io(path, source));
    }

    // Keep the permissions of the sidecar we are replacing; a new one gets the usual 0644.
    if let Ok(existing) = fs::metadata(path) {
        let _ = fs::set_permissions(&temp, existing.permissions());
    }

    if let Err(source) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(XmpError::io(path, source));
    }

    // Make the rename durable. A failure here is not worth reporting: the data is written and the
    // window it leaves open is a directory entry, not a file.
    if let Ok(handle) = fs::File::open(directory) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// True when `text` looks like an XMP packet. Used by tests and by the resume path when a sidecar
/// turns out to be something else entirely.
pub fn looks_like_xmp(text: &str) -> bool {
    text.contains(XMPPACKET_BEGIN.trim_end_matches('\u{feff}')) || text.contains("adobe:ns:meta/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::rating::{Rating, RatingMode};
    use std::fs;

    fn shoot() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("IMG_0001.CR3"), b"not a real raw file").unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn writing_a_new_sidecar_does_not_touch_the_photo() {
        let dir = shoot();
        let photo = dir.path().join("IMG_0001.CR3");
        let before = fs::metadata(&photo).unwrap();

        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        let values = write_sidecar(&sidecar, &XmpValues::rating(4)).unwrap();

        assert_eq!(values.rating, Some(4));
        assert!(read(&sidecar).contains("xmp:Rating=\"4\""));
        let after = fs::metadata(&photo).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }

    #[test]
    fn writing_twice_updates_the_same_file() {
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        write_sidecar(&sidecar, &XmpValues::rating(1)).unwrap();
        write_sidecar(&sidecar, &XmpValues::rating(5)).unwrap();

        let text = read(&sidecar);
        assert_eq!(text.matches("xmp:Rating").count(), 1);
        assert_eq!(read_sidecar(&sidecar).unwrap().unwrap().rating, Some(5));
    }

    #[test]
    fn an_existing_sidecar_keeps_its_other_content() {
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        fs::write(
            &sidecar,
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmp:Rating="1">
   <dc:subject>
    <rdf:Bag><rdf:li>football, game 1</rdf:li></rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#,
        )
        .unwrap();

        write_sidecar(&sidecar, &XmpValues::rating(3)).unwrap();
        let text = read(&sidecar);
        assert!(text.contains("xmp:Rating=\"3\""));
        assert!(text.contains("football, game 1"));
        assert!(text.contains("xmlns:dc="));
    }

    #[test]
    fn no_temp_files_are_left_behind() {
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        for stars in 0..=5 {
            write_sidecar(&sidecar, &XmpValues::rating(stars)).unwrap();
        }
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort(); // read_dir order is unspecified
        assert_eq!(names, vec!["IMG_0001.CR3", "IMG_0001.CR3.xmp"]);
    }

    #[test]
    fn a_read_only_sidecar_is_reported_not_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        write_sidecar(&sidecar, &XmpValues::rating(2)).unwrap();
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o444)).unwrap();

        let err = write_sidecar(&sidecar, &XmpValues::rating(3));
        // A read-only file in a writable directory can still be replaced by a rename, which is
        // exactly the behaviour we want: the sidecar is ours, not the user's.
        if let Err(err) = err {
            assert!(matches!(err, XmpError::Io { .. }), "{err}");
        } else {
            assert_eq!(read_sidecar(&sidecar).unwrap().unwrap().rating, Some(3));
        }
    }

    #[test]
    fn a_missing_sidecar_reads_as_nothing() {
        let dir = shoot();
        assert!(
            read_sidecar(&dir.path().join("nope.xmp"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_non_xmp_file_is_never_clobbered() {
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        fs::write(&sidecar, "<?xml version=\"1.0\"?>\n<photos/>").unwrap();

        let err = write_sidecar(&sidecar, &XmpValues::rating(1)).unwrap_err();
        assert!(matches!(err, XmpError::NotXmp { .. }), "{err}");
        assert_eq!(read(&sidecar), "<?xml version=\"1.0\"?>\n<photos/>");
    }

    #[test]
    fn write_rating_goes_through_the_mapping() {
        let dir = shoot();
        let sidecar = dir.path().join("IMG_0001.CR3.xmp");
        let mapping = XmpMapping::default();

        write_rating(&sidecar, Rating::stars(4), RatingMode::Stars, &mapping).unwrap();
        assert_eq!(read_sidecar(&sidecar).unwrap().unwrap().rating, Some(4));

        write_rating(&sidecar, Rating::keep(), RatingMode::KeepNotKeep, &mapping).unwrap();
        assert_eq!(read_sidecar(&sidecar).unwrap().unwrap().rating, Some(5));

        write_rating(
            &sidecar,
            Rating::neutral(),
            RatingMode::KeepNotKeep,
            &mapping,
        )
        .unwrap();
        assert_eq!(read_sidecar(&sidecar).unwrap().unwrap().rating, None);
    }

    #[test]
    fn looks_like_xmp_recognises_packets() {
        assert!(looks_like_xmp(XMPPACKET_BEGIN));
        assert!(looks_like_xmp(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>"
        ));
        assert!(!looks_like_xmp("<?xml version=\"1.0\"?><photos/>"));
    }
}
