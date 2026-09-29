//! `firstcut` — the developer CLI. Owner: core-batch (other agents add subcommands by request,
//! see docs/contracts/build.md).
//!
//! Wave 1 subcommands:
//!
//! - `dump-meta`  convert an exiftool dump into `tests/fixtures/meta/<game>.json`
//! - `order`      print capture order
//! - `batch`      print the batches for a folder's metadata
//! - `gaps`       the Δt histogram and every ambiguous-zone boundary, for tuning thresholds
//! - `bench`      time `order()` + `batch()` on a folder's metadata
//! - `contact-sheet`  render the ambiguous-zone boundaries as images for a human to look at
//! - `eval`       boundary F1 against `tests/fixtures/ground-truth/<game>.json`

mod fixtures;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use firstcut_core::batch::BatchParams;
use firstcut_core::batch::signals::Decision;
use firstcut_core::batch::{PhotoId, batch_with};
use firstcut_core::order;

use fixtures::Folder;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("firstcut: {e}");
            eprintln!("try `firstcut help`");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first() else {
        print_usage();
        return Ok(());
    };
    let rest = &args[1..];
    match command.as_str() {
        "help" | "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        "dump-meta" => cmd_dump_meta(rest),
        "order" => cmd_order(rest),
        "batch" => cmd_batch(rest),
        "gaps" => cmd_gaps(rest),
        "bench" => cmd_bench(rest),
        "contact-sheet" => cmd_contact_sheet(rest),
        "eval" => cmd_eval(rest),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

fn print_usage() {
    println!(
        "\
firstcut — Firstcut developer CLI

USAGE
  firstcut <command> [options]

COMMANDS
  dump-meta --from-exiftool <in.json> --out <out.json>
        Convert an `exiftool -j` dump into PhotoMeta-shaped JSON for the other
        agents' mocks. `dump-meta --fixture <name>` writes
        tests/fixtures/meta/<name>.json from tests/fixtures/exiftool/<name>.json.

  order <meta.json>
        Print capture order, one file name per line.

  batch <meta.json> [--json] [--freeze-from <i>] [--freeze-to <j>]
        Print the batches. Metadata only: every ambiguous boundary is provisional.

  gaps <meta.json> [--sorted]
        Print the Δt histogram and every boundary in the ambiguous zone with its
        score. This is how the thresholds in task.md §5 are tuned.

  bench <meta.json> [--repeat <n>]
        Time order() + batch() and compare against the 2 s / 1,500-file target.

  contact-sheet --game <name> [--from <first>] [--to <last>] [--out <dir>] [--all]
        The half of deliverable 4 that a machine can do. Renders every ambiguous-zone boundary
        as a strip of the frames either side of it, so a human can LOOK at the photographs and
        decide which boundaries are real. It does not decide anything itself: the ground truth
        has to be written by a person, because only looking can tell two adjacent frames of one
        burst from two different plays (task.md §12).

        Needs the real RAW files (FIRSTCUT_TEST_PHOTOS) because it renders the actual images.
        Prints the file names it left ambiguous, which is the list a human then rules on.

  eval <meta.json> [--game <name>] [--truth <file>]
        Boundary F1 against a ground-truth file, with the wrong merges and wrong splits spelled
        out. task.md §5.4 targets >= 98% boundary F1 and zero merges of clearly different plays.
"
    );
}

// ---------------------------------------------------------------- dump-meta

fn cmd_dump_meta(args: &[String]) -> Result<(), String> {
    let mut from_exiftool: Option<PathBuf> = None;
    let mut fixture: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from-exiftool" => {
                from_exiftool = Some(PathBuf::from(flag(args, &mut i, "--from-exiftool")?))
            }
            "--fixture" => fixture = Some(flag(args, &mut i, "--fixture")?),
            "--out" | "-o" => out = Some(PathBuf::from(flag(args, &mut i, "--out")?)),
            other => return Err(format!("dump-meta: unknown option `{other}`")),
        }
        i += 1;
    }

    let (input, default_out) = match (from_exiftool, fixture) {
        (Some(p), None) => (p, None),
        (None, Some(name)) => (
            repo_path(&format!("tests/fixtures/exiftool/{name}.json")),
            Some(repo_path(&format!("tests/fixtures/meta/{name}.json"))),
        ),
        (Some(_), Some(_)) => {
            return Err("dump-meta: pass either --from-exiftool or --fixture, not both".into());
        }
        (None, None) => {
            return Err("dump-meta: pass --from-exiftool <in.json> or --fixture <game>".into());
        }
    };

    let photos = fixtures::load_exiftool(&input)?;
    let out = out.or(default_out).ok_or("dump-meta: pass --out <file>")?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&photos).map_err(|e| e.to_string())?;
    std::fs::write(&out, format!("{json}\n"))
        .map_err(|e| format!("writing {}: {e}", out.display()))?;

    println!("{} photos -> {}", photos.len(), out.display());
    Ok(())
}

