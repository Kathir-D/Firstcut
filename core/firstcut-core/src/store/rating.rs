//! Ratings, flags, color labels, tiers and the two rating modes (task.md §6).
//!
//! These are the values Firstcut persists in the session DB and writes to XMP sidecars, so the
//! types live here (not in `session.rs`): both [`crate::store`] and [`crate::xmp`] need them and
//! neither of them knows about Swift.
//!
//! Two modes exist (task.md §2, §6): **Stars** (0..=5 plus flag and color label) and
//! **Keep / Not keep** (one boolean). Both are always stored, even when a mode is inactive, because
//! switching mode mid-session must not lose data (§6).

use std::fmt;
use std::str::FromStr;

/// Pick / reject flag. Independent of stars (task.md §6.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Flag {
    #[default]
    None,
    Pick,
    Reject,
}

impl Flag {
    /// Value stored in `ratings.flag`.
    pub fn to_db(self) -> i64 {
        match self {
            Flag::None => 0,
            Flag::Pick => 1,
            Flag::Reject => 2,
        }
    }

    pub fn from_db(value: i64) -> Flag {
        match value {
            1 => Flag::Pick,
            2 => Flag::Reject,
            _ => Flag::None,
        }
    }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Flag::None => "none",
            Flag::Pick => "pick",
            Flag::Reject => "reject",
        })
    }
}

/// The five color labels Firstcut writes (task.md §6.3: labels 1–4 red, yellow, green, blue).
/// Purple is Lightroom's fifth label and is supported for import parity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    /// `xmp:Label` value, spelled the way Lightroom writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            ColorLabel::Red => "Red",
            ColorLabel::Yellow => "Yellow",
            ColorLabel::Green => "Green",
            ColorLabel::Blue => "Blue",
            ColorLabel::Purple => "Purple",
        }
    }

    /// `photoshop:Urgency` value (1–5), the legacy spelling read by older Lightroom versions.
    pub fn urgency(self) -> i64 {
        match self {
            ColorLabel::Red => 1,
            ColorLabel::Yellow => 2,
            ColorLabel::Green => 3,
            ColorLabel::Blue => 4,
            ColorLabel::Purple => 5,
        }
    }

    pub fn from_urgency(value: i64) -> Option<ColorLabel> {
        match value {
            1 => Some(ColorLabel::Red),
            2 => Some(ColorLabel::Yellow),
            3 => Some(ColorLabel::Green),
            4 => Some(ColorLabel::Blue),
            5 => Some(ColorLabel::Purple),
            _ => None,
        }
    }
}

impl fmt::Display for ColorLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ColorLabel {
    type Err = ();

    fn from_str(s: &str) -> Result<ColorLabel, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "red" => Ok(ColorLabel::Red),
            "yellow" => Ok(ColorLabel::Yellow),
            "green" => Ok(ColorLabel::Green),
            "blue" => Ok(ColorLabel::Blue),
            "purple" | "violet" => Ok(ColorLabel::Purple),
            _ => Err(()),
        }
    }
}

/// Which rating mode the app is in (Settings → General → Rating mode, task.md §6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum RatingMode {
    #[default]
    Stars,
    KeepNotKeep,
}

impl RatingMode {
    /// Value stored in `session.rating_mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            RatingMode::Stars => "stars",
            RatingMode::KeepNotKeep => "keep",
        }
    }

    pub fn to_db(self) -> &'static str {
        self.as_str()
    }

    pub fn from_db(value: &str) -> RatingMode {
        match value {
            "keep" => RatingMode::KeepNotKeep,
            _ => RatingMode::Stars,
        }
    }
}

impl fmt::Display for RatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RatingMode {
    type Err = ();

    fn from_str(s: &str) -> Result<RatingMode, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stars" => Ok(RatingMode::Stars),
            "keep" => Ok(RatingMode::KeepNotKeep),
            _ => Err(()),
        }
    }
}

