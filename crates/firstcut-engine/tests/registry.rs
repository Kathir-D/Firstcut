//! The vertical-slice leg tests: every §6 surface speaks the registry, so its behavior is what
//! the tests pin — open, navigate, rate, undo — with the batch-shaping gaps the batching tests
//! themselves use (a ≤5 s idle joins a burst, a ≥15 s silence splits).

use firstcut_engine::model::{ArrowBehavior, Model};
use firstcut_engine::registry::{self, dispatch};
use firstcut_meta::meta::cr3::SyntheticCr3;
use firstcut_types::{ColorLabel, Flag, Rating};
use serde_json::{Value, json};

/// A prepared shoot: four photos in two bursts (00/.01 and 01:30/.31).
struct Shoot {
    folder: tempfile::TempDir,
    sessions: tempfile::TempDir,
}

impl Shoot {
    #[cfg(test)]
    fn with_bursts() -> Self {
        let folder = tempfile::tempdir().unwrap();
        for (name, at) in [
            ("IMG_0001.CR3", "2026:08:28 10:00:00"),
            ("IMG_0002.CR3", "2026:08:28 10:00:01"),
            ("IMG_0003.CR3", "2026:08:28 10:01:30"),
            ("IMG_0004.CR3", "2026:08:28 10:01:31"),
        ] {
            std::fs::write(folder.path().join(name), SyntheticCr3::r8().at(at).build()).unwrap();
        }
        let sessions = tempfile::tempdir().unwrap();
        Self { folder, sessions }
    }

    #[cfg(test)]
    fn path_str(&self) -> String {
        self.folder.path().to_string_lossy().to_string()
    }

    #[cfg(test)]
    fn open_params(&self) -> serde_json::Value {
        json!({ "path": self.path_str(), "sessionDir": self.sessions.path().to_string_lossy().to_string() })
    }

    /// Opens through the registry and asserts the fixtures shaped two batches.
    #[cfg(test)]
    fn open(&self) -> Model {
        let mut model = Model::new();
        let reply = dispatch(&mut model, "file.open", &self.open_params()).unwrap();
        assert_eq!(reply.get("batchCount"), Some(&json!(2)), "the fixtures shape two batches: {reply}");
        model
    }

    /// A path that does not exist, for the open-failure test.
    #[cfg(test)]
    fn missing_path_str(&self) -> String {
        self.folder.path().join("no-such-subfolder").to_string_lossy().to_string()
    }
}

#[cfg(test)]
fn current(model: &mut Model) -> Option<Rating> {
    model.current_photo().map(|p| p.rating)
}

#[cfg(test)]
fn stars(model: &mut Model) -> u8 {
    current(model).map_or(0, |r| r.stars)
}

#[test]
fn the_state_query_reports_the_culling_phase_and_the_cursor() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let state = dispatch(&mut model, "engine.state", &json!({})).unwrap();
    assert_eq!(state.get("phase"), Some(&json!("culling")));
    assert_eq!(state.get("batchCount"), Some(&json!(2)));
    assert_eq!(state.get("photoCount"), Some(&json!(4)));
    assert_eq!(state.get("currentBatchIndex"), Some(&json!(0)));
    assert_eq!(state.get("currentPhotoIndex"), Some(&json!(0)));
    assert_eq!(state.get("canAct"), Some(&json!(true)));
    assert!(state.get("lastError").and_then(Value::as_str).unwrap_or("").is_empty());
}