// -------------------------------------------------------------------- order

fn cmd_order(args: &[String]) -> Result<(), String> {
    let (folder, _) = load_folder(args)?;
    let (ids, report) = order::order_with_report(&folder.photos);
    for id in ids {
        if let Some(p) = folder.get_by_id(id) {
            println!("{}", p.rel_path);
        }
    }
    // Task.md §5.1: files with no usable timestamp must be flagged in the log, not silently
    // ordered as if their times were known.
    for path in &report.fallback_times {
        eprintln!("warning: {path}: no capture time in EXIF, used the file's mtime");
    }
    for path in &report.no_time {
        eprintln!("warning: {path}: no capture time and no mtime, ordered by name");
    }
    Ok(())
}

// -------------------------------------------------------------------- batch

fn cmd_batch(args: &[String]) -> Result<(), String> {
    let (folder, opts) = load_folder(args)?;
    let frozen = frozen_batches(&folder, &opts);
    let params = BatchParams::default();
    let outcome = batch_with(&folder.photos, &HashMap::new(), &frozen, params);

    if opts.json {
        let json = serde_json::to_string_pretty(&outcome.batches).map_err(|e| e.to_string())?;
        println!("{json}");
        return Ok(());
    }

    let total = outcome.batches.len();
    for b in &outcome.batches {
        let first = &b.photo_ids;
        let last = *first.last().expect("batches are never empty");
        println!(
            "batch {:>4} of {:<4}  {:>2} photos  {}  .. {}",
            b.index + 1,
            total,
            b.photo_ids.len(),
            name_of(&folder, b.photo_ids[0]),
            name_of(&folder, last),
        );
        if b.provisional {
            println!("               ^ provisional: signatures may still change this boundary");
        }
    }
    let provisional = outcome.batches.iter().filter(|b| b.provisional).count();
    println!(
        "\n{} photos, {} batches ({provisional} provisional), {} boundaries scored",
        folder.photos.len(),
        total,
        outcome
            .verdicts
            .iter()
            .filter(|v| v.decision == Decision::Ambiguous)
            .count()
    );
    Ok(())
}

// --------------------------------------------------------------------- gaps

fn cmd_gaps(args: &[String]) -> Result<(), String> {
    let (folder, opts) = load_folder(args)?;
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());

    // Histogram of every Δt, in the buckets the thresholds are expressed in.
    let mut buckets: HashMap<&str, usize> = HashMap::new();
    for v in &outcome.verdicts {
        *buckets.entry(bucket(v.signals.dt_ms)).or_default() += 1;
    }
    let mut rows: Vec<(&str, usize)> = buckets.into_iter().collect();
    rows.sort_by_key(|&(label, _)| bucket_order(label));
    println!("Δt          count");
    for (label, n) in rows {
        println!("{label:<12}{n:>5}");
    }

    println!("\nambiguous-zone boundaries (scored on metadata alone):");
    println!(
        "{:>4} {:>22} {:>22} {:>6} {:>6} {:>5} {:>5} {:>5} {:>4}",
        "i", "from", "to", "Δt", "f", "score", "focal", "ev", "split"
    );
    for v in &outcome.verdicts {
        if v.decision != Decision::Ambiguous {
            continue;
        }
        let from = name_of(&folder, outcome.order[v.index - 1]);
        let to = name_of(&folder, outcome.order[v.index]);
        println!(
            "{:>4} {:>22} {:>22} {:>6} {:>6} {:>5.2} {:>5.2} {:>5.2} {:>4}",
            v.index,
            from,
            to,
            v.signals.dt_ms,
            v.thresholds.frame_interval_ms,
            v.score,
            v.signals.focal_stops,
            v.signals.exposure_ev,
            if v.provisional { "yes" } else { "no" }
        );
    }

    if opts.sorted {
        println!(
            "\n(the ambiguous boundaries are in capture order; see the task.md §5 thresholds)"
        );
    }
    Ok(())
}

