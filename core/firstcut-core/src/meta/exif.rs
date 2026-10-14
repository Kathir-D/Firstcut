//! Metadata for every file that is not a CR3: JPEG, HEIF, PNG, TIFF, and the TIFF-shaped RAW
//! formats (CR2, ARW, NEF, DNG, ORF, PEF, …), plus Fujifilm's RAF wrapper.
//!
//! todo.md §8: only Canon files were available for testing, so this reader is written from the
//! published container specs (TIFF 6.0, EXIF 2.32, JPEG/JFIF, ISO-BMFF, PNG) and checked with
//! synthetic files. The rule that matters most is the one in `meta/mod.rs`: **a file macOS can read
//! is never skipped.** Whatever this reader cannot understand comes back as a `Cr3` that is mostly
//! empty plus a warning, and the caller falls back to the file's own timestamp, flagged.
//!
//! The result is the same `Cr3` record the CR3 reader produces (the name is historical: it is the
//! "parsed header fields" record for every format), so `capture_time_from` and `meta_for` do not
//! care which reader ran.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::cr3::{self, ByteRange, Cr3, EmbeddedPreview, Tiff};
use super::{FileKind, RawFormat};

/// First read. Enough for a JPEG's APP1, a HEIC's `meta` box and the IFDs of most RAW files.
const FIRST_READ: u64 = 1 << 20;
/// The most a header is ever allowed to cost: a RAW whose Exif IFD sits past the first mebibyte
/// (some DNGs) is read again at this size, and one that is still not found is reported.
const MAX_READ: u64 = 16 << 20;

/// How many SubIFD offsets are followed. A RAW has one to three; the tag's `count` is a `u32` the
/// file chooses, and every offset costs a walk of an IFD whose own entry count is a `u16`, so an
/// unbounded list was ~10^12 reads inside a scan worker thread.
const MAX_SUB_IFDS: usize = 8;

/// Reads whatever `kind`'s container has to say about the photo.
///
/// Only an I/O error is an `Err`; a container this reader does not understand is a `Cr3` with a
/// warning, so the photo still appears.
pub(super) fn read(path: &Path, kind: FileKind, size: u64) -> Result<Cr3, String> {
    let mut file = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    read_from(&mut file, kind, size, &path.display().to_string())
}

/// Whether a capture time is even reachable in this container.
///
/// The ladder's exit condition is a `DateTimeOriginal`, so a format this module has no reader for
/// ran the full 1 MiB → 8 MiB → 16 MiB escalation — three reads and 25 MiB of allocation — for a
/// file that can never produce one, and the parse is identical at every rung.
fn carries_a_date(kind: FileKind) -> bool {
    !matches!(
        kind,
        FileKind::Raw(RawFormat::Crw) | FileKind::Raw(RawFormat::X3f)
    )
}

/// The escalation itself, over any reader. One buffer for the whole ladder: it used to build a
/// fresh `vec![0u8; len]` per rung, so the zeroing pass and the read pass both touched every page.
fn read_from<R: Read + Seek>(
    file: &mut R,
    kind: FileKind,
    size: u64,
    label: &str,
) -> Result<Cr3, String> {
    let mut head: Vec<u8> = Vec::new();
    // A CRW's record table is at the *end* of its heap, so it is read whole (CRWs are a few
    // mebibytes) rather than climbing a ladder whose lower rungs can never find it.
    let mut want = match kind {
        FileKind::Raw(RawFormat::Crw) => MAX_READ,
        _ => FIRST_READ,
    };
    loop {
        let len = size.min(want) as usize;
        head.clear();
        head.resize(len, 0);
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut head))
            .map_err(|e| format!("reading {label}: {e}"))?;
        let meta = parse(&head, kind);
        // Done when the capture time was found, or when there is nothing more to read.
        if !carries_a_date(kind)
            || meta.date_time_original.is_some()
            || size <= want
            || want >= MAX_READ
        {
            let far = match meta.date_time_original {
                None if carries_a_date(kind) => read_far_ifd0(file, &head, kind, size, label)?,
                _ => None,
            };
            let mut meta = far.unwrap_or(meta);
            if matches!(kind, FileKind::Raw(_)) {
                meta.preview = best_preview(file, size, &head, &meta.preview_candidates)
                    .or_else(|| scanned_preview(&head));
            }
            return Ok(meta);
        }
        want = (want * 8).min(MAX_READ);
    }
}

/// The largest file whose IFD0 is fetched from past [`MAX_READ`]. The buffer is the file's size but
/// zero-filled by the allocator, so only the two windows actually read are ever touched.
const MAX_SPARSE: u64 = 1 << 30;

/// A TIFF-shaped file whose IFD0 is beyond everything the ladder read. Phase One and Leaf write the
/// raw data first and the directories last, so a 22 MB IIQ keeps its IFD0 at byte 22,151,404.
///
/// The header is kept where it was and a window from IFD0 onward is read into a buffer laid out like
/// the file, so every offset the directories name still means what it says.
fn read_far_ifd0<R: Read + Seek>(
    file: &mut R,
    head: &[u8],
    kind: FileKind,
    size: u64,
    label: &str,
) -> Result<Option<Cr3>, String> {
    if !matches!(kind, FileKind::Tiff | FileKind::Raw(_)) || size > MAX_SPARSE {
        return Ok(None);
    }
    let Some(ifd0) = Tiff::new(head, 0).and_then(|t| t.u32_at(4)).map(u64::from) else {
        return Ok(None);
    };
    if ifd0 < head.len() as u64 || ifd0 >= size {
        return Ok(None);
    }
    let end = size.min(ifd0 + MAX_READ);
    let mut sparse = vec![0u8; size as usize];
    sparse[..head.len()].copy_from_slice(head);
    file.seek(SeekFrom::Start(ifd0))
        .and_then(|_| file.read_exact(&mut sparse[ifd0 as usize..end as usize]))
        .map_err(|e| format!("reading {label}: {e}"))?;
    Ok(Some(parse(&sparse, kind)))
}

/// Parses an in-memory header. Split from [`read`] so tests can hand it synthetic bytes.
pub(super) fn parse(head: &[u8], kind: FileKind) -> Cr3 {
    let mut meta = Cr3::default();
    match kind {
        FileKind::Jpeg => {
            jpeg(head, 0, &mut meta);
        }
        FileKind::Png => png(head, &mut meta),
        FileKind::Heif => heif(head, &mut meta),
        FileKind::Tiff => tiff_file(head, 0, &mut meta),
        FileKind::Raw(RawFormat::Raf) => raf(head, &mut meta),
        FileKind::Raw(RawFormat::Cr3) => {
            // The CR3 reader owns CR3; reaching here is a caller bug, not a bad file.
            meta.warnings
                .push("CR3 is read by the CR3 parser".to_string());
        }
        FileKind::Raw(RawFormat::Crw) => crw(head, &mut meta),
        FileKind::Raw(RawFormat::Mrw) => mrw(head, &mut meta),
        FileKind::Raw(RawFormat::X3f) => {
            meta.warnings
                .push("X3F has no reader; capture time is the file's".to_string());
        }
        // Some phones save a JPEG under a `.dng` name (a Galaxy S10's is JFIF, then the DNG).
        FileKind::Raw(_) if head.starts_with(&[0xff, 0xd8]) => {
            jpeg(head, 0, &mut meta);
        }
        FileKind::Raw(_) => tiff_file(head, 0, &mut meta),
    }
    meta
}

// ──────────────────────────────────────────────────────────────────────────────────── TIFF

/// A TIFF-shaped file: IFD0 at the header's offset, the Exif IFD through tag 0x8769.
///
/// `base` is where the TIFF header starts: 0 for a `.tif` or RAW, and the byte after `Exif\0\0`
/// for a JPEG's APP1. Every offset inside the stream is relative to it.
fn tiff_file(data: &[u8], base: usize, meta: &mut Cr3) {
    let Some(tiff) = Tiff::new(data, base) else {
        meta.warnings
            .push("not a TIFF stream (no byte-order mark)".to_string());
        return;
    };
    let Some(ifd0) = tiff.ifd0() else {
        meta.warnings
            .push("IFD0 is not inside the bytes read".to_string());
        return;
    };

    let mut exif_ifd = None;
    let mut sub_ifds = Vec::new();
    for entry in ifd0 {
        match entry.tag {
            0x8769 => exif_ifd = tiff.first_int(&entry),
            // SubIFDs: where DNG, ARW and NEF keep the full-size image whose dimensions IFD0 (the
            // preview) does not have. The count is a `u32` from the file and each offset starts
            // another walk of the stream, so the list is capped: a real file has one to three.
            0x014a => sub_ifds = tiff.ints(&entry).into_iter().take(MAX_SUB_IFDS).collect(),
            _ => {}
        }
    }
    // `apply_ifd0` consumes its iterator, so IFD0 is walked twice: once for the pointers above,
    // once for the fields. IFD0 is a dozen entries; this costs nothing.
    if let Some(entries) = tiff.ifd0() {
        cr3::apply_ifd0(&tiff, entries, meta);
    }
    let preview_size = (meta.width, meta.height);
    // Only a file's own IFDs hold previews; a JPEG's APP1 (base > 0) has a thumbnail at most.
    if base == 0 {
        collect_preview_candidates(&tiff, &sub_ifds, meta);
    }

    if let Some(offset) = exif_ifd.and_then(|o| usize::try_from(o).ok()) {
        exif_ifd_at(&tiff, offset, meta);
    } else {
        meta.warnings.push("no Exif IFD".to_string());
    }

    // Older bodies (Kodak's DCS, Canon's D2000, Leica's M8 DNG) write the capture settings into
    // IFD0 itself. Whatever the Exif IFD did not say is taken from there; the Exif IFD wins.
    if let Some(entries) = tiff.ifd0() {
        let mut from_ifd0 = Cr3::default();
        cr3::apply_exif(&tiff, entries, &mut from_ifd0);
        fill_missing(meta, from_ifd0);
    }
    // Panasonic's RW2 and RAW (TIFF magic 0x55 rather than 42) keep ISO in IFD0, tag 0x17.
    let panasonic_ifd0 = match meta.iso {
        None if tiff.u16_at(2) == Some(0x55) => tiff.ifd0(),
        _ => None,
    };
    for entry in panasonic_ifd0.into_iter().flatten() {
        if entry.tag == 0x0017 {
            meta.iso = tiff
                .first_int(&entry)
                .filter(|&v| (1..=i64::from(u32::MAX)).contains(&v))
                .map(|v| v as u32);
        }
    }

    // A RAW's IFD0 describes its embedded preview, which is smaller than the photo. The main
    // image is in a SubIFD (or is the Exif dimensions, read above); take the largest.
    for offset in sub_ifds {
        let Some(entries) = usize::try_from(offset)
            .ok()
            .and_then(|o| tiff.entries_at(o))
        else {
            continue;
        };
        let (mut w, mut h) = (0u32, 0u32);
        for entry in entries {
            match entry.tag {
                0x0100 => w = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
                0x0101 => h = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
                _ => {}
            }
        }
        if u64::from(w) * u64::from(h) > u64::from(meta.width) * u64::from(meta.height) {
            meta.width = w;
            meta.height = h;
        }
    }
    if meta.width == 0 || meta.height == 0 {
        (meta.width, meta.height) = preview_size;
    }
}

