//! The Finish Cull sheet's state machine (todo.md §9.7, docs/contracts/app-model.md).
//!
//! v1 kept this in `FinishStage` on `AppModel`; v2 keeps it in the engine for the same reason the
//! rest of the flow lives here: the dry run, the typed gate and the undo log must not be
//! re-decided by a view. The UI draws [`FinishStage`] and dispatches `finish.*` commands; it never
//! advances the stage itself.
//!
//! Nothing touches the disk before [`FinishStage::DryRun`] has been shown. Execution walks the
//! plan built by that stage (`firstcut_fileops`), and the report is the database-backed run that
//! `finish.undo` can reverse.

use std::collections::HashMap;

pub use firstcut_fileops::{FinishOptions, FinishPlan, KeptAction, UnkeptAction};
use firstcut_types::Tier;
use serde_json::{Value, json};

/// Totals for the summary screen and the progress HUD (v1 `FinishSummary`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishSummary {
    pub counts: HashMap<Tier, usize>,
    pub unvisited_batches: usize,
    pub batch_count: usize,
    pub total_photos: usize,
}

impl FinishSummary {
    pub fn count(&self, tier: Tier) -> usize {
        self.counts.get(&tier).copied().unwrap_or(0)
    }

    pub fn kept_count(&self) -> usize {
        self.count(Tier::Keep)
    }

    pub fn unkept_count(&self) -> usize {
        self.total_photos.saturating_sub(self.kept_count())
    }

    pub fn is_complete(&self) -> bool {
        self.unvisited_batches == 0
    }
}

/// What a Finish run (or an Undo Finish) did, for the report the sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishReport {
    pub done: u32,
    /// `(path, reason)` for every file that was left alone.
    pub failed: Vec<(String, String)>,
    /// False once a permanent delete has run, whatever else happened.
    pub undoable: bool,
    /// True when this report is the result of an Undo Finish.
    pub was_undo: bool,
}

impl FinishReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
}

/// One screen of the Finish sheet. `Confirm` exists so the "Confirm before Finish" setting routes
/// the decision through the sheet rather than an alert (app-model.md); the typed `DELETE` gate is
/// separate and lives on `DryRun`, where the plan is known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FinishStage {
    #[default]
    Hidden,
    Confirm,
    Summary(FinishSummary),
    Options {
        summary: FinishSummary,
        options: FinishOptions,
    },
    DryRun {
        summary: FinishSummary,
        options: FinishOptions,
        plan: FinishPlan,
        requires_typed_confirmation: bool,
    },
    Report {
        summary: FinishSummary,
        report: FinishReport,
    },
    Failed {
        summary: FinishSummary,
        message: String,
    },
}

impl FinishStage {
    pub fn is_visible(&self) -> bool {
        !matches!(self, FinishStage::Hidden)
    }

    pub fn summary(&self) -> Option<&FinishSummary> {
        match self {
            FinishStage::Hidden | FinishStage::Confirm => None,
            FinishStage::Summary(summary)
            | FinishStage::Options { summary, .. }
            | FinishStage::DryRun { summary, .. }
            | FinishStage::Report { summary, .. }
            | FinishStage::Failed { summary, .. } => Some(summary),
        }
    }

