//! Displaying a photograph that is not a RAW: PNG and TIFF decode in-process, HEIF through the
//! OS's own converter.
//!
//! A RAW carries JPEGs inside the container and a plain JPEG *is* its own image, so both are
//! answered by [`crate::read_range`]. The scanner also accepts `.png`, `.tif`/`.tiff` and
//! `.heic`/`.heif`/`.hif` as photographs (they get metadata, sidecars and a session row), and v1
//! displayed them through ImageIO. There is no pure-Rust HEVC decoder to put in the binary, so a
//! HEIF is converted by the platform's own tool — `sips` on macOS, WIC through PowerShell on
//! Windows, `heif-convert`/`ffmpeg`/ImageMagick on Linux — and the PNG that comes back is decoded
//! with the `image` crate. A machine without a converter gets a typed error naming what to
//! install, not a blank loupe.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use firstcut_meta::meta::{ByteRange, FileKind, PhotoMeta};

use crate::{DecodeError, DecodedImage, RgbaBuffer, apply_orientation, downsample_to, orientation_swaps_axes};

/// The display image for a photograph that has no embedded JPEG to read.
///
/// `target_edge` is the pane's longer side (`u32::MAX` for the native size). Orientation is
/// applied here unless the platform converter already baked it in, which is detected by the
/// frame's aspect against the stored sensor dimensions.
pub(crate) fn decode_still(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<DecodedImage, DecodeError> {
    let (image, baked_orientation) = match meta.kind {
        FileKind::Png | FileKind::Tiff => (decode_in_process(path)?, false),
        FileKind::Heif => decode_heif(path, meta, target_edge)?,
        // A JPEG is its own image; `read_range` already answers it. Reaching here means the scan
        // did not record `full_preview`, which is the honest fallback rather than an error.
        FileKind::Jpeg => {
            let whole = ByteRange { offset: 0, len: meta.file_size };
            (crate::read_range(path, whole)?, false)
        }
        // A RAW with no embedded JPEG has nothing to display; the caller should have had one.
        FileKind::Raw(_) => return Err(DecodeError::NoEmbeddedJpeg { path: path.display().to_string() }),
    };
    let image = if baked_orientation { image } else { apply_orientation(image, meta.orientation)? };
    Ok(downsample_to(image, target_edge))
}

/// PNG and TIFF are decoded by the `image` crate, which never applies EXIF orientation, so the
/// orientation seam above is the only rotation.
fn decode_in_process(path: &Path) -> Result<DecodedImage, DecodeError> {
    let reader = image::ImageReader::open(path)
        .map_err(|error| DecodeError::Still { path: path.display().to_string(), message: error.to_string() })?
        .with_guessed_format()
        .map_err(|error| DecodeError::Still { path: path.display().to_string(), message: error.to_string() })?;
    let dynamic = reader.decode().map_err(|error| DecodeError::Still { path: path.display().to_string(), message: error.to_string() })?;
    let rgba = dynamic.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(DecodedImage { width, height, rgba: RgbaBuffer::from_bytes(rgba.as_raw()) })
}

/// Converts a HEIF to PNG with the platform's tool, decodes the PNG, and reports whether the
/// converter already applied the EXIF orientation (the caller must not apply it twice).
fn decode_heif(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<(DecodedImage, bool), DecodeError> {
    let converted = temp_png_path();
    let result = convert_heif(path, target_edge, &converted)
        .and_then(|()| std::fs::read(&converted).map_err(|error| format!("reading the converted HEIF: {error}")));
    let _ = std::fs::remove_file(&converted);
    let bytes = result.map_err(|message| DecodeError::Heif { path: path.display().to_string(), message })?;
    let dynamic = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| DecodeError::Heif { path: path.display().to_string(), message: error.to_string() })?;
    let rgba = dynamic.to_rgba8();
    let (width, height) = rgba.dimensions();
    let baked = converter_baked_orientation(width, height, meta);
    Ok((DecodedImage { width, height, rgba: RgbaBuffer::from_bytes(rgba.as_raw()) }, baked))
}

/// A converter that applies the EXIF rotation returns the display aspect; one that only resizes
/// keeps the stored aspect. `meta.width`/`meta.height` are the stored sensor dimensions, so a
/// mismatch means the rotation is already in the pixels.
fn converter_baked_orientation(width: u32, height: u32, meta: &PhotoMeta) -> bool {
    if !orientation_swaps_axes(meta.orientation) || meta.width == 0 || meta.height == 0 {
        return false;
    }
    (width >= height) != (meta.width >= meta.height)
}

/// A unique PNG path in the temp directory. Eight decode workers can convert at once, so the
/// counter and the process id keep two conversions from writing the same file.
fn temp_png_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("firstcut-heif-{}-{stamp}-{count}.png", std::process::id()))
}