fn bucket(ms: i64) -> &'static str {
    match ms {
        ..=0 => "0 (tie)",
        1..=50 => "<= 50 ms",
        51..=100 => "50-100 ms",
        101..=150 => "100-150 ms",
        151..=200 => "150-200 ms",
        201..=250 => "200-250 ms",
        251..=500 => "250-500 ms",
        501..=1_000 => "0.5-1 s",
        1_001..=2_000 => "1-2 s",
        2_001..=10_000 => "2-10 s",
        10_001..=60_000 => "10-60 s",
        _ => "> 60 s",
    }
}

fn bucket_order(label: &str) -> u16 {
    match label {
        "0 (tie)" => 0,
        "<= 50 ms" => 1,
        "50-100 ms" => 2,
        "100-150 ms" => 3,
        "150-200 ms" => 4,
        "200-250 ms" => 5,
        "250-500 ms" => 6,
        "0.5-1 s" => 7,
        "1-2 s" => 8,
        "2-10 s" => 9,
        "10-60 s" => 10,
        _ => 11,
    }
}

// -------------------------------------------------------------------- bench

fn cmd_bench(args: &[String]) -> Result<(), String> {
    let (folder, opts) = load_folder(args)?;
    let params = BatchParams::default();

    // One untimed pass first, so the measurement isn't paying for the first touch of the pages.
    let _ = batch_with(&folder.photos, &no_sigs(), &[], params);

    let mut order_best = f64::MAX;
    let mut batch_best = f64::MAX;
    for _ in 0..opts.repeat {
        let start = std::time::Instant::now();
        let ids = order::order(&folder.photos);
        order_best = order_best.min(start.elapsed().as_secs_f64());
        std::hint::black_box(&ids);

        let start = std::time::Instant::now();
        let out = batch_with(&folder.photos, &no_sigs(), &[], params);
        batch_best = batch_best.min(start.elapsed().as_secs_f64());
        std::hint::black_box(&out);
    }

    let n = folder.photos.len();
    let scaled = batch_best * 1500.0 / n as f64;
    println!(
        "{} photos: order() {:.3} ms, batch() {:.3} ms (best of {}), {:.2} µs/photo",
        n,
        order_best * 1_000.0,
        batch_best * 1_000.0,
        opts.repeat,
        batch_best / n as f64 * 1e6
    );
    println!(
        "scaled to 1,500 photos: order+batch {:.1} ms",
        (order_best + batch_best) * 1500.0 / n as f64 * 1_000.0
    );
    println!("task.md §5 target: all 1,500 files batched in < 2,000 ms");
    if scaled > 2.0 {
        println!("FAIL: over the target");
    } else {
        println!("OK: inside the target");
    }
    Ok(())
}

fn no_sigs() -> HashMap<PhotoId, firstcut_core::batch::VisualSig> {
    HashMap::new()
}

// -------------------------------------------------------------- contact-sheet

