//! The Finish Cull sheet (todo.md §9.7, docs/contracts/app-model.md).
//!
//! The engine owns the stage machine (`firstcut_engine::finish::FinishStage`); this module only
//! draws the stage and dispatches `finish.*`. That split is what makes the sheet replayable from
//! the CLI and the control channel, and it is why the typed `DELETE` gate is enforced in the
//! model rather than here.
//!
//! v1's sheet had six screens — confirm, summary, options, dry run, executing, report — plus a
//! failure screen. v2 drops the executing screen: `finish.execute` is synchronous, exactly like
//! v1's backend call, so a progress screen would never paint. Everything else is the same shape,
//! including the confirm-before-Finish question living in the sheet rather than an alert.

use egui::{Color32, RichText};
use firstcut_engine::finish::{FinishReport, FinishStage, FinishSummary, TYPED_CONFIRMATION, kept_needs_path, unkept_needs_path};
use firstcut_engine::model::Model;
use firstcut_engine::registry::dispatch;
use firstcut_types::Tier;
use serde_json::{Value as Json, json};

const FOREGROUND: Color32 = Color32::from_rgb(232, 232, 236);
const DANGER: Color32 = Color32::from_rgb(240, 128, 128);
const KEEP_GREEN: Color32 = Color32::from_rgb(120, 200, 120);

/// The sheet's transient UI state. Everything durable lives in the engine's `FinishStage`; this
/// is only what a text field needs between frames.
#[derive(Default)]
pub struct FinishSheet {
    /// The typed confirmation. Cleared whenever a new dry run opens.
    typed: String,
    last_stage: Option<&'static str>,
}