/// The rating of one photo (docs/contracts/session-api.md).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rating {
    /// 0..=5, stars mode. Out-of-range values are clamped by [`Rating::new`].
    pub stars: u8,
    pub flag: Flag,
    pub label: Option<ColorLabel>,
    /// Keep / Not keep mode.
    pub keep: bool,
}

impl Rating {
    pub const MAX_STARS: u8 = 5;
    /// The rating every photo starts with in either mode.
    pub fn neutral() -> Rating {
        Rating::default()
    }

    /// Clamps `stars` into 0..=5 so a bad value from another tool can never reach the DB.
    pub fn new(stars: u8, flag: Flag, label: Option<ColorLabel>, keep: bool) -> Rating {
        Rating {
            stars: stars.min(Rating::MAX_STARS),
            flag,
            label,
            keep,
        }
    }

    pub fn stars(n: u8) -> Rating {
        Rating::new(n, Flag::None, None, false)
    }

    pub fn keep() -> Rating {
        Rating::new(0, Flag::None, None, true)
    }

    /// Keep mode's rating, kept or not.
    pub fn keep_with(keep: bool) -> Rating {
        Rating::new(0, Flag::None, None, keep)
    }

    /// True when nothing at all is set, in either mode.
    pub fn is_neutral(&self) -> bool {
        self.stars == 0 && self.flag == Flag::None && self.label.is_none() && !self.keep
    }

    pub fn is_rejected(&self) -> bool {
        self.flag == Flag::Reject
    }

    /// The tier this rating falls into, per mode (task.md §6.1 / §6.2). Drives the Finish summary
    /// and the "split by tier" folders.
    pub fn tier(&self, mode: RatingMode) -> Tier {
        match mode {
            RatingMode::Stars => {
                if self.flag == Flag::Reject {
                    return Tier::Rejected;
                }
                match self.stars {
                    4 | 5 => Tier::Keep,
                    3 => Tier::Good,
                    1 | 2 => Tier::Maybe,
                    _ => Tier::Unrated,
                }
            }
            RatingMode::KeepNotKeep => {
                if self.flag == Flag::Reject {
                    Tier::Rejected
                } else if self.keep {
                    Tier::Keep
                } else {
                    Tier::Unrated
                }
            }
        }
    }

    /// True for the tiers the Finish step treats as "kept" (task.md §9.7).
    pub fn is_kept(&self, mode: RatingMode) -> bool {
        matches!(self.tier(mode), Tier::Keep)
    }
}

/// Rating buckets shown in the Finish summary and used for split folders (task.md §6.1, §9.7).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tier {
    Keep,
    Good,
    Maybe,
    Unrated,
    Rejected,
}