/// Runs a converter that writes `out`, and checks that it actually produced the file. The temp
/// target is removed first so a previous attempt's partial output cannot be mistaken for this one.
fn run_converter(program: &str, args: &[OsString], out: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(out);
    match std::process::Command::new(program).args(args).output() {
        Ok(output) if output.status.success() && out.is_file() => Ok(()),
        Ok(output) => Err(format!("{program} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim())),
        Err(error) => Err(format!("could not run {program}: {error}")),
    }
}

/// `sips` ships with macOS, decodes HEIF, and preserves the EXIF orientation tag rather than
/// rotating the pixels (measured: a 400×200 orientation-6 HEIC converts to a 400×200 PNG with
/// the tag intact), so the seam above applies the rotation once. `-Z` asks for the target edge
/// so a 12 MP HEIC does not become a 12 MP PNG just to be reduced.
#[cfg(target_os = "macos")]
fn convert_heif(path: &Path, target_edge: u32, out: &Path) -> Result<(), String> {
    let mut args: Vec<OsString> = vec!["-s".into(), "format".into(), "png".into()];
    if target_edge != u32::MAX && target_edge > 0 {
        args.push("-Z".into());
        args.push(target_edge.to_string().into());
    }
    args.push("--out".into());
    args.push(out.into());
    args.push(path.into());
    run_converter("/usr/bin/sips", &args, out)
}

/// Windows has no command-line HEIF converter, but WIC (through WPF's `BitmapDecoder`) reads HEIF
/// when the HEIF Image Extension is installed, which is the default on Windows 10/11. The script
/// is static; the paths arrive as arguments, never interpolated into it.
#[cfg(windows)]
fn convert_heif(path: &Path, target_edge: u32, out: &Path) -> Result<(), String> {
    const SCRIPT: &str = r#"
param([string]$In, [string]$Out, [int]$Edge)
Add-Type -AssemblyName PresentationCore
$stream = [System.IO.File]::OpenRead($In)
try {
  $decoder = [System.Windows.Media.Imaging.BitmapDecoder]::Create($stream, 'None', 'OnLoad')
  $frame = $decoder.Frames[0]
  $long = [Math]::Max($frame.PixelWidth, $frame.PixelHeight)
  if ($Edge -gt 0 -and $long -gt $Edge) {
    $scale = $Edge / $long
    $transform = New-Object System.Windows.Media.ScaleTransform($scale, $scale)
    $frame = New-Object System.Windows.Media.Imaging.TransformedBitmap($frame, $transform)
  }
  $encoder = New-Object System.Windows.Media.Imaging.PngBitmapEncoder
  $encoder.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($frame))
  $file = [System.IO.File]::Create($Out)
  try { $encoder.Save($file) } finally { $file.Close() }
} finally { $stream.Close() }
"#;
    let script = temp_png_path().with_extension("ps1");
    std::fs::write(&script, SCRIPT).map_err(|error| format!("could not write the HEIF converter script: {error}"))?;
    let args: Vec<OsString> = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.clone().into_os_string(),
        "-In".into(),
        path.into(),
        "-Out".into(),
        out.into(),
        "-Edge".into(),
        target_edge.to_string().into(),
    ];
    let result = run_converter("powershell.exe", &args, out);
    let _ = std::fs::remove_file(&script);
    result
}

