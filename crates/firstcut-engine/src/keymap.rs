//! The keymap: which chord runs which command, in which rating mode (docs/contracts/app-model.md
//! §10; todo.md §9.8).
//!
//! Ported from v1 `App/Sources/Input/{KeyChord,Keymap,KeymapStore}.swift`. Two files, one
//! effective map:
//!
//! * `resources/DefaultKeymap.json` — shipped, read-only, compiled in (`include_str!`).
//! * `…/Firstcut/keymap.json` — the user's overrides, holding only the commands they changed,
//!   written atomically. A corrupt user file falls back to the defaults: a broken preferences
//!   file must never stop someone culling a shoot.
//!
//! The rules the flat table needs, exactly as v1 documented them:
//!
//! * **Mode-specific bindings.** P is `flag.pick` in Stars mode and `keep.toggle` in Keep mode; a
//!   binding may name the modes it applies to, and a mode-specific binding beats a global one on
//!   the same chord.
//! * **Alternate chords.** ⌘← and `[` both mean `batch.previous`; several bindings may name one
//!   command, and the first is what menus show.
//! * **Caps Lock and fn are state, not intent.** They are stripped from every chord except the
//!   synthesized Caps Lock press itself.
//!
//! The engine owns this because the CLI/control surfaces and the recorder must agree with the
//! window; the UI only asks the map which chord fired and performs the result.

use std::collections::HashSet;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use firstcut_types::RatingMode;
use serde::{Deserialize, Serialize};

/// A physical key, spelled the way v1's `Key` raw values were so `keymap.json` stays readable and
/// diffable. The order is only for diagnostics; lookup is by equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyName {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    LeftArrow,
    RightArrow,
    UpArrow,
    DownArrow,
    Escape,
    Return,
    Tab,
    Space,
    Delete,
    ForwardDelete,
    Home,
    End,
    PageUp,
    PageDown,
    KeypadEnter,
    Backtick,
    Minus,
    Equal,
    LeftBracket,
    RightBracket,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    CapsLock,
    Function,
}

impl KeyName {
    /// What the menu bar, the cheat sheet and the recorder print. ASCII on purpose: the headless
    /// test font has no ⌘/⇧/⌥ glyphs, and the platform's own menu wording is words anyway.
    pub fn symbol(self) -> &'static str {
        match self {
            KeyName::A => "A",
            KeyName::B => "B",
            KeyName::C => "C",
            KeyName::D => "D",
            KeyName::E => "E",
            KeyName::F => "F",
            KeyName::G => "G",
            KeyName::H => "H",
            KeyName::I => "I",
            KeyName::J => "J",
            KeyName::K => "K",
            KeyName::L => "L",
            KeyName::M => "M",
            KeyName::N => "N",
            KeyName::O => "O",
            KeyName::P => "P",
            KeyName::Q => "Q",
            KeyName::R => "R",
            KeyName::S => "S",
            KeyName::T => "T",
            KeyName::U => "U",
            KeyName::V => "V",
            KeyName::W => "W",
            KeyName::X => "X",
            KeyName::Y => "Y",
            KeyName::Z => "Z",
            KeyName::Zero => "0",
            KeyName::One => "1",
            KeyName::Two => "2",
            KeyName::Three => "3",
            KeyName::Four => "4",
            KeyName::Five => "5",
            KeyName::Six => "6",
            KeyName::Seven => "7",
            KeyName::Eight => "8",
            KeyName::Nine => "9",
            KeyName::LeftArrow => "Left",
            KeyName::RightArrow => "Right",
            KeyName::UpArrow => "Up",
            KeyName::DownArrow => "Down",
            KeyName::Escape => "Esc",
            KeyName::Return => "Enter",
            KeyName::Tab => "Tab",
            KeyName::Space => "Space",
            KeyName::Delete => "Backspace",
            KeyName::ForwardDelete => "Del",
            KeyName::Home => "Home",
            KeyName::End => "End",
            KeyName::PageUp => "PgUp",
            KeyName::PageDown => "PgDn",
            KeyName::KeypadEnter => "Enter",
            KeyName::Backtick => "`",
            KeyName::Minus => "-",
            KeyName::Equal => "=",
            KeyName::LeftBracket => "[",
            KeyName::RightBracket => "]",
            KeyName::Backslash => "\\",
            KeyName::Semicolon => ";",
            KeyName::Quote => "'",
            KeyName::Comma => ",",
            KeyName::Period => ".",
            KeyName::Slash => "/",
            KeyName::CapsLock => "Caps Lock",
            KeyName::Function => "Fn",
        }
    }

    pub fn is_modifier_key(self) -> bool {
        matches!(self, KeyName::CapsLock | KeyName::Function)
    }
}