    /// The stage name used by `finish.state` and the UI's title.
    pub fn name(&self) -> &'static str {
        match self {
            FinishStage::Hidden => "hidden",
            FinishStage::Confirm => "confirm",
            FinishStage::Summary(_) => "summary",
            FinishStage::Options { .. } => "options",
            FinishStage::DryRun { .. } => "dryRun",
            FinishStage::Report { .. } => "report",
            FinishStage::Failed { .. } => "failed",
        }
    }

    /// The whole stage as JSON, for the control channel, the CLI and tests. The UI reads the typed
    /// stage; this is so a headless driver can see the same sheet a person does.
    pub fn to_json(&self) -> Value {
        match self {
            FinishStage::Hidden => json!({ "stage": "hidden" }),
            FinishStage::Confirm => json!({ "stage": "confirm" }),
            FinishStage::Summary(summary) => json!({ "stage": "summary", "summary": summary_json(summary) }),
            FinishStage::Options { summary, options } => {
                json!({ "stage": "options", "summary": summary_json(summary), "unkept": unkept_json(&options.unkept), "kept": kept_json(&options.kept) })
            }
            FinishStage::DryRun { summary, options, plan, requires_typed_confirmation } => json!({
                "stage": "dryRun",
                "summary": summary_json(summary),
                "unkept": unkept_json(&options.unkept),
                "kept": kept_json(&options.kept),
                "ops": plan.ops.iter().map(|op| json!({ "kind": op.kind.as_str(), "from": op.from, "to": op.to })).collect::<Vec<_>>(),
                "previewLines": plan.preview_lines(),
                "warnings": plan.warnings,
                "bytesToCopy": plan.bytes_to_copy,
                "undoable": plan.is_undoable(),
                "requiresTypedConfirmation": requires_typed_confirmation,
            }),
            FinishStage::Report { summary, report } => json!({
                "stage": "report",
                "summary": summary_json(summary),
                "report": {
                    "done": report.done,
                    "failed": report.failed.iter().map(|(path, reason)| json!({ "path": path, "reason": reason })).collect::<Vec<_>>(),
                    "undoable": report.undoable,
                    "wasUndo": report.was_undo,
                }
            }),
            FinishStage::Failed { summary, message } => json!({ "stage": "failed", "summary": summary_json(summary), "message": message }),
        }
    }
}

fn summary_json(summary: &FinishSummary) -> Value {
    json!({
        "counts": {
            "keep": summary.count(Tier::Keep),
            "good": summary.count(Tier::Good),
            "maybe": summary.count(Tier::Maybe),
            "unrated": summary.count(Tier::Unrated),
            "rejected": summary.count(Tier::Rejected),
        },
        "keptCount": summary.kept_count(),
        "unkeptCount": summary.unkept_count(),
        "unvisitedBatches": summary.unvisited_batches,
        "batchCount": summary.batch_count,
        "totalPhotos": summary.total_photos,
        "isComplete": summary.is_complete(),
    })
}

fn unkept_json(action: &UnkeptAction) -> Value {
    match action {
        UnkeptAction::MarkRejectedInXmp => json!({ "action": "markRejected" }),
        UnkeptAction::MoveToSubfolder(folder) => json!({ "action": "subfolder", "folder": folder }),
        UnkeptAction::MoveToTrash => json!({ "action": "trash" }),
        UnkeptAction::DeletePermanently => json!({ "action": "delete" }),
        UnkeptAction::Nothing => json!({ "action": "nothing" }),
    }
}

fn kept_json(action: &KeptAction) -> Value {
    match action {
        KeptAction::None => json!({ "action": "none" }),
        KeptAction::CopyTo(path) => json!({ "action": "copy", "path": path }),
        KeptAction::MoveTo(path) => json!({ "action": "move", "path": path }),
        KeptAction::SplitByTier(path) => json!({ "action": "splitTier", "path": path }),
        KeptAction::SplitByStars(path) => json!({ "action": "splitStars", "path": path }),
        KeptAction::WriteList(path) => json!({ "action": "list", "path": path }),
    }
}

/// Whether a kept action needs a destination before a plan can be built. The sheet disables
/// Preview on the same rule; the model enforces it so a CLI cannot move files into "".
pub fn kept_needs_path(action: &KeptAction) -> bool {
    !matches!(action, KeptAction::None)
}

/// Whether an unkept action needs a subfolder name.
pub fn unkept_needs_path(action: &UnkeptAction) -> bool {
    matches!(action, UnkeptAction::MoveToSubfolder(_))
}

/// The typed word for a permanent delete. Also the only value `finish.execute` accepts when the
/// dry run says the gate is armed.
pub const TYPED_CONFIRMATION: &str = "DELETE";
