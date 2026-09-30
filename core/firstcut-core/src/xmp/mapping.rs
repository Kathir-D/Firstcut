//! How Firstcut's ratings map onto XMP, and back (todo.md §6).
//!
//! Ratings live in two places: the session database, which is what the app reads, and the XMP
//! sidecar, which is what Lightroom reads. The rules for translating between them are all here, in
//! one configurable place, because they are the part a user can argue with.
//!
//! Two things are worth knowing (both from todo.md §6.2):
//!
//! * A **keep has to be stored as a rating or a label**, because Lightroom does not read pick flags
//!   from XMP. The pick flag (`P`) therefore has no XMP representation at all and lives only in the
//!   database; losing it costs nothing, since Lightroom could not have shown it anyway.
//! * A **not-keep** is the absence of a rating, not `0`. Writing `xmp:Rating="-1"` (rejected) is
//!   available for the Finish step, and for an explicit reject flag, but not as the default.

use crate::store::rating::{ColorLabel, Flag, Rating, RatingMode};

use super::document::{SidecarValues, XmpValues};

/// The `xmp:Rating` value XMP uses for "rejected".
pub const REJECTED: i64 = -1;

/// How ratings are written to sidecars. All of it is a setting the app exposes; the defaults are
/// the ones todo.md §6.2 describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmpMapping {
    /// What a keep becomes in keep mode. 5 by default. Also the threshold on import: a sidecar
    /// rating at or above this counts as a keep.
    pub keep_rating: i64,
    /// When set, a keep is written as this colour label instead of a rating.
    pub keep_label: Option<ColorLabel>,
    /// What a not-keep becomes. `None` (the default) removes `xmp:Rating`; `Some(-1)` writes the
    /// rejected flag.
    pub not_keep_rating: Option<i64>,
    /// What a not-keep's colour label becomes. `None` removes `xmp:Label`.
    pub not_keep_label: Option<ColorLabel>,
}

impl Default for XmpMapping {
    fn default() -> XmpMapping {
        XmpMapping {
            keep_rating: 5,
            keep_label: None,
            not_keep_rating: None,
            not_keep_label: None,
        }
    }
}

impl XmpMapping {
    /// Stars mode: the stars go across as they are, and an explicit reject becomes -1.
    pub fn values_for(&self, rating: Rating, mode: RatingMode) -> XmpValues {
        match mode {
            RatingMode::Stars => XmpValues {
                rating: Some(if rating.flag == Flag::Reject {
                    REJECTED
                } else {
                    i64::from(rating.stars)
                }),
                label: rating.label.map(|label| label.as_str().to_string()),
            },
            RatingMode::KeepNotKeep => {
                if rating.flag == Flag::Reject {
                    // An explicit reject is worth writing down in either mode.
                    return XmpValues {
                        rating: Some(REJECTED),
                        label: rating.label.map(|label| label.as_str().to_string()),
                    };
                }
                if rating.keep {
                    match self.keep_label {
                        // Keep as a colour: the number must go, or Lightroom would show 5 stars on a
                        // photo the user thinks is merely labelled.
                        Some(label) => XmpValues {
                            rating: None,
                            label: Some(label.as_str().to_string()),
                        },
                        None => XmpValues {
                            rating: Some(self.keep_rating),
                            label: rating.label.map(|label| label.as_str().to_string()),
                        },
                    }
                } else {
                    XmpValues {
                        rating: self.not_keep_rating,
                        label: self.not_keep_label.map(|label| label.as_str().to_string()),
                    }
                }
            }
        }
    }

    /// Rebuilds a rating from a sidecar, for the "no database, import from XMP" path (todo.md §11).
    ///
    /// Returns `None` when the sidecar says nothing Firstcut understands, so an untouched photo
    /// does not get a row.
    pub fn rating_from(&self, values: &SidecarValues, mode: RatingMode) -> Option<Rating> {
        let label = sidecar_label(values);
        let stars = match values.rating {
            Some(value) if value >= 0 => u8::try_from(value).unwrap_or(0).min(Rating::MAX_STARS),
            _ => 0,
        };
        let rejected = values.rating == Some(REJECTED);

        match mode {
            RatingMode::Stars => {
                if values.rating.is_none() && label.is_none() {
                    return None;
                }
                Some(Rating::new(
                    stars,
                    if rejected { Flag::Reject } else { Flag::None },
                    label,
                    false,
                ))
            }
            RatingMode::KeepNotKeep => {
                if values.rating.is_none() && label.is_none() {
                    return None;
                }
                let keep = if rejected {
                    false
                } else if self.keep_label.is_some() {
                    label == self.keep_label
                } else {
                    values.rating.is_some_and(|value| value >= self.keep_rating)
                };
                Some(Rating::new(
                    stars,
                    if rejected { Flag::Reject } else { Flag::None },
                    label,
                    keep,
                ))
            }
        }
    }
}

