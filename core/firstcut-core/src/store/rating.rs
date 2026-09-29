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

/// Rating of one photo (docs/contracts/session-api.md).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rating {
    /// 0..=5, stars mode. Out-of-range values are clamped by [`Rating::new`].
    pub stars: u8,
    pub flag: Flag,
    pub label: Option<ColorLabel>,
    /// Keep / Not keep mode.
    pub keep: bool,
}

/// The star count at which a photo counts as a keep in **both** modes.
///
/// 4 and 5 stars are the "Keep" tier in task.md §6.1, and the Finish step only keeps the Keep
/// tier. Using the same threshold for the keep-mode display is what stops the filmstrip from
/// showing a red "not keep" ring on a photo the Finish step is about to move into the kept folder.
pub const KEEP_STARS: u8 = 4;

/// The rating **shown** for a photo in a given mode.
///
/// This is the whole mode-mapping rule (task.md §6: "Switching mode mid-session is allowed; existing
/// data is preserved and mapped"), in one pure function, and it is the only place the rule exists.
/// app-logic and ui call it; nobody re-derives the answer from `stars` or `keep` on their own, which
/// is how two implementations end up disagreeing about whether a photo is a keep.
///
/// | state in the source mode | shown in **Stars** | shown in **Keep / Not keep** |
/// | --- | --- | --- |
/// | keep, no stars | **5 stars** (task.md §6: "a keep ↔ 5 stars") | Keep |
/// | stars 4–5, `keep` unset | those stars | **Keep** |
/// | stars 1–3, `keep` unset | those stars | Not keep (they are Good/Maybe, not kept at Finish) |
/// | no stars, not kept | Unrated | Not keep |
/// | reject flag (X) | `xmp:Rating=-1`, Rejected tier | Rejected tier |
/// | colour label | unchanged, shown in both modes | unchanged |
///
/// **This function never fabricates a field.** It reports the *star count* the filmstrip draws; the
/// kept/not-kept question is answered by [`Rating::tier`], which does the mapping. An earlier
/// version also rewrote `keep` here, which made a round trip through keep mode materialise a keep
/// the user never set (`display(keep_mode) -> back` was not the identity), and it is the reason
/// this function and `tier` are now split.
///
/// Nothing is lost in either direction: the stored rating is never modified, only the view of it,
/// and the function is idempotent, so switching modes and back shows exactly what was there before.
///
/// **The returned value is a view, not a rating you may store.** In keep mode it promotes 4–5
/// stars to `keep: true` ([`KEEP_STARS`]), which is a statement about the *other* mode, so writing
/// this back would persist a keep the user never gave. To change what is stored, use
/// [`map_rating`] — the one path that converts a mode change into a real write.
pub fn display_rating(rating: &Rating, mode: RatingMode) -> Rating {
    match mode {
        // A keep with no stars would read as Unrated, so it shows as the 5 stars it means.
        RatingMode::Stars => Rating {
            stars: rating.effective_stars(),
            ..*rating
        },
        // Keep mode draws a ring, not a star row, so the stored rating is shown exactly as stored.
        // The mapped kept/not-kept answer lives in `tier` -- making it the identity here is what
        // keeps this function pure, so a round trip through the other mode cannot materialise a
        // keep the user never set.
        RatingMode::KeepNotKeep => *rating,
    }
}

/// The rating as it would be *stored* if the user worked in `to` mode: [`display_rating`] plus the
/// inactive mode's fields cleared, so a photo does not silently keep a "5 stars" it was only
/// showing. Only used when the user re-rates a photo after switching modes.
pub fn map_rating(rating: &Rating, from: RatingMode, to: RatingMode) -> Rating {
    if from == to {
        return *rating;
    }
    match to {
        RatingMode::Stars => {
            let shown = display_rating(rating, RatingMode::Stars);
            Rating::new(shown.stars, shown.flag, shown.label, false)
        }
        RatingMode::KeepNotKeep => {
            // Read the mapped answer, not the raw field: a 4-star photo is a keep in keep mode.
            let kept = rating.tier(RatingMode::KeepNotKeep) == Tier::Keep;
            Rating::new(0, rating.flag, rating.label, kept)
        }
    }
}