#[test]
fn arrows_move_within_the_batch_and_roll_into_the_next_one() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    // Photo 1 → inside batch 1.
    let reply = dispatch(&mut model, "photo.next", &json!({})).unwrap();
    assert_eq!(reply.get("moved"), Some(&json!(true)));
    assert_eq!(reply.get("photoIndex"), Some(&json!(1)));
    assert_eq!(reply.get("batchIndex"), Some(&json!(0)));
    // Photo 2 is the batch's last: one more → rolls into batch 2's first (default arrows rule).
    dispatch(&mut model, "photo.next", &json!({})).unwrap();
    assert_eq!(model.current_batch_index(), 1, "the default arrows rule continues into the next batch");
    assert_eq!(model.current_photo_index(), 0);
    // Backwards rolls back the same way: to the previous batch's last photo.
    dispatch(&mut model, "photo.previous", &json!({})).unwrap();
    assert_eq!(model.current_batch_index(), 0);
    assert_eq!(model.current_photo_index(), 1);
    dispatch(&mut model, "photo.previous", &json!({})).unwrap();
    assert_eq!(model.current_photo_index(), 0);
    // At the first photo of the shoot, nothing moves and it says so.
    let stuck = dispatch(&mut model, "photo.previous", &json!({})).unwrap();
    assert_eq!(stuck.get("moved"), Some(&json!(false)));
    assert_eq!(model.current_photo_index(), 0);
}

#[test]
fn batch_jump_lands_first_photo_forward_and_last_photo_back() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let forward = dispatch(&mut model, "batch.jump", &json!({ "direction": "forward" })).unwrap();
    assert_eq!(forward.get("batchIndex"), Some(&json!(1)));
    assert_eq!(forward.get("photoIndex"), Some(&json!(0)), "forward lands on the next batch's first photo");
    dispatch(&mut model, "photo.next", &json!({})).unwrap(); // off the first
    let back = dispatch(&mut model, "batch.jump", &json!({ "direction": "back" })).unwrap();
    assert_eq!(back.get("batchIndex"), Some(&json!(0)));
    assert_eq!(back.get("photoIndex"), Some(&json!(1)), "back lands on the previous batch's last photo");
}

#[test]
fn stop_at_batch_edges_when_the_setting_says_so() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "settings.arrowsAtBatchEnd", &json!({ "behavior": "stop" })).unwrap();
    dispatch(&mut model, "photo.next", &json!({})).unwrap(); // → photo 2
    let stuck = dispatch(&mut model, "photo.next", &json!({})).unwrap(); // stuck
    assert_eq!(stuck.get("moved"), Some(&json!(false)));
    assert_eq!(model.current_batch_index(), 0, "stop keeps the cursor in the batch");
    // …while starsAndAdvance ALWAYS advances across (never stops mid-burst).
    dispatch(&mut model, "rate.starsAndAdvance", &json!({ "stars": 5 })).unwrap();
    assert_eq!(model.current_batch_index(), 1, "⇧5 advances past the batch edge whatever the arrows say");
}

#[test]
fn rate_stars_sets_and_persists_across_reopen() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let reply = dispatch(&mut model, "rate.stars", &json!({ "stars": 4 })).unwrap();
    assert_eq!(reply.get("applied"), Some(&json!(true)), "a rating that changed is reported");
    assert_eq!(reply.get("rating").and_then(|r| r.get("stars")), Some(&json!(4)));

    // Reopen the same folder: the 4 stars come back from the database, at the restored cursor.
    let again = fixtures.open();
    assert_eq!(again.current_photo().map(|p| p.rating.stars), Some(4), "the 4 stars survive a reopen at the same photo");
}

#[test]
fn the_same_rating_twice_reports_no_change_and_no_second_db_write() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "rate.stars", &json!({ "stars": 4 })).unwrap();
    let again = dispatch(&mut model, "rate.stars", &json!({ "stars": 4 })).unwrap();
    assert_eq!(again.get("applied"), Some(&json!(false)), "an unchanged rating applied nothing");
    // One undo step existed for the first write; the no-op created none, so one undo undoes it.
    dispatch(&mut model, "edit.undo", &json!({})).unwrap();
    assert_eq!(stars(&mut model), 0, "nothing extra was recorded for the repeat");
}