// ─────────────────────────────────────────────────────────────────────────── embedded JPEGs

/// The most JPEG candidates opened per file. A RAW carries two or three.
const MAX_PREVIEW_CANDIDATES: usize = 16;
/// How much of a candidate is read to find its frame header. A preview's own Exif APP1 can be
/// tens of kilobytes, so the SOF is not always in the first few hundred bytes.
const PREVIEW_PROBE: u64 = 256 << 10;

fn push_candidate(meta: &mut Cr3, offset: u64, len: u64) {
    let range = ByteRange { offset, len };
    if len >= 2
        && meta.preview_candidates.len() < MAX_PREVIEW_CANDIDATES
        && !meta.preview_candidates.contains(&range)
    {
        meta.preview_candidates.push(range);
    }
}

/// The places a TIFF-shaped RAW keeps a JPEG: IFD0, the IFDs chained after it, and the SubIFDs.
/// lightcraft's `extract_tiff_preview` reads the same tags (github.com/storytold/lightcraft).
fn collect_preview_candidates(tiff: &Tiff<'_>, sub_ifds: &[i64], meta: &mut Cr3) {
    let mut ifds: Vec<usize> = Vec::new();
    let mut next = tiff.u32_at(4).map(|o| o as usize);
    while let Some(at) = next {
        if at == 0 || ifds.contains(&at) || ifds.len() >= MAX_SUB_IFDS {
            break;
        }
        ifds.push(at);
        next = tiff
            .u16_at(at)
            .and_then(|n| tiff.u32_at(at + 2 + usize::from(n) * 12))
            .map(|o| o as usize);
    }
    ifds.extend(sub_ifds.iter().filter_map(|&o| usize::try_from(o).ok()));

    for at in ifds {
        let Some(entries) = tiff.entries_at(at) else {
            continue;
        };
        let (mut interchange, mut interchange_len) = (None, None);
        let (mut compression, mut strip, mut strip_len) = (None, None, None);
        for entry in entries {
            match entry.tag {
                0x0201 => interchange = tiff.first_int(&entry),
                0x0202 => interchange_len = tiff.first_int(&entry),
                0x0103 => compression = tiff.first_int(&entry),
                0x0111 if entry.count == 1 => strip = tiff.first_int(&entry),
                0x0117 if entry.count == 1 => strip_len = tiff.first_int(&entry),
                // An UNDEFINED blob that is itself a JPEG: Panasonic's JpgFromRaw (0x2e) and kin.
                _ if entry.kind == 7
                    && entry.count > 1024
                    && tiff.bytes(entry.value_offset, 2) == Some(&[0xff, 0xd8][..]) =>
                {
                    push_candidate(meta, entry.value_offset as u64, u64::from(entry.count));
                }
                _ => {}
            }
        }
        if let (Some(a), Some(l)) = (interchange, interchange_len) {
            push_candidate(meta, a.max(0) as u64, l.max(0) as u64);
        }
        // 6 is old-style JPEG, 7 is JPEG. A 7 strip can also be the raw data in lossless JPEG,
        // which `colour_jpeg_size` rejects by its frame type.
        if let (Some(6 | 7), Some(a), Some(l)) = (compression, strip, strip_len) {
            push_candidate(meta, a.max(0) as u64, l.max(0) as u64);
        }
    }
}

/// Opens each candidate and keeps the largest that is a colour, baseline or progressive JPEG.
fn best_preview<R: Read + Seek>(
    file: &mut R,
    size: u64,
    head: &[u8],
    candidates: &[ByteRange],
) -> Option<EmbeddedPreview> {
    let mut best: Option<EmbeddedPreview> = None;
    for &range in candidates {
        if range
            .offset
            .checked_add(range.len)
            .is_none_or(|end| end > size)
        {
            continue;
        }
        let probe = range.len.min(PREVIEW_PROBE) as usize;
        let start = range.offset as usize;
        let read;
        let bytes = match start
            .checked_add(probe)
            .and_then(|end| head.get(start..end))
        {
            Some(bytes) => bytes,
            None => {
                let mut buf = vec![0u8; probe];
                if file
                    .seek(SeekFrom::Start(range.offset))
                    .and_then(|_| file.read_exact(&mut buf))
                    .is_err()
                {
                    continue;
                }
                read = buf;
                &read[..]
            }
        };
        let Some((width, height)) = colour_jpeg_size(bytes) else {
            continue;
        };
        let area = |p: &EmbeddedPreview| u64::from(p.width) * u64::from(p.height);
        let candidate = EmbeddedPreview {
            range,
            width,
            height,
        };
        if best.as_ref().is_none_or(|b| area(&candidate) > area(b)) {
            best = Some(candidate);
        }
    }
    best
}

/// How many `FF D8 FF` starts the byte scan will try. Raw sensor data is noise, and noise contains
/// that sequence; the frame-header check rejects those quickly, but not for free.
const MAX_SCANNED_STARTS: usize = 64;

/// The largest complete colour JPEG anywhere in `head`, for a RAW whose directories name none:
/// Leaf's MOS and Olympus's older ORF keep theirs in maker-specific blocks. This is lightcraft's
/// last resort too (github.com/storytold/lightcraft), limited here to bytes already read.
fn scanned_preview(head: &[u8]) -> Option<EmbeddedPreview> {
    let mut best: Option<EmbeddedPreview> = None;
    let starts = head
        .windows(3)
        .enumerate()
        .filter(|(_, w)| *w == [0xff, 0xd8, 0xff])
        .map(|(i, _)| i)
        .take(MAX_SCANNED_STARTS);
    for start in starts {
        let jpeg = &head[start..];
        let Some((width, height)) = colour_jpeg_size(jpeg) else {
            continue;
        };
        let Some(len) = jpeg_length(jpeg) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| {
            u64::from(width) * u64::from(height) > u64::from(b.width) * u64::from(b.height)
        }) {
            best = Some(EmbeddedPreview {
                range: ByteRange {
                    offset: start as u64,
                    len: len as u64,
                },
                width,
                height,
            });
        }
    }
    best
}

/// Where a JPEG ends: its segments up to the scan, then the entropy-coded data up to `FF D9`.
/// Inside the scan a literal `FF` is stuffed as `FF 00` and restarts are `FF D0`..`FF D7`, so the
/// first other `FF xx` is a marker, and for a single-scan JPEG that marker is EOI.
fn jpeg_length(jpeg: &[u8]) -> Option<usize> {
    let mut at = 2usize;
    loop {
        if *jpeg.get(at)? != 0xff {
            return None;
        }
        let marker = *jpeg.get(at + 1)?;
        let length = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        at += 2 + length;
        if marker == 0xda {
            break;
        }
    }
    while at + 1 < jpeg.len() {
        if jpeg[at] == 0xff {
            match jpeg[at + 1] {
                0x00 | 0xd0..=0xd7 | 0xff => {}
                0xd9 => return Some(at + 2),
                // Another scan (progressive) or a table between scans: still the same image.
                _ => {}
            }
        }
        at += 1;
    }
    None
}

/// A JPEG's size, if it is one ImageIO will draw as a photo: SOF0, SOF1 or SOF2 with three
/// components. Lossless JPEG (SOF3, how CR2 and DNG store raw data) and greyscale are refused.
fn colour_jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
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
        if marker == 0xff {
            at += 1;
            continue;
        }
        if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            at += 2;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let frame = jpeg.get(at + 5..at + 10)?;
            let height = u32::from(u16::from_be_bytes([frame[0], frame[1]]));
            let width = u32::from(u16::from_be_bytes([frame[2], frame[3]]));
            let colour = frame[4] == 3 && matches!(marker, 0xc0..=0xc2);
            return (colour && width > 0 && height > 0).then_some((width, height));
        }
        at += 2 + length.max(2);
    }
    None
}

