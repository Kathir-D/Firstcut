//! Boundary scoring against ground truth.
//!
//! Task.md §5.4: boundary precision/recall/F1, plus the two failure modes that matter to a user —
//! wrongly merged bursts (photos from two different plays hidden in one batch) and wrongly split
//! bursts (one play costing extra keystrokes). The first is much worse than the second, so both
//! are reported even when F1 alone would hide the difference.

use serde::{Deserialize, Serialize};

use crate::batch::{Batch, PhotoId};

/// The visually verified truth, in `tests/fixtures/ground-truth/<game>.json`.
///
/// Stored as file names, never as images (todo.md §5.4). The names are looked up in the ordered
/// sequence, so a ground-truth entry that is missing from the folder is reported rather than
/// silently shifting every boundary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundTruth {
    pub game: String,
    /// Where the metadata dump this was verified against came from.
    #[serde(default)]
    pub meta: String,
    /// Date a human last looked at the contact sheets.
    #[serde(default)]
    pub verified: String,
    /// Why the ambiguous-zone boundaries were decided the way they were.
    #[serde(default)]
    pub notes: String,
    /// Capture-ordered batches, each a list of file names.
    pub batches: Vec<Vec<String>>,
}

/// Precision, recall, F1 over batch-start positions, plus the two user-visible failure counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryMetrics {
    /// Boundaries the ground truth declares, excluding the start of the shoot.
    pub truth_boundaries: usize,
    pub predicted_boundaries: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    /// One predicted batch spanning two or more truth batches.
    pub wrong_merges: usize,
    /// One truth batch split across two or more predicted batches.
    pub wrong_splits: usize,
    /// Truth batches that no predicted batch overlaps at all.
    pub missed_batches: usize,
    /// Ground-truth file names the prediction could not be scored against: ones that are not in the
    /// metadata at all, and ones a hand-written file lists in two batches. Both make the numbers
    /// above wrong in a way no report would otherwise show — a name in two truth batches silently
    /// belongs to the *last* of them, which turns a merge into a split or the other way round.
    pub unusable_truth_names: Vec<String>,
    /// Predicted batches that combine frames from different truth batches. The worst outcome:
    /// photos from two different plays hidden in one batch.
    pub merge_examples: Vec<Vec<String>>,
}

impl BoundaryMetrics {
    /// Boundary F1 as a percentage, the number todo.md §5.4 sets the 98% target on.
    #[must_use]
    pub fn f1_percent(&self) -> f32 {
        self.f1 * 100.0
    }
}

/// Compare predicted batches against ground truth.
///
/// `truth` is given in file names and `predicted` in photo ids; `name_of` maps an id back to the file
/// name so both sides can be compared in one ordered space.
pub fn evaluate_boundaries<N: Fn(PhotoId) -> Option<String>>(
    predicted: &[Batch],
    truth: &GroundTruth,
    name_of: N,
) -> BoundaryMetrics {
    let pred_names: Vec<Vec<String>> = predicted
        .iter()
        .map(|b| b.photo_ids.iter().filter_map(|id| name_of(*id)).collect())
        .collect();
    evaluate_names(&pred_names, truth)
}

