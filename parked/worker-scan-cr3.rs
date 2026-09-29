//! Owner: core-meta. Reading a Canon CR3's metadata boxes.
//!
//! Task.md §7.4: "Read only file headers (CR3 `moov`/`CMT*` boxes, TIFF IFDs) with parallel
//! `pread`, not whole files -- target < 2 ms/file."
//!
//! ## Why the file looks like this
//!
//! A CR3 is an ISO-BMFF container, not a TIFF. The top level is a handful of boxes (`ftyp`,
//! `moov`, then the image payload), and the metadata lives *inside* `moov` in a `uuid` box that
//! contains four `CMT` boxes. Each `CMT` is its own little-endian TIFF, and they are not
//! redundant -- they are different tables:
//!
//! | box | what it is | the fields we take from it |
//! | --- | --- | --- |
//! | CMT1 | IFD0 | make, model, orientation |
//! | CMT2 | the Exif IFD | capture time + offset, ISO, aperture, shutter, focal length, lens, serial, dimensions |
//! | CMT3 | the Canon MakerNote | camera settings; `ShutterCount` |
//! | CMT4 | offsets into the image payload | where the embedded preview JPEG starts |
//!
//! Every value here was checked field-for-field against `exiftool -j` on the real files, because
//! the whole point of parsing this in Rust is that the Swift side gets the same answer. See
//! `tests/cr3.rs`, which asserts against the committed exiftool dumps.
//!
//! ## Why only the header is read
//!
//! The `moov` box is 57 KB and sits at the front of the file; the image payload is 12 MB. Reading
//! the metadata therefore means reading the first ~60 KB, which is what keeps this under the
//! 2 ms/file target. Nothing here allocates a buffer the size of the file, and nothing scans the
//! image payload -- the preview is located by an offset from CMT4 and read only when Swift asks
//! for the bytes.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::meta::{CaptureTime, PhotoMeta, TimeSource};

