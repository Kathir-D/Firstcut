//! The command registry (docs/v2-plan.md §4.3): every user-visible action is a stable string id
//! with one dispatch entry, so the UI, the CLI, the control channel, and the MCP server are all
//! the same verbs in the same shapes, and the keymap references ids, not code.
//!
//! Generated consumers (the menu, the ⌘? cheatsheet, MCP tool definitions) list [`REGISTRY`];
//! the semantic half of each id is stable across them. Nothing here may mutate a [`Model`]
//! except through its methods — the §4.3 one-way rule, stated in code.

use std::path::Path;

use serde_json::{Map, Value, json};

use firstcut_types::{ColorLabel, RatingMode, Tier};

use crate::model::Model;

/// One user-visible action: its stable id, one-line description, and its parameters (name, type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CommandDef {
    pub id: &'static str,
    pub description: &'static str,
    /// `(name, type)` pairs in order; empty for the parameter-less commands.
    pub params: &'static [(&'static str, &'static str)],
}

/// The M1 vertical-slice registry: every leg of §6 speaks these ids.
pub const REGISTRY: &[CommandDef] = &[
    // Open
    CommandDef {
        id: "file.open",
        description: "Open a shoot folder",
        params: &[("path", "folder path"), ("sessionDir", "optional: sessions directory override (tests, bench)")],
    },
    // Navigation
    CommandDef { id: "photo.previous", description: "Cursor one photo back (arrows rule at batch edges)", params: &[] },
    CommandDef { id: "photo.next", description: "Cursor one photo forward (arrows rule at batch edges)", params: &[] },
    CommandDef { id: "photo.goto", description: "Cursor to a photo inside the current batch", params: &[("index", "uint (0 .. batch size)")] },
    CommandDef { id: "batch.previous", description: "Enter the batch before the current one", params: &[] },
    CommandDef { id: "batch.next", description: "Enter the batch after the current one", params: &[] },
    CommandDef {
        id: "batch.jump",
        description: "Jump a whole batch: forward to the next batch's first photo, back to the previous batch's last",
        params: &[("direction", r#""forward" | "back""#)],
    },
    // Rating
    CommandDef { id: "rate.stars", description: "Set the current photo's stars (0..=5)", params: &[("stars", "uint 0..=5")] },
    CommandDef {
        id: "rate.starsAndAdvance",
        description: "Set the current photo's stars (1..=5) and advance whatever auto-advance says",
        params: &[("stars", "uint 1..=5")],
    },
    CommandDef { id: "flag.pick", description: "Toggle the current photo's pick flag", params: &[] },
    CommandDef { id: "flag.reject", description: "Reject the current photo", params: &[] },
    CommandDef { id: "flag.clear", description: "Clear the current photo's flag", params: &[] },
    CommandDef { id: "flag.toggle", description: "Toggle no-flag/pick on the current photo", params: &[] },
    CommandDef { id: "keep.set", description: "Keep the current photo (idempotent)", params: &[] },
    CommandDef { id: "keep.unset", description: "Un-keep the current photo (idempotent)", params: &[] },
    CommandDef { id: "keep.toggle", description: "Toggle keep on the current photo", params: &[] },
    CommandDef {
        id: "label.set",
        description: "Set or clear the current photo's colour label",
        params: &[("color", r#""red"|"yellow"|"green"|"blue"|"purple"|"none""#)],
    },
    CommandDef { id: "batch.reject", description: "Reject every photo in the current batch (Reject-Whole-Batch setting only)", params: &[] },
    // Edits
    CommandDef { id: "edit.undo", description: "Undo one rating step (navigates back to it)", params: &[] },
    CommandDef { id: "edit.redo", description: "Redo one rating step (navigates back to it)", params: &[] },
    // Settings the slice moves
    CommandDef { id: "settings.ratingMode", description: "Stars or Keep/Not-keep", params: &[("mode", r#""stars" | "keep""#)] },
    CommandDef { id: "settings.autoAdvance", description: "Auto-advance after every rating change", params: &[("on", "bool")] },
    CommandDef { id: "settings.autoAdvance.toggle", description: "Flip auto-advance for this session", params: &[] },
    CommandDef {
        id: "settings.arrowsAtBatchEnd",
        description: "What a plain arrow does at a batch edge: stop, or continue into the next batch",
        params: &[("behavior", r#""stop" | "continue""#)],
    },
    CommandDef { id: "settings.keepStars", description: "Stars that make a keep in stars mode", params: &[("stars", "4 | 5")] },
    // Queries
    CommandDef { id: "engine.state", description: "The cursor, phase, counts and last error, as JSON", params: &[] },
    CommandDef { id: "engine.commands", description: "This list, as JSON", params: &[] },
];

/// The one dispatch entry. A software surface calls `dispatch` with an id and a JSON object; it
/// never touches a `Model` field itself. Unknown ids and bad parameters are `Err`, not silent
/// no-ops, so a remote caller can tell a mistyped verb from an effect that did not apply.
pub fn dispatch(model: &mut Model, id: &str, params: &Value) -> Result<Value, String> {
    // Messages the session queued before this call (an XMP failure on an earlier rating, a
    // re-batch) answer right here, so a driver sees its own world before the next command runs.
    model.drain_session_messages();
    let result = dispatch_inner(model, id, params);
    // And a session message from THIS command (a failed sidecar write on a rating, say) is
    // reported against the result the caller just got.
    model.drain_session_messages();
    result
}

/// Whether the driver may act: only in the culling phase, and only outside transitions.
fn may_act(model: &Model) -> bool {
    model.can_act()
}

fn dispatch_inner(model: &mut Model, id: &str, params: &Value) -> Result<Value, String> {
    match id {
        "file.open" => {
            let path = string_param(params, "path")?;
            match params.get("sessionDir") {
                Some(Value::String(dir)) => model.open_folder_in(Path::new(&path), Path::new(dir))?,
                None => model.open_folder(Path::new(&path))?,
                Some(_) => return Err("sessionDir: expected a folder path (string)".to_string()),
            }
            Ok(state(model))
        }

        "photo.previous" => navigate(model, |m| m.move_photo(-1)),
        "photo.next" => navigate(model, |m| m.move_photo(1)),
        "photo.goto" => {
            let index = uint_param(params, "index")?;
            navigate(model, |m| m.go_to_photo(index as usize))
        }
        "batch.previous" => navigate(model, |m| m.move_batch(-1)),
        "batch.next" => navigate(model, |m| m.move_batch(1)),
        "batch.jump" => match string_param(params, "direction")?.as_str() {
            "forward" => navigate(model, |m| m.jump_batch(true)),
            "back" => navigate(model, |m| m.jump_batch(false)),
            other => Err(format!("batch.jump: direction is forward or back, not {other:?}")),
        },

        "rate.stars" => {
            let stars = uint_param(params, "stars")?;
            if stars > 5 {
                return Err("rate.stars: stars is 0..=5".to_string());
            }
            let applied = model.rate_stars(stars as u8);
            rate_result(model, applied)
        }
        "rate.starsAndAdvance" => {
            let stars = uint_param(params, "stars")?;
            if !(1..=5).contains(&stars) {
                return Err("rate.starsAndAdvance: stars is 1..=5".to_string());
            }
            let applied = model.rate_stars_and_advance(stars as u8);
            rate_result(model, applied)
        }
        "flag.pick" => let_applied(model, |m| m.toggle_pick_flag()),
        "flag.reject" => let_applied(model, |m| m.reject_flag()),
        "flag.clear" => let_applied(model, |m| m.unflag()),
        "flag.toggle" => let_applied(model, |m| m.toggle_flag()),
        "keep.set" => let_applied(model, |m| m.set_keep()),
        "keep.unset" => let_applied(model, |m| m.set_not_keep()),
        "keep.toggle" => let_applied(model, |m| m.toggle_keep()),
        "label.set" => {
            let color = string_param(params, "color")?;
            let label = color_label(&color).ok_or_else(|| format!("label.set: color is one of red yellow green blue purple none, not {color:?}"))?;
            let_applied(model, |m| m.set_label(label))
        }
        "batch.reject" => {
            let applied = model.reject_current_batch();
            let mut view = state(model);
            if let Some(object) = view.as_object_mut() {
                object.insert("reverted".to_string(), Value::Bool(applied));
            }
            Ok(view)
        }

        "edit.undo" | "edit.redo" => {
            let moved = if id == "edit.undo" { model.undo() } else { model.redo() };
            let mut view = json_move(moved, cursor_of(model));
            if let Some(object) = view.as_object_mut() {
                object.insert("edited".to_string(), Value::Bool(moved));
            }
            Ok(view)
        }

        "settings.ratingMode" => {
            let mode = match string_param(params, "mode")?.as_str() {
                "stars" => RatingMode::Stars,
                "keep" => RatingMode::KeepNotKeep,
                other => return Err(format!("settings.ratingMode: mode is stars or keep, not {other:?}")),
            };
            model.set_rating_mode(mode);
            Ok(json!({ "ratingMode": mode.to_db() }))
        }
        "settings.autoAdvance" => {
            model.set_auto_advance(bool_param(params, "on")?);
            Ok(json!({ "autoAdvance": model.auto_advance() }))
        }
        "settings.autoAdvance.toggle" => {
            model.toggle_auto_advance();
            Ok(json!({ "autoAdvance": model.auto_advance() }))
        }
        "settings.arrowsAtBatchEnd" => {
            model.set_arrow_behavior(match string_param(params, "behavior")?.as_str() {
                "stop" => crate::model::ArrowBehavior::Stop,
                "continue" => crate::model::ArrowBehavior::ContinueIntoNextBatch,
                other => return Err(format!("settings.arrowsAtBatchEnd: behavior is stop or continue, not {other:?}")),
            });
            Ok(json!({ "arrowBehavior": "set", "behavior": string_param(params, "behavior")? }))
        }
        "settings.keepStars" => {
            let stars = uint_param(params, "stars")?;
            model.set_keep_stars(stars as u8);
            Ok(json!({ "keepStars": model.settings().keep_stars }))
        }

        "engine.state" => Ok(state(model)),
        "engine.commands" => Ok(json!({ "commands": REGISTRY })),
        unknown => Err(format!("no such command: {unknown:?} (engine.commands lists them)")),
    }
}

// --- result shapes --------------------------------------------------------------------------

/// What a navigation command replies: whether anything moved, and where the cursor is now.
fn navigate(model: &mut Model, move_cursor: impl FnOnce(&mut Model)) -> Result<Value, String> {
    let (batch, photo) = (model.current_batch_index(), model.current_photo_index());
    let may_act_before = may_act(model);
    move_cursor(model);
    let moved = if may_act_before { (model.current_batch_index(), model.current_photo_index()) != (batch, photo) } else { false };
    Ok(json_move(moved, cursor_of(model)))
}

/// Rate-and-report, structured so the model is borrowed once (`rate_photo` may flush the XMP
/// writer, which the result builder also needs the model for).
fn let_applied(model: &mut Model, apply: impl FnOnce(&mut Model) -> Option<firstcut_types::Rating>) -> Result<Value, String> {
    let applied = apply(model);
    rate_result(model, applied)
}

/// What a rating command replies: whether it applied, and the rating now.
fn rate_result(model: &mut Model, applied: Option<firstcut_types::Rating>) -> Result<Value, String> {
    let mut view = json_move(applied.is_some(), cursor_of(model));
    if let Some(object) = view.as_object_mut() {
        // `applied` says the rating changed; a no-op (the same value again) stays false, and
        // `moved` is the cursor's key, not a rating's.
        object.insert("applied".to_string(), Value::Bool(applied.is_some()));
    }
    if let Some(rating) = applied
        && let Some(object) = view.as_object_mut()
    {
        object.insert("rating".to_string(), rating_json(rating));
    }
    Ok(view)
}

fn json_move(moved: bool, mut cursor: Value) -> Value {
    let mut view = Map::new();
    view.insert("moved".to_string(), Value::Bool(moved));
    if let Some(cursor_object) = cursor.as_object_mut() {
        view.append(cursor_object);
    }
    Value::Object(view)
}

/// The cursor, in JSON: the batch and photo it points at, and that photo's rating.
fn cursor_of(model: &Model) -> Value {
    let photo = model.current_photo();
    json!({
        "batchIndex": model.current_batch_index(),
        "photoIndex": model.current_photo_index(),
        "photoId": model.current_photo_id().map(|id| id.0),
        "file": photo.map_or(String::new(), |p| p.meta.rel_path.clone()),
        "rating": photo.map_or(Value::Null, |p| rating_json(p.rating)),
    })
}

/// The state query: what the surfaces bind to, as JSON.
fn state(model: &Model) -> Value {
    let mut counts = Map::new();
    for (tier, count) in &model.counts_by_tier() {
        counts.insert(tier_name(*tier).to_string(), (*count).into());
    }
    json!({
        "phase": phase_name(model),
        "folderName": model.folder_name(),
        "photoCount": model.photos().len(),
        "batchCount": model.batches().len(),
        "currentBatchIndex": model.current_batch_index(),
        "currentPhotoIndex": model.current_photo_index(),
        "canAct": model.can_act(),
        "autoAdvance": model.auto_advance(),
        "ratedCount": model.rated_count(),
        "countsByTier": Value::Object(counts),
        "lastError": model.last_error().unwrap_or(""),
    })
}

fn phase_name(model: &Model) -> &'static str {
    match model.phase() {
        crate::model::Phase::Welcome => "welcome",
        crate::model::Phase::Loading => "loading",
        crate::model::Phase::Culling => "culling",
    }
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Keep => "keep",
        Tier::Good => "good",
        Tier::Maybe => "maybe",
        Tier::Unrated => "unrated",
        Tier::Rejected => "rejected",
    }
}

/// A `Rating` as JSON, the read shape for every result that carries one.
fn rating_json(rating: firstcut_types::Rating) -> Value {
    json!({
        "stars": rating.stars,
        "flag": flag_name(rating.flag),
        "label": rating.label.map(ColorLabel::as_str),
        "keep": rating.keep,
    })
}

fn flag_name(flag: firstcut_types::Flag) -> &'static str {
    match flag {
        firstcut_types::Flag::None => "none",
        firstcut_types::Flag::Pick => "pick",
        firstcut_types::Flag::Reject => "reject",
    }
}

fn color_label(name: &str) -> Option<Option<ColorLabel>> {
    match name {
        "none" => Some(None),
        "red" => Some(Some(ColorLabel::Red)),
        "yellow" => Some(Some(ColorLabel::Yellow)),
        "green" => Some(Some(ColorLabel::Green)),
        "blue" => Some(Some(ColorLabel::Blue)),
        "purple" => Some(Some(ColorLabel::Purple)),
        _ => None,
    }
}

// --- parameter plumbing ----------------------------------------------------------------------

fn string_param(params: &Value, name: &str) -> Result<String, String> {
    match params.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(format!("{name}: expected a string")),
        None => Err(format!("{name}: missing")),
    }
}

fn uint_param(params: &Value, name: &str) -> Result<u64, String> {
    match params.get(name) {
        Some(Value::Number(number)) => number.as_u64().ok_or_else(|| format!("{name}: expected an unsigned integer")),
        Some(_) => Err(format!("{name}: expected a number")),
        None => Err(format!("{name}: missing")),
    }
}

fn bool_param(params: &Value, name: &str) -> Result<bool, String> {
    match params.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name}: expected a boolean")),
        None => Err(format!("{name}: missing")),
    }
}