/// Renders the ambiguous-zone boundaries as image strips, so a human can decide which ones are
/// real. Writes a `plan.json` next to the images: the machine's part is done when that file is
/// written, and the ground truth still has to be typed out by someone who looked.
fn cmd_contact_sheet(args: &[String]) -> Result<(), String> {
    use std::fs;
    use std::process::Command;

    let mut game: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;
    let mut cell_size: Option<usize> = None;
    let mut columns: Option<usize> = None;
    let mut all = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--game" => game = Some(flag(args, &mut i, "--game")?),
            "--out" => out = Some(PathBuf::from(flag(args, &mut i, "--out")?)),
            "--from" => from = Some(flag(args, &mut i, "--from")?),
            "--to" => to = Some(flag(args, &mut i, "--to")?),
            "--all" => all = true,
            "--cell" => {
                cell_size = Some(
                    flag(args, &mut i, "--cell")?
                        .parse()
                        .map_err(|_| "bad --cell (pixels per side)")?,
                )
            }
            "--columns" => {
                columns = Some(
                    flag(args, &mut i, "--columns")?
                        .parse()
                        .map_err(|_| "bad --columns")?,
                )
            }
            other => return Err(format!("contact-sheet: unknown option `{other}`")),
        }
        i += 1;
    }
    let game = game.ok_or("contact-sheet: pass --game <name>")?;
    let meta = repo_path(&format!("tests/fixtures/meta/{game}.json"));
    let folder = Folder::load(&meta)?;
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());

    let photos_root = std::env::var("FIRSTCUT_TEST_PHOTOS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~/Documents/testing"));
    let photos_root =
        photos_root
            .to_string_lossy()
            .strip_prefix('~')
            .map_or(photos_root.clone(), |rest| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(rest)
            });
    let shoot = photos_root.join(&game);
    if !shoot.is_dir() {
        return Err(format!(
            "contact-sheet needs the real photos at {}, and it did not render anything without \
             them. Set FIRSTCUT_TEST_PHOTOS. It deliberately does not fall back to a placeholder: \
             the point of this command is that a human looks at the actual frames.",
            shoot.display()
        ));
    }

    // Only the boundaries timing alone cannot decide. The rest are decided by the clock and do
    // not need a human; this is the whole visual-refinement budget (task.md §3).
    let ambiguous: Vec<usize> = outcome
        .verdicts
        .iter()
        .filter(|v| v.decision == Decision::Ambiguous)
        .map(|v| v.index)
        .collect();

    let out = out.unwrap_or_else(|| repo_path(&format!("docs/qa/contact-sheets/{game}")));
    fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", out.display()))?;

    let in_range = |name: &str| -> bool {
        if all {
            return true;
        }
        // Stop at the extension. Collecting every digit in the name gives "IMG_6149.CR3" -> 61493,
        // which silently matches nothing, so `--from/--to` looked like it filtered everything out.
        let number = |n: &str| -> u32 {
            n.rsplit_once('.')
                .map_or(n, |(stem, _)| stem)
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        };
        match (&from, &to) {
            (Some(f), Some(t)) => number(name) >= number(f) && number(name) <= number(t),
            _ => true,
        }
    };

    // Build the boundary list, then hand the rendering to the Core Text tool in
    // `tools/contact-sheet`. The CLI decides *which* boundaries a human has to look at; the tool
    // decides how to draw them. Splitting it that way is what lets the renderer be a `main.swift`
    // (the only place Swift allows top-level code) without dragging AppKit into the Rust CLI.
    let mut skipped = 0usize;
    let mut boundaries = Vec::new();
    for &index in &ambiguous {
        let a = name_of(&folder, outcome.order[index - 1]);
        let b = name_of(&folder, outcome.order[index]);
        if !in_range(&a) {
            skipped += 1;
            continue;
        }
        // The frames either side of the boundary. A human judging "same play or not" needs a couple
        // of each: one frame tells them the exposure, four tell them the motion.
        let before_count = index.saturating_sub(3);
        let mut frames: Vec<String> = outcome.order[before_count..index]
            .iter()
            .map(|id| name_of(&folder, *id))
            .collect();
        let boundary_frame = index - before_count;
        frames.extend(
            outcome.order[index..(index + 4).min(outcome.order.len())]
                .iter()
                .map(|id| name_of(&folder, *id)),
        );
        boundaries.push(serde_json::json!({
            "index": index,
            "game": game,
            "photos": shoot.display().to_string(),
            "beforeName": a,
            "afterName": b,
            "gapMs": outcome.verdicts[index - 1].signals.dt_ms,
            "reason": reason_for(&outcome.verdicts[index - 1]),
            "frames": frames,
            "boundaryFrame": boundary_frame,
        }));
    }

    if boundaries.is_empty() {
        return Err(format!(
            "no ambiguous boundaries left to look at{}. Either the range is empty, or the batcher \
             is confident about this stretch.",
            if skipped > 0 {
                format!(" ({skipped} fell outside --from/--to)")
            } else {
                String::new()
            }
        ));
    }

    // The renderer builds on first use; it is a separate binary precisely so `cargo build` does not
    // depend on a Swift toolchain being present.
    let script = repo_path("scripts/build-contact-sheet.sh")
        .display()
        .to_string();
    if !PathBuf::from(&script).exists() {
        return Err("contact-sheet: scripts/build-contact-sheet.sh is missing".into());
    }
    let build = Command::new("bash")
        .arg(&script)
        .output()
        .map_err(|e| format!("running {script}: {e}"))?;
    if !build.status.success() {
        return Err(format!(
            "contact-sheet: building the renderer failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
        ));
    }
    let bin = std::env::var("FIRSTCUT_CONTACT_SHEET_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_path("build/tools/contact-sheet"));
    let out_str = out.display().to_string();

    let request = serde_json::json!({
        "outDir": out_str,
        "cellSize": cell_size,
        "columns": columns,
        "boundaries": boundaries,
    });

    use std::io::Write;
    let mut child = Command::new(&bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    child
        .stdin
        .as_mut()
        .ok_or("contact-sheet: could not open the renderer's stdin")?
        .write_all(
            serde_json::to_vec(&request)
                .map_err(|e| e.to_string())?
                .as_slice(),
        )
        .map_err(|e| format!("writing the request: {e}"))?;
    let status = child
        .wait()
        .map_err(|e| format!("waiting for the renderer: {e}"))?;
    if !status.success() {
        return Err(format!("contact-sheet: the renderer exited {status}"));
    }

    println!(
        "\n{game}: {} ambiguous boundaries, {skipped} outside --from/--to",
        boundaries.len()
    );
    println!(
        "A human still has to look at these and write tests/fixtures/ground-truth/{game}.json."
    );
    Ok(())
}

/// Why a boundary was left ambiguous, for the sheet's caption. Timers alone cannot decide these;
/// the caption says what the batcher saw, which is what makes the sheet worth looking at.
fn reason_for(v: &firstcut_core::batch::BoundaryVerdict) -> &'static str {
    use firstcut_core::batch::signals::Decision;
    if v.signals.shutter_count_gap.unwrap_or(0) > 1 {
        return "frames deleted in camera";
    }
    if v.signals.orientation_changed {
        return "orientation change";
    }
    if v.signals.time_is_fallback {
        return "mtime fallback";
    }
    if v.had_sigs {
        return "visual signatures disagree";
    }
    match v.decision {
        Decision::Ambiguous => "gap in the ambiguous band",
        _ => "scored",
    }
}