/// Copies every capture setting `from` has and `meta` does not.
fn fill_missing(meta: &mut Cr3, from: Cr3) {
    meta.date_time_original = meta.date_time_original.take().or(from.date_time_original);
    meta.subsec_time_original = meta
        .subsec_time_original
        .take()
        .or(from.subsec_time_original);
    meta.offset_time_original = meta
        .offset_time_original
        .take()
        .or(from.offset_time_original);
    meta.body_serial_number = meta.body_serial_number.take().or(from.body_serial_number);
    meta.lens_model = meta.lens_model.take().or(from.lens_model);
    meta.exposure_time_s = meta.exposure_time_s.or(from.exposure_time_s);
    meta.f_number = meta.f_number.or(from.f_number);
    meta.exposure_comp_ev = meta.exposure_comp_ev.or(from.exposure_comp_ev);
    meta.focal_length_mm = meta.focal_length_mm.or(from.focal_length_mm);
    meta.iso = meta.iso.or(from.iso);
    meta.metering_mode = meta.metering_mode.take().or(from.metering_mode);
}

/// The Exif IFD at `offset`: capture settings, Exif dimensions, and the MakerNote.
fn exif_ifd_at(tiff: &Tiff<'_>, offset: usize, meta: &mut Cr3) {
    let Some(entries) = tiff.entries_at(offset) else {
        meta.warnings
            .push("the Exif IFD is not inside the bytes read".to_string());
        return;
    };
    let mut maker_note = None;
    let (mut pixel_x, mut pixel_y) = (0u32, 0u32);
    for entry in entries {
        match entry.tag {
            0x927c => maker_note = Some((entry.value_offset, entry.count as usize)),
            0xa002 => pixel_x = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
            0xa003 => pixel_y = tiff.first_int(&entry).unwrap_or(0).max(0) as u32,
            _ => {}
        }
    }
    if let Some(entries) = tiff.entries_at(offset) {
        cr3::apply_exif(tiff, entries, meta);
    }
    if pixel_x > 0 && pixel_y > 0 {
        // Exif's own dimensions describe the photo, which IFD0's do not always.
        meta.width = pixel_x;
        meta.height = pixel_y;
    }
    if let Some((at, len)) = maker_note {
        maker_note_at(tiff, at, len, meta);
    }
}

/// Canon (CR2) and Nikon MakerNotes. Sony and the rest are not decoded: their useful fields are
/// encrypted or undocumented, and ordering falls back to time plus sub-seconds (todo.md §5.1).
fn maker_note_at(tiff: &Tiff<'_>, at: usize, len: usize, meta: &mut Cr3) {
    let make = meta.make.as_deref().unwrap_or("").to_ascii_lowercase();
    if make.starts_with("canon") {
        // Canon's MakerNote is a plain IFD whose offsets are relative to the file's TIFF header.
        if let Some(entries) = tiff.entries_at(at) {
            cr3::apply_makernote(tiff, entries, meta);
        }
    } else if make.starts_with("nikon") {
        nikon_maker_note(tiff, at, len, meta);
    }
}