/// One modifier bit. `command` is the platform's own shortcut modifier (⌘ on the mac, Ctrl
/// elsewhere); `control` is the physical Ctrl key, which only matters for ⌃⌘F.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Modifier {
    #[serde(alias = "ctrl")]
    Control,
    #[serde(alias = "alt")]
    Option,
    Shift,
    #[serde(alias = "cmd", alias = "meta")]
    Command,
    CapsLock,
    #[serde(alias = "fn")]
    Function,
}

impl Modifier {
    pub fn label(self) -> &'static str {
        match self {
            Modifier::Control => "Ctrl",
            Modifier::Option => "Opt",
            Modifier::Shift => "Shift",
            Modifier::Command => "Cmd",
            Modifier::CapsLock => "Caps",
            Modifier::Function => "Fn",
        }
    }
}

/// A key plus modifiers. Modifiers are kept in the canonical order the JSON uses (control,
/// option, shift, command, capsLock, function), so equality and hashing are stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: KeyName,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

impl KeyChord {
    pub fn new(key: KeyName, modifiers: Vec<Modifier>) -> Self {
        let mut modifiers = modifiers;
        sort_modifiers(&mut modifiers);
        Self { key, modifiers }
    }

    /// What menus print: `Shift+Cmd+Z`, `Cmd+Left`, `P`, `Caps Lock`.
    pub fn display(&self) -> String {
        if self.key == KeyName::CapsLock {
            return KeyName::CapsLock.symbol().to_string();
        }
        let mut parts: Vec<&str> = self.modifiers.iter().map(|modifier| modifier.label()).collect();
        parts.push(self.key.symbol());
        parts.join("+")
    }

    /// Caps Lock and fn are state, not intent: ⇪ then P is still P. The one exception is the
    /// synthesized Caps Lock press itself, which is a real chord.
    pub fn normalized(&self) -> Self {
        if self.key.is_modifier_key() {
            return self.clone();
        }
        let modifiers = self.modifiers.iter().copied().filter(|modifier| !matches!(modifier, Modifier::CapsLock | Modifier::Function)).collect();
        KeyChord::new(self.key, modifiers)
    }
}

fn sort_modifiers(modifiers: &mut [Modifier]) {
    modifiers.sort_by_key(|modifier| match modifier {
        Modifier::Control => 0,
        Modifier::Option => 1,
        Modifier::Shift => 2,
        Modifier::Command => 3,
        Modifier::CapsLock => 4,
        Modifier::Function => 5,
    });
}

/// v1's `RatingMode` spelling in `keymap.json`: `"stars"` / `"keep"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeName {
    Stars,
    Keep,
}

impl ModeName {
    pub fn of(mode: RatingMode) -> Self {
        match mode {
            RatingMode::Stars => ModeName::Stars,
            RatingMode::KeepNotKeep => ModeName::Keep,
        }
    }
}

/// One line of the flat wire format:
/// `{ "command": "rate.stars", "argument": 3, "key": "three", "modifiers": [] }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    pub command: String,
    #[serde(default)]
    pub argument: Option<i64>,
    pub key: KeyName,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    /// `None` = every rating mode.
    #[serde(default)]
    pub modes: Option<Vec<ModeName>>,
}