/// [`evaluate_boundaries`] when both sides are already file names, capture-ordered.
#[must_use]
pub fn evaluate_names(predicted: &[Vec<String>], truth: &GroundTruth) -> BoundaryMetrics {
    let mut m = BoundaryMetrics::default();
    let known: std::collections::HashSet<&String> = truth.batches.iter().flatten().collect();
    let predicted_names: std::collections::HashSet<&String> = predicted.iter().flatten().collect();
    let mut unusable: Vec<String> = known
        .iter()
        .filter(|n| !predicted_names.contains(*n))
        .map(|n| (*n).clone())
        .collect();
    // A name in two truth batches is a hand-editing accident, and it is not harmless: the overlap
    // maps below can only give that name to one of the two batches — the last — which silently
    // turns a wrong merge into a wrong split or the other way round.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for batch in &truth.batches {
        for name in batch {
            if !seen.insert(name.as_str()) {
                unusable.push(name.clone());
            }
        }
    }
    unusable.sort();
    unusable.dedup();
    m.unusable_truth_names = unusable;

    // Boundaries as the *set of files that start a batch*, which is robust to a batch being merged
    // or split and does not depend on index arithmetic.
    //
    // The first photo of the shoot is not a boundary — both sides start there — so it is excluded
    // from both sets before counting, otherwise every run gains a spurious true positive.
    //
    // A truth batch whose photos are no longer in the folder is not a boundary the batcher could
    // have drawn either, so it is dropped *before* the shoot's start is removed. Counting it left a
    // false negative in the denominator for a batch nobody could have predicted.
    let comparable: Vec<&Vec<String>> = truth
        .batches
        .iter()
        .filter(|b| b.iter().any(|n| predicted_names.contains(n)))
        .collect();
    let shoot_start = comparable
        .first()
        .and_then(|b| b.first())
        .cloned()
        .or_else(|| predicted.first().and_then(|b| b.first()).cloned());

    let pred_starts: std::collections::HashSet<&String> = predicted
        .iter()
        .filter_map(|b| b.first())
        .filter(|n| shoot_start.as_deref() != Some(n.as_str()))
        .collect();
    let truth_starts: std::collections::HashSet<&String> = comparable
        .iter()
        .filter_map(|b| b.first())
        .filter(|n| shoot_start.as_deref() != Some(n.as_str()))
        .collect();

    for start in &pred_starts {
        if truth_starts.contains(start) {
            m.true_positives += 1;
        } else {
            m.false_positives += 1;
        }
    }
    for start in &truth_starts {
        if !pred_starts.contains(start) {
            m.false_negatives += 1;
        }
    }

    // Both denominators come from the *filtered* sets. Taking them from `predicted.len() - 1` and
    // `truth.batches.len() - 1` instead let precision read 1.0 with a false positive counted: the
    // shoot's first photo is not in either set but was counted in those denominators, so a deleted
    // first frame made the denominator one short. The 98% F1 target rests on these numbers.
    m.predicted_boundaries = pred_starts.len();
    m.truth_boundaries = truth_starts.len();
    let tp = m.true_positives as f32;
    m.precision = if m.predicted_boundaries == 0 {
        0.0
    } else {
        tp / m.predicted_boundaries as f32
    };
    m.recall = if m.truth_boundaries == 0 {
        0.0
    } else {
        tp / m.truth_boundaries as f32
    };
    m.f1 = if m.precision + m.recall == 0.0 {
        0.0
    } else {
        2.0 * m.precision * m.recall / (m.precision + m.recall)
    };

    // Overlap matrix: how many truth batches does each predicted batch touch?
    let mut truth_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (t, batch) in truth.batches.iter().enumerate() {
        for name in batch {
            truth_of.insert(name.as_str(), t);
        }
    }
    for batch in predicted {
        let mut touched: Vec<usize> = batch
            .iter()
            .filter_map(|n| truth_of.get(n.as_str()))
            .copied()
            .collect();
        touched.sort_unstable();
        touched.dedup();
        if touched.len() > 1 {
            m.wrong_merges += 1;
            if m.merge_examples.len() < 20 {
                m.merge_examples.push(batch.clone());
            }
        }
    }

    let mut pred_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (p, batch) in predicted.iter().enumerate() {
        for name in batch {
            pred_of.insert(name.as_str(), p);
        }
    }
    for batch in &truth.batches {
        let mut touched: Vec<usize> = batch
            .iter()
            .filter_map(|n| pred_of.get(n.as_str()))
            .copied()
            .collect();
        touched.sort_unstable();
        touched.dedup();
        match touched.len() {
            0 => m.missed_batches += 1,
            1 => {}
            _ => m.wrong_splits += 1,
        }
    }

    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth(batches: &[&[&str]]) -> GroundTruth {
        GroundTruth {
            game: "t".into(),
            meta: String::new(),
            verified: String::new(),
            notes: String::new(),
            batches: batches
                .iter()
                .map(|b| b.iter().map(|s| (*s).to_string()).collect())
                .collect(),
        }
    }

    fn names(batches: &[&[&str]]) -> Vec<Vec<String>> {
        batches
            .iter()
            .map(|b| b.iter().map(|s| (*s).to_string()).collect())
            .collect()
    }

    #[test]
    fn an_exact_match_is_a_perfect_score() {
        let t = truth(&[&["a", "b"], &["c", "d"], &["e"]]);
        let m = evaluate_names(&names(&[&["a", "b"], &["c", "d"], &["e"]]), &t);
        assert_eq!(m.f1, 1.0);
        assert_eq!(m.wrong_merges, 0);
        assert_eq!(m.wrong_splits, 0);
    }

    #[test]
    fn a_merge_loses_the_boundary_and_counts_as_a_wrong_merge() {
        // One predicted batch for two truth batches: there is no predicted boundary left to be a
        // false positive, so the whole truth boundary is missed. `wrong_merges` is what makes the
        // failure visible.
        let t = truth(&[&["a", "b"], &["c", "d"]]);
        let m = evaluate_names(&names(&[&["a", "b", "c", "d"]]), &t);
        assert_eq!(m.false_negatives, 1);
        assert_eq!(m.wrong_merges, 1);
        assert_eq!(m.wrong_splits, 0);
        assert_eq!(m.merge_examples.len(), 1);
        assert_eq!(m.f1, 0.0);
    }

    #[test]
    fn a_split_adds_a_boundary_and_counts_as_a_wrong_split() {
        let t = truth(&[&["a", "b"], &["c", "d"]]);
        let m = evaluate_names(&names(&[&["a", "b"], &["c"], &["d"]]), &t);
        assert_eq!(m.true_positives, 1);
        assert_eq!(m.false_positives, 1);
        assert_eq!(m.wrong_splits, 1);
        assert_eq!(m.wrong_merges, 0);
    }

    #[test]
    fn a_truth_batch_the_folder_no_longer_has_is_reported() {
        let t = truth(&[&["a", "b"], &["zz"]]);
        let m = evaluate_names(&names(&[&["a", "b"]]), &t);
        assert_eq!(m.unusable_truth_names, vec!["zz".to_string()]);
        assert_eq!(m.missed_batches, 1);
    }

    #[test]
    fn a_deleted_first_frame_cannot_inflate_precision_to_one() {
        // The shoot's first photo is not a boundary, so it is not in either set — but it was in
        // `predicted.len() - 1`, which made the denominator one short whenever it was missing from
        // the folder. A split the batcher got *wrong* then scored precision 1.0, and the headline
        // 98% F1 rests on precision.
        let t = truth(&[&["a", "b"], &["c"]]);
        let m = evaluate_names(&names(&[&["b"], &["c"]]), &t);
        assert_eq!(
            m.false_positives, 1,
            "'b' is not where the truth says a batch starts"
        );
        assert_eq!(m.predicted_boundaries, 2);
        assert_eq!(m.truth_boundaries, 1);
        assert!((m.precision - 0.5).abs() < 1e-6, "got {}", m.precision);
        assert!((m.recall - 1.0).abs() < 1e-6, "got {}", m.recall);
        assert!((m.f1 - 2.0 / 3.0).abs() < 1e-5, "got {}", m.f1);
    }

    #[test]
    fn a_truth_batch_that_is_entirely_gone_is_not_a_missed_boundary() {
        // Its photos are not in the folder, so no boundary could have been drawn there. Scoring it
        // as a false negative would drag F1 down for something the batcher never saw.
        let t = truth(&[&["zz", "yy"], &["b", "c"], &["d"]]);
        let m = evaluate_names(&names(&[&["b", "c"], &["d"]]), &t);
        assert_eq!(m.truth_boundaries, 1);
        assert_eq!(m.false_negatives, 0);
        assert_eq!(m.recall, 1.0);
        assert_eq!(
            m.missed_batches, 1,
            "still reported as a batch nothing overlapped"
        );
        assert_eq!(
            m.unusable_truth_names,
            vec!["yy".to_string(), "zz".to_string()]
        );
    }

    #[test]
    fn a_name_in_two_truth_batches_is_reported() {
        // Hand-edited ground truth does this, and it silently skews both directions: the name
        // belongs to the *last* batch it appears in, so the other batch looks split and the second
        // one looks merged.
        let t = truth(&[&["a", "b"], &["b", "c"]]);
        let m = evaluate_names(&names(&[&["a", "b"], &["c"]]), &t);
        assert_eq!(m.unusable_truth_names, vec!["b".to_string()]);
    }

    #[test]
    fn an_empty_prediction_scores_zero_without_dividing_by_zero() {
        let m = evaluate_names(&[], &truth(&[&["a"]]));
        assert_eq!(m.f1, 0.0);
        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
    }
}
