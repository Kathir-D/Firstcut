//! The pure-data view of one photo's row (docs/contracts/session-api.md), lifted verbatim from
//! v1 `store::records` at the M1 port.
//!
//! `firstcut-fileops` reads it and `firstcut-catalog` reads/writes it, but the ladder gives
//! fileops no edge to the catalog (fileops is above it), so the shared record lives here in
//! `firstcut-types`. Everything that needs to talk SQLite stays in `firstcut-catalog`.

/// One photograph's row in the `photos` table of the session database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoRow {
    pub id: u64,
    /// Primary file relative to the session folder, '/' separated.
    pub rel_path: String,
    /// Shared base name of the group; the XMP sidecar is named after it.
    pub group_key: String,
    /// RAW + paired JPEG/HEIF + any existing `.xmp`.
    pub companions: Vec<String>,
    pub file_size: u64,
    pub mtime_ms: Option<i64>,
    pub device: Option<i64>,
    pub ino: Option<i64>,
    /// `PhotoMeta` as JSON, opaque to this schema.
    pub meta_json: Option<String>,
    /// Position in capture order, set by the batcher.
    pub ordinal: Option<i64>,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
    /// False once the file is gone; the row is kept so a rating survives a file coming back.
    pub present: bool,
}