impl KeyBinding {
    pub fn chord(&self) -> KeyChord {
        KeyChord::new(self.key, self.modifiers.clone())
    }

    pub fn applies(&self, mode: RatingMode) -> bool {
        match self.modes.as_deref() {
            None => true,
            Some(modes) => modes.contains(&ModeName::of(mode)),
        }
    }

    pub fn is_command(&self, command: &str, argument: Option<i64>) -> bool {
        self.command == command && self.argument == argument
    }
}

fn modes_overlap(left: Option<&[ModeName]>, right: Option<&[ModeName]>) -> bool {
    match (left, right) {
        (None, _) | (_, None) => true,
        (Some(left), Some(right)) => left.iter().any(|mode| right.contains(mode)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
struct KeymapDocument {
    #[serde(default = "document_version")]
    version: u64,
    #[serde(default)]
    bindings: Vec<KeyBinding>,
}

fn document_version() -> u64 {
    1
}

/// The bindings, in display order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Keymap {
    pub bindings: Vec<KeyBinding>,
}

impl Keymap {
    /// The compiled-in defaults. A malformed resource would leave an empty map; the unit test
    /// below fails the build gate if that ever happens, so the app never ships with it.
    pub fn defaults() -> Self {
        Self::from_json(include_str!("../resources/DefaultKeymap.json")).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let document: KeymapDocument = serde_json::from_str(json).map_err(|error| format!("keymap: {error}"))?;
        Ok(Self { bindings: document.bindings })
    }

    pub fn to_json(&self) -> Result<String, String> {
        let document = KeymapDocument { version: document_version(), bindings: self.bindings.clone() };
        serde_json::to_string_pretty(&document).map_err(|error| format!("keymap: {error}"))
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The binding a chord runs in `mode`, with mode-specific bindings winning over global ones
    /// and the earliest binding winning inside a group.
    pub fn binding(&self, chord: &KeyChord, mode: RatingMode) -> Option<&KeyBinding> {
        let chord = chord.normalized();
        let candidates: Vec<&KeyBinding> = self.bindings.iter().filter(|binding| binding.chord() == chord).collect();
        candidates
            .iter()
            .copied()
            .find(|binding| binding.modes.as_ref().is_some_and(|modes| modes.contains(&ModeName::of(mode))))
            .or_else(|| candidates.into_iter().find(|binding| binding.applies(mode)))
    }

    /// Every chord bound to a command, in display order.
    pub fn chords_for(&self, command: &str, argument: Option<i64>) -> Vec<KeyChord> {
        self.bindings.iter().filter(|binding| binding.is_command(command, argument)).map(KeyBinding::chord).collect()
    }

    /// Every distinct chord bound to a command regardless of argument (the cheat sheet's view:
    /// `rate.stars` is `0` through `5`, not one row per star).
    pub fn chords_ignoring_argument(&self, command: &str) -> Vec<KeyChord> {
        let mut chords: Vec<KeyChord> = Vec::new();
        for binding in self.bindings.iter().filter(|binding| binding.command == command) {
            let chord = binding.chord();
            if !chords.contains(&chord) {
                chords.push(chord);
            }
        }
        chords
    }

    /// What a menu prints: the first chord that is live in `mode`.
    pub fn primary_chord(&self, command: &str, argument: Option<i64>, mode: RatingMode) -> Option<KeyChord> {
        self.bindings.iter().find(|binding| binding.is_command(command, argument) && binding.applies(mode)).map(KeyBinding::chord)
    }

    pub fn primary_label(&self, command: &str, argument: Option<i64>, mode: RatingMode) -> Option<String> {
        self.primary_chord(command, argument, mode).map(|chord| chord.display())
    }

    /// Binds `chord` to a command, replacing whatever held it in an overlapping mode. With `mode`
    /// set the new binding only applies in that mode (Stars' P vs Keep's P).
    pub fn bind(&mut self, chord: &KeyChord, command: &str, argument: Option<i64>, mode: Option<ModeName>) {
        let chord = chord.normalized();
        let new_modes = mode.map(|mode| vec![mode]);
        self.bindings.retain(|existing| {
            if existing.chord() != chord {
                return true;
            }
            !modes_overlap(existing.modes.as_deref(), new_modes.as_deref())
        });
        self.bindings.push(KeyBinding { command: command.to_string(), argument, key: chord.key, modifiers: chord.modifiers, modes: new_modes });
    }

    /// Drops every binding of one command (used when a recorder writes a command's new chord set).
    pub fn remove_command(&mut self, command: &str, argument: Option<i64>) {
        self.bindings.retain(|binding| !binding.is_command(command, argument));
    }

    /// The user's map layered over the shipped defaults, exactly as v1's `Keymap.layered(over:)`:
    /// a user entry replaces the default on the same chord *and* every default chord of the same
    /// command, so "rebind batch navigation" cannot leave an old alternate firing.
    pub fn layered(&self, defaults: &Keymap) -> Keymap {
        let overridden_chords: HashSet<KeyChord> = self.bindings.iter().map(KeyBinding::chord).collect();
        let overridden_commands: HashSet<(String, Option<i64>)> =
            self.bindings.iter().map(|binding| (binding.command.clone(), binding.argument)).collect();
        let mut merged = defaults.clone();
        merged.bindings.retain(|binding| {
            !overridden_chords.contains(&binding.chord()) && !overridden_commands.contains(&(binding.command.clone(), binding.argument))
        });
        merged.bindings.extend(self.bindings.iter().cloned());
        merged
    }

    /// The modes a command's current bindings restrict it to, if any. The recorder carries this
    /// over so rebinding P in Stars mode does not make it fire in Keep mode.
    pub fn modes_for(&self, command: &str, argument: Option<i64>) -> Option<Vec<ModeName>> {
        self.bindings.iter().find(|binding| binding.is_command(command, argument)).and_then(|binding| binding.modes.clone())
    }
}

#[derive(Debug)]
pub enum KeymapError {
    Read { path: String, source: std::io::Error },
    Write { path: String, source: std::io::Error },
    Parse { path: String, reason: String },
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeymapError::Read { path, source } => write!(f, "reading {path}: {source}"),
            KeymapError::Write { path, source } => write!(f, "writing {path}: {source}"),
            KeymapError::Parse { path, reason } => write!(f, "reading {path}: {reason}"),
        }
    }
}

/// The defaults plus the user's overrides, and the file that holds them.
#[derive(Debug)]
pub struct KeymapStore {
    path: PathBuf,
    defaults: Keymap,
    overrides: Keymap,
}

impl KeymapStore {
    /// Reads `path`; an unreadable or corrupt user file is the defaults (v1's rule), never an error.
    pub fn at(path: &Path) -> Self {
        let overrides = std::fs::read(path)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| Keymap::from_json(&text).ok())
            .unwrap_or_default();
        Self { path: path.to_path_buf(), defaults: Keymap::defaults(), overrides }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn defaults(&self) -> &Keymap {
        &self.defaults
    }

    pub fn overrides(&self) -> &Keymap {
        &self.overrides
    }

    pub fn effective(&self) -> Keymap {
        self.overrides.layered(&self.defaults)
    }

    pub fn set_overrides(&mut self, overrides: Keymap) -> Result<(), KeymapError> {
        self.overrides = overrides;
        self.save()
    }

    /// Throws away every user override.
    pub fn reset_to_defaults(&mut self) -> Result<(), KeymapError> {
        self.set_overrides(Keymap::default())
    }

    /// Replaces the overrides with the file at `path` and saves them as the user map.
    pub fn import(&mut self, path: &Path) -> Result<(), KeymapError> {
        let path_string = path.display().to_string();
        let bytes = std::fs::read(path).map_err(|source| KeymapError::Read { path: path_string.clone(), source })?;
        let text = String::from_utf8(bytes).map_err(|error| KeymapError::Parse { path: path_string.clone(), reason: error.to_string() })?;
        let overrides = Keymap::from_json(&text).map_err(|reason| KeymapError::Parse { path: path_string, reason })?;
        self.set_overrides(overrides)
    }

    /// Writes what the app actually runs (defaults + overrides) to `path`.
    pub fn export(&self, path: &Path) -> Result<(), KeymapError> {
        let path_string = path.display().to_string();
        let json = self.effective().to_json().map_err(|reason| KeymapError::Parse { path: path_string.clone(), reason })?;
        write_atomically(path, json.as_bytes()).map_err(|source| KeymapError::Write { path: path_string, source })
    }

    fn save(&self) -> Result<(), KeymapError> {
        let path_string = self.path.display().to_string();
        let json = self.overrides.to_json().map_err(|reason| KeymapError::Parse { path: path_string.clone(), reason })?;
        write_atomically(&self.path, json.as_bytes()).map_err(|source| KeymapError::Write { path: path_string, source })
    }
}

/// Temp file in the target's directory, then rename: a crash mid-save keeps the previous map.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "keymap: no directory to write in"))?;
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::Builder::new().tempfile_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(key: KeyName, modifiers: &[Modifier]) -> KeyChord {
        KeyChord::new(key, modifiers.to_vec())
    }

    #[test]
    fn the_shipped_defaults_parse_and_keep_the_v1_shortcuts() {
        let defaults = Keymap::defaults();
        assert!(!defaults.is_empty(), "the compiled-in DefaultKeymap.json must parse");
        assert_eq!(defaults.primary_chord("photo.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("Right".to_string()));
        assert_eq!(defaults.primary_chord("batch.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("Cmd+Right".to_string()));
        assert_eq!(defaults.chords_for("rate.stars", Some(3)).first().map(KeyChord::display), Some("3".to_string()));
        assert_eq!(defaults.primary_chord("edit.redo", None, RatingMode::Stars).map(|chord| chord.display()), Some("Shift+Cmd+Z".to_string()));
        assert_eq!(defaults.primary_chord("batch.reject", None, RatingMode::Stars).map(|chord| chord.display()), Some("Shift+X".to_string()));
    }

    #[test]
    fn p_is_mode_specific_and_a_specific_binding_beats_a_global_one() {
        let defaults = Keymap::defaults();
        let p = chord(KeyName::P, &[]);
        assert_eq!(defaults.binding(&p, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("flag.pick"));
        assert_eq!(defaults.binding(&p, RatingMode::KeepNotKeep).map(|binding| binding.command.as_str()), Some("keep.toggle"));

        // Two rows can share the chord when their modes do not overlap; the specific one wins.
        let map = Keymap::from_json(
            r#"{"version":1,"bindings":[
                {"command":"flag.pick","key":"p","modifiers":[]},
                {"command":"keep.toggle","key":"p","modifiers":[],"modes":["keep"]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(map.binding(&p, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("flag.pick"));
        assert_eq!(map.binding(&p, RatingMode::KeepNotKeep).map(|binding| binding.command.as_str()), Some("keep.toggle"));
    }

    #[test]
    fn caps_lock_and_fn_are_state_except_on_their_own_key() {
        let map = Keymap::defaults();
        let p_with_caps = chord(KeyName::P, &[Modifier::CapsLock]);
        assert_eq!(map.binding(&p_with_caps, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("flag.pick"), "⇪P is P");
        let caps = chord(KeyName::CapsLock, &[Modifier::CapsLock]);
        assert_eq!(map.binding(&caps, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("settings.autoAdvance.toggle"));
    }

    #[test]
    fn the_user_map_replaces_the_default_chord_and_every_default_chord_of_that_command() {
        let defaults = Keymap::defaults();
        let mut overrides = Keymap::default();
        let new_chord = chord(KeyName::N, &[Modifier::Command]);
        overrides.bind(&new_chord, "batch.next", None, None);
        let effective = overrides.layered(&defaults);

        assert_eq!(effective.binding(&new_chord, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("batch.next"));
        assert!(effective.chords_for("batch.next", None).iter().all(|chord| chord == &new_chord), "the alternates are gone");
        assert_eq!(effective.primary_chord("batch.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("Cmd+N".to_string()));
    }

    #[test]
    fn the_wire_format_is_the_contract_shape() {
        let json = r#"{"version":1,"bindings":[
            {"command":"rate.stars","argument":3,"key":"three","modifiers":[]},
            {"command":"keep.toggle","key":"p","modifiers":[],"modes":["keep"]},
            {"command":"batch.next","key":"rightArrow","modifiers":["command","shift"]}
        ]}"#;
        let map = Keymap::from_json(json).unwrap();
        assert_eq!(map.bindings.len(), 3);
        assert_eq!(map.bindings.get(1).and_then(|binding| binding.modes.as_ref()).and_then(|modes| modes.first()), Some(&ModeName::Keep));
        let written = map.to_json().unwrap();
        let reparsed = Keymap::from_json(&written).unwrap();
        assert_eq!(reparsed, map);
        assert!(!written.contains("\"modes\":null"), "absent modes stay absent");
    }

    #[test]
    fn an_invalid_document_is_an_error_not_a_panic() {
        assert!(Keymap::from_json("not json").is_err());
        assert!(Keymap::from_json(r#"{"version":1,"bindings":[{"command":"x","key":"nope"}]}"#).is_err());
    }

    #[test]
    fn the_store_persists_overrides_atomically_and_falls_back_on_a_corrupt_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keymap.json");
        let mut store = KeymapStore::at(&path);
        assert!(store.overrides().is_empty());

        let mut overrides = Keymap::default();
        overrides.bind(&chord(KeyName::N, &[Modifier::Command]), "photo.next", None, None);
        store.set_overrides(overrides).unwrap();
        assert!(path.exists(), "the override file is written");

        let reread = KeymapStore::at(&path);
        assert_eq!(reread.overrides().bindings.len(), 1);
        assert_eq!(reread.effective().primary_chord("photo.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("Cmd+N".to_string()));

        std::fs::write(&path, b"{ broken").unwrap();
        let recovered = KeymapStore::at(&path);
        assert!(recovered.overrides().is_empty(), "a corrupt file is the defaults, not a crash");
        assert!(!recovered.effective().is_empty());
    }

    #[test]
    fn import_replaces_the_user_map_and_export_writes_the_effective_one() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("imported.json");
        let imported = r#"{"version":1,"bindings":[{"command":"batch.next","key":"slash","modifiers":[]}]}"#;
        std::fs::write(&source, imported).unwrap();

        let mut store = KeymapStore::at(&directory.path().join("keymap.json"));
        store.import(&source).unwrap();
        assert_eq!(store.overrides().bindings.len(), 1);
        assert_eq!(
            store.effective().binding(&chord(KeyName::Slash, &[]), RatingMode::Stars).map(|binding| binding.command.as_str()),
            Some("batch.next")
        );

        let exported = directory.path().join("exported.json");
        store.export(&exported).unwrap();
        let written = std::fs::read_to_string(&exported).unwrap();
        let map = Keymap::from_json(&written).unwrap();
        assert!(map.chords_for("photo.next", None).len() == 1, "export is the effective map, defaults included");
        assert_eq!(map.primary_chord("batch.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("/".to_string()));
    }

    #[test]
    fn resetting_returns_every_default() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = KeymapStore::at(&directory.path().join("keymap.json"));
        let mut overrides = Keymap::default();
        overrides.bind(&chord(KeyName::N, &[]), "photo.next", None, None);
        store.set_overrides(overrides).unwrap();
        store.reset_to_defaults().unwrap();
        assert!(store.overrides().is_empty());
        assert_eq!(store.effective().primary_chord("photo.next", None, RatingMode::Stars).map(|chord| chord.display()), Some("Right".to_string()));
    }
}