/// `Nikon\0` + a version + a *second* TIFF header, with offsets relative to that inner header.
fn nikon_maker_note(tiff: &Tiff<'_>, at: usize, len: usize, meta: &mut Cr3) {
    let Some(magic) = tiff.bytes(at, 6) else {
        return;
    };
    if magic != b"Nikon\0" || len < 18 {
        return;
    }
    let inner = tiff.base() + at + 10;
    let Some(nikon) = Tiff::new(tiff.data(), inner) else {
        return;
    };
    let Some(entries) = nikon.ifd0() else {
        return;
    };
    for entry in entries {
        match entry.tag {
            // ShutterCount. Newer bodies also carry an encrypted copy; this plain tag is present
            // on most and absent on the rest, and absence is not an error.
            0x00a7 => {
                meta.shutter_count = nikon.first_int(&entry).filter(|&n| n > 0).map(|n| n as u64);
            }
            // SerialNumber, as ASCII digits.
            0x001d if meta.body_serial_number.is_none() => {
                meta.body_serial_number = nikon.string(&entry);
            }
            _ => {}
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────────── JPEG

/// Finds the APP1 `Exif\0\0` segment and reads it as a TIFF stream, and the frame size from the
/// SOF marker. `at` is where the JPEG starts, so RAF (which wraps one) can reuse it.
///
/// Returns whether an Exif segment was found.
fn jpeg(data: &[u8], at: usize, meta: &mut Cr3) -> bool {
    if data.get(at..at + 2) != Some(&[0xff, 0xd8]) {
        meta.warnings.push("not a JPEG (no SOI marker)".to_string());
        return false;
    }
    let mut pos = at + 2;
    let mut found = false;
    let mut have_size = false;
    while pos + 4 <= data.len() {
        if data[pos] != 0xff {
            // A stray byte is not the end of the header chain: `cr3::jpeg_dimensions` steps over
            // it and keeps walking, and stopping here threw away every marker after it — including
            // the SOF, so the JPEG came back 0×0 (or with only the Exif thumbnail's size).
            pos += 1;
            continue;
        }
        let marker = data[pos + 1];
        // Fill bytes, and markers with no length.
        if marker == 0xff {
            pos += 1;
            continue;
        }
        if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            pos += 2;
            continue;
        }
        // Start of scan: everything after is entropy-coded data, and the headers are done.
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = usize::from(u16::from_be_bytes([data[pos + 2], data[pos + 3]]));
        if length < 2 {
            break;
        }
        let body = pos + 4;
        let end = pos + 2 + length;
        match marker {
            0xe1 if !found && data.get(body..body + 6) == Some(b"Exif\0\0") => {
                // The stream can extend past `data` for a huge APP1; `Tiff` bounds-checks.
                found = true;
                tiff_file(data, body + 6, meta);
            }
            // SOF0..SOF15 except DHT (c4), JPG (c8) and DAC (cc): height then width, big-endian.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) && !have_size => {
                if let Some(frame) = data.get(body + 1..body + 5) {
                    let height = u32::from(u16::from_be_bytes([frame[0], frame[1]]));
                    let width = u32::from(u16::from_be_bytes([frame[2], frame[3]]));
                    if width > 0 && height > 0 {
                        have_size = true;
                        // The frame is the real pixel size and wins over a thumbnail's IFD0.
                        meta.width = width;
                        meta.height = height;
                    }
                }
            }
            _ => {}
        }
        pos = end;
    }
    if !found {
        meta.warnings.push("JPEG has no Exif segment".to_string());
    }
    found
}

// ───────────────────────────────────────────────────────────────────────────────────── PNG

/// PNG: IHDR for the size, an `eXIf` chunk (a bare TIFF stream) for everything else.
fn png(data: &[u8], meta: &mut Cr3) {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.get(..8) != Some(SIGNATURE) {
        meta.warnings.push("not a PNG".to_string());
        return;
    }
    let mut pos = 8;
    let mut found = false;
    while let (Some(len), Some(kind)) = (data.get(pos..pos + 4), data.get(pos + 4..pos + 8)) {
        let length = u32::from_be_bytes(len.try_into().expect("four bytes")) as usize;
        let body = pos + 8;
        match kind {
            b"IHDR" => {
                if let Some(dims) = data.get(body..body + 8) {
                    meta.width = u32::from_be_bytes(dims[..4].try_into().expect("four bytes"));
                    meta.height = u32::from_be_bytes(dims[4..].try_into().expect("four bytes"));
                }
            }
            b"eXIf" => {
                found = true;
                let (w, h) = (meta.width, meta.height);
                tiff_file(data, body, meta);
                // IHDR is the pixel size; the Exif dimensions can be stale after an edit.
                if w > 0 && h > 0 {
                    meta.width = w;
                    meta.height = h;
                }
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        // length + 4 bytes of CRC.
        pos = match body.checked_add(length).and_then(|p| p.checked_add(4)) {
            Some(next) => next,
            None => break,
        };
    }
    if !found {
        meta.warnings.push("PNG has no eXIf chunk".to_string());
    }
}

// ───────────────────────────────────────────────────────────────────────────────────── RAF

/// Fujifilm RAF: a 16-byte magic, then (at 84 and 88, big-endian) the offset and length of the
/// full-size embedded JPEG, whose Exif segment holds the capture settings.
fn raf(data: &[u8], meta: &mut Cr3) {
    if data.get(..16) != Some(b"FUJIFILMCCD-RAW ") {
        meta.warnings.push("not a RAF (bad magic)".to_string());
        return;
    }
    let offset = data
        .get(84..88)
        .map(|b| u32::from_be_bytes(b.try_into().expect("four bytes")) as usize);
    let Some(offset) = offset else {
        meta.warnings.push("RAF header is truncated".to_string());
        return;
    };
    if let Some(len) = data
        .get(88..92)
        .map(|b| u32::from_be_bytes(b.try_into().expect("four bytes")))
    {
        push_candidate(meta, offset as u64, u64::from(len));
    }
    // RAF's SOF describes the JPEG preview, not the sensor, so it is only a fallback size.
    jpeg(data, offset, meta);
}

// ───────────────────────────────────────────────────────────────────────────────────── CRW

/// How deep CIFF subdirectories are followed. A camera nests two levels; the cap is on the file.
const MAX_CIFF_DEPTH: usize = 4;
/// The most records one CIFF directory may claim (the count is a `u16` from the file).
const MAX_CIFF_ENTRIES: usize = 256;

/// Canon's CRW (CIFF, 1996–2004): `II`, the header length, `HEAPCCDR`, then a heap of records
/// whose directory sits at the offset stored in the heap's last four bytes. Subdirectories are
/// heaps of their own. Read as exiftool's `CanonRaw` module documents it.
fn crw(data: &[u8], meta: &mut Cr3) {
    if data.get(..2) != Some(b"II") || data.get(6..14) != Some(b"HEAPCCDR") {
        meta.warnings.push("not a CRW (no CIFF header)".to_string());
        return;
    }
    let start = le32(data, 2).unwrap_or(0) as usize;
    ciff_heap(data, start, data.len(), 0, meta);
    if meta.date_time_original.is_none() {
        meta.warnings
            .push("CRW has no readable timestamp; capture time is the file's".to_string());
    }
}

fn ciff_heap(data: &[u8], start: usize, end: usize, depth: usize, meta: &mut Cr3) {
    if depth > MAX_CIFF_DEPTH || end > data.len() || end < start.saturating_add(4) {
        return;
    }
    let Some(table) = le32(data, end - 4).and_then(|t| start.checked_add(t as usize)) else {
        return;
    };
    let Some(count) = le16(data, table) else {
        return;
    };
    for i in 0..usize::from(count).min(MAX_CIFF_ENTRIES) {
        let at = table + 2 + i * 10;
        let (Some(tag), Some(size), Some(offset)) =
            (le16(data, at), le32(data, at + 2), le32(data, at + 6))
        else {
            return;
        };
        let (size, offset) = (size as usize, offset as usize);
        // Storage: a value of eight bytes or fewer can live in the record itself.
        let in_record = tag & 0xc000 == 0x4000;
        let value_at = if in_record {
            at + 2
        } else {
            match start.checked_add(offset) {
                Some(v) => v,
                None => continue,
            }
        };
        let value_len = if in_record { 8 } else { size };
        let Some(value) = value_at
            .checked_add(value_len)
            .and_then(|e| data.get(value_at..e))
        else {
            continue;
        };
        // Data types 5 and 6 (0x2800, 0x3000) are subdirectories.
        if !in_record && matches!((tag >> 11) & 7, 5 | 6) {
            ciff_heap(data, value_at, value_at + size, depth + 1, meta);
            continue;
        }
        match tag & 0x3fff {
            // MakeModel: two NUL-terminated strings.
            0x080a => {
                let mut parts = value
                    .split(|&b| b == 0)
                    .map(|p| String::from_utf8_lossy(p).trim().to_string());
                meta.make = parts.next().filter(|s| !s.is_empty());
                meta.model = parts.next().filter(|s| !s.is_empty());
            }
            // TimeStamp: seconds since 1970 *in local time*, which is what DateTimeOriginal is.
            0x180e => {
                meta.date_time_original = le32(value, 0)
                    .filter(|&s| s > 0)
                    .map(|s| exif_stamp(i64::from(s)));
            }
            // ImageSpec: width and height first.
            0x1810 => {
                if let (Some(w), Some(h)) = (le32(value, 0), le32(value, 4)) {
                    meta.width = w;
                    meta.height = h;
                }
            }
            // JpgFromRaw: the camera's JPEG of the frame.
            0x2007 => push_candidate(meta, value_at as u64, size as u64),
            // FocalLength: [type, focal length in mm, …] as u16.
            0x1029 => {
                meta.focal_length_mm = le16(value, 2).filter(|&f| f > 0).map(f32::from);
            }
            _ => {}
        }
    }
}

fn le16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// `YYYY:MM:DD hh:mm:ss` for seconds since 1970, the shape `DateTimeOriginal` is written in.
fn exif_stamp(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days, the inverse of `meta::days_from_civil`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}:{month:02}:{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// ───────────────────────────────────────────────────────────────────────────────────── MRW

/// Minolta's MRW (Dimage, Dynax, and Sony's A100): `\0MRM`, the header length, then blocks of
/// `\0` + three letters + a big-endian length. `\0TTW` is a TIFF stream with IFD0 and the Exif IFD.
fn mrw(data: &[u8], meta: &mut Cr3) {
    if data.get(..4) != Some(b"\0MRM") {
        meta.warnings.push("not an MRW (bad magic)".to_string());
        return;
    }
    let header_end = data
        .get(4..8)
        .map(|b| 8 + u32::from_be_bytes(b.try_into().expect("four bytes")) as usize)
        .unwrap_or(0)
        .min(data.len());
    let mut pos = 8;
    while pos + 8 <= header_end {
        let name = &data[pos..pos + 4];
        let len = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().expect("four bytes"));
        let body = pos + 8;
        if name == b"\0TTW" {
            tiff_file(data, body, meta);
            return;
        }
        pos = match body.checked_add(len as usize) {
            Some(next) => next,
            None => break,
        };
    }
    meta.warnings.push("MRW has no TTW block".to_string());
}

// ──────────────────────────────────────────────────────────────────────────────────── HEIF

/// One ISO-BMFF box.
struct Bmff {
    kind: [u8; 4],
    body: usize,
    end: usize,
}

fn bmff_boxes(data: &[u8], start: usize, end: usize) -> Vec<Bmff> {
    let end = end.min(data.len());
    let mut out = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().expect("four bytes"));
        let kind: [u8; 4] = data[pos + 4..pos + 8].try_into().expect("four bytes");
        let (header, size) = match size32 {
            0 => (8, end - pos),
            1 => match data.get(pos + 8..pos + 16) {
                Some(b) => (
                    16,
                    u64::from_be_bytes(b.try_into().expect("eight bytes")) as usize,
                ),
                None => break,
            },
            n => (8, n as usize),
        };
        if size < header {
            break;
        }
        // A box that claims to run past its container is corrupt, and ending it at the container's
        // boundary reads its fields out of bytes that belong to the *next* box. `Boxes` in
        // `meta::cr3` ends the walk here instead, and so does this one.
        let Some(next) = pos.checked_add(size) else {
            break;
        };
        if next > end {
            break;
        }
        out.push(Bmff {
            kind,
            body: pos + header,
            end: next,
        });
        pos = next;
    }
    out
}

fn be(data: &[u8], at: usize, width: usize) -> Option<u64> {
    let bytes = data.get(at..at.checked_add(width)?)?;
    Some(bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
}

/// HEIF/HEIC: the `Exif` item, located through `iinf` (which item is Exif) and `iloc` (where it
/// is), whose payload is a 4-byte offset to a TIFF header followed by the TIFF stream.
fn heif(data: &[u8], meta: &mut Cr3) {
    let top = bmff_boxes(data, 0, data.len());
    if !top.iter().any(|b| &b.kind == b"ftyp") {
        meta.warnings.push("not an ISO-BMFF file".to_string());
        return;
    }
    let Some(meta_box) = top.iter().find(|b| &b.kind == b"meta") else {
        meta.warnings.push("HEIF has no meta box".to_string());
        return;
    };
    // `meta` is a full box: four bytes of version and flags before its children.
    let children = bmff_boxes(data, meta_box.body + 4, meta_box.end);

    // `ispe` first, and unconditionally: every early return below used to run before it, so a HEIC
    // whose Exif item could not be located reported 0×0 despite carrying the size in plain sight.
    heif_size(data, &children, meta);
    let mut exif_item: Option<u32> = None;
    if let Some(iinf) = children.iter().find(|b| &b.kind == b"iinf") {
        let version = data.get(iinf.body).copied().unwrap_or(0);
        // Version/flags (4), then a u16 entry count (v0) or a u32 one, then the `infe` boxes.
        let first_infe = if version == 0 {
            iinf.body + 6
        } else {
            iinf.body + 8
        };
        for infe in bmff_boxes(data, first_infe, iinf.end) {
            if &infe.kind != b"infe" {
                continue;
            }
            let v = data.get(infe.body).copied().unwrap_or(0);
            // version 2: u16 id; version 3: u32 id. Then u16 protection, then the 4CC type.
            let (id, type_at) = match v {
                2 => (be(data, infe.body + 4, 2), infe.body + 8),
                3 => (be(data, infe.body + 4, 4), infe.body + 10),
                _ => continue,
            };
            if data.get(type_at..type_at + 4) == Some(b"Exif") {
                exif_item = id.map(|i| i as u32);
                break;
            }
        }
    }
    let Some(item) = exif_item else {
        meta.warnings.push("HEIF has no Exif item".to_string());
        return;
    };

    let Some((offset, length)) = children
        .iter()
        .find(|b| &b.kind == b"iloc")
        .and_then(|iloc| iloc_extent(data, iloc, item))
    else {
        meta.warnings
            .push("the HEIF Exif item is not inside the bytes read".to_string());
        return;
    };
    let Some(payload) = data.get(offset..offset.saturating_add(length)) else {
        meta.warnings
            .push("the HEIF Exif item is not inside the bytes read".to_string());
        return;
    };
    // The payload begins with the length of any prefix before the TIFF header.
    let Some(prefix) = be(payload, 0, 4).map(|p| p as usize) else {
        meta.warnings
            .push("the HEIF Exif item has no TIFF header prefix".to_string());
        return;
    };
    tiff_file(data, offset.saturating_add(4).saturating_add(prefix), meta);
    // And again, because the rule is that `ispe` beats the Exif block: the first call is what makes
    // the early returns above report a size at all, this one is what still wins when the Exif block
    // disagrees. `heif_size` only ever grows the pair, and it is a short box walk.
    heif_size(data, &children, meta);
}

/// `ispe` (image spatial extent) inside `iprp/ipco`: the pixel size of the primary image, which
/// beats whatever the Exif block claims.
fn heif_size(data: &[u8], meta_children: &[Bmff], meta: &mut Cr3) {
    let Some(iprp) = meta_children.iter().find(|b| &b.kind == b"iprp") else {
        return;
    };
    for ipco in bmff_boxes(data, iprp.body, iprp.end) {
        if &ipco.kind != b"ipco" {
            continue;
        }
        for property in bmff_boxes(data, ipco.body, ipco.end) {
            if &property.kind == b"ispe" {
                // version/flags (4), width (4), height (4).
                if let (Some(w), Some(h)) = (
                    be(data, property.body + 4, 4),
                    be(data, property.body + 8, 4),
                ) {
                    let (w, h) = (w as u32, h as u32);
                    // The first ispe is the primary image in practice; a larger one is a grid.
                    if w > 0
                        && h > 0
                        && u64::from(w) * u64::from(h)
                            > u64::from(meta.width) * u64::from(meta.height)
                    {
                        meta.width = w;
                        meta.height = h;
                    }
                }
            }
        }
    }
}

/// The first extent of item `item` in an `iloc` box, as `(file offset, length)`.
fn iloc_extent(data: &[u8], iloc: &Bmff, item: u32) -> Option<(usize, usize)> {
    let version = *data.get(iloc.body)?;
    let sizes = *data.get(iloc.body + 4)?;
    let (offset_size, length_size) = (usize::from(sizes >> 4), usize::from(sizes & 0xf));
    let base_and_index = *data.get(iloc.body + 5)?;
    let base_size = usize::from(base_and_index >> 4);
    let index_size = if version == 1 || version == 2 {
        usize::from(base_and_index & 0xf)
    } else {
        0
    };
    let (count, mut pos) = if version < 2 {
        (be(data, iloc.body + 6, 2)?, iloc.body + 8)
    } else {
        (be(data, iloc.body + 6, 4)?, iloc.body + 10)
    };
    for _ in 0..count {
        let id_width = if version < 2 { 2 } else { 4 };
        let id = be(data, pos, id_width)? as u32;
        pos += id_width;
        if version == 1 || version == 2 {
            pos += 2; // construction method
        }
        pos += 2; // data reference index
        let base = be(data, pos, base_size).unwrap_or(0) as usize;
        pos += base_size;
        let extents = be(data, pos, 2)?;
        pos += 2;
        let mut first = None;
        for n in 0..extents {
            pos += index_size;
            let offset = be(data, pos, offset_size)? as usize;
            pos += offset_size;
            let length = be(data, pos, length_size)? as usize;
            pos += length_size;
            if n == 0 {
                // Both come from the file; a corrupt pair must not overflow.
                first = base.checked_add(offset).map(|at| (at, length));
            }
        }
        if id == item {
            return first;
        }
    }
    None
}

// ───────────────────────────────────────────────────────────────────────────────────── tests

#[cfg(test)]
pub(crate) mod fixtures {
    //! Byte-level builders for the containers above, so each reader is exercised on a file whose
    //! contents the test chose. Little-endian TIFF, like every camera in the test set.

    /// One IFD at stream offset `at`: `(tag, type, bytes)` entries, values of four bytes or fewer
    /// inline and the rest appended after the IFD, which is how a camera lays them out.
    pub fn ifd(at: usize, entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let table = 2 + entries.len() * 12 + 4;
        let mut external = Vec::new();
        let mut out = (entries.len() as u16).to_le_bytes().to_vec();
        for (tag, kind, bytes) in entries {
            let unit = match kind {
                3 => 2,
                4 => 4,
                _ => 1,
            };
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&((bytes.len() / unit) as u32).to_le_bytes());
            if bytes.len() <= 4 {
                let mut slot = bytes.clone();
                slot.resize(4, 0);
                out.extend_from_slice(&slot);
            } else {
                out.extend_from_slice(&((at + table + external.len()) as u32).to_le_bytes());
                external.extend_from_slice(bytes);
                if bytes.len() % 2 == 1 {
                    external.push(0);
                }
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&external);
        out
    }

    pub fn ascii(text: &str) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    /// A little-endian TIFF stream: IFD0 (make, model, orientation 6, Exif pointer) and an Exif
    /// IFD (ISO, `DateTimeOriginal`, sub-seconds, optionally pixel dimensions).
    pub fn tiff(
        date: &str,
        subsec: &str,
        iso: u16,
        make: &str,
        pixel: Option<(u32, u32)>,
    ) -> Vec<u8> {
        let ifd0 = |exif_at: u32| {
            vec![
                (0x010f, 2, ascii(make)),
                (0x0110, 2, ascii("Test Body")),
                (0x0112, 3, 6u16.to_le_bytes().to_vec()),
                (0x8769, 4, exif_at.to_le_bytes().to_vec()),
            ]
        };
        // The IFD's size does not depend on the pointer's value, so build it once to measure.
        let exif_at = 8 + ifd(8, &ifd0(0)).len();
        let mut exif = vec![
            (0x8827, 3, iso.to_le_bytes().to_vec()),
            (0x9003, 2, ascii(date)),
            (0x9291, 2, ascii(subsec)),
        ];
        if let Some((w, h)) = pixel {
            exif.push((0xa002, 4, w.to_le_bytes().to_vec()));
            exif.push((0xa003, 4, h.to_le_bytes().to_vec()));
        }

        let mut out = b"II\x2a\x00".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend(ifd(8, &ifd0(exif_at as u32)));
        assert_eq!(out.len(), exif_at);
        out.extend(ifd(exif_at, &exif));
        out
    }

    /// `Exif\0\0` + a TIFF stream, wrapped as the APP1 segment of a JPEG with a SOF0 of
    /// `width × height`, then a start-of-scan marker.
    pub fn jpeg(tiff: &[u8], width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(tiff);
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        // SOF0: length 17, precision 8, height, width, 3 components.
        out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        out.extend_from_slice(&[0xff, 0xda, 0x00, 0x02]);
        out
    }

    pub fn png(tiff: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0, 0, 0, 0]); // CRC: the reader does not check it
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"eXIf", tiff);
        chunk(&mut out, b"IDAT", &[]);
        out
    }

    /// A RAF: the 16-byte magic, padding, the JPEG offset at 84, then the JPEG.
    pub fn raf(jpeg: &[u8]) -> Vec<u8> {
        let mut out = b"FUJIFILMCCD-RAW ".to_vec();
        out.resize(84, 0);
        out.extend_from_slice(&148u32.to_be_bytes());
        out.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
        out.resize(148, 0);
        out.extend_from_slice(jpeg);
        out
    }

    fn bmff(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A HEIC with the `Exif` item declared in `iinf` (v2 `infe`), located by `iloc` (v0), and an
    /// `ispe` of `width × height`. `mdat` follows `meta`, so the offset is computed after.
    pub fn heic(tiff: &[u8], width: u32, height: u32) -> Vec<u8> {
        let ftyp = bmff(b"ftyp", b"heic\0\0\0\0mif1heic");

        let mut infe_body = vec![2, 0, 0, 0]; // version 2, flags
        infe_body.extend_from_slice(&7u16.to_be_bytes()); // item id 7
        infe_body.extend_from_slice(&0u16.to_be_bytes()); // protection
        infe_body.extend_from_slice(b"Exif");
        infe_body.extend_from_slice(b"\0"); // empty name
        let infe = bmff(b"infe", &infe_body);
        let mut iinf_body = vec![0, 0, 0, 0];
        iinf_body.extend_from_slice(&1u16.to_be_bytes());
        iinf_body.extend_from_slice(&infe);
        let iinf = bmff(b"iinf", &iinf_body);

        let mut ispe_body = vec![0, 0, 0, 0];
        ispe_body.extend_from_slice(&width.to_be_bytes());
        ispe_body.extend_from_slice(&height.to_be_bytes());
        let ipco = bmff(b"ipco", &bmff(b"ispe", &ispe_body));
        let iprp = bmff(b"iprp", &ipco);

        // iloc v0: offset_size 4, length_size 4, base_offset_size 0; one item, one extent.
        let build = |offset: u32| {
            let mut body = vec![0, 0, 0, 0, 0x44, 0x00];
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&7u16.to_be_bytes());
            body.extend_from_slice(&0u16.to_be_bytes());
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&offset.to_be_bytes());
            body.extend_from_slice(&((tiff.len() + 4) as u32).to_be_bytes());
            bmff(b"iloc", &body)
        };

        let mut meta_children = Vec::new();
        meta_children.extend_from_slice(&iinf);
        meta_children.extend_from_slice(&build(0));
        meta_children.extend_from_slice(&iprp);
        let meta_len = 8 + 4 + meta_children.len();
        let mdat_payload_at = ftyp.len() + meta_len + 8;

        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend_from_slice(&iinf);
        meta_body.extend_from_slice(&build(mdat_payload_at as u32));
        meta_body.extend_from_slice(&iprp);

        let mut payload = 0u32.to_be_bytes().to_vec(); // no prefix before the TIFF header
        payload.extend_from_slice(tiff);

        let mut out = ftyp;
        out.extend(bmff(b"meta", &meta_body));
        out.extend(bmff(b"mdat", &payload));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{self, heic, tiff};
    use super::*;

    fn tiff_bytes() -> Vec<u8> {
        tiff(
            "2026:08:27 19:54:49",
            "84",
            1600,
            "SONY",
            Some((6000, 4000)),
        )
    }

    fn assert_read(meta: &Cr3) {
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2026:08:27 19:54:49")
        );
        assert_eq!(meta.subsec_time_original.as_deref(), Some("84"));
        assert_eq!(meta.iso, Some(1600));
        assert_eq!(meta.make.as_deref(), Some("SONY"));
        assert_eq!(meta.model.as_deref(), Some("Test Body"));
        assert_eq!(meta.orientation, Some(6));
    }

    #[test]
    fn a_tiff_shaped_raw_is_read_from_its_ifds() {
        for raw in [
            RawFormat::Arw,
            RawFormat::Nef,
            RawFormat::Dng,
            RawFormat::Cr2,
            RawFormat::Orf,
        ] {
            let meta = parse(&tiff_bytes(), FileKind::Raw(raw));
            assert_read(&meta);
            assert_eq!((meta.width, meta.height), (6000, 4000), "{raw:?}");
        }
    }

    #[test]
    fn a_tif_file_is_read_the_same_way() {
        assert_read(&parse(&tiff_bytes(), FileKind::Tiff));
    }

    #[test]
    fn a_jpeg_is_read_from_its_exif_segment_and_frame_header() {
        let meta = parse(&fixtures::jpeg(&tiff_bytes(), 4000, 3000), FileKind::Jpeg);
        assert_read(&meta);
        // The SOF is the real pixel size and beats the Exif tag.
        assert_eq!((meta.width, meta.height), (4000, 3000));
    }

    #[test]
    fn a_jpeg_without_exif_still_yields_its_size_and_a_warning() {
        let plain = fixtures::jpeg(&[], 1200, 800);
        // An empty TIFF stream has no byte-order mark, which is as good as no Exif.
        let meta = parse(&plain, FileKind::Jpeg);
        assert_eq!((meta.width, meta.height), (1200, 800));
        assert!(meta.date_time_original.is_none());
        assert!(!meta.warnings.is_empty());
    }

    #[test]
    fn a_png_reads_ihdr_and_its_exif_chunk() {
        let meta = parse(&fixtures::png(&tiff_bytes(), 2048, 1024), FileKind::Png);
        assert_read(&meta);
        assert_eq!((meta.width, meta.height), (2048, 1024));
    }

    #[test]
    fn a_raf_is_read_through_its_embedded_jpeg() {
        let meta = parse(
            &fixtures::raf(&fixtures::jpeg(&tiff_bytes(), 1000, 700)),
            FileKind::Raw(RawFormat::Raf),
        );
        assert_read(&meta);
    }

    #[test]
    fn a_heic_exif_item_is_found_through_iinf_and_iloc() {
        let meta = parse(&heic(&tiff_bytes(), 4032, 3024), FileKind::Heif);
        assert_read(&meta);
        assert_eq!(
            (meta.width, meta.height),
            (6000, 4000),
            "ispe never shrinks a larger Exif size"
        );
        let small = parse(
            &heic(
                &tiff("2026:01:02 03:04:05", "", 100, "Apple", None),
                4032,
                3024,
            ),
            FileKind::Heif,
        );
        assert_eq!(
            small.date_time_original.as_deref(),
            Some("2026:01:02 03:04:05")
        );
        assert_eq!((small.width, small.height), (4032, 3024));
    }

    #[test]
    fn garbage_is_a_warning_not_a_failure() {
        for kind in [
            FileKind::Jpeg,
            FileKind::Png,
            FileKind::Heif,
            FileKind::Tiff,
            FileKind::Raw(RawFormat::Arw),
            FileKind::Raw(RawFormat::Raf),
            FileKind::Raw(RawFormat::Crw),
            FileKind::Raw(RawFormat::X3f),
        ] {
            let meta = parse(&[0x13, 0x37, 0x00, 0xff, 0xd8, 0xff], kind);
            assert!(meta.date_time_original.is_none(), "{kind:?}");
            assert!(!meta.warnings.is_empty(), "{kind:?} gave no warning");
        }
        // And nothing indexes past the end of an empty file.
        for kind in [
            FileKind::Jpeg,
            FileKind::Png,
            FileKind::Heif,
            FileKind::Tiff,
        ] {
            let _ = parse(&[], kind);
        }
    }

    #[test]
    fn a_corrupted_file_never_panics() {
        // Flipped bytes land in lengths, offsets and counts; each format must survive them.
        let samples = [
            (fixtures::jpeg(&tiff_bytes(), 4000, 3000), FileKind::Jpeg),
            (heic(&tiff_bytes(), 4032, 3024), FileKind::Heif),
            (fixtures::png(&tiff_bytes(), 10, 10), FileKind::Png),
            (tiff_bytes(), FileKind::Tiff),
            (tiff_bytes(), FileKind::Raw(crate::meta::RawFormat::Nef)),
            (
                fixtures::raf(&fixtures::jpeg(&tiff_bytes(), 40, 30)),
                FileKind::Raw(crate::meta::RawFormat::Raf),
            ),
        ];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for (whole, kind) in &samples {
            for _ in 0..3000 {
                let mut bytes = whole.clone();
                for _ in 0..1 + next() % 6 {
                    let at = (next() as usize) % bytes.len();
                    if next() % 2 == 0 && at + 4 <= bytes.len() {
                        bytes[at..at + 4].copy_from_slice(&(next() as u32).to_be_bytes());
                    } else {
                        bytes[at] = next() as u8;
                    }
                }
                let _ = parse(&bytes, *kind);
            }
        }
    }

    #[test]
    fn a_truncated_file_never_panics() {
        let whole = fixtures::jpeg(&tiff_bytes(), 4000, 3000);
        for cut in 0..whole.len() {
            let _ = parse(&whole[..cut], FileKind::Jpeg);
        }
        let heic = heic(&tiff_bytes(), 4032, 3024);
        for cut in 0..heic.len() {
            let _ = parse(&heic[..cut], FileKind::Heif);
        }
        let png = fixtures::png(&tiff_bytes(), 10, 10);
        for cut in 0..png.len() {
            let _ = parse(&png[..cut], FileKind::Png);
        }
    }

    #[test]
    fn a_heic_that_cannot_find_its_exif_item_still_knows_its_size() {
        // `ispe` sits in the plain sight of the `meta` box, and it used to be read *after* every
        // early return: no Exif item, no `iloc` entry, no payload prefix — all three left the photo
        // 0×0.
        let mut bytes = heic(&tiff_bytes(), 4032, 3024);
        let at = bytes
            .windows(4)
            .position(|w| w == b"Exif")
            .expect("an Exif item type");
        bytes[at..at + 4].copy_from_slice(b"null");

        let meta = parse(&bytes, FileKind::Heif);

        assert_eq!((meta.width, meta.height), (4032, 3024));
        assert!(meta.date_time_original.is_none());
        assert!(
            !meta.warnings.is_empty(),
            "the missing item is still reported"
        );
    }

    #[test]
    fn a_jpeg_with_a_stray_byte_before_its_frame_header_still_knows_its_size() {
        // One `0x00` between the APP1 and the SOF0. The walk broke on it, losing every marker after
        // it — including the frame header, so the JPEG came back with only the Exif thumbnail's
        // dimensions instead of the real ones.
        let whole = fixtures::jpeg(&tiff_bytes(), 4000, 3000);
        let sof = whole
            .windows(2)
            .position(|w| w == [0xff, 0xc0])
            .expect("a SOF0");
        let mut bytes = whole;
        bytes.insert(sof, 0x00);

        let meta = parse(&bytes, FileKind::Jpeg);

        assert_eq!(
            (meta.width, meta.height),
            (4000, 3000),
            "the SOF is the real size"
        );
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2026:08:27 19:54:49")
        );
    }

    #[test]
    fn a_box_that_claims_to_run_past_its_container_ends_the_walk() {
        // Ending a corrupt box at its container's boundary reads its fields out of the *next*
        // box's bytes, which is worse than not walking it at all.
        let mut data = 255u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"free");
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"moov");

        let walk = bmff_boxes(&data, 0, data.len());
        assert!(walk.is_empty(), "{} boxes walked", walk.len());
        // A box that fits is still walked, and a zero size still means "to the end".
        let good = heic(&tiff_bytes(), 10, 10);
        assert!(bmff_boxes(&good, 0, good.len()).len() >= 3);
    }

    /// A TIFF whose IFD0 claims `sub_ifds` SubIFDs, each with its own dimensions.
    fn tiff_with_sub_ifds(width: u32, height: u32, sub_ifds: &[(u32, u32)]) -> Vec<u8> {
        let array_at = 8 + 2 + 12 * 3 + 4;
        let first_sub_at = array_at + sub_ifds.len() * 4;
        // Tag 0x014a's value is the ARRAY of offsets, not one offset, so the tag has to carry
        // every one of them: `ifd` derives the element count from the byte length, and anything
        // over four bytes is written out of line at `array_at`.
        let offsets: Vec<u8> = (0..sub_ifds.len())
            .flat_map(|index| ((first_sub_at + index * 30) as u32).to_le_bytes())
            .collect();
        let entries = vec![
            (0x0100, 4, width.to_le_bytes().to_vec()),
            (0x0101, 4, height.to_le_bytes().to_vec()),
            (0x014a, 4, offsets),
        ];
        let mut out = b"II\x2a\x00".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend(fixtures::ifd(8, &entries));
        for (w, h) in sub_ifds {
            out.extend(fixtures::ifd(
                0,
                &[
                    (0x0100, 4, w.to_le_bytes().to_vec()),
                    (0x0101, 4, h.to_le_bytes().to_vec()),
                ],
            ));
        }
        out
    }

    #[test]
    fn only_the_first_few_sub_ifds_of_a_raw_are_followed() {
        // Tag 0x014a's count is a `u32` from the file and each offset starts another walk of an
        // IFD. A real RAW has one to three; this one claims eleven, so the last one — the largest —
        // is past the cap and is not read.
        let mut sub = vec![(100u32, 80u32)];
        sub.extend((1..=10u32).map(|i| (5000 + i * 10, 4000)));

        let meta = parse(
            &tiff_with_sub_ifds(60, 40, &sub),
            FileKind::Raw(RawFormat::Dng),
        );

        assert_eq!(
            (meta.width, meta.height),
            (5070, 4000),
            "the largest of the first eight, not of all eleven"
        );
    }

    #[test]
    fn an_ifd_directory_that_claims_sixty_five_thousand_entries_is_walked_briefly() {
        // The count is a `u16` and every entry costs three reads of the stream. A camera writes tens
        // of them; the entry at 600 is past the cap, so a file that claims 65,535 of them is a
        // few hundred reads rather than a few hundred thousand.
        let mut entries = vec![(0x0100u16, 4u16, 100u32.to_le_bytes().to_vec())];
        for _ in 0..599 {
            entries.push((0x0112, 3, 1u16.to_le_bytes().to_vec()));
        }
        entries.push((0x0100, 4, 9000u32.to_le_bytes().to_vec()));

        let mut out = b"II\x2a\x00".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        for (tag, kind, bytes) in &entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            let mut inline = bytes.clone();
            inline.resize(4, 0);
            out.extend_from_slice(&inline);
        }
        out.extend_from_slice(&0u32.to_le_bytes());

        let started = std::time::Instant::now();
        let meta = parse(&out, FileKind::Raw(RawFormat::Arw));
        let elapsed = started.elapsed();

        assert_eq!(meta.width, 100, "entry 600 is past the cap");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "a 65,535-entry directory took {elapsed:?}"
        );
    }

    #[test]
    fn a_format_this_module_has_no_reader_for_is_read_once_however_big_it_is() {
        // Counts the reads, so "one read" and "the whole ladder" are different answers.
        struct Counting {
            inner: std::io::Cursor<Vec<u8>>,
            reads: usize,
        }
        impl Read for Counting {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                self.inner.read(buf)
            }
        }
        impl Seek for Counting {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(pos)
            }
        }

        // A CIFF-ish header and then five mebibytes of nothing. The ladder's exit condition is a
        // `DateTimeOriginal`, which a CRW can never produce, so it used to run all three rungs —
        // and re-allocate 25 MiB — to arrive at the same answer three times.
        let mut whole = vec![0u8; 5 << 20];
        whole[..4].copy_from_slice(b"HEAP");
        let size = whole.len() as u64;
        let mut reader = Counting {
            inner: std::io::Cursor::new(whole),
            reads: 0,
        };
        let meta =
            read_from(&mut reader, FileKind::Raw(RawFormat::Crw), size, "the crw").expect("a read");
        assert_eq!(reader.reads, 1, "one read, not the whole ladder");
        assert!(meta.date_time_original.is_none());
        assert!(!meta.warnings.is_empty(), "the format is still reported");

        // A format that can carry a date still escalates: its directories sit past the first mebibyte,
        // which is the whole reason the ladder exists. The TIFF header is at 0, as a camera writes
        // it; the IFDs are at 3 MiB, which is what a DNG with a large thumbnail looks like.
        let ifd0_at = 3usize << 20;
        // One entry, so the directory holds nothing that has to be stored *after* it: the Exif IFD
        // goes here, and the builder would otherwise put a value in the middle of it.
        let exif_at = ifd0_at + 2 + 12 + 4;
        let ifd0 = [(0x8769, 4, (exif_at as u32).to_le_bytes().to_vec())];
        let mut whole = b"II\x2a\x00".to_vec();
        whole.extend_from_slice(&(ifd0_at as u32).to_le_bytes());
        whole.resize(ifd0_at, 0);
        whole.extend(fixtures::ifd(ifd0_at, &ifd0));
        assert_eq!(
            whole.len(),
            exif_at,
            "the Exif IFD is where the pointer says it is"
        );
        whole.extend(fixtures::ifd(
            exif_at,
            &[
                (0x8827, 3, 1600u16.to_le_bytes().to_vec()),
                (0x9003, 2, fixtures::ascii("2026:08:27 19:54:49")),
                (0x9291, 2, fixtures::ascii("84")),
            ],
        ));
        let size = whole.len() as u64;
        let mut reader = Counting {
            inner: std::io::Cursor::new(whole),
            reads: 0,
        };
        let meta =
            read_from(&mut reader, FileKind::Raw(RawFormat::Arw), size, "the raw").expect("a read");
        assert_eq!(reader.reads, 2, "1 MiB, then the rest of the file");
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2026:08:27 19:54:49")
        );
        assert_eq!(meta.iso, Some(1600));
    }

    #[test]
    fn capture_settings_written_into_ifd0_are_read_from_there() {
        // A Kodak DCS or Leica M8 layout: no Exif IFD, the settings sit in IFD0 itself.
        let mut raw = b"II\x2a\x00".to_vec();
        raw.extend_from_slice(&8u32.to_le_bytes());
        raw.extend(fixtures::ifd(
            8,
            &[
                (0x010f, 2, fixtures::ascii("Kodak")),
                (0x8827, 3, 80u16.to_le_bytes().to_vec()),
                (0x9003, 2, fixtures::ascii("2017:04:14 12:34:50")),
            ],
        ));
        let meta = parse(&raw, FileKind::Raw(RawFormat::Dcr));
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2017:04:14 12:34:50")
        );
        assert_eq!(meta.iso, Some(80));
        assert_eq!(meta.make.as_deref(), Some("Kodak"));

        // Where both say something, the Exif IFD wins.
        let exif_at = 8 + 2 + 2 * 12 + 4;
        let mut both = b"II\x2a\x00".to_vec();
        both.extend_from_slice(&8u32.to_le_bytes());
        both.extend(fixtures::ifd(
            8,
            &[
                (0x8769, 4, (exif_at as u32).to_le_bytes().to_vec()),
                (0x8827, 3, 25u16.to_le_bytes().to_vec()),
            ],
        ));
        assert_eq!(both.len(), exif_at);
        both.extend(fixtures::ifd(
            exif_at,
            &[(0x8827, 3, 1600u16.to_le_bytes().to_vec())],
        ));
        assert_eq!(parse(&both, FileKind::Raw(RawFormat::Dng)).iso, Some(1600));
    }

    #[test]
    fn a_panasonic_rw2_reads_iso_from_ifd0() {
        let mut raw = b"IIU\x00".to_vec();
        raw.extend_from_slice(&8u32.to_le_bytes());
        raw.extend(fixtures::ifd(
            8,
            &[
                (0x0017, 3, 800u16.to_le_bytes().to_vec()),
                (0x010f, 2, fixtures::ascii("Panasonic")),
            ],
        ));
        assert_eq!(parse(&raw, FileKind::Raw(RawFormat::Rw2)).iso, Some(800));
        // Tag 0x17 means something else in an ordinary TIFF.
        raw[2] = 0x2a;
        assert_eq!(parse(&raw, FileKind::Raw(RawFormat::Dng)).iso, None);
    }

    #[test]
    fn an_ifd0_past_the_read_limit_is_fetched_from_where_it_is() {
        // Phase One's layout: the raw data first, the directories at the end of a big file.
        let ifd0_at = (MAX_READ as usize) + (3 << 20);
        let exif_at = ifd0_at + 2 + 12 + 4;
        let mut whole = b"II\x2a\x00".to_vec();
        whole.extend_from_slice(&(ifd0_at as u32).to_le_bytes());
        whole.resize(ifd0_at, 0);
        whole.extend(fixtures::ifd(
            ifd0_at,
            &[(0x8769, 4, (exif_at as u32).to_le_bytes().to_vec())],
        ));
        whole.extend(fixtures::ifd(
            exif_at,
            &[(0x9003, 2, fixtures::ascii("2017:12:02 20:25:16"))],
        ));
        let size = whole.len() as u64;
        let mut reader = std::io::Cursor::new(whole);
        let meta =
            read_from(&mut reader, FileKind::Raw(RawFormat::Iiq), size, "the iiq").expect("a read");
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2017:12:02 20:25:16")
        );

        // An IFD0 offset past the end of the file is a warning, not a read error.
        let mut short = b"II\x2a\x00".to_vec();
        short.extend_from_slice(&u32::MAX.to_le_bytes());
        let size = short.len() as u64;
        let meta = read_from(
            &mut std::io::Cursor::new(short),
            FileKind::Raw(RawFormat::Iiq),
            size,
            "the iiq",
        )
        .expect("not an error");
        assert!(meta.date_time_original.is_none());
    }

    #[test]
    fn a_jpeg_saved_as_a_dng_is_read_as_a_jpeg() {
        let jpeg = fixtures::jpeg(&tiff_bytes(), 4032, 3024);
        let meta = parse(&jpeg, FileKind::Raw(RawFormat::Dng));
        assert_read(&meta);
        assert_eq!((meta.width, meta.height), (4032, 3024));
    }

    /// A CRW: header, then a heap whose last four bytes point at its record table. One record is
    /// a subdirectory (type 5), the way a camera nests ImageProps.
    fn crw_bytes() -> Vec<u8> {
        let mut out = b"II".to_vec();
        out.extend_from_slice(&26u32.to_le_bytes());
        out.extend_from_slice(b"HEAPCCDR");
        out.resize(26, 0);
        let heap = out.len();

        // The subdirectory heap: a TimeStamp in the heap, and the focal length, which (storage
        // bits 0x4000) lives in the record's own eight bytes: [type 0, 44 mm, …] as u16.
        let mut sub = Vec::new();
        sub.extend_from_slice(&1_000_000_000u32.to_le_bytes()); // 2001-09-09 01:46:40
        sub.extend_from_slice(&[0; 8]);
        let table = sub.len();
        sub.extend_from_slice(&2u16.to_le_bytes());
        sub.extend_from_slice(&0x180eu16.to_le_bytes());
        sub.extend_from_slice(&12u32.to_le_bytes());
        sub.extend_from_slice(&0u32.to_le_bytes());
        sub.extend_from_slice(&0x5029u16.to_le_bytes());
        sub.extend_from_slice(&[0, 0, 0x2c, 0x00, 0, 0, 0, 0]);
        sub.extend_from_slice(&(table as u32).to_le_bytes());

        let mut root = Vec::new();
        let make_at = root.len();
        root.extend_from_slice(b"Canon\0Canon PowerShot Pro70\0");
        let sub_at = root.len();
        root.extend_from_slice(&sub);
        let table = root.len();
        root.extend_from_slice(&2u16.to_le_bytes());
        for (tag, size, offset) in [
            (0x080au16, 28u32, make_at),
            (0x300a, sub.len() as u32, sub_at),
        ] {
            root.extend_from_slice(&tag.to_le_bytes());
            root.extend_from_slice(&size.to_le_bytes());
            root.extend_from_slice(&(offset as u32).to_le_bytes());
        }
        root.extend_from_slice(&(table as u32).to_le_bytes());
        assert_eq!(heap, 26);
        out.extend(root);
        out
    }

    #[test]
    fn a_crw_is_read_through_its_ciff_heap() {
        let meta = parse(&crw_bytes(), FileKind::Raw(RawFormat::Crw));
        assert_eq!(meta.make.as_deref(), Some("Canon"));
        assert_eq!(meta.model.as_deref(), Some("Canon PowerShot Pro70"));
        assert_eq!(
            meta.date_time_original.as_deref(),
            Some("2001:09:09 01:46:40")
        );
        assert_eq!(meta.focal_length_mm, Some(44.0));
    }

    #[test]
    fn a_damaged_crw_is_a_warning() {
        // Every truncation parses without a panic; what it finds in the remains is not the point.
        let whole = crw_bytes();
        for cut in 0..whole.len() {
            let _ = parse(&whole[..cut], FileKind::Raw(RawFormat::Crw));
        }
        let mut wrong = crw_bytes();
        let end = wrong.len();
        wrong[end - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
        let meta = parse(&wrong, FileKind::Raw(RawFormat::Crw));
        assert!(!meta.warnings.is_empty());
    }

    #[test]
    fn an_mrw_is_read_through_its_ttw_block() {
        let tiff = tiff_bytes();
        let mut out = b"\0MRM".to_vec();
        let blocks_len = 8 + 4 + 8 + tiff.len();
        out.extend_from_slice(&(blocks_len as u32).to_be_bytes());
        out.extend_from_slice(b"\0PRD");
        out.extend_from_slice(&4u32.to_be_bytes());
        out.extend_from_slice(&[1, 2, 3, 4]);
        out.extend_from_slice(b"\0TTW");
        out.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
        out.extend_from_slice(&tiff);
        assert_read(&parse(&out, FileKind::Raw(RawFormat::Mrw)));
        assert!(
            !parse(b"\0MRM\0\0\0\x08\0PRD", FileKind::Raw(RawFormat::Mrw))
                .warnings
                .is_empty()
        );
    }

    #[test]
    fn the_exif_stamp_is_civil_time() {
        assert_eq!(exif_stamp(0), "1970:01:01 00:00:00");
        assert_eq!(exif_stamp(951_782_400), "2000:02:29 00:00:00");
    }

    /// A bare JPEG of `width × height`: SOF of the given type and component count, a scan, EOI.
    fn frame(sof: u8, components: u8, width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff, sof, 0x00, 0x11, 0x08];
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&[components, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        out.extend_from_slice(&[0xff, 0xda, 0x00, 0x02]);
        out.extend_from_slice(&[0x12, 0xff, 0x00, 0x34, 0xff, 0xd0, 0x56]); // stuffed FF, a restart
        out.extend_from_slice(&[0xff, 0xd9]);
        out
    }

    /// A RAW whose IFD0 points at `small` through 0x201/0x202 and whose IFD1 holds `large` as a
    /// JPEG-compressed strip, the way a NEF or ARW lays out its previews.
    fn raw_with_previews(small: &[u8], large: &[u8]) -> Vec<u8> {
        let ifd0_len = 2 + 2 * 12 + 4;
        let ifd1_at = 8 + ifd0_len;
        let ifd1_len = 2 + 3 * 12 + 4;
        let small_at = ifd1_at + ifd1_len;
        let large_at = small_at + small.len();
        let mut out = b"II\x2a\x00".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        let mut ifd0 = fixtures::ifd(
            8,
            &[
                (0x0201, 4, (small_at as u32).to_le_bytes().to_vec()),
                (0x0202, 4, (small.len() as u32).to_le_bytes().to_vec()),
            ],
        );
        let next = ifd0.len() - 4;
        ifd0[next..].copy_from_slice(&(ifd1_at as u32).to_le_bytes());
        out.extend(ifd0);
        out.extend(fixtures::ifd(
            ifd1_at,
            &[
                (0x0103, 3, 7u16.to_le_bytes().to_vec()),
                (0x0111, 4, (large_at as u32).to_le_bytes().to_vec()),
                (0x0117, 4, (large.len() as u32).to_le_bytes().to_vec()),
            ],
        ));
        assert_eq!(out.len(), small_at);
        out.extend_from_slice(small);
        out.extend_from_slice(large);
        out
    }

    fn preview_of(whole: Vec<u8>, kind: FileKind) -> Option<EmbeddedPreview> {
        let size = whole.len() as u64;
        read_from(&mut std::io::Cursor::new(whole), kind, size, "the raw")
            .expect("a read")
            .preview
    }

    #[test]
    fn a_raw_reports_its_largest_embedded_jpeg() {
        let small = frame(0xc0, 3, 160, 120);
        let large = frame(0xc0, 3, 1616, 1080);
        let whole = raw_with_previews(&small, &large);
        let preview = preview_of(whole.clone(), FileKind::Raw(RawFormat::Arw)).expect("a preview");
        assert_eq!((preview.width, preview.height), (1616, 1080));
        let at = preview.range.offset as usize;
        assert_eq!(&whole[at..at + preview.range.len as usize], &large[..]);
    }

    #[test]
    fn raw_data_in_lossless_jpeg_and_greyscale_jpegs_are_not_previews() {
        let small = frame(0xc0, 3, 160, 120);
        // A 7-compressed strip that is the sensor data (SOF3), as a CR2 or DNG stores it.
        let lossless = frame(0xc3, 3, 6000, 4000);
        let preview = preview_of(
            raw_with_previews(&small, &lossless),
            FileKind::Raw(RawFormat::Dng),
        )
        .expect("the thumbnail is still a preview");
        assert_eq!((preview.width, preview.height), (160, 120));
        let grey = frame(0xc0, 1, 6000, 4000);
        let preview = preview_of(
            raw_with_previews(&small, &grey),
            FileKind::Raw(RawFormat::Nef),
        )
        .expect("the thumbnail is still a preview");
        assert_eq!((preview.width, preview.height), (160, 120));
    }

    #[test]
    fn a_preview_range_past_the_end_of_the_file_is_dropped() {
        let small = frame(0xc0, 3, 160, 120);
        let large = frame(0xc0, 3, 1616, 1080);
        let mut whole = raw_with_previews(&small, &large);
        whole.truncate(whole.len() - 4);
        let preview = preview_of(whole, FileKind::Raw(RawFormat::Arw)).expect("the small one");
        assert_eq!((preview.width, preview.height), (160, 120));
    }

    #[test]
    fn a_tif_or_jpeg_does_not_report_a_preview() {
        let whole = raw_with_previews(&frame(0xc0, 3, 160, 120), &frame(0xc0, 3, 1616, 1080));
        assert!(preview_of(whole, FileKind::Tiff).is_none());
    }

    #[test]
    fn a_raf_reports_its_jpeg() {
        let jpeg = frame(0xc0, 3, 1280, 960);
        let whole = fixtures::raf(&jpeg);
        let preview = preview_of(whole, FileKind::Raw(RawFormat::Raf)).expect("a preview");
        assert_eq!((preview.width, preview.height), (1280, 960));
        assert_eq!(
            preview.range,
            ByteRange {
                offset: 148,
                len: jpeg.len() as u64
            }
        );
    }

    #[test]
    fn a_jpeg_with_no_directory_pointing_at_it_is_found_by_scanning() {
        // A Leaf MOS keeps its preview in a private block: noise, the JPEG, more noise.
        let jpeg = frame(0xc0, 3, 332, 244);
        let mut whole = b"MM\x00\x2a\x00\x00\x00\x08\x00\x00\x00\x00".to_vec();
        whole.extend_from_slice(&[0xff, 0xd8, 0xff, 0x00, 0x13]); // a false start in the noise
        let at = whole.len();
        whole.extend_from_slice(&jpeg);
        whole.extend_from_slice(&[0x55; 64]);
        let preview = preview_of(whole, FileKind::Raw(RawFormat::Mos)).expect("a preview");
        assert_eq!((preview.width, preview.height), (332, 244));
        assert_eq!(
            preview.range,
            ByteRange {
                offset: at as u64,
                len: jpeg.len() as u64
            }
        );
    }
}