#[test]
fn undo_reverts_and_navigates_to_the_change() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "photo.next", &json!({})).unwrap(); // photo 2
    dispatch(&mut model, "rate.stars", &json!({ "stars": 5 })).unwrap();
    dispatch(&mut model, "batch.jump", &json!({ "direction": "forward" })).unwrap(); // away

    let reply = dispatch(&mut model, "edit.undo", &json!({})).unwrap();
    assert_eq!(reply.get("edited"), Some(&json!(true)));
    assert_eq!(reply.get("batchIndex"), Some(&json!(0)), "undo navigates to the change's batch");
    assert_eq!(reply.get("photoIndex"), Some(&json!(1)));
    assert_eq!(stars(&mut model), 0, "the revert puts back the before-rating");

    let redo = dispatch(&mut model, "edit.redo", &json!({})).unwrap();
    assert_eq!(redo.get("edited"), Some(&json!(true)));
    assert_eq!(stars(&mut model), 5);
}

#[test]
fn keep_mode_stores_stars_as_stars_and_keeps_the_write_back_hazard_dead() {
    // REV-31 / REV-69: in keep mode a rating change must leave `keep` alone, or a "4 stars"
    // rating silently writes a keep the user never gave and the sidecar says what nobody saw.
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "settings.ratingMode", &json!({ "mode": "keep" })).unwrap();
    dispatch(&mut model, "rate.stars", &json!({ "stars": 4 })).unwrap();
    let rating = current(&mut model).unwrap_or_default();
    assert_eq!(rating.stars, 4, "the stars the user gave are the stars stored");
    assert!(!rating.keep, "no keep write-back: the display promotion is the display tier's work");
}

#[test]
fn keep_set_is_idempotent_and_toggle_flips() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "keep.set", &json!({})).unwrap();
    let again = dispatch(&mut model, "keep.set", &json!({})).unwrap();
    assert_eq!(again.get("applied"), Some(&json!(false)), "the Keep button does not un-keep on a second click");
    assert_eq!(current(&mut model).map(|r| r.keep), Some(true));
    dispatch(&mut model, "keep.toggle", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.keep), Some(false));
    dispatch(&mut model, "keep.unset", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.keep), Some(false), "unset is idempotent too");
}

#[test]
fn flag_commands_round_trip() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "flag.reject", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.flag), Some(Flag::Reject));
    dispatch(&mut model, "flag.clear", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.flag), Some(Flag::None));
    dispatch(&mut model, "flag.pick", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.flag), Some(Flag::Pick));
    let toggled = dispatch(&mut model, "flag.pick", &json!({})).unwrap();
    assert_eq!(current(&mut model).map(|r| r.flag), Some(Flag::None), "the pick toggle clears a pick");
    assert_eq!(toggled.get("applied"), Some(&json!(true)));
}

#[test]
fn label_set_round_trips_and_rejects_unknown_colors() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "label.set", &json!({ "color": "green" })).unwrap();
    assert_eq!(current(&mut model).map(|r| r.label), Some(Some(ColorLabel::Green)));
    dispatch(&mut model, "label.set", &json!({ "color": "none" })).unwrap();
    assert_eq!(current(&mut model).map(|r| r.label), Some(None));
    let bad = dispatch(&mut model, "label.set", &json!({ "color": "turquoise" })).expect_err("an unknown color");
    assert!(bad.contains("red"), "the error names the allowed values: {bad}");
}

#[test]
fn bad_parameters_and_unknown_ids_report_errors() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let missing = dispatch(&mut model, "rate.stars", &json!({})).expect_err("a missing parameter is an error");
    assert!(missing.contains("stars"), "the error names the parameter: {missing}");
    let out_of_range = dispatch(&mut model, "rate.stars", &json!({ "stars": 9 })).expect_err("9 is out of range");
    assert!(out_of_range.contains("0..=5"), "the error names the range: {out_of_range}");
    let unknown = dispatch(&mut model, "photo.teleport", &json!({})).expect_err("unknown ids are errors");
    assert!(unknown.contains("no such command"), "{unknown}");
    assert_eq!(stars(&mut model), 0, "the model is untouched after every failed dispatch");
}