/// Linux has no single guaranteed converter. `heif-convert` (libheif-tools) is the closest to
/// standard, `ffmpeg` is common on editing machines, and ImageMagick is the last try. A machine
/// with none of them gets the install hint.
#[cfg(all(unix, not(target_os = "macos")))]
fn convert_heif(path: &Path, target_edge: u32, out: &Path) -> Result<(), String> {
    let _ = target_edge;
    let attempts: [(&str, Vec<OsString>); 4] = [
        ("heif-convert", vec!["-q".into(), "100".into(), path.into(), out.into()]),
        ("ffmpeg", vec!["-y".into(), "-loglevel".into(), "error".into(), "-i".into(), path.into(), "-frames:v".into(), "1".into(), out.into()]),
        ("magick", vec![path.into(), out.into()]),
        ("convert", vec![path.into(), out.into()]),
    ];
    let mut last = String::new();
    for (program, args) in &attempts {
        match run_converter(program, args, out) {
            Ok(()) => return Ok(()),
            Err(error) => last = error,
        }
    }
    Err(format!("no HEIF converter could read it (install libheif-tools, ffmpeg or ImageMagick); last error: {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apply_orientation, decode_display_at};
    use firstcut_meta::meta::{FileKind, PhotoMeta};

    /// A 3×2 RGBA frame whose pixels are all distinct, so a rotation cannot pass by accident.
    fn pixels() -> Vec<u8> {
        (0..6u8).flat_map(|n| [n, n.wrapping_mul(2), n.wrapping_mul(3), 255]).collect()
    }

    fn meta(kind: FileKind, orientation: u8) -> PhotoMeta {
        PhotoMeta {
            id: firstcut_types::PhotoId(1),
            rel_path: "still".to_string(),
            companions: Vec::new(),
            kind,
            file_size: 0,
            capture_time: None,
            shutter_count: None,
            file_number: None,
            camera_make: None,
            camera_model: None,
            camera_serial: None,
            lens_model: None,
            focal_length_mm: None,
            exposure_time_s: None,
            f_number: None,
            iso: None,
            exposure_comp_ev: None,
            metering_mode: None,
            drive_mode: None,
            shutter_mode: None,
            orientation,
            width: 3,
            height: 2,
            af: None,
            preview: None,
            full_preview: None,
            device: None,
            ino: None,
            warnings: Vec::new(),
        }
    }

    fn write_still(name: &str, format: image::ImageFormat) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let frame = image::RgbaImage::from_raw(3, 2, pixels()).unwrap();
        frame.save_with_format(&path, format).unwrap();
        (dir, path)
    }

    #[test]
    fn png_displays_at_its_own_size_with_the_tag_applied_once() {
        let (_dir, path) = write_still("frame.png", image::ImageFormat::Png);
        let upright = decode_still(&path, &meta(FileKind::Png, 1), u32::MAX).unwrap();
        assert_eq!((upright.width, upright.height), (3, 2));
        assert_eq!(upright.rgba.as_slice(), pixels());

        let rotated = decode_still(&path, &meta(FileKind::Png, 6), u32::MAX).unwrap();
        let expected = apply_orientation(upright, 6).unwrap();
        assert_eq!((rotated.width, rotated.height), (2, 3), "orientation 6 swaps the axes");
        assert_eq!(rotated, expected, "the tag must be applied, and only once");
    }

    #[test]
    fn tiff_displays_at_its_own_size_with_the_tag_applied_once() {
        let (_dir, path) = write_still("frame.tif", image::ImageFormat::Tiff);
        let upright = decode_still(&path, &meta(FileKind::Tiff, 1), u32::MAX).unwrap();
        assert_eq!((upright.width, upright.height), (3, 2));
        assert_eq!(upright.rgba.as_slice(), pixels());

        let rotated = decode_still(&path, &meta(FileKind::Tiff, 8), u32::MAX).unwrap();
        let expected = apply_orientation(upright, 8).unwrap();
        assert_eq!((rotated.width, rotated.height), (2, 3));
        assert_eq!(rotated, expected);
    }

    #[test]
    fn the_display_path_reaches_stills_that_have_no_embedded_jpeg() {
        let (_dir, path) = write_still("frame.png", image::ImageFormat::Png);
        let image = decode_display_at(&path, &meta(FileKind::Png, 1), u32::MAX).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
    }

    #[test]
    fn a_raw_with_no_embedded_jpeg_still_reports_that() {
        let error = decode_still(Path::new("/nonexistent.CR3"), &meta(FileKind::Raw(firstcut_meta::meta::RawFormat::Cr3), 1), u32::MAX).unwrap_err();
        assert!(matches!(error, DecodeError::NoEmbeddedJpeg { .. }), "{error}");
    }

    #[test]
    fn a_heif_that_no_converter_can_read_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.heic");
        std::fs::write(&path, b"not a heif at all").unwrap();
        let error = decode_still(&path, &meta(FileKind::Heif, 1), 400).unwrap_err();
        assert!(matches!(error, DecodeError::Heif { .. }), "{error}");
        assert!(error.to_string().contains("broken.heic"), "{error}");
    }

    /// macOS ships a HEIF converter (`sips`), so the real conversion is exercised here rather
    /// than only the error path. A machine whose `sips` cannot write HEIC skips the test.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_real_heic_displays_through_the_platform_converter() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("source.png");
        image::RgbaImage::from_raw(3, 2, pixels()).unwrap().save(&png).unwrap();
        let heic = dir.path().join("photo.heic");
        let Ok(output) = std::process::Command::new("/usr/bin/sips").args(["-s", "format", "heic"]).arg(&png).arg("--out").arg(&heic).output() else {
            eprintln!("no sips here; HEIF conversion not exercised");
            return;
        };
        if !output.status.success() || !heic.is_file() {
            eprintln!("sips cannot write HEIC here; HEIF conversion not exercised");
            return;
        }
        let image = decode_still(&heic, &meta(FileKind::Heif, 1), u32::MAX).unwrap();
        assert_eq!((image.width, image.height), (3, 2), "a HEIC displays at its own size");
    }

    #[test]
    fn a_converter_that_bakes_the_rotation_is_detected_by_aspect() {
        let stored = meta(FileKind::Heif, 6);
        // Stored 3×2, shown 2×3: a converter that rotated returns a taller frame.
        assert!(converter_baked_orientation(2, 3, &stored));
        // A converter that only resized keeps the stored aspect.
        assert!(!converter_baked_orientation(3, 2, &stored));
        // Orientation 1 never swaps, so there is nothing to double-apply.
        let upright = meta(FileKind::Heif, 1);
        assert!(!converter_baked_orientation(2, 3, &upright));
    }

    #[test]
    fn temp_paths_are_unique_so_workers_cannot_share_one_file() {
        let first = temp_png_path();
        let second = temp_png_path();
        assert_ne!(first, second);
    }
}
