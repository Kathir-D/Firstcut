//! Reading Canon CR3 metadata out of the file itself, with no RAW decoder.
//!
//! A CR3 is an ISO-BMFF container. The parts that matter are:
//!
//! ```text
//! ftyp
//! moov
//!   trak (handler "pict")     the full-resolution JPEG: 6000×4000 on an R8
//!   trak (handler "meta")     a CTMD sample holding a *second* copy of the EXIF
//!   uuid 85c0b687-…             CNCV, CCTP, CTBO, CMT1, CMT2, CMT3, CMT4, THMB (160×120)
//! uuid eaf42b5e-…               XMP
//! uuid b9fbb7dc-…               PRVW, a 1620×1080 preview JPEG
//! mdat                         the sensor data
//! ```
//!
//! **A CR3 carries three JPEGs and they are not interchangeable** (todo.md §7.5, fact 1). `THMB` is
//! 160×120, `PRVW` is 1620×1080, and the first `trak` is the sensor's full 6000×4000. Measured on
//! `Game1JENKS/IMG_6117.CR3`, not assumed. The loupe needs the `trak` one — "100%" has to mean the
//! full frame — while `PRVW` is the cheap one to decode for a first-photo fast path, and conflating
//! them silently caps the viewer at 1620 px. They are separate fields for that reason: `thumbnail`,
//! `preview` (PRVW) and `full_preview` (the `trak` sample).
//!
//! The four `CMT` boxes are the primary copy of the metadata and are small, fixed-offset TIFF
//! streams: `CMT1` is IFD0, `CMT2` the Exif IFD, `CMT3` the MakerNote, `CMT4` GPS. The R8's
//! `ShutterCount` is *not* in them — it lives in the MakerNote inside the `meta` track's CTMD
//! sample, which is why [`Cr3::parse`] seeks for it rather than reading a whole file. Every read
//! here is a seek plus a bounded slice, so scanning 1,500 files reads 1,500 headers, not 20 GB.
//!
//! Nothing in this module is allowed to panic on malformed input: a file that does not parse comes
//! back as [`Cr3Error`] and the scanner reports it (todo.md §8).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The `uuid` box that carries the Canon `CMT*` metadata boxes.
const UUID_CANON: [u8; 16] = [
    0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48,
];
/// The `uuid` box that carries the full-size `PRVW` preview JPEG.
const UUID_PRVW: [u8; 16] = [
    0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16,
];

/// How much of the file the header walk reads. Every `CMT` box and the `PRVW` box live near the
/// front of a CR3, and the `meta` track's sample offset lives in `moov`; 1 MiB covers all of it
/// with room to spare, and it is the only read that is not a seek.
const HEAD_BYTES: u64 = 1 << 20;

/// `ShutterCount` sits at this byte offset inside the R6m2/R8 `CameraInfo` blob.
const SHUTTER_COUNT_OFFSET: usize = 0x0d29;

#[derive(Debug, thiserror::Error)]
pub enum Cr3Error {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// The file is not an ISO-BMFF container at all.
    #[error("not a CR3: no ftyp box")]
    NotACr3,

    #[error("no CMT1 box: the file has no Canon metadata")]
    NoMetadata,

    #[error("CMT{box_name} is not a TIFF stream")]
    NotTiff { box_name: &'static str },

    #[error("truncated: wanted {wanted} bytes at {offset}")]
    Truncated { offset: u64, wanted: usize },

    /// A value the file does not make sense for. Reported, never fatal: the rest still parses.
    #[error("{field} is out of range: {value}")]
    OutOfRange { field: &'static str, value: i64 },
}

/// A contiguous run of bytes inside the file, which is all a preview is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteRange {
    pub offset: u64,
    pub len: u64,
}

/// An embedded JPEG the pipeline can hand straight to ImageIO.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPreview {
    pub range: ByteRange,
    pub width: u32,
    pub height: u32,
}

/// One AF point, normalized into 0..=1 over the AF image rectangle.
///
/// Canon records AF coordinates in pixels relative to the *centre* of the AF image, so `x` here is
/// the point's centre in normalized sensor space. A negative Canon coordinate is a real point
/// (the AF grid overhangs the cropped frame on some lenses), so values are clamped rather than
/// dropped: losing a point would silently change what the overlay claims about the frame.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfPoint {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub in_focus: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfInfo {
    pub area_mode: Option<String>,
    pub image_width: u32,
    pub image_height: u32,
    pub points: Vec<AfPoint>,
    /// Indices of the AF points the camera reports as in focus, in the same units exiftool prints.
    pub points_in_focus: Vec<u16>,
}

/// Everything this module can read out of one CR3. `Option` means "the file did not say".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cr3 {
    pub make: Option<String>,
    pub model: Option<String>,
    /// `DateTimeOriginal`, the local wall-clock reading, `YYYY:MM:DD hh:mm:ss`.
    pub date_time_original: Option<String>,
    pub subsec_time_original: Option<String>,
    pub offset_time_original: Option<String>,
    pub body_serial_number: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub exposure_time_s: Option<f32>,
    pub f_number: Option<f32>,
    pub iso: Option<u32>,
    pub exposure_comp_ev: Option<f32>,
    pub metering_mode: Option<String>,
    pub drive_mode: Option<String>,
    pub shutter_mode: Option<String>,
    /// EXIF orientation, 1..=8. `None` when the file has no orientation tag.
    pub orientation: Option<u8>,
    /// Sensor dimensions *before* the orientation is applied (photo-meta.md).
    pub width: u32,
    pub height: u32,
    pub shutter_count: Option<u64>,
    pub af: Option<AfInfo>,
    /// The `PRVW` box's preview JPEG: **1620×1080**, not full size. Cheap to decode and a good
    /// first-photo fast path, but a 14" viewer needs more than that.
    pub preview: Option<EmbeddedPreview>,
    /// The full-resolution JPEG (6000×4000 on an R8) carried in the first `trak`. This is what the
    /// loupe draws and what "100%" means; ImageIO finds it by parsing the container itself, which
    /// is why a display decode costs ~166 ms. Decoding it from this byte range instead skips the
    /// container parse (todo.md §7.5).
    pub full_preview: Option<EmbeddedPreview>,
    /// The 160×120 thumbnail from `THMB`, if the file has one.
    pub thumbnail: Option<EmbeddedPreview>,
    /// Non-fatal problems worth showing in the log.
    pub warnings: Vec<String>,
}

impl Cr3 {
    /// Parses one CR3, reading headers only.
    pub fn parse(path: &Path) -> Result<Cr3, Cr3Error> {
        let mut file = File::open(path).map_err(|source| Cr3Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        let size = file
            .metadata()
            .map_err(|source| Cr3Error::Io {
                path: path.display().to_string(),
                source,
            })?
            .len();

        // The header window, borrowed out of a per-thread slot rather than allocated per file: a
        // 2,900-frame shoot allocated 2,900 fresh mebibytes and first-touched every one of them
        // on the scan's critical path, and `read_exact` writes all of it again immediately.
        let mut slot = HEAD_BUFFER.with(std::cell::RefCell::take);
        let parsed = parse_header(&mut file, path, &mut slot, size.min(HEAD_BYTES) as usize);
        HEAD_BUFFER.with(|cell| *cell.borrow_mut() = slot);
        parsed
    }
}

thread_local! {
    /// The header window each worker thread reuses. Taken out for the duration of a parse and put
    /// back afterwards, so no borrow is held across the walk and a panic costs the buffer, not the
    /// thread.
    static HEAD_BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn parse_header(
    file: &mut File,
    path: &Path,
    head: &mut Vec<u8>,
    want: usize,
) -> Result<Cr3, Cr3Error> {
    head.clear();
    head.resize(want, 0);
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(head))
        .map_err(|source| Cr3Error::Io {
            path: path.display().to_string(),
            source,
        })?;
    let head = &*head;
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return Err(Cr3Error::NotACr3);
    }

    let mut meta = Cr3::default();
    let mut cmt = CmtBoxes::default();
    let mut moov: Option<(usize, usize)> = None;
    let mut traks: Vec<(usize, usize)> = Vec::new();