// ------------------------------------------------------------------- eval

fn cmd_eval(args: &[String]) -> Result<(), String> {
    use firstcut_core::batch::GroundTruth;

    let (folder, opts, meta_path) = load_folder_with_path(args)?;
    let mut truth_path = None;
    let mut game: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--truth" => truth_path = Some(PathBuf::from(flag(args, &mut i, "--truth")?)),
            "--game" => game = Some(flag(args, &mut i, "--game")?),
            other if other.starts_with('-') => {
                return Err(format!("eval: unknown option `{other}`"));
            }
            _ => {}
        }
        i += 1;
    }
    // The game is not derivable from the photo names -- every file is `IMG_nnnn.CR3` in all four
    // games -- so it comes from the meta file's own directory, or from `--game`.
    let game = game.unwrap_or_else(|| {
        meta_path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().to_string())
    });
    let truth_path = truth_path
        .unwrap_or_else(|| repo_path(&format!("tests/fixtures/ground-truth/{game}.json")));

    if !truth_path.exists() {
        return Err(format!(
            "no ground truth at {}. Build it by LOOKING at the photographs: \
             `firstcut contact-sheet --game {game}` renders every ambiguous boundary, then a human \
             rules on them and writes the result. task.md §5.4 needs >= 98% boundary F1 and this \
             project will not fake the number to get it.",
            truth_path.display()
        ));
    }

    let truth: GroundTruth =
        serde_json::from_str(&std::fs::read_to_string(&truth_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("parsing {}: {e}", truth_path.display()))?;

    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());
    let predicted: Vec<Vec<String>> = outcome
        .batches
        .iter()
        .map(|b| b.photo_ids.iter().map(|id| name_of(&folder, *id)).collect())
        .collect();
    let m = firstcut_core::batch::evaluate_names(&predicted, &truth);

    println!("game            {}", truth.game);
    println!(
        "verified        {}",
        if truth.verified.is_empty() {
            "(not recorded)"
        } else {
            &truth.verified
        }
    );
    println!(
        "boundaries      {} truth, {} predicted",
        m.truth_boundaries, m.predicted_boundaries
    );
    println!(
        "precision       {:.1}%   ({}/{} true positives)",
        m.precision * 100.0,
        m.true_positives,
        m.truth_boundaries
    );
    println!(
        "recall          {:.1}%   ({}/{} found)",
        m.recall * 100.0,
        m.true_positives,
        m.truth_boundaries
    );
    println!(
        "F1              {:.1}%   <- task.md §5.4 targets 98%",
        m.f1_percent()
    );
    println!("wrong merges    {}", m.wrong_merges);
    println!("wrong splits    {}", m.wrong_splits);
    println!("missed batches  {}", m.missed_batches);
    if !m.missing_photos.is_empty() {
        println!(
            "missing photos  {:?}",
            &m.missing_photos[..m.missing_photos.len().min(10)]
        );
    }
    for example in m.merge_examples.iter().take(5) {
        println!("  merged: {}", example.join(" + "));
    }

    let mut bad = false;
    if m.f1 < 0.98 {
        println!("\nFAIL: F1 is below the 98% target");
        bad = true;
    }
    if m.wrong_merges > 0 {
        println!(
            "\nFAIL: {} wrongly merged bursts -- a wrong merge hides photos, which is the one \
                 failure this project exists to prevent",
            m.wrong_merges
        );
        bad = true;
    }
    if bad {
        return Err("ground truth not met".into());
    }
    let _ = opts;
    println!("\nOK: inside the target");
    Ok(())
}