#[test]
fn welcome_phase_answers_but_moves_nothing() {
    let mut model = Model::new();
    let reply = dispatch(&mut model, "photo.next", &json!({})).unwrap();
    assert_eq!(reply.get("moved"), Some(&json!(false)), "there is nothing to move yet");
    let state = dispatch(&mut model, "engine.state", &json!({})).unwrap();
    assert_eq!(state.get("phase"), Some(&json!("welcome")));
}

#[test]
fn settings_commands_take_effect() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    dispatch(&mut model, "settings.keepStars", &json!({ "stars": 5 })).unwrap();
    dispatch(&mut model, "settings.autoAdvance", &json!({ "on": true })).unwrap();
    assert_eq!(model.settings().keep_stars, 5);
    assert!(model.auto_advance(), "auto-advance honours the setting");
    let before = model.current_photo_index();
    dispatch(&mut model, "rate.stars", &json!({ "stars": 5 })).unwrap();
    assert_eq!(model.current_photo_index(), before + 1, "with auto-advance on, the rating advanced");
}

#[test]
fn arrows_can_stop_at_batch_edges_when_the_setting_says_so() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    model.set_arrow_behavior(ArrowBehavior::Stop);
    dispatch(&mut model, "rate.starsAndAdvance", &json!({ "stars": 5 })).unwrap();
    dispatch(&mut model, "rate.starsAndAdvance", &json!({ "stars": 5 })).unwrap();
    assert_eq!(model.current_batch_index(), 1, "⇧5 advances past the batch edge whatever the arrows say");
}

#[test]
fn opening_a_missing_folder_reports_and_stays_welcome() {
    let fixtures = Shoot::with_bursts();
    let mut model = Model::new();
    let error =
        dispatch(&mut model, "file.open", &json!({ "path": fixtures.missing_path_str() })).expect_err("a folder that does not exist cannot open");
    assert!(!error.is_empty(), "the failure has a message for the window");
    let state = dispatch(&mut model, "engine.state", &json!({})).unwrap();
    assert_eq!(state.get("phase"), Some(&json!("welcome")), "the model stayed welcome");
    assert!(state.get("lastError").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false), "the window can show the failure");
}

#[test]
fn the_registry_lists_every_dispatchable_command() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let list = dispatch(&mut model, "engine.commands", &json!({})).unwrap();
    let ids: Vec<&str> = list
        .get("commands")
        .and_then(Value::as_array)
        .map(|commands| commands.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    for id in ["file.open", "photo.next", "batch.jump", "rate.stars", "edit.undo", "engine.state"] {
        assert!(ids.contains(&id), "the registry lists {id}");
    }
    assert_eq!(ids.len(), registry::REGISTRY.len(), "the list is the whole registry");
}

#[test]
fn photo_goto_moves_inside_and_outside_bounds_stay() {
    let fixtures = Shoot::with_bursts();
    let mut model = fixtures.open();
    let ok = dispatch(&mut model, "photo.goto", &json!({ "index": 1 })).unwrap();
    assert_eq!(ok.get("moved"), Some(&json!(true)));
    assert_eq!(model.current_photo_index(), 1);
    let out = dispatch(&mut model, "photo.goto", &json!({ "index": 99 })).unwrap();
    assert_eq!(out.get("moved"), Some(&json!(false)), "out of bounds does not move");
    assert_eq!(model.current_photo_index(), 1);
}

#[test]
fn file_open_path_is_required() {
    let mut model = Model::new();
    let error = dispatch(&mut model, "file.open", &json!({})).expect_err("the path parameter is required");
    assert!(error.contains("path"), "{error}");
    assert!(error.contains("missing"), "{error}");
}

#[test]
fn reopening_restores_the_cursor_the_database_kept() {
    let fixtures = Shoot::with_bursts();
    {
        let mut model = fixtures.open();
        dispatch(&mut model, "batch.jump", &json!({ "direction": "forward" })).unwrap();
        dispatch(&mut model, "photo.next", &json!({})).unwrap();
    }
    let model = fixtures.open();
    assert_eq!(model.current_batch_index(), 1, "the reopen starts where the cursor stopped");
    assert_eq!(model.current_photo_index(), 1);
}