/// The tier to show, in the one call ui needs.
pub fn display_tier(rating: &Rating, mode: RatingMode) -> Tier {
    display_rating(rating, mode).tier(mode)
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
    ///
    /// **This is the one place the cross-mode mapping happens.** Everything the user can see or
    /// that the Finish step will act on — the filmstrip ring, the tier counts, the split-folder
    /// names and the keep/delete decision — goes through here, so they cannot disagree. In
    /// particular a 4- or 5-star photo reads as Keep in keep mode and a keep reads as 5 stars in
    /// stars mode, and Finish keeps exactly what the UI showed as kept.
    pub fn tier(&self, mode: RatingMode) -> Tier {
        match mode {
            RatingMode::Stars => {
                if self.flag == Flag::Reject {
                    return Tier::Rejected;
                }
                // A keep made in keep mode means the same as 5 stars (task.md §6: "a keep ↔ 5
                // stars"), so it must not read as Unrated in stars mode.
                let stars = self.effective_stars();
                match stars {
                    4 | 5 => Tier::Keep,
                    3 => Tier::Good,
                    1 | 2 => Tier::Maybe,
                    _ => Tier::Unrated,
                }
            }
            RatingMode::KeepNotKeep => {
                if self.flag == Flag::Reject {
                    Tier::Rejected
                } else if self.keep || self.stars >= KEEP_STARS {
                    Tier::Keep
                } else {
                    Tier::Unrated
                }
            }
        }
    }

    /// The star count this photo counts as in **stars** mode, applying the keep ↔ 5 stars mapping.
    /// A keep made in keep mode has `stars == 0`; without this it would show as Unrated.
    pub fn effective_stars(&self) -> u8 {
        if self.stars == 0 && self.keep {
            Rating::MAX_STARS
        } else {
            self.stars
        }
    }

    /// **The single answer to "is this photo kept in this mode".**
    ///
    /// Task.md §9.7: the Finish step only keeps the Keep tier, and this is what decides it. It is
    /// a thin wrapper over [`Rating::tier`] on purpose, and it exists so there is **exactly one**
    /// function to call. REV-78: the Finish planner used to read the raw `keep` field while the UI
    /// showed `display_rating`, so a 4-star photo in keep mode got a green Keep ring and was then
    /// moved to the trash. Two implementations of one rule, and the user lost the photo.
    ///
    /// Everything that acts on or shows a keep goes through here: the filmstrip ring, the tier
    /// counts, the split-folder names, and `fileops::plan_finish`. If you are about to compare
    /// `rating.keep` directly, you are about to reintroduce REV-78 — call this instead.
    ///
    /// The invariant this guarantees is asserted by
    /// `the_ui_never_shows_a_keep_that_finish_would_trash`, and again at the level that actually
    /// moves files in `fileops::tests`.
    #[must_use]
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
        // A keep made in keep mode is a keep in stars mode too: task.md §6 maps it to 5 stars.
        // An earlier version asserted Unrated here, which is the REV-69 bug -- the user's keeps
        // appeared to vanish when they switched modes.
        assert_eq!(Rating::keep().tier(RatingMode::Stars), Tier::Keep);
        assert_eq!(Rating::keep().effective_stars(), 5);
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

    #[test]
    fn a_keep_shows_as_five_stars_in_stars_mode() {
        // task.md §6: "a keep ↔ 5 stars by default". Without this the user's keeps look Unrated.
        let keep = Rating::keep();
        assert_eq!(
            keep.stars, 0,
            "stored as-is: nothing is written into the stars field"
        );
        let shown = display_rating(&keep, RatingMode::Stars);
        assert_eq!(shown.stars, 5);
        assert_eq!(display_tier(&keep, RatingMode::Stars), Tier::Keep);
        // Still a keep in keep mode, unchanged.
        assert_eq!(display_rating(&keep, RatingMode::KeepNotKeep), keep);
    }

    #[test]
    fn a_starred_photo_shows_as_a_keep_in_keep_mode() {
        assert_eq!(
            display_tier(&Rating::stars(5), RatingMode::KeepNotKeep),
            Tier::Keep
        );
        assert_eq!(
            display_tier(&Rating::stars(4), RatingMode::KeepNotKeep),
            Tier::Keep
        );
        // 3 stars is "Good", and the Finish step does not keep it, so keep mode must not claim it.
        assert_eq!(
            display_tier(&Rating::stars(3), RatingMode::KeepNotKeep),
            Tier::Unrated
        );
        assert!(!display_rating(&Rating::stars(3), RatingMode::KeepNotKeep).keep);
    }

    #[test]
    fn the_display_never_contradicts_the_finish_decision() {
        // Whatever the UI shows as a keep is exactly what Finish keeps, in either mode. Both the
        // display and Finish go through `display_tier`, so the assertion is that the *stored*
        // fields and the *displayed* fields agree about kept-ness wherever they can: a 4–5 star
        // photo is a keep in both modes, a 3-star photo is not, and a keep stays a keep.
        //
        // Note the deliberate asymmetry: in keep mode `display_rating` promotes 4 stars to
        // `keep: true`, so a stars-mode photo and its keep-mode view disagree on the raw `keep`
        // field. That is the display doing its job — the stored value is untouched — and it is why
        // a displayed rating must never be written back (see `map_rating`).
        for stars in 0..=5u8 {
            let rating = Rating::stars(stars);
            let expected_stars_mode = matches!(stars, 4 | 5);
            let expected_tier = if expected_stars_mode {
                Tier::Keep
            } else if stars == 3 {
                Tier::Good
            } else if matches!(stars, 1 | 2) {
                Tier::Maybe
            } else {
                Tier::Unrated
            };
            assert_eq!(
                display_tier(&rating, RatingMode::Stars),
                expected_tier,
                "{stars} stars shown in stars mode"
            );
            assert_eq!(
                display_tier(&rating, RatingMode::KeepNotKeep) == Tier::Keep,
                expected_stars_mode,
                "{stars} stars shown in keep mode"
            );
        }
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            let keep = Rating::keep();
            assert!(
                display_rating(&keep, mode).is_kept(mode),
                "a keep is a keep in {mode}"
            );
        }
    }

    #[test]
    fn the_display_is_idempotent() {
        let states = [
            Rating::neutral(),
            Rating::stars(1),
            Rating::stars(3),
            Rating::stars(4),
            Rating::stars(5),
            Rating::keep(),
            Rating::new(0, Flag::Reject, None, false),
            Rating::new(4, Flag::Pick, Some(ColorLabel::Blue), true),
        ];
        for state in states {
            for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
                let once = display_rating(&state, mode);
                assert_eq!(display_rating(&once, mode), once, "{state:?} in {mode}");
            }
        }
    }

    #[test]
    fn the_display_preserves_the_tier_across_a_mode_round_trip() {
        // `display_rating` is a one-way projection: in keep mode it *synthesises* `keep: true` for
        // a 4–5 star photo, so a full-field round trip through it is not invertible and must not
        // be asserted to be. What must hold is the part the user actually sees — the tier — and
        // the field belonging to the mode they return to.
        let states = [
            Rating::neutral(),
            Rating::stars(1),
            Rating::stars(3),
            Rating::stars(4),
            Rating::stars(5),
            Rating::keep(),
            Rating::new(0, Flag::Reject, None, false),
            Rating::new(4, Flag::Pick, Some(ColorLabel::Blue), true),
        ];
        for state in states {
            for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
                let before = display_tier(&state, mode);
                let there = display_rating(&state, mode);
                let back = display_rating(&there, other(mode));
                assert_eq!(display_tier(&back, mode), before, "{state:?} in {mode}");

                // And the mode's own field survives, which is what "nothing the user set is lost"
                // means once the cross-mode synthesis is accounted for.
                match mode {
                    RatingMode::Stars => assert_eq!(back.stars, there.stars, "{state:?} stars"),
                    RatingMode::KeepNotKeep => assert_eq!(back.keep, there.keep, "{state:?} keep"),
                }
            }
        }
    }

    #[test]
    fn display_never_mutates_the_stored_rating() {
        // `display_*` is a view. If it ever wrote the mapped value back, switching modes would
        // silently rewrite the user's ratings -- the failure REV-69 describes.
        let states = [
            Rating::neutral(),
            Rating::stars(4),
            Rating::keep(),
            Rating::stars(1),
        ];
        for state in states {
            for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
                let _ = display_rating(&state, mode);
                let _ = display_tier(&state, mode);
                let _ = map_rating(&state, mode, other(mode));
                assert_eq!(state, {
                    // `state` is Copy, so this only proves the signatures do not take &mut.
                    let round_tripped: Rating = state;
                    round_tripped
                });
            }
        }
        // A keep still reports stars == 0 after being displayed in stars mode, i.e. the 5 stars
        // were never written back to the stored rating.
        let keep = Rating::keep();
        let shown = display_rating(&keep, RatingMode::Stars);
        assert_eq!(shown.stars, 5);
        assert_eq!(keep.stars, 0);
    }

    #[test]
    fn map_rating_clears_the_inactive_mode_and_is_the_identity_within_one_mode() {
        let keep = Rating::keep();
        assert_eq!(
            map_rating(&keep, RatingMode::KeepNotKeep, RatingMode::KeepNotKeep),
            keep
        );

        let as_stars = map_rating(&keep, RatingMode::KeepNotKeep, RatingMode::Stars);
        assert_eq!(as_stars, Rating::stars(5));
        assert!(!as_stars.keep, "the keep field belongs to keep mode only");

        let four = Rating::stars(4);
        let as_keep = map_rating(&four, RatingMode::Stars, RatingMode::KeepNotKeep);
        assert!(as_keep.keep);
        assert_eq!(as_keep.stars, 0, "the stars belong to stars mode only");
        // And back again: the 4 stars are still what the user meant.
        assert_eq!(
            map_rating(&as_keep, RatingMode::KeepNotKeep, RatingMode::Stars),
            Rating::stars(5)
        );
    }

    #[test]
    fn labels_and_flags_survive_a_mode_switch() {
        let rating = Rating::new(3, Flag::Reject, Some(ColorLabel::Purple), false);
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            let shown = display_rating(&rating, mode);
            assert_eq!(shown.flag, Flag::Reject);
            assert_eq!(shown.label, Some(ColorLabel::Purple));
            assert_eq!(display_tier(&rating, mode), Tier::Rejected);
        }
    }

    fn other(mode: RatingMode) -> RatingMode {
        match mode {
            RatingMode::Stars => RatingMode::KeepNotKeep,
            RatingMode::KeepNotKeep => RatingMode::Stars,
        }
    }
}