impl FinishSheet {
    pub fn show(&mut self, ctx: &egui::Context, model: &mut Model) {
        let stage = model.finish_stage().clone();
        if !stage.is_visible() {
            self.last_stage = None;
            return;
        }
        if self.last_stage != Some(stage.name()) {
            if stage.name() == "dryRun" {
                self.typed.clear();
            }
            self.last_stage = Some(stage.name());
        }

        let mut actions: Vec<(&'static str, Json)> = Vec::new();
        egui::Window::new("Finish Cull")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(620.0)
            .show(ctx, |window| {
                window.set_min_width(560.0);
                window.set_max_width(680.0);
                match &stage {
                    FinishStage::Hidden => {}
                    FinishStage::Confirm => confirm_screen(window, &mut actions),
                    FinishStage::Summary(summary) => summary_screen(window, summary, &mut actions),
                    FinishStage::Options { summary, options } => {
                        options_screen(window, summary, options, &mut actions);
                    }
                    FinishStage::DryRun { summary, plan, requires_typed_confirmation, .. } => {
                        dry_run_screen(window, summary, plan, *requires_typed_confirmation, &mut self.typed, &mut actions);
                    }
                    FinishStage::Report { summary, report } => report_screen(window, summary, report, &mut actions),
                    FinishStage::Failed { summary, message } => failed_screen(window, summary, message, &mut actions),
                }
            });

        for (id, params) in actions {
            // The sheet is allowed to fail at dispatch (a stale click after the stage moved);
            // the model keeps the reason in `last_error`, which the error alert shows.
            let _ = dispatch(model, id, &params);
        }
    }
}

fn confirm_screen(window: &mut egui::Ui, actions: &mut Vec<(&'static str, Json)>) {
    window.label(RichText::new("Finish this cull?").strong().color(FOREGROUND));
    window.add_space(6.0);
    window.label("First you will see what Finish would do, then choose where the photos go. Nothing is moved, copied or deleted until you confirm the dry run.");
    window.add_space(12.0);
    window.horizontal(|row| {
        if row.button("Cancel").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
        if row.button("Continue").clicked() {
            actions.push(("finish.confirm", Json::Null));
        }
    });
}

fn summary_screen(window: &mut egui::Ui, summary: &FinishSummary, actions: &mut Vec<(&'static str, Json)>) {
    window.label(RichText::new(format!("{} photos in {} batches", summary.total_photos, summary.batch_count)).strong().color(FOREGROUND));
    window.add_space(8.0);
    egui::Grid::new("finish-summary-tiers").num_columns(2).spacing([24.0, 4.0]).show(window, |grid| {
        for tier in Tier::ALL {
            let color = tier_color(tier);
            grid.horizontal(|row| {
                let (rect, _) = row.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                row.painter().circle_filled(rect.center(), 5.0, color);
                row.label(RichText::new(tier.to_string()).color(FOREGROUND));
            });
            grid.label(RichText::new(summary.count(tier).to_string()).monospace().color(if tier == Tier::Keep { FOREGROUND } else { Color32::GRAY }));
            grid.end_row();
        }
    });
    if summary.unvisited_batches > 0 {
        window.add_space(10.0);
        let plural = if summary.unvisited_batches == 1 { "batch was" } else { "batches were" };
        window.label(
            RichText::new(format!("{} {} never opened. Photos in them count as not kept.", summary.unvisited_batches, plural))
                .color(Color32::from_rgb(230, 180, 80)),
        );
    }
    window.add_space(10.0);
    window.label(RichText::new("Only the Keep tier is kept. Originals are never modified until you confirm on the next screens.").small().weak());
    window.add_space(12.0);
    window.horizontal(|row| {
        if row.button("Cancel").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
        if row.button("Choose What Happens Next").clicked() {
            actions.push(("finish.options", Json::Null));
        }
    });
}

fn options_screen(
    window: &mut egui::Ui,
    summary: &FinishSummary,
    options: &firstcut_engine::finish::FinishOptions,
    actions: &mut Vec<(&'static str, Json)>,
) {
    window.label(RichText::new(format!("{} kept, {} not kept", summary.kept_count(), summary.unkept_count())).strong().color(FOREGROUND));
    window.add_space(4.0);

    window.label(RichText::new("NOT KEPT").small().weak());
    let mut unkept_kind = unkept_kind_of(&options.unkept);
    let mut folder = match &options.unkept {
        firstcut_engine::finish::UnkeptAction::MoveToSubfolder(folder) => folder.clone(),
        _ => "_Not kept".to_string(),
    };
    let before = (unkept_kind, folder.clone());
    for (kind, label) in [
        (UnkeptKind::MarkRejected, "Mark as rejected in XMP"),
        (UnkeptKind::Subfolder, "Move to a subfolder"),
        (UnkeptKind::Trash, "Move to Trash"),
        (UnkeptKind::Delete, "Delete permanently"),
        (UnkeptKind::Nothing, "Do nothing"),
    ] {
        window.radio_value(&mut unkept_kind, kind, label);
    }
    if unkept_kind == UnkeptKind::Subfolder {
        window.horizontal(|row| {
            row.label("Subfolder");
            row.add(egui::TextEdit::singleline(&mut folder).hint_text("_Not kept"));
        });
        if folder.trim().is_empty() {
            window.label(RichText::new("Give the subfolder a name before previewing.").small().color(DANGER));
        }
    }
    if unkept_kind == UnkeptKind::Delete {
        window.label(RichText::new("A permanent delete cannot be undone.").small().color(DANGER));
    }
    if before != (unkept_kind, folder.clone()) {
        actions.push(("finish.unkept", json!({ "action": unkept_kind.id(), "folder": folder })));
    }

    window.add_space(8.0);
    window.label(RichText::new("KEPT").small().weak());
    let mut kept_kind = kept_kind_of(&options.kept);
    let mut path = match &options.kept {
        firstcut_engine::finish::KeptAction::CopyTo(path)
        | firstcut_engine::finish::KeptAction::MoveTo(path)
        | firstcut_engine::finish::KeptAction::SplitByTier(path)
        | firstcut_engine::finish::KeptAction::SplitByStars(path)
        | firstcut_engine::finish::KeptAction::WriteList(path) => path.clone(),
        firstcut_engine::finish::KeptAction::None => String::new(),
    };
    let before = (kept_kind, path.clone());
    for (kind, label) in [
        (KeptKind::None, "Do nothing"),
        (KeptKind::Copy, "Copy to a folder"),
        (KeptKind::Move, "Move to a folder"),
        (KeptKind::SplitTier, "Split by tier"),
        (KeptKind::SplitStars, "Split by stars"),
        (KeptKind::List, "Write a list"),
    ] {
        window.radio_value(&mut kept_kind, kind, label);
    }
    if kept_kind != KeptKind::None {
        window.horizontal(|row| {
            row.label("Destination");
            row.add(egui::TextEdit::singleline(&mut path).hint_text("Destination path"));
        });
        let hint = match kept_kind {
            KeptKind::Copy | KeptKind::Move => "A relative path is taken from the shoot folder.",
            KeptKind::List => "The file name for the list.",
            _ => "The root folder for the split.",
        };
        window.label(RichText::new(hint).small().weak());
        if path.trim().is_empty() {
            window.label(RichText::new("Give the destination a name before previewing.").small().color(DANGER));
        }
    }
    if before != (kept_kind, path.clone()) {
        actions.push(("finish.kept", json!({ "action": kept_kind.id(), "path": path })));
    }

    window.add_space(12.0);
    window.horizontal(|row| {
        if row.button("Back").clicked() {
            actions.push(("finish.back", Json::Null));
        }
        if row.button("Cancel").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
        let ready = options_ready(&options.unkept, &options.kept, &folder, &path);
        if row.add_enabled(ready, egui::Button::new("Preview")).clicked() {
            actions.push(("finish.preview", Json::Null));
        }
    });
}

fn dry_run_screen(
    window: &mut egui::Ui,
    summary: &FinishSummary,
    plan: &firstcut_engine::finish::FinishPlan,
    requires_typed_confirmation: bool,
    typed: &mut String,
    actions: &mut Vec<(&'static str, Json)>,
) {
    window.label(
        RichText::new(format!("Dry run — {} file operation{}", plan.ops.len(), if plan.ops.len() == 1 { "" } else { "s" }))
            .strong()
            .color(FOREGROUND),
    );
    window
        .label(RichText::new(format!("{} kept, {} not kept. Nothing has changed yet.", summary.kept_count(), summary.unkept_count())).small().weak());

    if plan.bytes_to_copy > 0 {
        window.label(RichText::new(format!("{} would be copied.", bytes_label(plan.bytes_to_copy))).small().weak());
    }
    for warning in &plan.warnings {
        window.label(RichText::new(warning).small().color(Color32::from_rgb(230, 180, 80)));
    }
    if !plan.is_undoable() {
        window.label(RichText::new("This run deletes permanently. It cannot be undone.").small().color(DANGER));
    }

    window.add_space(6.0);
    egui::ScrollArea::vertical().max_height(200.0).show(window, |list| {
        for line in plan.preview_lines() {
            list.label(RichText::new(line).monospace().small().color(FOREGROUND));
        }
    });

    if requires_typed_confirmation {
        window.add_space(8.0);
        window.label(RichText::new(format!("Type {} to confirm the permanent delete.", TYPED_CONFIRMATION)).color(DANGER));
        let response = window.add(egui::TextEdit::singleline(typed).hint_text(TYPED_CONFIRMATION));
        response.request_focus();
    }

    window.add_space(12.0);
    window.horizontal(|row| {
        if row.button("Back").clicked() {
            actions.push(("finish.back", Json::Null));
        }
        if row.button("Cancel").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
        let confirmed = !requires_typed_confirmation || typed == TYPED_CONFIRMATION;
        let label = if plan.is_undoable() { "Finish" } else { "Delete Permanently" };
        let button = if plan.is_undoable() { egui::Button::new(label) } else { egui::Button::new(RichText::new(label).color(DANGER)) };
        if row.add_enabled(confirmed && !plan.is_empty(), button).clicked() {
            if requires_typed_confirmation {
                actions.push(("finish.execute", json!({ "confirm": typed })));
            } else {
                actions.push(("finish.execute", Json::Null));
            }
        }
    });
}

fn report_screen(window: &mut egui::Ui, summary: &FinishSummary, report: &FinishReport, actions: &mut Vec<(&'static str, Json)>) {
    let title = if report.was_undo { "Finish undone" } else { "Finished" };
    window.label(RichText::new(title).strong().color(FOREGROUND));
    window.add_space(4.0);
    if report.was_undo {
        window.label(format!("{} file operation{} reversed.", report.done, if report.done == 1 { "" } else { "s" }));
    } else {
        window.label(format!("{} file operation{} done.", report.done, if report.done == 1 { "" } else { "s" }));
    }
    window.label(RichText::new(format!("{} kept, {} not kept when the run started.", summary.kept_count(), summary.unkept_count())).small().weak());

    if report.is_clean() {
        window.label(RichText::new("Everything went through.").color(KEEP_GREEN));
    } else {
        window.label(
            RichText::new(format!(
                "{} file{} could not be handled and {} left alone:",
                report.failed.len(),
                if report.failed.len() == 1 { "" } else { "s" },
                if report.failed.len() == 1 { "was" } else { "were" }
            ))
            .color(DANGER),
        );
        egui::ScrollArea::vertical().max_height(150.0).show(window, |list| {
            for (path, reason) in &report.failed {
                list.label(RichText::new(file_name(path)).color(FOREGROUND));
                list.label(RichText::new(reason).small().weak());
            }
        });
    }
    if !report.was_undo && !report.undoable {
        window.label(RichText::new("This run included a permanent delete, so it cannot be undone.").small().color(DANGER));
    }

    window.add_space(12.0);
    window.horizontal(|row| {
        if report.undoable && !report.was_undo && row.button("Undo Finish").clicked() {
            actions.push(("finish.undo", Json::Null));
        }
        if row.button("Done").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
    });
}

fn failed_screen(window: &mut egui::Ui, _summary: &FinishSummary, message: &str, actions: &mut Vec<(&'static str, Json)>) {
    window.label(RichText::new("Finish could not run").strong().color(DANGER));
    window.add_space(4.0);
    window.label(RichText::new(message).color(FOREGROUND));
    window.add_space(12.0);
    window.horizontal(|row| {
        if row.button("Back to Options").clicked() {
            actions.push(("finish.back", Json::Null));
        }
        if row.button("Close").clicked() {
            actions.push(("finish.cancel", Json::Null));
        }
    });
}

// ── action helpers, shared with Settings → General ───────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnkeptKind {
    MarkRejected,
    Subfolder,
    Trash,
    Delete,
    Nothing,
}

impl UnkeptKind {
    pub(crate) fn id(self) -> &'static str {
        match self {
            UnkeptKind::MarkRejected => "markRejected",
            UnkeptKind::Subfolder => "subfolder",
            UnkeptKind::Trash => "trash",
            UnkeptKind::Delete => "delete",
            UnkeptKind::Nothing => "nothing",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            UnkeptKind::MarkRejected => "Mark as rejected in XMP",
            UnkeptKind::Subfolder => "Move to a subfolder",
            UnkeptKind::Trash => "Move to Trash",
            UnkeptKind::Delete => "Delete permanently",
            UnkeptKind::Nothing => "Do nothing",
        }
    }

    pub(crate) fn apply(self, folder: &str) -> firstcut_engine::finish::UnkeptAction {
        match self {
            UnkeptKind::MarkRejected => firstcut_engine::finish::UnkeptAction::MarkRejectedInXmp,
            UnkeptKind::Subfolder => firstcut_engine::finish::UnkeptAction::MoveToSubfolder(folder.to_string()),
            UnkeptKind::Trash => firstcut_engine::finish::UnkeptAction::MoveToTrash,
            UnkeptKind::Delete => firstcut_engine::finish::UnkeptAction::DeletePermanently,
            UnkeptKind::Nothing => firstcut_engine::finish::UnkeptAction::Nothing,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeptKind {
    None,
    Copy,
    Move,
    SplitTier,
    SplitStars,
    List,
}

impl KeptKind {
    pub(crate) fn id(self) -> &'static str {
        match self {
            KeptKind::None => "none",
            KeptKind::Copy => "copy",
            KeptKind::Move => "move",
            KeptKind::SplitTier => "splitTier",
            KeptKind::SplitStars => "splitStars",
            KeptKind::List => "list",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            KeptKind::None => "Do nothing",
            KeptKind::Copy => "Copy to a folder",
            KeptKind::Move => "Move to a folder",
            KeptKind::SplitTier => "Split by tier",
            KeptKind::SplitStars => "Split by stars",
            KeptKind::List => "Write a list",
        }
    }

    pub(crate) fn apply(self, path: &str) -> firstcut_engine::finish::KeptAction {
        match self {
            KeptKind::None => firstcut_engine::finish::KeptAction::None,
            KeptKind::Copy => firstcut_engine::finish::KeptAction::CopyTo(path.to_string()),
            KeptKind::Move => firstcut_engine::finish::KeptAction::MoveTo(path.to_string()),
            KeptKind::SplitTier => firstcut_engine::finish::KeptAction::SplitByTier(path.to_string()),
            KeptKind::SplitStars => firstcut_engine::finish::KeptAction::SplitByStars(path.to_string()),
            KeptKind::List => firstcut_engine::finish::KeptAction::WriteList(path.to_string()),
        }
    }
}

pub(crate) fn unkept_kind_of(action: &firstcut_engine::finish::UnkeptAction) -> UnkeptKind {
    match action {
        firstcut_engine::finish::UnkeptAction::MarkRejectedInXmp => UnkeptKind::MarkRejected,
        firstcut_engine::finish::UnkeptAction::MoveToSubfolder(_) => UnkeptKind::Subfolder,
        firstcut_engine::finish::UnkeptAction::MoveToTrash => UnkeptKind::Trash,
        firstcut_engine::finish::UnkeptAction::DeletePermanently => UnkeptKind::Delete,
        firstcut_engine::finish::UnkeptAction::Nothing => UnkeptKind::Nothing,
    }
}

pub(crate) fn kept_kind_of(action: &firstcut_engine::finish::KeptAction) -> KeptKind {
    match action {
        firstcut_engine::finish::KeptAction::None => KeptKind::None,
        firstcut_engine::finish::KeptAction::CopyTo(_) => KeptKind::Copy,
        firstcut_engine::finish::KeptAction::MoveTo(_) => KeptKind::Move,
        firstcut_engine::finish::KeptAction::SplitByTier(_) => KeptKind::SplitTier,
        firstcut_engine::finish::KeptAction::SplitByStars(_) => KeptKind::SplitStars,
        firstcut_engine::finish::KeptAction::WriteList(_) => KeptKind::List,
    }
}

/// The folder text for an unkept action, or v1's default when it is not a subfolder.
pub(crate) fn unkept_folder(action: &firstcut_engine::finish::UnkeptAction) -> String {
    match action {
        firstcut_engine::finish::UnkeptAction::MoveToSubfolder(folder) => folder.clone(),
        _ => "_Not kept".to_string(),
    }
}

/// The path text for a kept action, or empty for `None`.
pub(crate) fn kept_path(action: &firstcut_engine::finish::KeptAction) -> String {
    match action {
        firstcut_engine::finish::KeptAction::None => String::new(),
        firstcut_engine::finish::KeptAction::CopyTo(path)
        | firstcut_engine::finish::KeptAction::MoveTo(path)
        | firstcut_engine::finish::KeptAction::SplitByTier(path)
        | firstcut_engine::finish::KeptAction::SplitByStars(path)
        | firstcut_engine::finish::KeptAction::WriteList(path) => path.clone(),
    }
}

fn options_ready(unkept: &firstcut_engine::finish::UnkeptAction, kept: &firstcut_engine::finish::KeptAction, folder: &str, path: &str) -> bool {
    (!unkept_needs_path(unkept) || !folder.trim().is_empty()) && (!kept_needs_path(kept) || !path.trim().is_empty())
}

fn tier_color(tier: Tier) -> Color32 {
    match tier {
        Tier::Keep => KEEP_GREEN,
        Tier::Good => Color32::from_rgb(220, 200, 90),
        Tier::Maybe => Color32::from_rgb(220, 150, 80),
        Tier::Unrated => Color32::GRAY,
        Tier::Rejected => DANGER,
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned())
}

fn bytes_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let suffix = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {suffix}") }
}