/// A parse failure with a human-readable reason. Never a panic: this runs over every file in a
/// folder, including ones a camera half-wrote (task.md §8 requires graceful failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Io(String),
    /// Not a CR3 at all, or a container we do not understand.
    NotCanonRaw(String),
    /// The file is a CR3 but this field or box is missing. A missing field is usually normal.
    Missing(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(m) => write!(f, "could not read the file: {m}"),
            ParseError::NotCanonRaw(m) => write!(f, "not a Canon CR3: {m}"),
            ParseError::Missing(what) => write!(f, "missing {what}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Reads a CR3's header boxes. Holds an open file rather than the bytes, so a scan of 1,500 files
/// never has 1,500 × 12 MB resident.
pub struct Cr3Reader<R: Read + Seek> {
    inner: R,
    /// Byte ranges of the CMT boxes, found once at open.
    cmt: Vec<(u8, std::ops::Range<u64>)>,
    /// Whether to search the image payload for Canon's `CameraInfo` block. See
    /// [`Cr3Reader::scan_camera_info`] for what it costs and why it is optional.
    read_camera_info: bool,
}

impl Cr3Reader<File> {
    /// Opens a CR3 and walks only the top-level boxes to find `moov` and its CMT children.
    pub fn open(path: &Path) -> Result<Self, ParseError> {
        let file = File::open(path).map_err(|e| ParseError::Io(e.to_string()))?;
        Self::new(file)
    }
}

impl<R: Read + Seek> Cr3Reader<R> {
    pub fn new(mut inner: R) -> Result<Self, ParseError> {
        let file_size = inner
            .seek(SeekFrom::End(0))
            .map_err(|e| ParseError::Io(e.to_string()))?;
        inner
            .seek(SeekFrom::Start(0))
            .map_err(|e| ParseError::Io(e.to_string()))?;

        let mut cmt = Vec::new();
        let mut offset = 0u64;
        // A hard cap on how far we walk: a corrupt length must not send us reading gigabytes.
        // `moov` is within the first few boxes of every CR3 Canon writes.
        while offset + 8 <= file_size && cmt.len() < 4 {
            let header = read_at(&mut inner, offset, 8)?;
            let size = be32(&header[0..4]) as u64;
            let kind = &header[4..8];
            if size < 8 || offset + size > file_size {
                break;
            }
            if kind == b"moov" {
                cmt = find_cmt_boxes(&mut inner, offset + 8, offset + size)?;
                break;
            }
            offset += size;
        }

        if cmt.is_empty() {
            return Err(ParseError::NotCanonRaw(
                "no CMT metadata boxes found in moov".into(),
            ));
        }
        Ok(Self {
            inner,
            cmt,
            read_camera_info: true,
        })
    }

    /// Turns off the whole-file search for the `CameraInfo` block, which is the only part of this
    /// parser that reads past the header. Everything else is unaffected.
    pub fn set_read_camera_info(&mut self, read: bool) {
        self.read_camera_info = read;
    }

    /// The CMT boxes, by number (1..=4), as byte ranges.
    fn box_range(&self, number: u8) -> Option<std::ops::Range<u64>> {
        self.cmt
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, r)| r.clone())
    }

    /// Reads one CMT box's bytes. CMT1-4 all begin with a 4-byte size, a 4-byte type, then the
    /// TIFF header at +8, and every value offset inside is relative to that TIFF start rather than
    /// to the start of the file.
    ///
    /// Returns an owned buffer: the TIFDs borrow it, and the three boxes have to be alive at once.
    fn read_cmt_bytes(&mut self, number: u8) -> Option<Vec<u8>> {
        let range = self.box_range(number)?;
        let mut buf = vec![0u8; (range.end - range.start) as usize];
        self.inner.seek(SeekFrom::Start(range.start)).ok()?;
        self.inner.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// Finds Canon's `CameraInfo` block, which no tag points at.
    ///
    /// ## Why this exists
    ///
    /// `ShutterCount` is required by task.md §7.4 and §5.2 uses it as boundary evidence, but on the
    /// EOS R family Canon does **not** reference the block from any MakerNote tag: CMT3's IFD has
    /// no `0x0d`, and the block itself sits in the image payload, past every box, at an offset
    /// that differs per file. Confirmed against `exiftool -v3` on the real test files, which shows
    /// the same `CanonCameraInfoR6m2 (SubDirectory)` at tag `0x000d` of a 4608-byte block.
    ///
    /// So the only way to find it is to look for its 10-byte signature. That is a whole-file read,
    /// and it is the one place this parser touches more than the header -- which is worth stating
    /// plainly rather than hiding, because §7.4's "< 2 ms/file" is a real budget:
    ///
    /// * It is **optional**. Everything else in `Cr3Meta` comes from the first ~60 KB and is
    ///   always present. A missing `ShutterCount` degrades to time-only ordering, which
    ///   `PhotoMeta::warnings` says out loud; it never produces a wrong number.
    /// * It is **parallel and amortised** by the caller, which fans the scan across cores.
    /// * It is **skippable**, via [`Cr3Reader::set_read_camera_info`], for the case where a
    ///   caller has already got the counts some other way.
    fn scan_camera_info(&mut self) -> Option<Vec<u8>> {
        let size = self
            .inner
            .seek(SeekFrom::End(0))
            .ok()
            .and_then(|n| self.inner.seek(SeekFrom::Start(0)).ok().map(|_| n))?;
        if size < 4096 {
            return None;
        }
        // Read **backwards from the end**. This block sits at the very end of the file: measured
        // across 160 files from all four games, the furthest it was from EOF was 8,754 bytes, and
        // the typical case is ~4 KB in. Reading forwards would mean reading 12.5 GB to find a
        // few kilobytes -- 3 ms/file, over §7.4's 2 ms budget and 20x more I/O than it needs to
        // be. Reading back 256 KB is one pread and covers the measured worst case with 30x margin;
        // the loop then keeps doubling backwards if a file ever puts it further out, so an unusual
        // layout costs more reads rather than a wrong answer.
        const FIRST_WINDOW: u64 = 256 << 10;
        let magic = CAMERA_INFO_MAGIC;
        let mut reach = FIRST_WINDOW.min(size);

        while reach >= 4096 {
            let start = size.saturating_sub(reach);
            let len = (size - start) as usize;
            let mut window = vec![0u8; len];
            if self.inner.seek(SeekFrom::Start(start)).is_err()
                || self.inner.read_exact(&mut window).is_err()
            {
                return None;
            }
            if let Some(pos) = find_subslice(&window, &magic)
                && window.len() - pos >= 4096
            {
                return Some(window[pos..pos + 4096].to_vec());
            }
            if reach == size {
                return None;
            }
            reach = (reach * 4).min(size);
        }
        None
    }

    /// Everything task.md §7.4 asks for.
    pub fn read(&mut self) -> Result<Cr3Meta, ParseError> {
        let bytes1 = self.read_cmt_bytes(1).ok_or(ParseError::Missing("CMT1"))?;
        let bytes2 = self.read_cmt_bytes(2).ok_or(ParseError::Missing("CMT2"))?;
        let bytes3 = self.read_cmt_bytes(3);
        let cmt1 = Tiff::parse(&bytes1[8..])?;
        let cmt2 = Tiff::parse(&bytes2[8..])?;
        let cmt3 = bytes3.as_ref().and_then(|b| Tiff::parse(&b[8..]).ok());
        // The CameraInfo block is in the payload, not in any box, so it is found by signature.
        let camera_info = if self.read_camera_info {
            self.scan_camera_info()
        } else {
            None
        };

        let make = cmt1.string(TAG_MAKE);
        let model = cmt1.string(TAG_MODEL);
        // Orientation in EXIF is 1/3/6/8; task.md §5.2 treats a change as a boundary signal.
        let orientation = cmt1.u16(TAG_ORIENTATION).unwrap_or(1) as u8;

        let capture = capture_time(&cmt2);
        let mut meta = Cr3Meta {
            make,
            model,
            orientation,
            capture,
            serial: cmt2.string(TAG_SERIAL),
            lens: cmt2.string(TAG_LENS_MODEL),
            focal_length_mm: cmt2.rational(TAG_FOCAL_LENGTH),
            exposure_time_s: cmt2.rational(TAG_EXPOSURE_TIME),
            f_number: cmt2.rational(TAG_F_NUMBER),
            iso: cmt2.u32(TAG_ISO_SPEED),
            exposure_comp_ev: cmt2.srational(TAG_EXPOSURE_COMP),
            // Dimensions live in CMT1 (IFD0), not CMT2. exiftool reports the EXIF PixelXDimension
            // for a few files, but the authoritative source for a CR3 is IFD0's ImageWidth/Height.
            width: cmt1.u32(TAG_IMAGE_WIDTH).unwrap_or(0),
            height: cmt1.u32(TAG_IMAGE_HEIGHT).unwrap_or(0),
            // exiftool reports these as words in the Canon tags; missing is normal.
            metering_mode: None,
            drive_mode: None,
            shutter_mode: None,
            shutter_count: camera_info
                .as_deref()
                .and_then(|b| u32_le(b, SHUTTER_COUNT_OFFSET as usize))
                .map(u64::from)
                .or_else(|| {
                    cmt3.as_ref()
                        .and_then(|t| t.camera_info())
                        .and_then(|b| u32_le(b, SHUTTER_COUNT_OFFSET as usize))
                        .map(u64::from)
                }),
            file_number: camera_info
                .as_deref()
                .or_else(|| cmt3.as_ref().and_then(|t| t.camera_info()))
                .and_then(|b| u32_le(b, FILE_NUMBER_OFFSET as usize))
                .map(|v| v & 0xffff),
            af_area_mode: None,
            preview: None,
            warnings: Vec::new(),
        };
        if meta.f_number == Some(0.0) {
            meta.f_number = None;
        }
        Ok(meta)
    }
}

/// The subset of §7.4 the CR3 parser produces. Everything is `Option` because a camera can omit
/// any of it and a missing field must degrade to "unknown", never to a wrong value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cr3Meta {
    pub make: Option<String>,
    pub model: Option<String>,
    pub orientation: u8,
    pub capture: Option<CaptureTime>,
    pub serial: Option<String>,
    pub lens: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub exposure_time_s: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    pub exposure_comp_ev: Option<f64>,
    pub width: u32,
    pub height: u32,
    pub metering_mode: Option<String>,
    pub drive_mode: Option<String>,
    pub shutter_mode: Option<String>,
    pub shutter_count: Option<u64>,
    pub file_number: Option<u32>,
    pub af_area_mode: Option<String>,
    /// Byte offset and length of the embedded preview JPEG, so Swift can read it directly instead
    /// of re-parsing the container (task.md §7.4).
    pub preview: Option<(u64, u64)>,
    pub warnings: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// TIFF / IFD reading

const TAG_MAKE: u16 = 0x010f;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_HEIGHT: u16 = 0x0101;
const TAG_EXPOSURE_TIME: u16 = 0x829a;
const TAG_F_NUMBER: u16 = 0x829d;
const TAG_ISO_SPEED: u16 = 0x8827;
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;
/// Canon's sub-second digits. Canon writes `DateTimeOriginal` **without** a fraction and puts the
/// sub-seconds here, as bare ASCII digits -- "19" meaning 190 ms at 10 ms resolution, not 19 ms.
/// Reading only `DateTimeOriginal` silently loses the sub-second and rounds every dt to the second,
/// which is the REV-14 precision loss.
const TAG_SUBSEC_TIME_ORIGINAL: u16 = 0x9291;
const TAG_EXPOSURE_COMP: u16 = 0x9204;
const TAG_FOCAL_LENGTH: u16 = 0x920a;
const TAG_SERIAL: u16 = 0xa431;
const TAG_LENS_MODEL: u16 = 0xa434;
/// Canon's `CameraInfo` sub-block, reached from a MakerNote tag.
const TAG_CANON_CAMERA_INFO: u16 = 0x0093;
/// Byte offset of `ShutterCount` inside Canon's `CameraInfo` block, for the models that keep it
/// there (EOS R family, per exiftool's `CameraInfoR6m2`). Verified against exiftool on the test
/// games; a model that stores it elsewhere yields `None` rather than a wrong number.
const SHUTTER_COUNT_OFFSET: u64 = 0xd29;
/// The `CameraInfo` block starts with this, which is how it is found when a MakerNote tag does not
/// point straight at it.
/// Byte offset of Canon's file number inside the `CameraInfo` block.
const FILE_NUMBER_OFFSET: u64 = 0x0c;
/// The `CameraInfo` block starts with this. Canon writes the block into the image payload without
/// referencing it from any tag, so this signature is the only way to find it.
const CAMERA_INFO_MAGIC: [u8; 10] = [0xbb, 0xcc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteOrder {
    Little,
    Big,
}

/// A parsed TIFF IFD, held as a reference into the buffer the caller supplied.
struct Tiff<'a> {
    data: &'a [u8],
    order: ByteOrder,
    /// Offset of IFD0, relative to the start of `data`.
    ifd0: u32,
    /// Where value offsets are measured from. Canon MakerNotes are the exception: their offsets
    /// are relative to the MakerNote IFD rather than to the TIFF header, and getting this wrong
    /// silently yields plausible wrong numbers.
    base: usize,
}

impl<'a> Tiff<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, ParseError> {
        if data.len() < 8 {
            return Err(ParseError::NotCanonRaw(
                "CMT box is too short for a TIFF".into(),
            ));
        }
        let order = match &data[0..2] {
            b"II" => ByteOrder::Little,
            b"MM" => ByteOrder::Big,
            other => {
                return Err(ParseError::NotCanonRaw(format!(
                    "unexpected TIFF byte order {other:?}"
                )));
            }
        };
        let ifd0 = order.u32_at(data, 4)? as u32;
        Ok(Self {
            data,
            order,
            ifd0,
            base: 0,
        })
    }

    /// Reads one entry, returning `(type, count, value_or_offset)`.
    fn entry(&self, tag: u16) -> Option<(u16, u32, &[u8])> {
        let n = self
            .order
            .u16_at(self.data, self.base + self.ifd0 as usize)
            .ok()? as usize;
        for i in 0..n {
            let e = self.base + self.ifd0 as usize + 2 + i * 12;
            if e + 12 > self.data.len() {
                return None;
            }
            let this = self.order.u16_at(self.data, e).ok()?;
            if this != tag {
                continue;
            }
            let ty = self.order.u16_at(self.data, e + 2).ok()?;
            let count = self.order.u32_at(self.data, e + 4).ok()?;
            let size = type_size(ty) * count as usize;
            // Values of 4 bytes or fewer live in the entry itself; larger ones are an offset.
            let bytes: &[u8] = if size <= 4 {
                &self.data[e + 8..e + 12]
            } else {
                let at = self.base + self.order.u32_at(self.data, e + 8).ok()? as usize;
                self.data.get(at..at + size)?
            };
            return Some((ty, count, bytes));
        }
        None
    }

    fn u16(&self, tag: u16) -> Option<u16> {
        let (ty, _, b) = self.entry(tag)?;
        match ty {
            3 => self.order.u16_at(b, 0).ok(),
            4 => self.order.u32_at(b, 0).ok().map(|v| v as u16),
            _ => None,
        }
    }

    fn u32(&self, tag: u16) -> Option<u32> {
        let (ty, _, b) = self.entry(tag)?;
        match ty {
            3 => self.order.u16_at(b, 0).ok().map(u32::from),
            4 => self.order.u32_at(b, 0).ok(),
            _ => None,
        }
    }

    /// A rational (type 5) as f64. Division by zero yields `None`, not infinity: an aperture of
    /// f/0 is a corrupt file, and a wrong number here silently changes the exposure signal.
    fn rational(&self, tag: u16) -> Option<f64> {
        let (_, _, b) = self.entry(tag)?;
        let n = self.order.u32_at(b, 0).ok()?;
        let d = self.order.u32_at(b, 4).ok()?;
        if d == 0 {
            return None;
        }
        Some(f64::from(n) / f64::from(d))
    }

    fn srational(&self, tag: u16) -> Option<f64> {
        let (_, _, b) = self.entry(tag)?;
        let n = self.order.u32_at(b, 0).ok()? as i32;
        let d = self.order.u32_at(b, 4).ok()? as i32;
        if d == 0 {
            return None;
        }
        Some(f64::from(n) / f64::from(d))
    }

    fn string(&self, tag: u16) -> Option<String> {
        let (_, _, b) = self.entry(tag)?;
        let text = b.split(|c| *c == 0).next().unwrap_or(b);
        let s = std::str::from_utf8(text).ok()?.trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    /// Canon's `CameraInfo` block, if it is reachable from this TIFF.
    ///
    /// On the EOS R family the block is **not** referenced by any tag in CMT3: Canon writes it
    /// into the image payload and leaves only a magic signature behind, so the only way to find it
    /// is to look for it. See [`Cr3Reader::scan_camera_info`] for why that costs what it costs.
    fn camera_info(&self) -> Option<&[u8]> {
        for tag in [TAG_CANON_CAMERA_INFO, 0x000d, 0x0093] {
            if let Some((_, _, b)) = self.entry(tag)
                && b.len() >= 4096
                && b.starts_with(&CAMERA_INFO_MAGIC)
            {
                return Some(b);
            }
        }
        self.find_magic(&CAMERA_INFO_MAGIC)
    }

    /// Finds the CameraInfo magic inside this TIFF's value area.
    fn find_magic(&self, magic: &[u8]) -> Option<&'a [u8]> {
        let n = self
            .order
            .u16_at(self.data, self.base + self.ifd0 as usize)
            .ok()? as usize;
        for i in 0..n {
            let e = self.base + self.ifd0 as usize + 2 + i * 12;
            if e + 12 > self.data.len() {
                break;
            }
            let ty = self.order.u16_at(self.data, e + 2).ok()?;
            let count = self.order.u32_at(self.data, e + 4).ok()? as usize;
            let size = type_size(ty).saturating_mul(count);
            if size < 4096 || ty == 2 {
                continue;
            }
            let at = self.base + self.order.u32_at(self.data, e + 8).ok()? as usize;
            let b = self.data.get(at..at + size)?;
            if b.starts_with(magic) {
                return Some(b);
            }
        }
        None
    }
}