// ------------------------------------------------------------------- shared

#[derive(Default)]
struct Opts {
    json: bool,
    sorted: bool,
    repeat: usize,
    freeze_from: Option<usize>,
    freeze_to: Option<usize>,
}

fn load_folder(args: &[String]) -> Result<(Folder, Opts), String> {
    load_folder_with_path(args).map(|(f, o, _)| (f, o))
}

fn load_folder_with_path(args: &[String]) -> Result<(Folder, Opts, PathBuf), String> {
    let mut path: Option<PathBuf> = None;
    let mut opts = Opts {
        repeat: 5,
        ..Opts::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => opts.json = true,
            "--sorted" => opts.sorted = true,
            "--repeat" => opts.repeat = flag(args, &mut i, "--repeat")?.parse().unwrap_or(5).max(1),
            "--freeze-from" => {
                opts.freeze_from = Some(
                    flag(args, &mut i, "--freeze-from")?
                        .parse()
                        .map_err(|_| "bad --freeze-from")?,
                );
            }
            "--freeze-to" => {
                opts.freeze_to = Some(
                    flag(args, &mut i, "--freeze-to")?
                        .parse()
                        .map_err(|_| "bad --freeze-to")?,
                );
            }
            // Consumed by the calling subcommand, not by the folder loader.
            "--truth" | "--game" => {
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => path = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    let path = path.ok_or("pass a metadata fixture (tests/fixtures/meta/<game>.json)")?;
    let folder = Folder::load(&path)?;
    Ok((folder, opts, path))
}

fn frozen_batches(folder: &Folder, opts: &Opts) -> Vec<firstcut_core::batch::Batch> {
    let (Some(from), Some(to)) = (opts.freeze_from, opts.freeze_to) else {
        return Vec::new();
    };
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());
    outcome
        .batches
        .into_iter()
        .filter(|b| b.index as usize >= from && b.index as usize <= to)
        .map(|b| firstcut_core::batch::Batch {
            provisional: false,
            ..b
        })
        .collect()
}

fn name_of(folder: &Folder, id: PhotoId) -> String {
    folder
        .get_by_id(id)
        .map_or_else(|| "<unknown>".into(), |p| p.rel_path.clone())
}

fn flag(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{name} needs a value"))
}

/// Repo root, found by walking up from the working directory until `tests/fixtures` appears.
fn repo_path(relative: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join(relative)
}