    for box_ in Boxes::new(head, 0, head.len()) {
        match &box_.kind {
            b"moov" => {
                moov = Some((box_.body, box_.end));
                for child in Boxes::new(head, box_.body, box_.end) {
                    if child.kind == *b"uuid" && child.usertype() == Some(UUID_CANON) {
                        cmt.scan_canon_uuid(head, child.usertype_end(), child.end);
                    }
                    if child.kind == *b"trak" {
                        traks.push((child.body, child.end));
                    }
                }
            }
            b"uuid" => match box_.usertype() {
                Some(UUID_CANON) => cmt.scan_canon_uuid(head, box_.usertype_end(), box_.end),
                Some(UUID_PRVW) => {
                    if let Some(preview) = prvw_preview(head, box_.body, box_.end) {
                        meta.preview = Some(preview);
                    } else {
                        meta.warnings
                            .push("PRVW box has no usable JPEG".to_string());
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // The full-resolution JPEG is whichever track's sample is actually a JPEG. The first `trak`
    // in an R8 CR3 is the *RAW* one, so this has to look at all of them.
    if !traks.is_empty() {
        meta.full_preview = trak_jpeg(head, &traks);
    }

    cmt.apply_thumbnail(&mut meta);

    let Some(ifd0) = cmt.cmt1 else {
        return Err(Cr3Error::NoMetadata);
    };
    // An unreadable CMT1 costs the IFD0 fields — dimensions, make, model, orientation — and
    // nothing else: CMT2 and CMT3 are separate boxes and are read below either way. Failing the
    // whole file on them threw away a capture time that was sitting right there.
    if let Err(err) = read_ifd0(head, ifd0, &mut meta) {
        meta.warnings.push(format!("CMT1 unreadable: {err}"));
    }
    if let Some(exif) = cmt.cmt2 {
        read_exif(head, exif, &mut meta);
    }
    if let Some(maker) = cmt.cmt3 {
        read_makernote(head, maker, &mut meta);
    }

    // ShutterCount only exists in the meta track's copy of the MakerNote, so this is the one
    // field that costs a seek. Skipped when the file has no `moov` to point at one.
    if let Some((start, end)) = moov {
        if let Some(count) = read_shutter_count(file, head, start, end) {
            meta.shutter_count = Some(count);
        } else {
            meta.warnings
                .push("no shutter count: the meta track has no MakerNote".to_string());
        }
    }

    Ok(meta)
}

/// Where each `CMT*` box's payload begins, plus the `THMB` box.
#[derive(Default)]
struct CmtBoxes {
    cmt1: Option<usize>,
    cmt2: Option<usize>,
    cmt3: Option<usize>,
    thumbnail: Option<EmbeddedPreview>,
}

impl CmtBoxes {
    /// The Canon `uuid` body is a flat list of boxes: `CNCV`, `CCTP`, `CTBO`, `free`, `CMT1`…`THMB`.
    ///
    /// `end` is the `uuid` box's own end, not the end of the header window. Walking past it adopted
    /// every `CMT1`/`THMB` in the rest of `moov` as Canon's metadata, so a `THMB` belonging to some
    /// other box overwrote the thumbnail and a `CMT1` overwrote IFD0.
    fn scan_canon_uuid(&mut self, head: &[u8], start: usize, end: usize) {
        for box_ in Boxes::new(head, start, end) {
            match &box_.kind {
                b"CMT1" => self.cmt1 = Some(box_.body),
                b"CMT2" => self.cmt2 = Some(box_.body),
                b"CMT3" => self.cmt3 = Some(box_.body),
                b"THMB" => self.thumbnail = thmb_thumbnail(head, box_.body, box_.end),
                _ => {}
            }
        }
    }

    fn apply_thumbnail(&self, meta: &mut Cr3) {
        meta.thumbnail = self.thumbnail;
    }
}

// ─────────────────────────────────────────────────────────────────────────── ISO-BMFF

/// One box header, resolved. `body`/`end` are absolute offsets into the buffer they came from.
struct BoxHeader<'a> {
    kind: [u8; 4],
    body: usize,
    end: usize,
    uuid: Option<&'a [u8]>,
}

impl<'a> BoxHeader<'a> {
    fn usertype(&self) -> Option<[u8; 16]> {
        self.uuid.and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
    }

    /// Where a `uuid` box's payload starts, i.e. past its 16-byte type. A Canon `uuid` is followed
    /// immediately by the `CMT*` boxes, so the sub-walk has to start here rather than at the body.
    fn usertype_end(&self) -> usize {
        self.body + 16
    }
}

/// Big-endian box walker. Every offset it hands out is bounds-checked against `data`, and a box
/// with an impossible size ends the walk rather than being trusted, so a corrupt file cannot make
/// the parser read out of range.
struct Boxes<'a> {
    data: &'a [u8],
    offset: usize,
    end: usize,
}

impl<'a> Boxes<'a> {
    fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self {
            data,
            offset: start,
            end: end.min(data.len()),
        }
    }
}

impl<'a> Iterator for Boxes<'a> {
    type Item = BoxHeader<'a>;

    fn next(&mut self) -> Option<BoxHeader<'a>> {
        if self.offset + 8 <= self.end {
            let start = self.offset;
            let size =
                u32::from_be_bytes(self.data[start..start + 4].try_into().expect("four bytes"));
            let kind: [u8; 4] = self.data[start + 4..start + 8]
                .try_into()
                .expect("four bytes");
            let (size, header) = match size {
                // 64-bit size: the 8-byte length follows the header.
                1 => {
                    if start + 16 > self.end {
                        return None;
                    }
                    (
                        u64::from_be_bytes(
                            self.data[start + 8..start + 16]
                                .try_into()
                                .expect("eight bytes"),
                        ),
                        16usize,
                    )
                }
                // 0 means "to the end of the container".
                0 => ((self.end - start) as u64, 8usize),
                size => (u64::from(size), 8usize),
            };
            if size < header as u64 {
                return None;
            }
            // `size` can come from a 64-bit `largesize` field, so it is not a `usize` until it has
            // been narrowed, and `start + size` is not an addition that can be trusted to land
            // inside the buffer. `LittleBoxes` and `bmff_boxes` bound it the same way; here the
            // unwrapped addition overflowed in a debug build and walked backwards in a release one.
            let size = usize::try_from(size).ok()?;
            let end = start.checked_add(size)?;
            if end > self.end {
                return None;
            }
            self.offset = end;
            let body = start + header;
            let uuid = (kind == *b"uuid" && body + 16 <= end).then(|| &self.data[body..body + 16]);
            return Some(BoxHeader {
                kind,
                body,
                end,
                uuid,
            });
        }
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────── TIFF

/// A TIFF stream inside a box: `CMT1`, `CMT2`, `CMT3`, or a MakerNote payload.
///
/// Offsets in the file are relative to the TIFF header, so every accessor takes a stream-relative
/// offset and adds `base`. Reads past the end of the buffer return `None` instead of panicking,
/// which is what makes a truncated file a warning rather than a crash.
pub(super) struct Tiff<'a> {
    data: &'a [u8],
    base: usize,
    little: bool,
}

/// One IFD entry, resolved to where its value actually is.
pub(super) struct Entry {
    pub(super) tag: u16,
    pub(super) kind: u16,
    pub(super) count: u32,
    /// Offset of the value, stream-relative (already resolved for the ≤4-byte inline case).
    pub(super) value_offset: usize,
}

/// The most entries one IFD may claim.
///
/// The count is a `u16`, so a file can name 65,535 of them, and every entry costs a walk of the
/// stream. A camera writes tens — Canon's MakerNote IFD0 has a handful — and the ones that do not
/// (a DNG with SubIFDs) are still well under a hundred, so a cap this loose costs nothing real and
/// turns a crafted count from ~10^10 reads into ~10^3.
const MAX_IFD_ENTRIES: usize = 512;

/// The most integers read out of one array entry. CanonCameraSettings is indexed to 0x05 and
/// CanonFileInfo to 0x17, so a real array is tens of elements; the cap is on the file's claim.
const MAX_ARRAY_INTS: usize = 1024;

/// The TIFF field types this parser understands. Anything else is skipped, which is what lets a
/// MakerNote carry dozens of uninteresting tags without a match arm for each.
fn type_size(kind: u16) -> Option<usize> {
    Some(match kind {
        1 | 2 | 6 | 7 => 1, // BYTE ASCII SBYTE UNDEFINED
        3 | 8 => 2,         // SHORT SSHORT
        4 | 9 => 4,         // LONG SLONG
        5 | 10 => 8,        // RATIONAL SRATIONAL
        11 => 4,            // FLOAT
        12 => 8,            // DOUBLE
        _ => return None,
    })
}

impl<'d> Tiff<'d> {
    pub(super) fn new(data: &'d [u8], base: usize) -> Option<Tiff<'d>> {
        let order = data.get(base..base + 2)?;
        let little = match order {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        Some(Tiff { data, base, little })
    }

    pub(super) fn data(&self) -> &'d [u8] {
        self.data
    }

    pub(super) fn base(&self) -> usize {
        self.base
    }

    /// `len` bytes at the stream-relative `offset`.
    pub(super) fn bytes(&self, offset: usize, len: usize) -> Option<&'d [u8]> {
        let start = self.base.checked_add(offset)?;
        self.data.get(start..start.checked_add(len)?)
    }

    pub(super) fn u16_at(&self, offset: usize) -> Option<u16> {
        let bytes = self.data.get(self.base.checked_add(offset)?..)?.get(..2)?;
        Some(match self.little {
            true => u16::from_le_bytes(bytes.try_into().expect("two bytes")),
            false => u16::from_be_bytes(bytes.try_into().expect("two bytes")),
        })
    }

    pub(super) fn u32_at(&self, offset: usize) -> Option<u32> {
        let bytes = self.data.get(self.base.checked_add(offset)?..)?.get(..4)?;
        Some(match self.little {
            true => u32::from_le_bytes(bytes.try_into().expect("four bytes")),
            false => u32::from_be_bytes(bytes.try_into().expect("four bytes")),
        })
    }

    /// The first IFD, which in a `CMT*` box is IFD0.
    pub(super) fn ifd0(&self) -> Option<Entries<'_, '_>> {
        self.entries_at(self.u32_at(4)? as usize)
    }

    pub(super) fn entries_at(&self, offset: usize) -> Option<Entries<'_, '_>> {
        let count = (self.u16_at(offset)? as usize).min(MAX_IFD_ENTRIES);
        Some(Entries {
            tiff: self,
            offset,
            count,
            index: 0,
        })
    }

    pub(super) fn value_bytes(&self, entry: &Entry) -> Option<&'d [u8]> {
        let size = type_size(entry.kind)?.checked_mul(entry.count as usize)?;
        self.data
            .get(self.base.checked_add(entry.value_offset)?..)?
            .get(..size)
    }

    pub(super) fn string(&self, entry: &Entry) -> Option<String> {
        let bytes = self.value_bytes(entry)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
        (!text.is_empty()).then_some(text)
    }

    pub(super) fn first_int(&self, entry: &Entry) -> Option<i64> {
        self.first_int_at(entry, 0)
    }

    /// One element of an integer array, read where it is rather than by materialising the array.
    ///
    /// `first_int` used to build the whole `Vec<i64>` to take its head, and `count` is a field in
    /// the file: one crafted LONG made it allocate ~2 MB, for each of the seven tags every photo
    /// asks for, before the scan had read a single preview.
    pub(super) fn first_int_at(&self, entry: &Entry, index: usize) -> Option<i64> {
        let bytes = self.value_bytes(entry)?;
        let width = match entry.kind {
            1 | 7 => 1,
            6 => 1,
            3 | 8 => 2,
            4 | 9 => 4,
            _ => return None,
        };
        let element = bytes.get(index.checked_mul(width)?..)?.get(..width)?;
        Some(match (entry.kind, self.little) {
            (6, _) => i64::from(element[0] as i8),
            (3, true) | (8, true) => i64::from(u16::from_le_bytes([element[0], element[1]])),
            (3, false) | (8, false) => i64::from(u16::from_be_bytes([element[0], element[1]])),
            (4, true) | (9, true) => {
                i64::from(u32::from_le_bytes(element.try_into().expect("four bytes")))
            }
            (4, false) | (9, false) => {
                i64::from(u32::from_be_bytes(element.try_into().expect("four bytes")))
            }
            _ => i64::from(element[0]),
        })
    }

    /// Reads `count` integers of the entry's type. Out-of-range and unknown types give an empty
    /// vector, so a caller that indexes gets `None` from `first_int` rather than a bogus number.
    pub(super) fn ints(&self, entry: &Entry) -> Vec<i64> {
        let Some(bytes) = self.value_bytes(entry) else {
            return Vec::new();
        };
        // A file may claim four billion elements of a four-byte type; the vector below is sized
        // from that claim, and every walk that follows it is sized from the vector. A camera writes
        // tens, so a real array is never near the cap.
        let width = type_size(entry.kind).unwrap_or(1);
        let count = (entry.count as usize).min(MAX_ARRAY_INTS);
        let bytes = &bytes[..count.saturating_mul(width).min(bytes.len())];
        match entry.kind {
            1 | 7 => bytes.iter().map(|&b| i64::from(b)).collect(),
            6 => bytes.iter().map(|&b| i64::from(b as i8)).collect(),
            3 | 8 => (0..count)
                .filter_map(|i| {
                    bytes.get(i * 2..i * 2 + 2).map(|b| match self.little {
                        true => u16::from_le_bytes(b.try_into().expect("two bytes")),
                        false => u16::from_be_bytes(b.try_into().expect("two bytes")),
                    })
                })
                .map(i64::from)
                .collect(),
            4 | 9 => (0..count)
                .filter_map(|i| {
                    bytes.get(i * 4..i * 4 + 4).map(|b| match self.little {
                        true => u32::from_le_bytes(b.try_into().expect("four bytes")),
                        false => u32::from_be_bytes(b.try_into().expect("four bytes")),
                    })
                })
                .map(i64::from)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The first element of an unsigned RATIONAL, as `f32`.
    pub(super) fn first_rational(&self, entry: &Entry) -> Option<f32> {
        let (numerator, denominator) = self.rational_pair(entry)?;
        let value = f64::from(numerator) / f64::from(denominator);
        // f32 cannot hold every rational a camera writes; values that overflow it are reported by
        // the caller's bounds check rather than becoming `inf` here.
        (value.is_finite() && value.abs() <= f64::from(f32::MAX)).then_some(value as f32)
    }

    /// The first element of a signed RATIONAL, as `f32`. EXIF's `ExposureCompensation` is one.
    ///
    /// Read as unsigned, a −1/3 EV stored as `0xFFFFFFFF / 3` comes out as 1,431,655,765 EV: it
    /// passes the finite guard and reaches the UI as a real number. All four exiftool fixtures
    /// record `0`, which is why the agreement test could not see it.
    pub(super) fn first_srational(&self, entry: &Entry) -> Option<f32> {
        let (numerator, denominator) = self.rational_pair(entry)?;
        let value = f64::from(numerator as i32) / f64::from(denominator);
        (value.is_finite() && value.abs() <= f64::from(f32::MAX)).then_some(value as f32)
    }

    fn rational_pair(&self, entry: &Entry) -> Option<(u32, u32)> {
        let bytes = self.value_bytes(entry)?;
        let (numerator, denominator) = match self.little {
            true => (
                u32::from_le_bytes(bytes.get(..4)?.try_into().expect("four bytes")),
                u32::from_le_bytes(bytes.get(4..8)?.try_into().expect("four bytes")),
            ),
            false => (
                u32::from_be_bytes(bytes.get(..4)?.try_into().expect("four bytes")),
                u32::from_be_bytes(bytes.get(4..8)?.try_into().expect("four bytes")),
            ),
        };
        (denominator != 0).then_some((numerator, denominator))
    }
}

pub(super) struct Entries<'t, 'd> {
    tiff: &'t Tiff<'d>,
    offset: usize,
    count: usize,
    index: usize,
}

impl Iterator for Entries<'_, '_> {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        if self.index >= self.count {
            return None;
        }
        let entry_offset = self.offset + 2 + self.index * 12;
        self.index += 1;
        let tag = self.tiff.u16_at(entry_offset)?;
        let kind = self.tiff.u16_at(entry_offset + 2)?;
        let count = self.tiff.u32_at(entry_offset + 4)?;
        let size = type_size(kind).and_then(|size| size.checked_mul(count as usize));
        // A value of four bytes or fewer lives in the entry itself, at entry_offset + 8.
        let value_offset = match size {
            Some(size) if size <= 4 => entry_offset + 8,
            _ => self.tiff.u32_at(entry_offset + 8)? as usize,
        };
        Some(Entry {
            tag,
            kind,
            count,
            value_offset,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────── tag readers

/// EXIF IFD0.
fn read_ifd0(head: &[u8], base: usize, meta: &mut Cr3) -> Result<(), Cr3Error> {
    let tiff = Tiff::new(head, base).ok_or(Cr3Error::NotTiff { box_name: "1" })?;
    let entries = tiff.ifd0().ok_or(Cr3Error::NoMetadata)?;
    apply_ifd0(&tiff, entries, meta);
    Ok(())
}

/// IFD0's tags: dimensions, make, model, orientation.
pub(super) fn apply_ifd0(tiff: &Tiff<'_>, entries: Entries<'_, '_>, meta: &mut Cr3) {
    for entry in entries {
        match entry.tag {
            0x0100 => meta.width = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
            0x0101 => meta.height = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
            0x010f => meta.make = tiff.string(&entry),
            0x0110 => meta.model = tiff.string(&entry),
            0x0112 => {
                let value = tiff.first_int(&entry).unwrap_or(1);
                // The contract says 1..=8; anything else is a corrupt tag, not a rotation.
                meta.orientation = (1..=8).contains(&value).then_some(value as u8);
            }
            _ => {}
        }
    }
}

/// The Exif IFD, which is where the capture settings live.
fn read_exif(head: &[u8], base: usize, meta: &mut Cr3) {
    let Some(tiff) = Tiff::new(head, base) else {
        meta.warnings.push("CMT2 is not a TIFF stream".to_string());
        return;
    };
    let Some(entries) = tiff.ifd0() else {
        meta.warnings.push("CMT2 has no readable IFD".to_string());
        return;
    };
    apply_exif(&tiff, entries, meta);
}

/// The Exif IFD's tags, from whichever container it was found in.
pub(super) fn apply_exif(tiff: &Tiff<'_>, entries: Entries<'_, '_>, meta: &mut Cr3) {
    for entry in entries {
        match entry.tag {
            0x9003 => meta.date_time_original = tiff.string(&entry),
            0x9291 => meta.subsec_time_original = tiff.string(&entry),
            0x9011 => meta.offset_time_original = tiff.string(&entry),
            0xa431 => meta.body_serial_number = tiff.string(&entry),
            0xa434 => meta.lens_model = tiff.string(&entry),
            0x829a => meta.exposure_time_s = tiff.first_rational(&entry),
            0x829d => meta.f_number = tiff.first_rational(&entry),
            0x9204 => meta.exposure_comp_ev = tiff.first_srational(&entry),
            0x920a => meta.focal_length_mm = tiff.first_rational(&entry),
            0x8827 => {
                meta.iso = tiff
                    .first_int(&entry)
                    .filter(|&value| (1..=u32::MAX as i64).contains(&value))
                    .map(|value| value as u32);
            }
            0x9207 => {
                meta.metering_mode = tiff
                    .first_int(&entry)
                    .and_then(|value| metering_mode(value as u16).map(str::to_string));
            }
            _ => {}
        }
    }
}

/// The Canon MakerNote: drive mode, shutter mode and the AF grid.
fn read_makernote(head: &[u8], base: usize, meta: &mut Cr3) {
    let Some(tiff) = Tiff::new(head, base) else {
        meta.warnings.push("CMT3 is not a TIFF stream".to_string());
        return;
    };
    let Some(entries) = tiff.ifd0() else {
        meta.warnings.push("CMT3 has no readable IFD".to_string());
        return;
    };
    apply_makernote(&tiff, entries, meta);
}

/// The Canon MakerNote's tags, from a `CMT3` box or from a CR2's Exif IFD.
pub(super) fn apply_makernote(tiff: &Tiff<'_>, entries: Entries<'_, '_>, meta: &mut Cr3) {
    for entry in entries {
        match entry.tag {
            // CanonCameraSettings: an array of int16 indexed by tag number.
            0x0001 => {
                let values = tiff.ints(&entry);
                if let Some(&drive) = values.get(0x05) {
                    meta.drive_mode = drive_mode(drive as u16).map(str::to_string);
                }
            }
            // CanonAFInfo2: the AF grid, in a fixed-layout int16 record.
            0x0026 => {
                if let Some(bytes) = tiff.value_bytes(&entry) {
                    meta.af = af_info(bytes);
                }
            }
            // CanonFileInfo: ShutterMode lives at index 0x17.
            0x0093 => {
                let values = tiff.ints(&entry);
                if let Some(&mode) = values.get(0x17) {
                    meta.shutter_mode =
                        shutter_mode(u16::try_from(mode).unwrap_or(u16::MAX)).map(str::to_string);
                }
            }
            _ => {}
        }
    }
}

/// EXIF `MeteringMode`, spelled the way exiftool spells it so the fixtures line up.
fn metering_mode(value: u16) -> Option<&'static str> {
    Some(match value {
        0 => "Unknown",
        1 => "Average",
        2 => "Center-weighted average",
        3 => "Spot",
        4 => "Multi-spot",
        5 => "Evaluative",
        6 => "Partial",
        255 => "(Other)",
        _ => return None,
    })
}

/// Canon `ContinuousDrive`. The contract prefers this over the coarser `DriveMode` because §3 says
/// the continuous rate is what varies per burst.
fn drive_mode(value: u16) -> Option<&'static str> {
    Some(match value {
        0 => "Single",
        1 => "Continuous",
        2 => "Movie",
        3 => "Continuous, Speed Priority",
        4 => "Continuous, Low",
        5 => "Continuous, High",
        6 => "Silent Single",
        8 => "Continuous, High+",
        9 => "Single, Silent",
        10 => "Continuous, Silent",
        _ => return None,
    })
}

fn shutter_mode(value: u16) -> Option<&'static str> {
    Some(match value {
        0 => "Mechanical",
        1 => "Electronic First Curtain",
        2 => "Electronic",
        _ => return None,
    })
}

fn af_area_mode(value: u16) -> Option<&'static str> {
    Some(match value {
        0 => "Off (Manual Focus)",
        1 => "AF Point Expansion (surround)",
        2 => "Single-point AF",
        4 => "Auto",
        5 => "Face Detect AF",
        6 => "Face + Tracking",
        7 => "Zone AF",
        8 => "AF Point Expansion (4 point)",
        9 => "Spot AF",
        10 => "AF Point Expansion (8 point)",
        11 => "Flexizone Multi (49 point)",
        12 => "Flexizone Multi (9 point)",
        13 => "Flexizone Single",
        14 => "Large Zone AF",
        16 => "Large Zone AF (vertical)",
        17 => "Large Zone AF (horizontal)",
        19 => "Flexible Zone AF 1",
        20 => "Flexible Zone AF 2",
        21 => "Flexible Zone AF 3",
        22 => "Whole Area AF",
        _ => return None,
    })
}

/// Header int16 fields of the `CanonAFInfo2` record, in order.
const AF_AREA_MODE: usize = 1;
const AF_NUM_POINTS: usize = 2;
const AF_VALID_POINTS: usize = 3;
const AF_IMAGE_WIDTH: usize = 6;
const AF_IMAGE_HEIGHT: usize = 7;
/// Widths, heights, x positions, y positions and the in-focus bitmask follow the header, each
/// `NumAFPoints` wide, except the bitmask which is 16 bits per 16 points.
const AF_HEADER_FIELDS: usize = 8;

/// Reads the AF grid out of a `CanonAFInfo2` record.
///
/// The four point arrays are always present but only the first `ValidAFPoints` entries mean
/// anything, and the bitmask is `(NumAFPoints + 15) / 16` words. Both counts are read from the
/// record rather than assumed, because a body with a different AF layout writes different ones.
fn af_info(record: &[u8]) -> Option<AfInfo> {
    let field = |index: usize| -> i64 {
        record
            .get(index * 2..index * 2 + 2)
            .map(|b| i16::from_le_bytes(b.try_into().expect("two bytes")) as i64)
            .unwrap_or(0)
    };
    let count = field(AF_NUM_POINTS).max(0) as usize;
    if count == 0 {
        return None;
    }
    let valid = (field(AF_VALID_POINTS).max(0) as usize).min(count);
    let image_width = field(AF_IMAGE_WIDTH).max(0) as u32;
    let image_height = field(AF_IMAGE_HEIGHT).max(0) as u32;

    let mut offset = AF_HEADER_FIELDS * 2;
    let mut take = |len: usize| -> Option<Vec<i64>> {
        let slice = record.get(offset..offset + len * 2)?;
        offset += len * 2;
        Some(
            slice
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
                .collect(),
        )
    };
    let widths = take(count)?;
    let heights = take(count)?;
    let xs = take(count)?;
    let ys = take(count)?;
    let bit_words = count.div_ceil(16);
    let in_focus = take(bit_words)?;
    // One extra word past the bitmask is AFPointsSelected, which nothing here needs.
    let _selected = record.get(offset..offset + bit_words * 2);

    let points_in_focus: Vec<u16> = in_focus
        .iter()
        .enumerate()
        .flat_map(|(word, &bits)| {
            (0..16).filter_map(move |bit| {
                let index = (word * 16 + bit) as u16;
                (bits & (1 << bit) != 0).then_some(index)
            })
        })
        .collect();
    // The same bits, one flag per point, built once. Asking `points_in_focus` whether it holds a
    // point is a linear scan, and `count` is an i16 straight from the file, so a crafted
    // `CanonAFInfo2` claiming 32,767 points turned each valid point into 32,767 comparisons —
    // ~10^9 of them, on a scan worker thread, for a folder that then never opens.
    let mut focused = vec![false; count];
    for (word, &bits) in in_focus.iter().enumerate() {
        for bit in 0..16 {
            let Some(flag) = focused.get_mut(word * 16 + bit) else {
                break;
            };
            *flag = bits & (1 << bit) != 0;
        }
    }

    let scale_x = if image_width > 0 {
        image_width as f32
    } else {
        1.0
    };
    let scale_y = if image_height > 0 {
        image_height as f32
    } else {
        1.0
    };
    // Canon records positions relative to the centre of the AF image, so half the image is added
    // back before normalizing into 0..=1. The half-size offset stays in pixels until the single
    // division at the end, so a point's centre is not scaled twice.
    let points = (0..valid)
        .map(|i| {
            let raw_w = widths[i] as f32;
            let raw_h = heights[i] as f32;
            let center_x = (xs[i] as f32 + raw_w / 2.0 + scale_x / 2.0) / scale_x;
            let center_y = (ys[i] as f32 + raw_h / 2.0 + scale_y / 2.0) / scale_y;
            AfPoint {
                x: center_x.clamp(0.0, 1.0),
                y: center_y.clamp(0.0, 1.0),
                w: (raw_w / scale_x).clamp(0.0, 1.0),
                h: (raw_h / scale_y).clamp(0.0, 1.0),
                in_focus: focused[i],
            }
        })
        .collect();

    Some(AfInfo {
        area_mode: af_area_mode(field(AF_AREA_MODE).clamp(0, u16::MAX as i64) as u16)
            .map(str::to_string),
        image_width,
        image_height,
        points,
        points_in_focus,
    })
}

// ─────────────────────────────────────────────────────────────────────────── previews

/// The full-resolution JPEG, found through the sample table of whichever `trak` holds one.
///
/// A CR3 holds three JPEGs, and which one you get decides the cost of everything downstream
/// (todo.md §7.5, fact 1). Measured on `Game1JENKS/IMG_3181.CR3` and `IMG_6117.CR3`:
///
/// | where | size | what it is for |
/// | --- | --- | --- |
/// | `THMB` | 160×120 | the file's own thumbnail box; the core reads it for a cheap existence check |
/// | `PRVW` | 1620×1080 | 7% of the pixels — a fast first photo, not the viewer |
/// | first `trak` sample | **6000×4000** | the sensor's full size, and what "100%" has to mean |
///
/// **The sample entry's codec is `CRAW` for every image track, including the one whose sample is a
/// JPEG.** The first `trak` in an R8 CR3 is the *RAW* track, and its `stsd` says `CRAW` while its
/// sample is in fact the full-size JPEG — so the four-character code is no help at all, and neither
/// is a handler type (all three image tracks are `vide`; only the `meta` track is `meta`). What
/// identifies the JPEG is the sample's own bytes: `FF D8` at its offset, and an `SOF` marker that
/// gives the dimensions. So every track's sample table is read, each sample is tested, and the
/// largest JPEG wins.
///
/// The returned range deliberately may extend past the 1 MiB header window — the JPEG itself is
/// megabytes. Only its first bytes are needed to identify it, and the app seeks to the range.
fn trak_jpeg(head: &[u8], traks: &[(usize, usize)]) -> Option<EmbeddedPreview> {
    let mut best: Option<EmbeddedPreview> = None;
    for &(body, end) in traks {
        let Some(sample) = trak_sample(head, body, end) else {
            continue;
        };
        // Only the leading bytes are needed to identify it, and `jpeg_dimensions` stops at the first
        // `SOF`. It is bounds-checked, so handing it the rest of the header window is safe — and the
        // window has to be generous, because a Canon's EXIF and ICC segments can push `SOF` well
        // past the first few dozen bytes.
        let offset = sample.offset as usize;
        // A later track can start past the 1 MiB header window, which is not a reason to give up on
        // the track that was already identified: `continue`, not `?`. Using `?` here returned from
        // the whole function and threw the good answer away.
        let Some(tail) = head.get(offset..) else {
            continue;
        };
        if !tail.starts_with(&[0xff, 0xd8]) {
            continue;
        }
        let (width, height) = match jpeg_dimensions(tail) {
            Some(dimensions) if dimensions.0 > 0 && dimensions.1 > 0 => dimensions,
            _ => continue,
        };
        let better = best.as_ref().is_none_or(|current| {
            u64::from(width) * u64::from(height)
                > u64::from(current.width) * u64::from(current.height)
        });
        if better {
            best = Some(EmbeddedPreview {
                range: sample,
                width,
                height,
            });
        }
    }
    best
}

/// A track's single sample: its size from `stsz` and its offset from `stco`/`co64`.
///
/// The JPEG and `meta` tracks each hold exactly one sample, so the first table entry is the sample
/// and `stsc`/`stts` — which only matter for a track with several samples or a varying frame rate —
/// are not needed. A track that uses neither `stco` nor `co64`, or an `stsz` with no entries,
/// yields `None` rather than a guess.
fn trak_sample(head: &[u8], start: usize, end: usize) -> Option<ByteRange> {
    let mut stbl: Option<(usize, usize)> = None;
    for box_ in Boxes::new(head, start, end) {
        if box_.kind == *b"mdia" {
            for child in Boxes::new(head, box_.body, box_.end) {
                if child.kind != *b"minf" {
                    continue;
                }
                for grandchild in Boxes::new(head, child.body, child.end) {
                    if grandchild.kind == *b"stbl" {
                        stbl = Some((grandchild.body, grandchild.end));
                    }
                }
            }
        }
    }
    let (stbl_start, stbl_end) = stbl?;

    let mut size: Option<u64> = None;
    let mut offset: Option<u64> = None;
    for box_ in Boxes::new(head, stbl_start, stbl_end) {
        match &box_.kind {
            // Both are FullBoxes: 4 bytes of version+flags before the payload.
            b"stsz" => size = first_sample_size(head, box_.body + 4, box_.end),
            b"stco" => offset = first_chunk_offset32(head, box_.body + 4, box_.end),
            b"co64" => offset = first_chunk_offset64(head, box_.body + 4, box_.end),
            _ => {}
        }
    }
    let len = size.filter(|value| *value >= 2)?;
    Some(ByteRange {
        offset: offset?,
        len,
    })
}

/// An `hdlr` box's handler type, which is the four bytes after its 8-byte header + 4-byte
/// version/flags + 4-byte pre_defined.
#[allow(dead_code)]
fn handler_type(head: &[u8], body: usize) -> Option<[u8; 4]> {
    head.get(body + 8..body + 12)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
}

/// `stsz`: a uniform `sample_size`, or — when it is zero — a table whose first entry is the size.
///
/// `end` is the box's own end, used to reject an entry that runs past it. `Boxes` has already
/// bounded the box itself, so this is belt and braces: a truncated file must not make the parser
/// read a sample size out of whatever bytes follow.
fn first_sample_size(head: &[u8], start: usize, end: usize) -> Option<u64> {
    if start + 4 > end {
        return None;
    }
    let uniform = be32(head, start)?;
    if uniform != 0 {
        return Some(u64::from(uniform));
    }
    // sample_count sits between the uniform size and the table; the first entry is 4 bytes past it.
    if start + 12 > end {
        return None;
    }
    be32(head, start + 8).map(u64::from)
}

/// `stco`: a 32-bit chunk offset table; the first entry is the one sample's offset.
fn first_chunk_offset32(head: &[u8], start: usize, end: usize) -> Option<u64> {
    // 4 bytes of entry_count, then the first entry.
    if start + 8 > end {
        return None;
    }
    be32(head, start + 4).map(u64::from)
}

/// `co64`: the same table in 64 bits, for a file over 4 GB.
fn first_chunk_offset64(head: &[u8], start: usize, end: usize) -> Option<u64> {
    if start + 12 > end {
        return None;
    }
    be64(head, start + 4)
}

fn be32(head: &[u8], at: usize) -> Option<u32> {
    head.get(at..at + 4)
        .map(|b| u32::from_be_bytes(b.try_into().expect("four bytes")))
}

fn be64(head: &[u8], at: usize) -> Option<u64> {
    head.get(at..at + 8)
        .map(|b| u64::from_be_bytes(b.try_into().expect("eight bytes")))
}

/// The `PRVW` box: a big-endian sub-box whose 20-byte header precedes a **1620×1080** JPEG.
///
/// Not the full-size image — that is the first `trak`'s sample, which [`trak_jpeg`] reads. The two
/// are kept apart because they cost very different amounts to decode, and a caller that wants "the
/// picture" and gets `PRVW` has silently capped the viewer at 1620 px.
fn prvw_preview(head: &[u8], start: usize, end: usize) -> Option<EmbeddedPreview> {
    let marker = head[start..end].windows(4).position(|w| w == b"PRVW")? + start;
    let be16 = |at: usize| -> Option<u32> {
        let bytes = head.get(at..at + 2)?;
        Some(u32::from(u16::from_be_bytes(
            bytes.try_into().expect("two bytes"),
        )))
    };
    let be32 = |at: usize| -> Option<u64> {
        let bytes = head.get(at..at + 4)?;
        Some(u64::from(u32::from_be_bytes(
            bytes.try_into().expect("four bytes"),
        )))
    };
    // Offsets are from the `PRVW` marker: +0x0a width, +0x0c height, +0x10 length, +0x14 JPEG.
    let width = be16(marker + 0x0a)?;
    let height = be16(marker + 0x0c)?;
    let len = be32(marker + 0x10)?;
    let offset = marker + 0x14;
    let len = usize::try_from(len).ok()?;
    // The declared length is the file's own claim, and the range goes straight to Swift as a seek
    // and a read. Clamped to what is left of the box, so a box claiming 0xFFFF_FFFF cannot turn
    // into a 4 GiB read of whatever follows in the file. (The 1 MiB header window is *not* the
    // bound: the preview itself is megabytes and lives well past it.)
    let len = len.min(end.saturating_sub(offset));
    let body = head.get(offset..offset + len.min(2))?;
    if !body.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    (width > 0 && height > 0).then_some(EmbeddedPreview {
        range: ByteRange {
            offset: offset as u64,
            len: len as u64,
        },
        width,
        height,
    })
}

/// The `THMB` box: a 16-byte header, then a 160×120 JPEG. The dimensions are not in the box, so
/// they are read out of the JPEG's own `SOF` marker — the file is the authority, not a constant.
fn thmb_thumbnail(head: &[u8], start: usize, end: usize) -> Option<EmbeddedPreview> {
    let payload = head.get(start..end)?;
    let offset = payload
        .windows(2)
        .position(|w| w == [0xff, 0xd8])
        .map(|at| start + at)?;
    let len = (end - offset) as u64;
    let payload = head.get(offset..end)?;
    let (width, height) = jpeg_dimensions(payload)?;
    Some(EmbeddedPreview {
        range: ByteRange {
            offset: offset as u64,
            len,
        },
        width,
        height,
    })
}

/// Width and height from a JPEG's first start-of-frame marker.
///
/// Walking the marker chain is the only way to learn the size without decoding pixels, which is
/// the whole point of not having a RAW decoder. Every segment length is bounds-checked, so a
/// truncated JPEG returns `None` rather than reading past the end.
fn jpeg_dimensions(jpeg: &[u8]) -> Option<(u32, u32)> {
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2usize;
    while at + 4 <= jpeg.len() {
        if jpeg[at] != 0xff {
            at += 1;
            continue;
        }
        let marker = jpeg[at + 1];
        // Standalone markers carry no length: SOI, EOI and every restart marker.
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        // Entropy-coded data after `SOS`: the dimensions are behind us by then.
        if marker == 0xda {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(
            jpeg.get(at + 2..at + 4)?.try_into().expect("two bytes"),
        ));
        // `SOF0`..`SOF15` except the four markers that are not frame headers.
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let bytes = jpeg.get(at + 5..at + 9)?;
            let height = u16::from_be_bytes(bytes[0..2].try_into().expect("two bytes"));
            let width = u16::from_be_bytes(bytes[2..4].try_into().expect("two bytes"));
            return (width > 0 && height > 0).then_some((u32::from(width), u32::from(height)));
        }
        at += 2 + length.max(2);
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────── ShutterCount

/// Finds the `meta` track's CTMD sample and reads `ShutterCount` out of its MakerNote.
///
/// The primary `CMT3` does not carry the R8's shutter count; Canon puts a second, richer
/// MakerNote in the sample table of a track whose handler type is `meta`. Reaching it is a walk of
/// `moov` for the chunk offset and sample size, then one seek.
fn read_shutter_count(file: &mut File, head: &[u8], start: usize, end: usize) -> Option<u64> {
    let (offset, size) = meta_sample(head, start, end)?;
    // A CTMD sample is a few hundred KB; anything larger is not the metadata track we are after.
    if size == 0 || size > 8 << 20 {
        return None;
    }
    let mut sample = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(&mut sample).ok()?;
    shutter_count_in_sample(&sample)
}

/// The offset and size of the `meta` track's single sample, from `moov`.
fn meta_sample(head: &[u8], start: usize, end: usize) -> Option<(u64, u64)> {
    for trak in Boxes::new(head, start, end).filter(|b| b.kind == *b"trak") {
        let mut handler = None;
        let mut chunk_offset = None;
        let mut sample_size = None;
        // Not every `trak` has a `mdia`; a video track in a body that also stores the metadata
        // separately looks exactly like one that does.
        let Some(mdia) = Boxes::new(head, trak.body, trak.end).find(|b| b.kind == *b"mdia") else {
            continue;
        };
        for box_ in Boxes::new(head, mdia.body, mdia.end) {
            match &box_.kind {
                // `hdlr` is a full box: 4 bytes of version/flags, 4 of pre-defined, then the type.
                b"hdlr" => handler = head.get(box_.body + 8..box_.body + 12),
                b"minf" => {
                    let Some(stbl) =
                        Boxes::new(head, box_.body, box_.end).find(|b| b.kind == *b"stbl")
                    else {
                        continue;
                    };
                    for child in Boxes::new(head, stbl.body, stbl.end) {
                        // Every table here is a full box: version/flags, then its own fields. The
                        // offsets differ per table, so each is read at its own layout rather than
                        // through one shared helper.
                        let be32 = |at: usize| -> Option<u64> {
                            let bytes = head.get(at..at + 4)?;
                            Some(u64::from(u32::from_be_bytes(
                                bytes.try_into().expect("four bytes"),
                            )))
                        };
                        match &child.kind {
                            // stco: version/flags, entry_count, then 32-bit chunk offsets.
                            b"stco" => {
                                let count = be32(child.body + 4).unwrap_or(0);
                                if count == 1 {
                                    chunk_offset =
                                        be32(child.body + 8).map(|offset| offset as usize);
                                }
                            }
                            // co64: the same, with 64-bit offsets.
                            b"co64" => {
                                let count = be32(child.body + 4).unwrap_or(0);
                                if count == 1 {
                                    // `?` here aborted the whole search for this file and threw
                                    // away an `stco` that had already been read; a table truncated
                                    // inside its own box skips only itself.
                                    chunk_offset =
                                        head.get(child.body + 8..child.body + 16).map(|bytes| {
                                            u64::from_be_bytes(
                                                bytes.try_into().expect("eight bytes"),
                                            ) as usize
                                        });
                                }
                            }
                            // stsz: version/flags, sample_size, sample_count, then the sizes.
                            b"stsz" => {
                                let uniform = be32(child.body + 4).unwrap_or(0);
                                let samples = be32(child.body + 8).unwrap_or(0);
                                if samples == 1 {
                                    sample_size = if uniform > 0 {
                                        Some(uniform)
                                    } else {
                                        be32(child.body + 12)
                                    };
                                } else {
                                    // A multi-sample or empty metadata track is not a shape this
                                    // parser handles; refusing beats reading the wrong sample.
                                    sample_size = None;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if handler != Some(b"meta") {
            continue;
        }
        let index = chunk_offset?;
        // The chunk offset is an absolute file offset, which for these files is far past the
        // header window — that is what the seek in `read_shutter_count` is for.
        let offset = index as u64;
        return Some((offset, sample_size?));
    }
    None
}

/// The CTMD sample is a little-endian box list. Records 7, 8 and 9 each hold a full EXIF tree.
fn shutter_count_in_sample(sample: &[u8]) -> Option<u64> {
    for box_ in LittleBoxes::new(sample, 0, sample.len()) {
        // The top two bits of the tag are flags; the low 16 bits are the record type.
        if !matches!(box_.tag & 0xffff, 7..=9) {
            continue;
        }
        let payload = sample.get(box_.body..box_.end)?;
        // Each ExifInfo payload opens with four flag bytes before its own little-endian boxes.
        for child in LittleBoxes::new(payload, 4, payload.len()) {
            if child.tag != 0x927c {
                continue;
            }
            let makernote = payload.get(child.body..child.end)?;
            // The MakerNote is a bare TIFF stream; older bodies prefix it with a length word, so
            // the header is located rather than assumed.
            let start = makernote
                .windows(2)
                .position(|w| w == b"II" || w == b"MM")?;
            let Some(tiff) = Tiff::new(makernote, start) else {
                continue;
            };
            let Some(entries) = tiff.ifd0() else {
                continue;
            };
            for entry in entries {
                // 0x000d is CanonCameraInfo, whose layout differs per body; the R6m2/R8 one keeps
                // the shutter count at a fixed offset.
                if entry.tag != 0x000d {
                    continue;
                }
                let Some(bytes) = tiff.value_bytes(&entry) else {
                    continue;
                };
                if let Some(count) = shutter_count_from_camera_info(bytes) {
                    return Some(count);
                }
            }
        }
    }
    None
}

/// The R8's `ShutterCount` is an int32 at a fixed offset in the `CameraInfo` blob. The blob
/// inherits the MakerNote's byte order, which is little-endian on every body this parser has seen,
/// so the bytes are read that way rather than guessing.
///
/// The value has to be plausible — a body that zeroes the blob, or a layout that moved, reads as
/// zero or a random number, and a nonsense sequence number would corrupt capture ordering.
fn shutter_count_from_camera_info(blob: &[u8]) -> Option<u64> {
    let bytes = blob.get(SHUTTER_COUNT_OFFSET..SHUTTER_COUNT_OFFSET + 4)?;
    let value = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
    // A camera that has fired at least once is at 1; anything above 2^24 has been this many years
    // at 20 fps, so it is not a real counter.
    (1..=0x0100_0000)
        .contains(&value)
        .then_some(u64::from(value))
}

/// Little-endian box walker, for the CTMD sample and the boxes inside it.
struct LittleBoxes<'a> {
    data: &'a [u8],
    offset: usize,
    end: usize,
}

/// A little-endian box. `tag` is a number, not a four-character code: the CTMD records are
/// identified by tag value, and the top two bits are flags rather than part of the record type.
struct LittleBox {
    tag: u32,
    body: usize,
    end: usize,
}

impl<'a> LittleBoxes<'a> {
    fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self {
            data,
            offset: start,
            end: end.min(data.len()),
        }
    }
}

impl<'a> Iterator for LittleBoxes<'a> {
    type Item = LittleBox;

    fn next(&mut self) -> Option<LittleBox> {
        if self.offset + 8 <= self.end {
            let start = self.offset;
            let size =
                u32::from_le_bytes(self.data[start..start + 4].try_into().expect("four bytes"))
                    as usize;
            let tag = u32::from_le_bytes(
                self.data[start + 4..start + 8]
                    .try_into()
                    .expect("four bytes"),
            );
            if size < 8 || start + size > self.end {
                return None;
            }
            self.offset = start + size;
            return Some(LittleBox {
                tag,
                body: start + 8,
                end: start + size,
            });
        }
        None
    }
}

// `BoxHeader.kind` is `[u8; 4]` everywhere, so the little-endian walker stores its u32 back into
// one. Keeping the conversion in one place means the two walkers cannot disagree about layout.

/// A minimal but structurally valid CR3, built to order, for tests and fixtures.
///
/// A real R8 file is 12 MB and lives outside the repo, so anything that needs a CR3 on disk — a
/// scanner test, a session test, the FFI smoke test — makes one of these instead. Only the boxes
/// the parser reads are written: `ftyp`, and a `moov` holding Canon's `uuid` with a `CMT1` (IFD0)
/// and a `CMT2` (Exif). Anything that needs a real camera's values still has to use a real file.
///
/// The camera is an R8 at 6000×4000 by default, because every caller wanted exactly that and the
/// only things tests actually vary are the timestamp and the sub-second digits — which is what the
/// setters are for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticCr3 {
    make: &'static str,
    model: &'static str,
    width: u16,
    height: u16,
    date_time_original: String,
    subsec: String,
    offset: &'static str,
    serial: &'static str,
}

impl SyntheticCr3 {
    /// A Canon EOS R8 frame, serial `122022006902`, at 6000×4000 in `-06:00`.
    #[must_use]
    pub fn r8() -> Self {
        SyntheticCr3 {
            make: "Canon",
            model: "Canon EOS R8",
            width: 6000,
            height: 4000,
            date_time_original: "2026:08:27 19:54:49".to_string(),
            subsec: "40".to_string(),
            offset: "-06:00",
            serial: "122022006902",
        }
    }

    /// `DateTimeOriginal`, the local wall-clock reading, `YYYY:MM:DD hh:mm:ss`.
    #[must_use]
    pub fn at(mut self, date_time_original: &str) -> Self {
        self.date_time_original = date_time_original.to_string();
        self
    }

    /// `SubSecTimeOriginal`. Two digits means 10 ms resolution, which is what an R8 writes.
    #[must_use]
    pub fn subsec(mut self, subsec: &str) -> Self {
        self.subsec = subsec.to_string();
        self
    }

    /// The bytes, ready to write to a `.CR3`.
    #[must_use]
    pub fn build(self) -> Vec<u8> {
        let Self {
            make,
            model,
            width,
            height,
            date_time_original,
            subsec,
            offset,
            serial,
        } = self;
        let cmt1 = tiff_stream(&[
            (0x0100, 3, width.to_le_bytes().to_vec()),
            (0x0101, 3, height.to_le_bytes().to_vec()),
            (0x010f, 2, make.as_bytes().to_vec()),
            (0x0110, 2, model.as_bytes().to_vec()),
            (0x0112, 3, 1u16.to_le_bytes().to_vec()),
        ]);
        let cmt2 = tiff_stream(&[
            (0x9003, 2, date_time_original.as_bytes().to_vec()),
            (0x9291, 2, subsec.as_bytes().to_vec()),
            (0x9011, 2, offset.as_bytes().to_vec()),
            (0xa431, 2, serial.as_bytes().to_vec()),
        ]);
        let mut uuid_body = Vec::new();
        uuid_body.extend_from_slice(&be_box(b"CNCV", b"synthetic "));
        uuid_body.extend_from_slice(&be_box(b"CMT1", &cmt1));
        uuid_body.extend_from_slice(&be_box(b"CMT2", &cmt2));
        uuid_body.extend_from_slice(&be_box(b"CMT3", &tiff_stream(&[])));

        let mut canon = UUID_CANON.to_vec();
        canon.extend_from_slice(&uuid_body);
        let mut moov_body = be_box(b"uuid", &canon);
        moov_body.extend_from_slice(&be_box(b"free", b""));

        let mut out = be_box(b"ftyp", b"crx \0\0\0\x01crx isom");
        out.extend_from_slice(&be_box(b"moov", &moov_body));
        out.extend_from_slice(&be_box(b"free", b""));
        out
    }
}

fn bytes_of(entries: &[(u16, u16, Vec<u8>)], index: usize) -> &[u8] {
    &entries[index].2
}

fn be_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A little-endian TIFF stream whose IFD0 holds `entries` as ASCII or SHORT.
fn tiff_stream(entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    // The `count` field is the element count, not the byte count: one SHORT is 1, and an ASCII
    // string is however many bytes it is. Getting this wrong truncates every string to 4 bytes.
    let element_count = |kind: u16, bytes: &[u8]| -> u32 {
        if kind == 3 {
            1
        } else {
            bytes.len().max(1) as u32
        }
    };
    // 8 bytes of header, 2 for the count, 12 per entry, 4 for the next-IFD pointer, then any
    // value too large to sit inline.
    let header = 8 + 2 + entries.len() * 12 + 4;
    let mut values_at = header;
    let mut inline = Vec::new();
    let mut external = Vec::new();
    let mut offsets = Vec::new();
    for (_, kind, bytes) in entries {
        let size = if *kind == 3 { 2 } else { bytes.len().max(1) };
        if size <= 4 {
            let mut slot = vec![0u8; 4];
            slot[..size.min(4)].copy_from_slice(&bytes[..size.min(4)]);
            inline.push(slot);
            offsets.push(0);
        } else {
            inline.push(vec![0u8; 4]);
            offsets.push(values_at);
            external.extend_from_slice(bytes);
            if bytes.len() % 2 == 1 {
                external.push(0);
            }
            values_at += bytes.len() + bytes.len() % 2;
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (index, (tag, kind, _)) in entries.iter().enumerate() {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        let count = element_count(*kind, bytes_of(entries, index));
        out.extend_from_slice(&count.to_le_bytes());
        if offsets[index] == 0 {
            out.extend_from_slice(&inline[index]);
        } else {
            out.extend_from_slice(&(offsets[index] as u32).to_le_bytes());
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&external);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_orientation_outside_one_to_eight_is_not_a_rotation() {
        // The contract says 1..=8. A camera that wrote 0 or 99 has not rotated the frame, and
        // pretending otherwise would flip the display.
        for value in [0, 9, 99, -1] {
            assert!(!((1..=8).contains(&value)));
        }
    }

    #[test]
    fn af_points_are_normalized_around_the_centre() {
        // A 600x800 point at Canon (0, 0) on a 6000x4000 AF image. Canon coordinates run from the
        // centre of the image, so x = (0 + 300 + 3000) / 6000 = 0.55 and y = (0 + 400 + 2000) /
        // 4000 = 0.6: the point's own centre, shifted by half the image.
        let mut record = vec![0u8; 16];
        let put = |record: &mut Vec<u8>, index: usize, value: i16| {
            record[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut record, AF_NUM_POINTS, 1);
        put(&mut record, AF_VALID_POINTS, 1);
        put(&mut record, AF_IMAGE_WIDTH, 6000);
        put(&mut record, AF_IMAGE_HEIGHT, 4000);
        // widths, heights, xs, ys, then the in-focus bitmask.
        record.extend_from_slice(&600i16.to_le_bytes());
        record.extend_from_slice(&800i16.to_le_bytes());
        record.extend_from_slice(&0i16.to_le_bytes());
        record.extend_from_slice(&0i16.to_le_bytes());
        record.extend_from_slice(&1i16.to_le_bytes());

        let af = af_info(&record).expect("a point");
        assert_eq!(af.image_width, 6000);
        assert_eq!(af.points_in_focus, vec![0]);
        let point = af.points[0];
        // f32 division, so the tolerance is well above f32 epsilon rather than exact.
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(point.x, 0.55), "got {x}", x = point.x);
        assert!(close(point.y, 0.60), "got {y}", y = point.y);
        assert!(close(point.w, 0.1), "600/6000 = 0.1, got {w}", w = point.w);
        assert!(close(point.h, 0.2), "800/4000 = 0.2, got {h}", h = point.h);
        assert!(point.in_focus);
    }

    #[test]
    fn an_af_point_outside_the_frame_is_clamped_not_dropped() {
        // The AF grid overhangs the cropped frame on some lenses. Clamping keeps the overlay
        // honest about where the point is; dropping it would silently change the focus story.
        let mut record = vec![0u8; 16];
        let put = |record: &mut Vec<u8>, index: usize, value: i16| {
            record[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut record, AF_NUM_POINTS, 1);
        put(&mut record, AF_VALID_POINTS, 1);
        put(&mut record, AF_IMAGE_WIDTH, 1000);
        put(&mut record, AF_IMAGE_HEIGHT, 1000);
        record.extend_from_slice(&10i16.to_le_bytes());
        record.extend_from_slice(&10i16.to_le_bytes());
        record.extend_from_slice(&(-9000i16).to_le_bytes());
        record.extend_from_slice(&(-9000i16).to_le_bytes());
        record.extend_from_slice(&0i16.to_le_bytes());

        let af = af_info(&record).expect("a point");
        assert_eq!(af.points.len(), 1, "the point is still reported");
        assert!((0.0..=1.0).contains(&af.points[0].x));
        assert!((0.0..=1.0).contains(&af.points[0].y));
    }

    #[test]
    fn the_in_focus_bitmask_spans_words() {
        // Point 20 is the fifth bit of the second word.
        let mut record = vec![0u8; 16];
        let put = |record: &mut Vec<u8>, index: usize, value: i16| {
            record[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut record, AF_NUM_POINTS, 32);
        put(&mut record, AF_VALID_POINTS, 0);
        put(&mut record, AF_IMAGE_WIDTH, 6000);
        put(&mut record, AF_IMAGE_HEIGHT, 4000);
        // Four point arrays of 32 entries each...
        for _ in 0..4 * 32 {
            record.extend_from_slice(&0i16.to_le_bytes());
        }
        // ...then 32 points = 2 bitmask words; bit 4 of word 1 is point 20.
        record.extend_from_slice(&0i16.to_le_bytes());
        record.extend_from_slice(&(1i16 << 4).to_le_bytes());

        let af = af_info(&record).expect("a grid");
        assert_eq!(af.points_in_focus, vec![20]);
        assert!(af.points.is_empty(), "no valid points, so no rectangles");
    }

    #[test]
    fn a_shutter_count_that_is_not_a_count_is_rejected() {
        // Zero means the body did not fill the blob; a 2^24-shutter camera has been firing for
        // 400 years. Either would wreck capture ordering if it were believed.
        let mut blob = vec![0u8; SHUTTER_COUNT_OFFSET + 4];
        assert!(shutter_count_from_camera_info(&blob).is_none());

        blob[SHUTTER_COUNT_OFFSET..SHUTTER_COUNT_OFFSET + 4]
            .copy_from_slice(&0x8301u32.to_le_bytes());
        assert_eq!(shutter_count_from_camera_info(&blob), Some(33537));

        blob[SHUTTER_COUNT_OFFSET..SHUTTER_COUNT_OFFSET + 4]
            .copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(shutter_count_from_camera_info(&blob).is_none());
    }

    #[test]
    fn a_truncated_camera_info_blob_is_not_read_out_of_range() {
        assert!(shutter_count_from_camera_info(&[0u8; 4]).is_none());
    }

    #[test]
    fn a_box_walker_stops_at_a_size_that_does_not_fit() {
        // A corrupt length must end the walk, not send the parser off the end of the buffer.
        let data = [0u8, 0, 0, 0xff, 0xff, b'm', b'o', b'o', b'v'];
        assert_eq!(Boxes::new(&data, 0, data.len()).count(), 0);

        let good = [0u8, 0, 0, 8, b'm', b'o', b'o', b'v'];
        assert_eq!(Boxes::new(&good, 0, good.len()).count(), 1);
    }

    #[test]
    fn a_zero_length_box_means_rest_of_container() {
        let mut data = vec![0u8, 0, 0, 0, b'f', b'r', b'e', b'e', 1, 2, 3, 4];
        data[0..4].copy_from_slice(&0u32.to_be_bytes());
        let found: Vec<([u8; 4], usize)> = Boxes::new(&data, 0, data.len())
            .map(|b| (b.kind, b.end))
            .collect();
        assert_eq!(found, vec![(*b"free", data.len())]);
    }

    #[test]
    fn a_synthetic_cr3_reads_back_the_values_it_was_built_with() {
        // The builder exists so the scanner and session tests can run without the 42 GB corpus.
        // It is only worth having if the parser round-trips it, so that is asserted here.
        let bytes = SyntheticCr3::r8().subsec("84").build();
        let path = std::env::temp_dir().join("firstcut-synthetic.CR3");
        std::fs::write(&path, &bytes).unwrap();
        let parsed = Cr3::parse(&path).expect("a synthetic file must parse");
        let _ = std::fs::remove_file(&path);

        assert_eq!(parsed.make.as_deref(), Some("Canon"));
        assert_eq!(parsed.model.as_deref(), Some("Canon EOS R8"));
        assert_eq!(parsed.width, 6000);
        assert_eq!(parsed.height, 4000);
        assert_eq!(parsed.orientation, Some(1));
        assert_eq!(
            parsed.date_time_original.as_deref(),
            Some("2026:08:27 19:54:49")
        );
        assert_eq!(parsed.subsec_time_original.as_deref(), Some("84"));
        assert_eq!(parsed.offset_time_original.as_deref(), Some("-06:00"));
        assert_eq!(parsed.body_serial_number.as_deref(), Some("122022006902"));
        // No preview box, so nothing is claimed rather than something wrong.
        assert!(parsed.preview.is_none());
    }

    #[test]
    fn a_corrupted_cr3_never_panics() {
        // Cards and cables corrupt files. Flipped bytes land in box sizes, IFD offsets and counts,
        // and a panic here would take the whole scan down with it, so every mutation must come back
        // as a value or an error.
        let whole = SyntheticCr3::r8().subsec("84").build();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mutated.CR3");
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3000 {
            let mut bytes = whole.clone();
            for _ in 0..1 + next() % 6 {
                let at = (next() as usize) % bytes.len();
                // Half the time a big word, which is what an offset or a count gone wrong reads as.
                if next() % 2 == 0 && at + 4 <= bytes.len() {
                    bytes[at..at + 4].copy_from_slice(&(next() as u32).to_be_bytes());
                } else {
                    bytes[at] = next() as u8;
                }
            }
            std::fs::write(&path, &bytes).unwrap();
            let _ = Cr3::parse(&path);
        }
        // And again over the box headers alone, where the sizes and the 64-bit `largesize` fields
        // live. The pass above only reaches those by luck, and a walker that trusts one of those
        // fields is what this module's first bug was.
        for _ in 0..3000 {
            let mut bytes = whole.clone();
            for _ in 0..1 + next() % 3 {
                let at = (next() as usize) % 48.min(bytes.len());
                if at + 4 <= bytes.len() {
                    bytes[at..at + 4].copy_from_slice(&(next() as u32).to_be_bytes());
                }
            }
            std::fs::write(&path, &bytes).unwrap();
            let _ = Cr3::parse(&path);
        }
    }

    #[test]
    fn an_unknown_tiff_type_is_skipped_rather_than_guessed() {
        // Type 0 and the reserved types have no size this parser knows, so they must produce no
        // values instead of a wrong number.
        assert_eq!(type_size(0), None);
        assert_eq!(type_size(99), None);
        assert_eq!(type_size(3), Some(2));
        assert_eq!(type_size(5), Some(8));
    }

    /// A little-endian TIFF with one IFD0, for the entries the synthetic builder cannot express.
    ///
    /// `tiff_stream` writes the element *count* field as the value's byte length, which is right
    /// only for the byte-sized types a synthetic CR3 uses — a two-million-element LONG or an
    /// SRATIONAL would come out with a count that puts its value outside the stream.
    fn tiff_with(entries: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
        let directory = 2 + entries.len() * 12 + 4;
        let mut out = b"II\x2a\x00".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut values = Vec::new();
        for (tag, kind, count, bytes) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            if type_size(*kind).unwrap_or(1) * *count as usize <= 4 {
                let mut inline = bytes.clone();
                inline.resize(4, 0);
                out.extend_from_slice(&inline);
            } else {
                out.extend_from_slice(&((8 + directory + values.len()) as u32).to_le_bytes());
                values.extend_from_slice(bytes);
                if values.len() % 2 == 1 {
                    values.push(0);
                }
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&values);
        out
    }

    #[test]
    fn a_box_walker_rejects_a_64_bit_size_that_would_overflow_the_offset() {
        // A `largesize` of `u64::MAX`. `start + size` left `usize` entirely: a panic in a debug
        // build, and a walk that went backwards over the header in a release one. The sibling
        // walkers already refused; this is the one that did not.
        let mut data = vec![
            0u8, 0, 0, 8, b'f', b'r', b'e', b'e', 0, 0, 0, 1, b'u', b'u', b'i', b'd',
        ];
        data.extend_from_slice(&[0xff; 8]);
        assert_eq!(Boxes::new(&data, 0, data.len()).count(), 1);
    }

    #[test]
    fn first_int_reads_one_element_without_materialising_the_array() {
        // Two million LONGs. `first_int` built the whole `Vec<i64>` to take its head, and it runs
        // for seven tags on every photograph: one crafted entry cost ~16 MB per call.
        let count = 2_000_000u32;
        let mut values = Vec::with_capacity(count as usize * 4);
        for index in 0..count {
            values.extend_from_slice(&index.to_le_bytes());
        }
        let bytes = tiff_with(&[(0x0100, 4, count, values)]);
        let tiff = Tiff::new(&bytes, 0).expect("a TIFF");
        let entry = tiff.ifd0().expect("an IFD0").next().expect("one entry");

        let started = std::time::Instant::now();
        assert_eq!(tiff.first_int(&entry), Some(0));
        let reading = started.elapsed();
        assert!(
            reading < std::time::Duration::from_millis(100),
            "reading one element of a {count}-element array took {reading:?}"
        );
        // It indexes into the array rather than always reading the head.
        assert_eq!(tiff.first_int_at(&entry, 1), Some(1));
        assert_eq!(tiff.first_int_at(&entry, 1_999_999), Some(1_999_999));
        assert_eq!(tiff.first_int_at(&entry, 2_000_000), None);
    }

    #[test]
    fn a_negative_exposure_compensation_is_a_negative_number() {
        // −1/3 EV is an SRATIONAL of `0xFFFFFFFF / 3`. Read as an unsigned rational it came out as
        // 1,431,655,765 EV, which passes the finite guard and every bounds check on the way to the
        // UI. All four exiftool fixtures record `0`, so the agreement test cannot see this.
        let mut meta = Cr3::default();
        let stream = tiff_with(&[(0x9204, 10, 1, srational(-1, 3))]);
        let tiff = Tiff::new(&stream, 0).expect("a TIFF");
        apply_exif(&tiff, tiff.ifd0().expect("an IFD0"), &mut meta);
        let ev = meta.exposure_comp_ev.expect("a value");
        assert!((ev + 1.0 / 3.0).abs() < 1e-6, "got {ev}");

        let mut meta = Cr3::default();
        let stream = tiff_with(&[(0x9204, 10, 1, srational(2, 3))]);
        let tiff = Tiff::new(&stream, 0).expect("a TIFF");
        apply_exif(&tiff, tiff.ifd0().expect("an IFD0"), &mut meta);
        let ev = meta.exposure_comp_ev.expect("a value");
        assert!((ev - 2.0 / 3.0).abs() < 1e-6, "got {ev}");
    }

    fn srational(numerator: i32, denominator: u32) -> Vec<u8> {
        let mut out = (numerator as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&denominator.to_le_bytes());
        out
    }

    #[test]
    fn an_af_grid_of_thirty_two_thousand_points_is_read_not_searched() {
        // `NumAFPoints` is an i16 straight out of the record, and each valid point used to ask
        // whether the whole in-focus list contained it: 32,767 × 32,767 comparisons, on a scan
        // worker thread, for a folder that then never opened.
        let count = i16::MAX as usize;
        let mut record = vec![0u8; 16];
        let put = |record: &mut Vec<u8>, index: usize, value: i16| {
            record[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut record, AF_NUM_POINTS, i16::MAX);
        put(&mut record, AF_VALID_POINTS, i16::MAX);
        put(&mut record, AF_IMAGE_WIDTH, 6000);
        put(&mut record, AF_IMAGE_HEIGHT, 4000);
        for _ in 0..4 * count {
            record.extend_from_slice(&0i16.to_le_bytes());
        }
        for _ in 0..count.div_ceil(16) {
            record.extend_from_slice(&u16::MAX.to_le_bytes());
        }

        let started = std::time::Instant::now();
        let af = af_info(&record).expect("a grid");
        let elapsed = started.elapsed();

        assert_eq!(af.points.len(), count);
        assert!(
            af.points_in_focus.contains(&0),
            "every bit of the mask is set"
        );
        assert!(af.points_in_focus.contains(&(count as u16 - 1)));
        assert!(af.points.iter().all(|point| point.in_focus));
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "a {count}-point AF record took {elapsed:?}"
        );
    }

    #[test]
    fn a_preview_range_is_bounded_by_the_box_that_declares_it() {
        // The `PRVW` length is a field in the file, and the range goes to Swift as a seek and a
        // read. `0xFFFF_FFFF` became a trusted 4 GiB range; the box is 46 bytes long here.
        let mut body = UUID_PRVW.to_vec();
        body.extend_from_slice(b"PRVW");
        // The offsets are from the marker itself: +0x0a width, +0x0c height, +0x10 length, +0x14
        // JPEG, so there are two reserved bytes between the dimensions and the length.
        body.extend_from_slice(&[0u8; 6]);
        body.extend_from_slice(&1620u16.to_be_bytes());
        body.extend_from_slice(&1080u16.to_be_bytes());
        body.extend_from_slice(&[0u8; 2]);
        body.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
        body.extend_from_slice(&[0xff, 0xd8, 0xff, 0xd9]);
        let head = be_box(b"uuid", &body);

        let preview = prvw_preview(&head, 8, head.len()).expect("a preview");
        assert_eq!((preview.width, preview.height), (1620, 1080));
        let room = head.len() - preview.range.offset as usize;
        assert!(
            preview.range.len as usize <= room,
            "claimed {} bytes of a box with {room} left in it",
            preview.range.len
        );
    }

    #[test]
    fn a_truncated_chunk_offset_table_does_not_cost_the_other_one() {
        // A `co64` that claims one entry and then ends. The `?` on its value aborted the whole
        // search for the file and threw away the `stco` that had already been read.
        let mut stbl = be_box(b"co64", &[0, 0, 0, 0, 0, 0, 0, 1]);
        let mut stco = vec![0u8; 4];
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&0x1234u32.to_be_bytes());
        stbl.extend_from_slice(&be_box(b"stco", &stco));
        let mut stsz = vec![0u8; 4];
        stsz.extend_from_slice(&64u32.to_be_bytes()); // a uniform sample_size
        stsz.extend_from_slice(&1u32.to_be_bytes()); // of one sample
        stbl.extend_from_slice(&be_box(b"stsz", &stsz));

        let mut minf = be_box(b"stbl", &stbl);
        minf.extend_from_slice(&be_box(b"free", b""));
        let mut mdia = be_box(b"minf", &minf);
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(b"meta");
        mdia.extend_from_slice(&be_box(b"hdlr", &hdlr));
        let moov = be_box(b"trak", &be_box(b"mdia", &mdia));

        assert_eq!(meta_sample(&moov, 0, moov.len()), Some((0x1234, 64)));
    }

    #[test]
    fn an_unreadable_ifd0_does_not_cost_the_exif_ifd() {
        // CMT1 is IFD0 and CMT2 is the Exif IFD: separate boxes, and the capture time is in CMT2.
        // Returning `Err` failed the whole file over a box the others did not need.
        let mut bytes = SyntheticCr3::r8().subsec("84").build();
        let at = bytes
            .windows(4)
            .position(|w| w == b"CMT1")
            .expect("a CMT1 box");
        // The TIFF byte-order mark is the first thing inside the box, right after its type.
        bytes[at + 4] = b'X';
        bytes[at + 5] = b'X';
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken-cmt1.CR3");
        std::fs::write(&path, &bytes).unwrap();

        let parsed = Cr3::parse(&path).expect("CMT2 is still readable");

        assert_eq!(
            parsed.date_time_original.as_deref(),
            Some("2026:08:27 19:54:49")
        );
        assert_eq!(parsed.body_serial_number.as_deref(), Some("122022006902"));
        // The IFD0 fields really are lost, and that is said rather than hidden.
        assert_eq!(parsed.width, 0);
        assert!(
            parsed.warnings.iter().any(|w| w.contains("CMT1")),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn canon_metadata_stops_at_the_end_of_the_uuid_box() {
        // `moov` holds the Canon `uuid` and then, as a sibling box, a second `CMT1` with a
        // different width. The walk ran to the end of the header window rather than to the uuid's
        // own end, so the stray box was adopted as Canon's IFD0.
        let dims = |width: u16| {
            tiff_stream(&[
                (0x0100, 3, width.to_le_bytes().to_vec()),
                (0x0101, 3, 4000u16.to_le_bytes().to_vec()),
            ])
        };
        let mut canon = UUID_CANON.to_vec();
        canon.extend_from_slice(&be_box(b"CMT1", &dims(6000)));
        let mut moov_body = be_box(b"uuid", &canon);
        moov_body.extend_from_slice(&be_box(b"CMT1", &dims(1234)));
        let mut bytes = be_box(b"ftyp", b"crx \0\0\0\x01crx isom");
        bytes.extend_from_slice(&be_box(b"moov", &moov_body));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("decoy.CR3");
        std::fs::write(&path, &bytes).unwrap();

        let parsed = Cr3::parse(&path).expect("a synthetic file");
        assert_eq!(
            parsed.width, 6000,
            "the real CMT1, not a sibling of the uuid box"
        );
    }
}