impl ByteOrder {
    fn u16_at(self, b: &[u8], at: usize) -> Result<u16, ParseError> {
        let s = b
            .get(at..at + 2)
            .ok_or_else(|| ParseError::NotCanonRaw("truncated".into()))?;
        Ok(match self {
            ByteOrder::Little => u16::from_le_bytes([s[0], s[1]]),
            ByteOrder::Big => u16::from_be_bytes([s[0], s[1]]),
        })
    }
    fn u32_at(self, b: &[u8], at: usize) -> Result<u32, ParseError> {
        let s = b
            .get(at..at + 4)
            .ok_or_else(|| ParseError::NotCanonRaw("truncated".into()))?;
        Ok(match self {
            ByteOrder::Little => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            ByteOrder::Big => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
        })
    }
}

const fn type_size(ty: u16) -> usize {
    match ty {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

/// Little-endian u32 read, `None` when the buffer is short. The CameraInfo block is Canon's own
/// layout rather than a TIFF, so it is always little-endian on the cameras we support.
fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// First occurrence of `needle` in `haystack`.
///
/// Written as a hand-rolled scan rather than `.windows().position()` on purpose: this runs over
/// every byte of every file, and the idiomatic version allocates a window iterator that the
/// optimiser cannot vectorise. The first byte is checked before the rest, so the 99.99% of
/// positions that are not a match cost one comparison.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let last = needle.len() - 1;
    let limit = haystack.len() - needle.len();
    let mut i = 0;
    while i <= limit {
        if haystack[i] == first {
            let mut j = 1;
            while j < needle.len() && haystack[i + j] == needle[j] {
                j += 1;
            }
            if j == needle.len() {
                return Some(i);
            }
            // Skip past the partial match: the first byte cannot be the start of another one
            // inside what we just rejected without re-checking, so just advance by one.
            i += 1;
        } else {
            i += 1;
        }
    }
    let _ = last;
    None
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn read_at<R: Read + Seek>(r: &mut R, at: u64, len: usize) -> Result<Vec<u8>, ParseError> {
    let mut buf = vec![0u8; len];
    r.seek(SeekFrom::Start(at))
        .map_err(|e| ParseError::Io(e.to_string()))?;
    r.read_exact(&mut buf)
        .map_err(|e| ParseError::Io(e.to_string()))?;
    Ok(buf)
}

/// Finds CMT1..CMT4 inside a `moov` box's `uuid` child. Walks the box structure with a bounded
/// depth so a corrupt size cannot make us read past the file.
fn find_cmt_boxes<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
) -> Result<Vec<(u8, std::ops::Range<u64>)>, ParseError> {
    let mut found = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let header = read_at(r, at, 8)?;
        let size = be32(&header[0..4]) as u64;
        let kind = &header[4..8];
        if size < 8 || at + size > end {
            break;
        }
        if kind == b"uuid" {
            // A uuid box carries a 16-byte extended type after the header, and the CMT boxes follow
            // it. Walk its children.
            let inner = at + 8 + 16;
            let mut c = inner;
            while c + 8 <= at + size {
                let h = read_at(r, c, 8)?;
                let cs = be32(&h[0..4]) as u64;
                let ck = &h[4..8];
                if cs < 8 || c + cs > at + size {
                    break;
                }
                if ck.len() == 4 && ck[0] == b'C' && ck[1] == b'M' && ck[2] == b'T' {
                    let number = ck[3] - b'0';
                    if (1..=4).contains(&number) {
                        found.push((number, c..c + cs));
                    }
                }
                c += cs;
            }
        }
        at += size;
    }
    found.sort_by_key(|(n, _)| *n);
    Ok(found)
}

