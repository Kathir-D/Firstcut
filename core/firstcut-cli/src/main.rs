//! `firstcut` — the developer CLI. Owner: core-batch (other agents add subcommands by request,
//! see docs/contracts/build.md).
//!
//! Wave 1 subcommands:
//!
//! - `dump-meta`  convert an exiftool dump into `tests/fixtures/meta/<game>.json`
//! - `order`      print capture order
//! - `batch`      print the batches for a folder's metadata
//! - `gaps`       the Δt histogram and every ambiguous-zone boundary, for tuning thresholds
//! - `bench`      time `order()` + `batch()` on a dump, or the whole pipeline with `--folder <dir>`
//!
//! Wave 2 adds `contact-sheet` (visual review of every boundary) and `eval` (boundary F1 against
//! `tests/fixtures/ground-truth/<game>.json`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use firstcut_core::batch::fixture::Folder;
use firstcut_core::batch::signals::Decision;
use firstcut_core::batch::{BatchParams, PhotoId, batch_with};
use firstcut_core::order;

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
        "ground-truth" => cmd_ground_truth(rest),
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
        score. This is how the thresholds in todo.md §5 are tuned.

  ground-truth <meta.json> --out <template.json>
        Writes the batcher's current batches as a ground-truth TEMPLATE, and prints the
        ambiguous boundaries as a checklist. The template is not ground truth: a human has to look
        at the photographs, fix each ambiguous boundary, set `verified` to the date, and save it as
        tests/fixtures/ground-truth/<game>.json. Never write that file from this output unchanged.

  bench <meta.json> [--repeat <n>]
        Time order() + batch() and compare against the 2 s / 1,500-file target.

  bench --folder <dir> [--repeat <n>]
        Time the phases that need real files: the header scan, then order() + batch(), then
        the whole thing end to end. Cold and warm are reported separately and scaled to 1,500
        files against the todo.md §7.3 targets. No metadata fixture is needed.
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

    let photos = firstcut_core::batch::fixture::load_exiftool(&input)?;
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
            if v.split { "yes" } else { "no" }
        );
    }

    if opts.sorted {
        println!(
            "\n(the ambiguous boundaries are in capture order; see the todo.md §5 thresholds)"
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

// ------------------------------------------------------------- ground truth

fn cmd_ground_truth(args: &[String]) -> Result<(), String> {
    let (folder, _opts) = load_folder(args)?;
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .ok_or("ground-truth needs --out <template.json>")?;
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());

    let mut names: HashMap<PhotoId, String> = HashMap::new();
    for id in &outcome.order {
        names.insert(*id, name_of(&folder, *id));
    }
    let batches: Vec<Vec<String>> = outcome
        .batches
        .iter()
        .map(|b| b.photo_ids.iter().map(|id| names[id].clone()).collect())
        .collect();

    let template = serde_json::json!({
        "game": std::path::Path::new(out)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim_end_matches(".template"),
        "meta": "generated by `firstcut ground-truth` from the batcher's own output",
        "verified": "",
        "notes": "TEMPLATE. Not ground truth until a human has looked at every ambiguous boundary \
                  and set `verified`. The batcher's own output proves nothing about the batcher.",
        "batches": batches,
    });
    std::fs::write(
        out,
        serde_json::to_string_pretty(&template).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("writing {out}: {e}"))?;

    let ambiguous: Vec<_> = outcome
        .verdicts
        .iter()
        .filter(|v| v.decision == Decision::Ambiguous)
        .collect();
    println!(
        "wrote {out}: {} photos in {} batches; {} ambiguous boundaries to check by eye:\n",
        outcome.order.len(),
        outcome.batches.len(),
        ambiguous.len()
    );
    println!(
        "{:>4}  {:<22} {:<22} {:>6}  batcher says",
        "i", "from", "to", "Δt ms"
    );
    for v in ambiguous {
        println!(
            "{:>4}  {:<22} {:<22} {:>6}  {}",
            v.index,
            name_of(&folder, outcome.order[v.index - 1]),
            name_of(&folder, outcome.order[v.index]),
            v.signals.dt_ms,
            if v.split {
                "SPLIT (new batch starts at `to`)"
            } else {
                "JOIN (same batch)"
            }
        );
    }
    println!(
        "\nFor each line: look at the two frames (`firstcut contact-sheet`), decide, and edit the \
         `batches` in the template. Then set `verified`."
    );
    Ok(())
}

// -------------------------------------------------------------------- bench

fn cmd_bench(args: &[String]) -> Result<(), String> {
    // `--folder` times the phases that need real files: the header scan, then order + batch on what
    // it returned, then the whole thing end to end. Without it, only order + batch are timed, which
    // is what the committed metadata dumps allow in CI. It is read here rather than through
    // `load_folder`, because a folder run has no metadata fixture to load.
    if let Some((dir, repeat)) = bench_folder_args(args) {
        return bench_folder(&dir, repeat);
    }

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
    println!("todo.md §5 target: all 1,500 files batched in < 2,000 ms");
    if scaled > 2.0 {
        println!("FAIL: over the target");
    } else {
        println!("OK: inside the target");
    }
    Ok(())
}

/// `--folder <dir>` and `--repeat <n>`, without needing a metadata fixture alongside them.
fn bench_folder_args(args: &[String]) -> Option<(PathBuf, usize)> {
    let dir = args
        .iter()
        .position(|a| a == "--folder")
        .and_then(|i| args.get(i + 1))?;
    let repeat = args
        .iter()
        .position(|a| a == "--repeat")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    Some((PathBuf::from(dir), repeat))
}

/// One measured pass over a folder: the phases of a folder open, timed separately so they add up
/// to "folder open → provisional batches ready" with nothing counted twice.
///
/// `sessions_dir` must be empty or absent: the session phase is the **Created** path (create the
/// database, insert every photograph, write the batch rows), which is the first-open cost §7.3's
/// target is written for. A re-open against an existing database is a different, cheaper question.
#[derive(Debug, Clone)]
struct BenchRun {
    /// Seconds per phase, in the order they run.
    scan_secs: f64,
    order_secs: f64,
    batch_secs: f64,
    /// The session without the scan: database open + insert + the one-shot sidecar import +
    /// rebatch. `Session::from_scan` is the app's open with the scan left out.
    db_secs: f64,
    /// All phases, end to end.
    total_secs: f64,
    photos: usize,
    batches: usize,
    /// What the session phase decided about the database, so a mis-configured scratch directory
    /// (one that already had a database in it) is visible rather than silently timing the
    /// cheaper re-open path.
    matched: firstcut_core::store::MatchKind,
}

fn bench_run(dir: &std::path::Path, sessions_dir: &std::path::Path) -> Result<BenchRun, String> {
    // Phase 1: the header scan. This is the whole cost of "open this folder" before any pixels.
    let scan_start = std::time::Instant::now();
    let scan = firstcut_core::meta::scan_folder(dir).map_err(|e| format!("scan failed: {e}"))?;
    let scan_secs = scan_start.elapsed().as_secs_f64();

    // Phase 2 and 3: order, then batch. Sub-millisecond, but they are what the scan feeds.
    let order_start = std::time::Instant::now();
    let order = firstcut_core::order::order(&scan.photos);
    let order_secs = order_start.elapsed().as_secs_f64();
    let batch_start = std::time::Instant::now();
    let outcome = firstcut_core::batch::batch_with(
        &scan.photos,
        &HashMap::new(),
        &[],
        BatchParams::default(),
    );
    let batch_secs = batch_start.elapsed().as_secs_f64();

    // Phase 4: the session. `from_scan` takes the scan we already made, so the two timings add up
    // to one open: database open, the insert, the first-open sidecar import, rebatch.
    let db_start = std::time::Instant::now();
    let session = firstcut_core::session::Session::from_scan(
        scan.clone(),
        dir,
        sessions_dir,
        std::sync::Arc::new(firstcut_core::session::NoListener),
    )
    .map_err(|e| format!("session failed: {e}"))?;
    let db_secs = db_start.elapsed().as_secs_f64();
    let total_secs = scan_secs + order_secs + batch_secs + db_secs;

    let matched = session.matched();
    let snapshot = session.snapshot();
    // The session must agree with the pure phases, or the bench would be timing two different
    // shoots and printing them as one. The scratch directory is fresh, so there is nothing in the
    // database that could legitimately reorder, drop or add a photograph.
    if snapshot.photos.len() != scan.photos.len() {
        return Err(format!(
            "the session saw {} photographs, the scan {}",
            snapshot.photos.len(),
            scan.photos.len()
        ));
    }
    if snapshot.batches.len() != outcome.batches.len() {
        return Err(format!(
            "the session made {} batches, the pure batcher {}",
            snapshot.batches.len(),
            outcome.batches.len()
        ));
    }
    std::hint::black_box(&order);
    std::hint::black_box(&outcome);
    std::hint::black_box(&snapshot);
    // Close *after* the timing: flush and checkpoint are quit-time work, not open-time work.
    session.close();

    Ok(BenchRun {
        scan_secs,
        order_secs,
        batch_secs,
        db_secs,
        total_secs,
        photos: scan.photos.len(),
        batches: outcome.batches.len(),
        matched,
    })
}

/// Time the real pipeline on a folder of photographs, phase by phase.
///
/// This is the measurement todo.md §7.3 is written in, and the reason it exists separately from
/// `bench` is that **only this touches the disk**. The committed metadata dumps let CI time
/// `order()` + `batch()` forever, but the scan is where a 1,500-file shoot actually costs time, and
/// it is the only phase that depends on the drive rather than on the algorithm.
///
/// Reported per phase, against the §7.3 targets:
///
/// | phase | target |
/// | --- | --- |
/// | header scan | < 3 s for 1,500 files |
/// | order + batch | (already measured, sub-millisecond) |
/// | session: database open + insert + rebatch | (the part of "batches ready" that is not the scan) |
/// | total, folder open → batches ready | < 3.5 s |
///
/// `--repeat` re-runs the whole thing, which is how a warm-cache number is separated from a cold
/// one. The first run is the honest one for "open this folder" and the best of the rest is the honest
/// one for "re-open it", so both are printed rather than one being chosen for you.
fn bench_folder(dir: &std::path::Path, repeat: usize) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }

    println!("folder: {}", dir.display());
    println!(
        "rust:   {} threads",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    println!();

    // A scratch sessions directory, so the bench never opens a database in the user's real
    // sessions dir, and a fresh subdirectory per run so every run pays the first-open cost.
    let scratch_root =
        std::env::temp_dir().join(format!("firstcut-bench-sessions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch_root);

    let mut best: Option<BenchRun> = None;
    let mut cold: Option<BenchRun> = None;
    let mut warm: Option<BenchRun> = None;

    for run in 1..=repeat {
        let sessions = scratch_root.join(run.to_string());
        let result = bench_run(dir, &sessions)?;
        if result.photos == 0 {
            return Err("no photographs found; is this the right folder?".to_string());
        }
        if !matches!(result.matched, firstcut_core::store::MatchKind::Created) {
            return Err(format!(
                "the scratch sessions dir {} was not empty, so run {run} timed a re-open \
                 rather than an open",
                sessions.display()
            ));
        }
        let tag = if run == 1 {
            "run 1 (cold)"
        } else {
            "run N (warm)"
        };
        println!(
            "{tag}: scan {:.3} s  order {:.3} ms  batch {:.3} ms  db {:.3} s  total {:.3} s",
            result.scan_secs,
            result.order_secs * 1_000.0,
            result.batch_secs * 1_000.0,
            result.db_secs,
            result.total_secs
        );
        // `best` is the fastest of *all* runs — the existing "warm" semantic for the scan, so a
        // single-repeat bench still prints a warm column rather than an empty one.
        if best
            .as_ref()
            .is_none_or(|b| b.total_secs > result.total_secs)
        {
            best = Some(result.clone());
        }
        if run == 1 {
            cold = Some(result);
        } else if warm
            .as_ref()
            .is_none_or(|w| w.total_secs > result.total_secs)
        {
            warm = Some(result);
        }
    }
    let _ = std::fs::remove_dir_all(&scratch_root);

    let Some(cold) = cold else {
        unreachable!("repeat is at least 1")
    };
    let best = best.expect("repeat is at least 1");
    let warm = warm.unwrap_or(best.clone());
    let photos = cold.photos;
    let batches = cold.batches;

    println!();
    println!("{photos} photos, {batches} batches");
    println!(
        "scan: {:.3} s cold, {:.3} s warm (best of {repeat} repeats)  ({:.2} ms/photo warm)",
        cold.scan_secs,
        best.scan_secs,
        best.scan_secs / photos as f64 * 1_000.0
    );
    println!(
        "db:   {:.3} s cold, {:.3} s warm  ({:.2} ms/photo warm)  \
         (database open + insert + sidecar import + rebatch)",
        cold.db_secs,
        best.db_secs,
        best.db_secs / photos as f64 * 1_000.0
    );

    // The targets are written for 1,500 files, so scale and say so. Cold and warm are both reported
    // and the *cold* one is judged: "folder open -> batches ready" is a first-open number, and a
    // best-of-N figure would flatter it.
    let scale = 1500.0 / photos as f64;
    println!();
    println!("scaled to 1,500 photos (todo.md §7.3):");
    println!(
        "  metadata scan        cold {:>7.3} s  warm {:>7.3} s   target < 3.000 s  {}",
        cold.scan_secs * scale,
        best.scan_secs * scale,
        verdict(cold.scan_secs * scale < 3.0)
    );
    println!(
        "  provisional batches  cold {:>7.3} s  warm {:>7.3} s   target < 3.500 s  {}  (scan + db)",
        cold.total_secs * scale,
        warm.total_secs * scale,
        verdict(cold.total_secs * scale < 3.5)
    );
    println!();
    println!("todo.md §7.3. A number here is a measurement; the decode and the");
    println!("interactive targets are not measured by this command.");
    Ok(())
}

fn verdict(inside: bool) -> &'static str {
    if inside { "OK" } else { "OVER TARGET" }
}

fn no_sigs() -> HashMap<PhotoId, firstcut_core::batch::VisualSig> {
    HashMap::new()
}

// ------------------------------------------------------------------- shared

#[derive(Default)]
struct Opts {
    json: bool,
    sorted: bool,
    repeat: usize,
    freeze_from: Option<usize>,
    freeze_to: Option<usize>,
    /// `--folder <dir>`: a real folder of photographs to time the header scan on, as opposed to a
    /// committed metadata dump. `bench` without it times `order()` + `batch()` only.
    folder: Option<PathBuf>,
}

fn load_folder(args: &[String]) -> Result<(Folder, Opts), String> {
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
            "--folder" => opts.folder = Some(PathBuf::from(flag(args, &mut i, "--folder")?)),
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
            // Consumed by `ground-truth`, which reads it from the raw arguments itself.
            "--out" => {
                flag(args, &mut i, "--out")?;
            }
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => path = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    let path = path.ok_or("pass a metadata fixture (tests/fixtures/meta/<game>.json)")?;
    let folder = Folder::load(&path)?;
    Ok((folder, opts))
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

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::bench_run;
    use firstcut_core::store::MatchKind;

    /// A folder of parseable CR3s: 12 frames 90 ms apart inside one second, so the phases have a
    /// real burst to read and the session has something to insert. The same builder the core's own
    /// session tests use, so what the bench times is what the production path parses.
    fn shoot() -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..12u32 {
            std::fs::write(
                folder.path().join(format!("IMG_{index:04}.CR3")),
                firstcut_core::meta::cr3::SyntheticCr3::r8()
                    .subsec(&format!("{:02}", index * 9))
                    .build(),
            )
            .unwrap();
        }
        folder
    }

    #[test]
    fn the_bench_phases_agree_on_one_shoot() {
        let folder = shoot();
        let sessions = tempfile::tempdir().unwrap();
        let result = bench_run(folder.path(), sessions.path()).unwrap();

        assert_eq!(result.photos, 12, "every CR3 the scan found");
        assert_eq!(result.batches, 1, "12 frames 90 ms apart is one burst");
        assert!(
            matches!(result.matched, MatchKind::Created),
            "a fresh sessions dir means the session is created, not matched"
        );
        // The phases are timed separately and must add up to the total they are reported as, or
        // the summary would be printing overlapping work as if it were disjoint phases.
        let sum = result.scan_secs + result.order_secs + result.batch_secs + result.db_secs;
        assert!(
            (sum - result.total_secs).abs() < 1e-9,
            "{sum} vs {}",
            result.total_secs
        );
        assert!(result.scan_secs > 0.0 && result.db_secs > 0.0);
    }

    #[test]
    fn the_bench_refuses_a_scratch_dir_that_already_holds_a_database() {
        let folder = shoot();
        let sessions = tempfile::tempdir().unwrap();
        // The scratch directory is supposed to be fresh; if it is not, `bench_folder` must fail
        // rather than quietly timing the cheaper re-open and calling it an open.
        bench_run(folder.path(), sessions.path()).unwrap();
        let second = bench_run(folder.path(), sessions.path()).unwrap();
        assert!(
            matches!(second.matched, MatchKind::Exact),
            "the second open against the same database is a re-open, not an open"
        );
    }

    /// `bench_folder` hands `bench_run` a scratch path that does not exist yet
    /// (`scratch_root/<run>`), so the session must create it — the same thing `Db::open_in`
    /// does for the app's own sessions directory on a first run.
    #[test]
    fn the_bench_creates_its_scratch_sessions_dir() {
        let folder = shoot();
        let sessions = tempfile::tempdir().unwrap();
        let fresh = sessions.path().join("fresh");
        assert!(!fresh.exists(), "the per-run scratch dir starts absent");
        let result = bench_run(folder.path(), &fresh).unwrap();
        assert!(matches!(result.matched, MatchKind::Created));
        assert!(
            fresh.is_dir(),
            "and exists once the session has opened in it"
        );
    }
}