/// What we are about to write, read back as if it were already in the file. `urgency` is not
/// written by Firstcut, so it is never part of a value we produced.
impl From<&XmpValues> for SidecarValues {
    fn from(values: &XmpValues) -> SidecarValues {
        SidecarValues {
            rating: values.rating,
            label: values.label.clone(),
            urgency: None,
        }
    }
}

/// The colour a sidecar says a photo has, from `xmp:Label` or, failing that, `photoshop:Urgency`.
pub fn sidecar_label(values: &SidecarValues) -> Option<ColorLabel> {
    values
        .label
        .as_deref()
        .and_then(|label| label.parse().ok())
        .or_else(|| values.urgency.and_then(ColorLabel::from_urgency))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sidecar(rating: Option<i64>, label: Option<&str>, urgency: Option<i64>) -> SidecarValues {
        SidecarValues {
            rating,
            label: label.map(str::to_string),
            urgency,
        }
    }

    #[test]
    fn stars_mode_writes_the_stars() {
        let mapping = XmpMapping::default();
        for stars in 0..=5u8 {
            let values = mapping.values_for(Rating::stars(stars), RatingMode::Stars);
            assert_eq!(values.rating, Some(i64::from(stars)));
            assert_eq!(values.label, None);
        }
    }

    #[test]
    fn stars_mode_writes_a_reject_as_minus_one() {
        let mapping = XmpMapping::default();
        let values =
            mapping.values_for(Rating::new(4, Flag::Reject, None, false), RatingMode::Stars);
        assert_eq!(values.rating, Some(REJECTED));
    }

    #[test]
    fn the_pick_flag_has_no_xmp_form() {
        // Lightroom cannot read it, so it is not written; the database keeps it (todo.md §6.2).
        let values = XmpMapping::default()
            .values_for(Rating::new(3, Flag::Pick, None, false), RatingMode::Stars);
        assert_eq!(values.rating, Some(3));
    }

    #[test]
    fn colour_labels_cross_over() {
        let rating = Rating::new(3, Flag::None, Some(ColorLabel::Blue), false);
        let values = XmpMapping::default().values_for(rating, RatingMode::Stars);
        assert_eq!(values.label.as_deref(), Some("Blue"));
    }

    #[test]
    fn keep_mode_writes_five_stars_by_default() {
        let mapping = XmpMapping::default();
        let values = mapping.values_for(Rating::keep(), RatingMode::KeepNotKeep);
        assert_eq!(values.rating, Some(5));
        assert_eq!(values.label, None);
    }

    #[test]
    fn keep_mode_can_write_a_colour_instead() {
        let mapping = XmpMapping {
            keep_label: Some(ColorLabel::Green),
            ..XmpMapping::default()
        };
        let values = mapping.values_for(Rating::keep(), RatingMode::KeepNotKeep);
        assert_eq!(
            values.rating, None,
            "the number must not linger next to the label"
        );
        assert_eq!(values.label.as_deref(), Some("Green"));
    }

    #[test]
    fn not_keep_removes_the_rating_by_default() {
        let values = XmpMapping::default().values_for(Rating::neutral(), RatingMode::KeepNotKeep);
        assert_eq!(values.rating, None);
        assert_eq!(values.label, None);
    }

    #[test]
    fn not_keep_can_write_rejected_when_asked() {
        let mapping = XmpMapping {
            not_keep_rating: Some(REJECTED),
            ..XmpMapping::default()
        };
        let values = mapping.values_for(Rating::neutral(), RatingMode::KeepNotKeep);
        assert_eq!(values.rating, Some(REJECTED));
    }

    #[test]
    fn a_keep_survives_a_round_trip_through_the_sidecar() {
        let mapping = XmpMapping::default();
        let values = mapping.values_for(Rating::keep(), RatingMode::KeepNotKeep);
        assert_eq!(values.rating, Some(5));
        let back = mapping
            .rating_from(&SidecarValues::from(&values), RatingMode::KeepNotKeep)
            .expect("a written keep must be readable");
        assert!(back.keep);
    }

    #[test]
    fn a_not_keep_leaves_no_trace_in_xmp() {
        // There is nothing to import: to Lightroom, a not-keep is the absence of a rating. The
        // database is what remembers it, which is exactly why the database is the source of truth.
        let mapping = XmpMapping::default();
        let values = mapping.values_for(Rating::neutral(), RatingMode::KeepNotKeep);
        assert!(values.is_empty());
        assert!(
            mapping
                .rating_from(&SidecarValues::from(&values), RatingMode::KeepNotKeep)
                .is_none()
        );
    }

    #[test]
    fn stars_round_trip_through_a_sidecar() {
        let mapping = XmpMapping::default();
        for stars in 0..=5u8 {
            let rating = Rating::new(stars, Flag::None, Some(ColorLabel::Red), false);
            let values = mapping.values_for(rating, RatingMode::Stars);
            let back = mapping
                .rating_from(&SidecarValues::from(&values), RatingMode::Stars)
                .unwrap();
            assert_eq!(back, rating);
        }
        let rejected = Rating::new(0, Flag::Reject, None, false);
        let values = mapping.values_for(rejected, RatingMode::Stars);
        assert_eq!(
            mapping
                .rating_from(&SidecarValues::from(&values), RatingMode::Stars)
                .unwrap(),
            rejected
        );
    }

    #[test]
    fn a_lightroom_sidecar_imports_into_stars() {
        let mapping = XmpMapping::default();
        let imported = mapping
            .rating_from(&sidecar(Some(4), None, Some(1)), RatingMode::Stars)
            .unwrap();
        assert_eq!(imported.stars, 4);
        assert_eq!(imported.flag, Flag::None);
        // No xmp:Label, but the legacy urgency says red.
        assert_eq!(imported.label, Some(ColorLabel::Red));
    }

    #[test]
    fn an_empty_sidecar_imports_to_nothing() {
        let mapping = XmpMapping::default();
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            assert!(
                mapping
                    .rating_from(&sidecar(None, None, None), mode)
                    .is_none()
            );
        }
    }

    #[test]
    fn keep_mode_reads_back_its_own_keeps() {
        let mapping = XmpMapping {
            keep_rating: 4,
            ..XmpMapping::default()
        };
        let four = mapping
            .rating_from(&sidecar(Some(4), None, None), RatingMode::KeepNotKeep)
            .unwrap();
        assert!(four.keep);
        let three = mapping
            .rating_from(&sidecar(Some(3), None, None), RatingMode::KeepNotKeep)
            .unwrap();
        assert!(!three.keep);
    }

    #[test]
    fn keep_mode_reads_back_a_colour_keep() {
        let mapping = XmpMapping {
            keep_label: Some(ColorLabel::Green),
            ..XmpMapping::default()
        };
        let green = mapping
            .rating_from(&sidecar(None, Some("Green"), None), RatingMode::KeepNotKeep)
            .unwrap();
        assert!(green.keep);
        let red = mapping
            .rating_from(&sidecar(None, Some("Red"), None), RatingMode::KeepNotKeep)
            .unwrap();
        assert!(!red.keep);
    }

    #[test]
    fn a_rejected_sidecar_imports_as_rejected_in_keep_mode() {
        let mapping = XmpMapping::default();
        let imported = mapping
            .rating_from(
                &sidecar(Some(REJECTED), None, None),
                RatingMode::KeepNotKeep,
            )
            .unwrap();
        assert!(!imported.keep);
        assert_eq!(imported.flag, Flag::Reject);
    }

    #[test]
    fn an_out_of_range_rating_is_clamped_on_import() {
        let mapping = XmpMapping::default();
        let imported = mapping
            .rating_from(&sidecar(Some(9), None, None), RatingMode::Stars)
            .unwrap();
        assert_eq!(imported.stars, 5);
    }
}