impl Tier {
    /// Every tier, in the order the Finish summary lists them.
    pub const ALL: [Tier; 5] = [
        Tier::Keep,
        Tier::Good,
        Tier::Maybe,
        Tier::Unrated,
        Tier::Rejected,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Keep => "Keep",
            Tier::Good => "Good",
            Tier::Maybe => "Maybe",
            Tier::Unrated => "Unrated",
            Tier::Rejected => "Rejected",
        }
    }

    /// Folder name for "split into subfolders by tier" (task.md §9.7: `5 Keep`, `3 Good`,
    /// `1 Maybe`). Stars mode uses the star count so the name is meaningful in Finder; keep mode
    /// has no stars, so it uses 1/0.
    pub fn split_dir(self, mode: RatingMode) -> String {
        let stars = match mode {
            RatingMode::Stars => match self {
                Tier::Keep => "5",
                Tier::Good => "3",
                Tier::Maybe => "1",
                Tier::Unrated => "0",
                Tier::Rejected => "-1",
            },
            RatingMode::KeepNotKeep => match self {
                Tier::Keep => "1",
                Tier::Rejected => "-1",
                _ => "0",
            },
        };
        format!("{stars} {}", self.as_str())
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_are_clamped() {
        assert_eq!(Rating::new(9, Flag::None, None, false).stars, 5);
        assert_eq!(Rating::new(5, Flag::None, None, false).stars, 5);
    }

    #[test]
    fn stars_mode_tiers_match_task_md_6_1() {
        let mode = RatingMode::Stars;
        let tier = |n: u8| Rating::stars(n).tier(mode);
        assert_eq!(tier(5), Tier::Keep);
        assert_eq!(tier(4), Tier::Keep);
        assert_eq!(tier(3), Tier::Good);
        assert_eq!(tier(2), Tier::Maybe);
        assert_eq!(tier(1), Tier::Maybe);
        assert_eq!(tier(0), Tier::Unrated);
        // X beats the stars.
        assert_eq!(
            Rating::new(5, Flag::Reject, None, false).tier(mode),
            Tier::Rejected
        );
        // P has no effect on the tier.
        assert_eq!(
            Rating::new(3, Flag::Pick, None, false).tier(mode),
            Tier::Good
        );
    }

    #[test]
    fn keep_mode_tiers_match_task_md_6_2() {
        let mode = RatingMode::KeepNotKeep;
        // Every photo starts as not keep => Unrated, handled at the end.
        assert_eq!(Rating::neutral().tier(mode), Tier::Unrated);
        assert_eq!(Rating::keep().tier(mode), Tier::Keep);
        // Stars written in the other mode are preserved, not reinterpreted.
        let mixed = Rating::new(4, Flag::None, None, true);
        assert_eq!(mixed.tier(mode), Tier::Keep);
        assert_eq!(mixed.tier(RatingMode::Stars), Tier::Keep);
        // In stars mode a keep alone is not a keep.
        assert_eq!(Rating::keep().tier(RatingMode::Stars), Tier::Unrated);
    }

    #[test]
    fn flag_round_trips_through_db_values() {
        for flag in [Flag::None, Flag::Pick, Flag::Reject] {
            assert_eq!(Flag::from_db(flag.to_db()), flag);
        }
        assert_eq!(Flag::from_db(99), Flag::None);
    }

    #[test]
    fn labels_round_trip_in_both_spellings() {
        for label in [
            ColorLabel::Red,
            ColorLabel::Yellow,
            ColorLabel::Green,
            ColorLabel::Blue,
            ColorLabel::Purple,
        ] {
            assert_eq!(label.as_str().parse::<ColorLabel>().unwrap(), label);
            assert_eq!(ColorLabel::from_urgency(label.urgency()).unwrap(), label);
        }
        // "Violet" is Photoshop's word for the purple label.
        assert_eq!("violet".parse::<ColorLabel>().unwrap(), ColorLabel::Purple);
        assert_eq!("  green ".parse::<ColorLabel>().unwrap(), ColorLabel::Green);
        assert!("chartreuse".parse::<ColorLabel>().is_err());
        assert_eq!(ColorLabel::from_urgency(9), None);
    }

    #[test]
    fn rating_mode_round_trips() {
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            assert_eq!(mode.to_db().parse::<RatingMode>().unwrap(), mode);
            assert_eq!(RatingMode::from_db(mode.to_db()), mode);
        }
        // Anything unknown falls back to stars rather than losing the session.
        assert_eq!(RatingMode::from_db("nonsense"), RatingMode::Stars);
    }

    #[test]
    fn split_dirs_match_task_md_9_7() {
        assert_eq!(Tier::Keep.split_dir(RatingMode::Stars), "5 Keep");
        assert_eq!(Tier::Good.split_dir(RatingMode::Stars), "3 Good");
        assert_eq!(Tier::Maybe.split_dir(RatingMode::Stars), "1 Maybe");
        assert_eq!(Tier::Keep.split_dir(RatingMode::KeepNotKeep), "1 Keep");
    }

    #[test]
    fn neutral_and_kept() {
        assert!(Rating::neutral().is_neutral());
        assert!(!Rating::stars(1).is_neutral());
        assert!(!Rating::keep().is_neutral());
        assert!(Rating::stars(4).is_kept(RatingMode::Stars));
        assert!(!Rating::stars(3).is_kept(RatingMode::Stars));
    }
}