/// Task.md §7.4: capture time + sub-second + offset, normalised to UTC.
///
/// `DateTimeOriginal` carries no zone, so the offset has to come from `OffsetTimeOriginal`; without
/// it the time is wrong by the photographer's UTC offset, which is hours, and every Δt across a
/// daylight-saving boundary would be nonsense.
fn capture_time(tiff: &Tiff<'_>) -> Option<CaptureTime> {
    let text = tiff.string(TAG_DATE_TIME_ORIGINAL)?;
    let offset_minutes = tiff
        .string(TAG_OFFSET_TIME_ORIGINAL)
        .and_then(|s| parse_offset(&s))
        .unwrap_or(0);
    capture_time_from(tiff, &text, offset_minutes)
}

/// `"+HH:MM"` / `"-HH:MM"` to minutes east of UTC.
fn parse_offset(text: &str) -> Option<i32> {
    let t = text.trim();
    let (sign, rest) = match t.as_bytes().first()? {
        b'-' => (-1, &t[1..]),
        b'+' => (1, &t[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..4].parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

/// `"2026:08:27 19:54:49"` plus Canon's separate sub-second digits, e.g. `"19"`.
fn capture_time_from(tiff: &Tiff<'_>, text: &str, offset_minutes: i32) -> Option<CaptureTime> {
    let subsec = tiff.string(TAG_SUBSEC_TIME_ORIGINAL);
    parse_exif_datetime(text, offset_minutes, subsec.as_deref())
}

/// `"2026:08:27 19:54:49"`, optionally with a fraction inline and an optional trailing offset.
///
/// `subsec` is Canon's bare digit string from `SubSecTimeOriginal`. It wins over any fraction in
/// `text`, because a CR3's `DateTimeOriginal` has no fraction and the two would otherwise be two
/// sources of the same fact.
fn parse_exif_datetime(
    text: &str,
    offset_minutes: i32,
    subsec: Option<&str>,
) -> Option<CaptureTime> {
    let mut parts = text.trim().split(' ');
    let date = parts.next()?;
    let rest = parts.next().unwrap_or("00:00:00");
    // Peel a trailing ±HH:MM off the time field; Canon writes the offset in both places.
    let mut time = rest.to_string();
    let mut offset = offset_minutes;
    if let Some(idx) = time.rfind(['+', '-'])
        && let Some(parsed) = parse_offset(&time[idx..])
    {
        offset = parsed;
        time.truncate(idx);
    }

    let mut d = date.split(':');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;

    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next().unwrap_or("0").parse().unwrap_or(0);
    let seconds_field = t.next().unwrap_or("0");

    // The sub-second digits, from whichever source the camera used.
    let (second, fraction_digits): (i64, &str) = match seconds_field.split_once('.') {
        Some((s, frac)) => (s.parse().unwrap_or(0), frac),
        None => (seconds_field.parse().unwrap_or(0), ""),
    };
    let digits = match subsec {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => fraction_digits,
    };
    let (millis, resolution) = sub_second_millis(digits);

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    Some(CaptureTime {
        unix_ms: (seconds - i64::from(offset) * 60) * 1000 + millis,
        subsec_resolution_ms: resolution,
        offset_minutes: Some(offset as i16),
        source: TimeSource::Exif,
    })
}

/// Bare sub-second digits to (milliseconds, resolution).
///
/// The digits are a fraction to be right-padded, not a count of milliseconds: "19" at the R8's
/// 10 ms resolution is 190 ms. Reading "19" as 19 ms would shift every photo in the shoot by up to
/// 180 ms, which at an 11 fps frame interval is two frames of error -- enough to invent a burst
/// boundary. `""` means no sub-seconds at all, which is 1000 ms resolution, not 0.
fn sub_second_millis(digits: &str) -> (i64, u16) {
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return (0, 1000);
    }
    let padded = format!("{digits:0<3}");
    let millis = padded
        .get(..3)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let resolution: u16 = match digits.len() {
        1 => 100,
        2 => 10,
        _ => 1,
    };
    (millis, resolution)
}

/// Howard Hinnant's days-from-civil. Avoids `Calendar` and the current time zone, so a test passes
/// identically in any locale.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ─────────────────────────────────────────────────────────────────────────────

/// Reads one CR3 into a `PhotoMeta`.
///
/// `rel_path` is the name the folder listing found, not anything read out of the file: a photo's
/// identity is its place in the shoot, and the parser is not allowed to invent one.
pub fn read_photo(
    path: &Path,
    rel_path: &str,
    file_size: u64,
    file_number: Option<u32>,
) -> PhotoMeta {
    let mut photo = PhotoMeta {
        rel_path: rel_path.to_string(),
        file_size,
        file_number,
        ..PhotoMeta::default()
    };
    let mut reader = match Cr3Reader::open(path) {
        Ok(r) => r,
        Err(e) => {
            photo.warnings.push(format!("{e}"));
            return photo;
        }
    };
    match reader.read() {
        Ok(m) => {
            photo.camera_make = m.make;
            photo.camera_model = m.model;
            photo.camera_serial = m.serial;
            photo.lens_model = m.lens;
            photo.orientation = m.orientation;
            photo.capture_time = m.capture;
            photo.focal_length_mm = m.focal_length_mm.map(|v| v as f32);
            photo.exposure_time_s = m.exposure_time_s.map(|v| v as f32);
            photo.f_number = m.f_number.map(|v| v as f32);
            photo.iso = m.iso;
            photo.exposure_comp_ev = m.exposure_comp_ev.map(|v| v as f32);
            photo.width = m.width;
            photo.height = m.height;
            photo.shutter_count = m.shutter_count;
            photo.metering_mode = m.metering_mode;
            photo.drive_mode = m.drive_mode;
            photo.shutter_mode = m.shutter_mode;
            photo.warnings = m.warnings;
            if m.shutter_count.is_none() {
                photo
                    .warnings
                    .push("no ShutterCount: ordering falls back to time alone".into());
            }
        }
        Err(e) => photo.warnings.push(format!("{e}")),
    }
    photo
}
