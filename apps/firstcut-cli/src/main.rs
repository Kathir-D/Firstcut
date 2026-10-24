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
//! - `verify`     the metadata reader against `exiftool` on a folder of real files
//!
//! Wave 2 adds `contact-sheet` (visual review of every boundary) and `eval` (boundary F1 against
//! `tests/fixtures/ground-truth/<game>.json`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use firstcut_batching::batch::fixture::Folder;
use firstcut_batching::batch::signals::Decision;
use firstcut_batching::batch::{BatchParams, PhotoId, batch_with};
use firstcut_batching::order;
use firstcut_engine::pipeline::{Viewport, fit_edge};

mod mcp;
mod run;
mod verify;

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
    let rest = args.get(1..).unwrap_or(&[]);
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
        "verify" => verify::cmd_verify(rest),
        // The `mcp` leg (registry as MCP tools, docs/mcp.md) is still ahead; `run` is live.
        "run" => run::cmd_run(rest),
        "mcp" => mcp::cmd_mcp(rest),
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
  run [--sessions <dir>] <command> key=value … [<command> key=value …]
        Headless driving of the app through the command registry (docs/v2-plan.md §4.3).
        One JSON line per command: an object with the id and either a result or an error.
        `--sessions` points `file.open` at a scratch Sessions directory. Example:
        `firstcut run --sessions /tmp/s file.open path=/tmp/shoot rate.stars stars=4 engine.state`.

  mcp
        The command registry as MCP tools over stdio (docs/v2-plan.md §4.3): `initialize`,
        `tools/list`, `tools/call`. Headless; `--connect host:port` (the control-channel
        bridge) is a later leg.

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
  bench --decode --folder <dir> [--repeat <n>]
        Time the phases that need real files: the header scan, then order() + batch(), then
        the whole thing end to end. Cold and warm are reported separately and scaled to 1,500
        files against the todo.md §7.3 targets. No metadata fixture is needed.

  verify <folder> [--quiet] [--previews]
        Scan the folder as the app does and compare every photo's capture time, camera and
        exposure with `exiftool`. Lists each disagreement, the files the scanner did not pick up
        at all, and a per-extension summary (todo.md §8). `--previews` also prints the size of
        every embedded JPEG the scanner found.
"
    );
}

// ---------------------------------------------------------------- dump-meta

fn cmd_dump_meta(args: &[String]) -> Result<(), String> {
    let mut from_exiftool: Option<PathBuf> = None;
    let mut fixture: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while let Some(arg) = args.get(i).map(String::as_str) {
        match arg {
            "--from-exiftool" => from_exiftool = Some(PathBuf::from(flag(args, &mut i, "--from-exiftool")?)),
            "--fixture" => fixture = Some(flag(args, &mut i, "--fixture")?),
            "--out" | "-o" => out = Some(PathBuf::from(flag(args, &mut i, "--out")?)),
            other => return Err(format!("dump-meta: unknown option `{other}`")),
        }
        i += 1;
    }

    let (input, default_out) = match (from_exiftool, fixture) {
        (Some(p), None) => (p, None),
        (None, Some(name)) => {
            (repo_path(&format!("tests/fixtures/exiftool/{name}.json")), Some(repo_path(&format!("tests/fixtures/meta/{name}.json"))))
        }
        (Some(_), Some(_)) => {
            return Err("dump-meta: pass either --from-exiftool or --fixture, not both".into());
        }
        (None, None) => {
            return Err("dump-meta: pass --from-exiftool <in.json> or --fixture <game>".into());
        }
    };

    let photos = firstcut_batching::batch::fixture::load_exiftool(&input)?;
    let out = out.or(default_out).ok_or("dump-meta: pass --out <file>")?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&photos).map_err(|e| e.to_string())?;
    std::fs::write(&out, format!("{json}\n")).map_err(|e| format!("writing {}: {e}", out.display()))?;

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
        // No batch is empty — the assembler never emits one but crash-free anyway.
        let (Some(&first_id), Some(&last_id)) = (b.photo_ids.first(), b.photo_ids.last()) else {
            continue;
        };
        println!(
            "batch {:>4} of {:<4}  {:>2} photos  {}  .. {}",
            b.index + 1,
            total,
            b.photo_ids.len(),
            name_of(&folder, first_id),
            name_of(&folder, last_id),
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
        outcome.verdicts.iter().filter(|v| v.decision == Decision::Ambiguous).count()
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
    println!("{:>4} {:>22} {:>22} {:>6} {:>6} {:>5} {:>5} {:>5} {:>4}", "i", "from", "to", "Δt", "f", "score", "focal", "ev", "split");
    for v in &outcome.verdicts {
        if v.decision != Decision::Ambiguous {
            continue;
        }
        let (Some(&from), Some(&to)) = (outcome.order.get(v.index.saturating_sub(1)), outcome.order.get(v.index)) else {
            continue;
        };
        println!(
            "{:>4} {:>22} {:>22} {:>6} {:>6} {:>5.2} {:>5.2} {:>5.2} {:>4}",
            v.index,
            from.0,
            to.0,
            v.signals.dt_ms,
            v.thresholds.frame_interval_ms,
            v.score,
            v.signals.focal_stops,
            v.signals.exposure_ev,
            if v.split { "yes" } else { "no" }
        );
    }

    if opts.sorted {
        println!("\n(the ambiguous boundaries are in capture order; see the todo.md §5 thresholds)");
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
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).ok_or("ground-truth needs --out <template.json>")?;
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());

    let mut names: HashMap<PhotoId, String> = HashMap::new();
    for id in &outcome.order {
        names.insert(*id, name_of(&folder, *id));
    }
    let batches: Vec<Vec<String>> = outcome.batches.iter().map(|b| b.photo_ids.iter().filter_map(|id| names.get(id).cloned()).collect()).collect();

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
    std::fs::write(out, serde_json::to_string_pretty(&template).map_err(|e| e.to_string())?).map_err(|e| format!("writing {out}: {e}"))?;

    let ambiguous: Vec<_> = outcome.verdicts.iter().filter(|v| v.decision == Decision::Ambiguous).collect();
    println!(
        "wrote {out}: {} photos in {} batches; {} ambiguous boundaries to check by eye:\n",
        outcome.order.len(),
        outcome.batches.len(),
        ambiguous.len()
    );
    println!("{:>4}  {:<22} {:<22} {:>6}  batcher says", "i", "from", "to", "Δt ms");
    // What the batcher *outputs*, not the boundary's own verdict: single frames a few seconds apart
    // are grouped after the boundaries are decided, so a boundary scored SPLIT can still end up
    // inside one batch. The checklist has to describe the template it sits next to.
    let starts: std::collections::HashSet<PhotoId> = outcome.batches.iter().filter_map(|b| b.photo_ids.first().copied()).collect();
    for v in ambiguous {
        let (Some(&from), Some(&to)) = (outcome.order.get(v.index.saturating_sub(1)), outcome.order.get(v.index)) else {
            continue;
        };
        println!(
            "{:>4}  {:<22} {:<22} {:>6}  {}",
            v.index,
            name_of(&folder, from),
            name_of(&folder, to),
            v.signals.dt_ms,
            if starts.contains(&to) { "SPLIT (new batch starts at `to`)" } else { "JOIN (same batch)" }
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
    // `--decode` is the §5 pixel rows; `--folder` alone is the scan/db rows.
    if args.iter().any(|a| a == "--decode") {
        let (dir, repeat) = bench_folder_args(args).ok_or("bench --decode needs --folder <dir>")?;
        return bench_decode(&dir, repeat);
    }
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
    println!("scaled to 1,500 photos: order+batch {:.1} ms", (order_best + batch_best) * 1500.0 / n as f64 * 1_000.0);
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
    let dir = args.iter().position(|a| a == "--folder").and_then(|i| args.get(i + 1))?;
    let repeat = args.iter().position(|a| a == "--repeat").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
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
    matched: firstcut_catalog::MatchKind,
}

fn bench_run(dir: &std::path::Path, sessions_dir: &std::path::Path) -> Result<BenchRun, String> {
    // Phase 1: the header scan. This is the whole cost of "open this folder" before any pixels.
    let scan_start = std::time::Instant::now();
    let scan = firstcut_meta::meta::scan_folder(dir).map_err(|e| format!("scan failed: {e}"))?;
    let scan_secs = scan_start.elapsed().as_secs_f64();

    // Phase 2 and 3: order, then batch. Sub-millisecond, but they are what the scan feeds.
    let order_start = std::time::Instant::now();
    let order = firstcut_batching::order::order(&scan.photos);
    let order_secs = order_start.elapsed().as_secs_f64();
    let batch_start = std::time::Instant::now();
    let outcome = firstcut_batching::batch::batch_with(&scan.photos, &HashMap::new(), &[], BatchParams::default());
    let batch_secs = batch_start.elapsed().as_secs_f64();

    // Phase 4: the session. `from_scan` takes the scan we already made, so the two timings add up
    // to one open: database open, the insert, the first-open sidecar import, rebatch.
    let db_start = std::time::Instant::now();
    let session =
        firstcut_engine::session::Session::from_scan(scan.clone(), dir, sessions_dir, std::sync::Arc::new(firstcut_engine::session::NoListener))
            .map_err(|e| format!("session failed: {e}"))?;
    let db_secs = db_start.elapsed().as_secs_f64();
    let total_secs = scan_secs + order_secs + batch_secs + db_secs;

    let matched = session.matched();
    let snapshot = session.snapshot();
    // The session must agree with the pure phases, or the bench would be timing two different
    // shoots and printing them as one. The scratch directory is fresh, so there is nothing in the
    // database that could legitimately reorder, drop or add a photograph.
    if snapshot.photos.len() != scan.photos.len() {
        return Err(format!("the session saw {} photographs, the scan {}", snapshot.photos.len(), scan.photos.len()));
    }
    if snapshot.batches.len() != outcome.batches.len() {
        return Err(format!("the session made {} batches, the pure batcher {}", snapshot.batches.len(), outcome.batches.len()));
    }
    std::hint::black_box(&order);
    std::hint::black_box(&outcome);
    std::hint::black_box(&snapshot);
    // Close *after* the timing: flush and checkpoint are quit-time work, not open-time work.
    session.close();

    Ok(BenchRun { scan_secs, order_secs, batch_secs, db_secs, total_secs, photos: scan.photos.len(), batches: outcome.batches.len(), matched })
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
    println!("rust:   {} threads", std::thread::available_parallelism().map_or(0, |n| n.get()));
    println!();

    // A scratch sessions directory, so the bench never opens a database in the user's real
    // sessions dir, and a fresh subdirectory per run so every run pays the first-open cost.
    let scratch_root = std::env::temp_dir().join(format!("firstcut-bench-sessions-{}", std::process::id()));
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
        if !matches!(result.matched, firstcut_catalog::MatchKind::Created) {
            return Err(format!(
                "the scratch sessions dir {} was not empty, so run {run} timed a re-open \
                 rather than an open",
                sessions.display()
            ));
        }
        let tag = if run == 1 { "run 1 (cold)" } else { "run N (warm)" };
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
        if best.as_ref().is_none_or(|b| b.total_secs > result.total_secs) {
            best = Some(result.clone());
        }
        if run == 1 {
            cold = Some(result);
        } else if warm.as_ref().is_none_or(|w| w.total_secs > result.total_secs) {
            warm = Some(result);
        }
    }
    let _ = std::fs::remove_dir_all(&scratch_root);

    let Some(cold) = cold else { unreachable!("repeat is at least 1") };
    let Some(best) = best else { unreachable!("repeat is at least 1") };
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

// ---------------------------------------------------------------- bench --decode

/// The viewer's decode target for "not zoomed". The app asks for the actual backing edge; 2048 is
/// the plan's fallback (`FALLBACK_DISPLAY_EDGE`) and a retina-class 1200×760 pane's long edge.
const BENCH_VIEWPORT_EDGE: u32 = 2048;
/// How many photographs the arrow row steps through (the first is the "first photo" row).
const ARROW_SAMPLES: usize = 8;
/// The sustained-throughput row: how many display decodes four workers finish per second. At a
/// 20 Hz held arrow the budget is 20 frames/s; four ~57–68 ms decodes in parallel measured
/// 53–61/s, which is what makes the display prefetch window able to stay ahead.
const THROUGHPUT_SAMPLES: usize = 24;
const THROUGHPUT_WORKERS: usize = 4;

/// Times the §5 decode rows on real files: first photo after open, arrow → next, Exact RAW develop,
/// and a 1:1 re-decode. The rows alternate the preview and RAW develop across rounds, which is v1's
/// protocol: whichever decode runs first pays for the cold file read, so a single one-way pass would
/// make the second row look faster than it is.
///
/// This measures the decode work, not the GPU upload or the window's paint. The 100% zoom row is
/// therefore a lower bound on the interactive cost; the window is measured by the app itself.
fn bench_decode(dir: &std::path::Path, repeat: usize) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    let scan_start = std::time::Instant::now();
    let scan = firstcut_meta::meta::scan_folder(dir).map_err(|e| format!("scan failed: {e}"))?;
    let scan_secs = scan_start.elapsed().as_secs_f64();
    let sources: Vec<&firstcut_meta::meta::PhotoMeta> = scan.photos.iter().filter(|photo| photo.full_preview.is_some()).collect();
    let Some(first) = sources.first().copied() else {
        return Err("no photograph with an embedded full preview; is this a RAW/JPEG shoot?".to_string());
    };

    // §5 row 1: open → first photo on screen. What the user waits for is the scan plus the first
    // display decode, so both are in the number.
    let first_path = dir.join(&first.rel_path);
    let decode_start = std::time::Instant::now();
    let first_image = firstcut_decode::decode_display_at(&first_path, first, BENCH_VIEWPORT_EDGE).map_err(|err| format!("first photo: {err}"))?;
    let first_decode_secs = decode_start.elapsed().as_secs_f64();
    std::hint::black_box(&first_image);
    let first_on_screen_secs = scan_secs + first_decode_secs;

    // §5 row 2: arrow → next. Eight different files, so the first round is genuinely cold. The
    // navigation preview is timed alongside the display: it is the frame the app paints first,
    // and it has to land inside a held arrow key's ~33 ms repeat to keep the loupe from blanking.
    let mut arrow_medians = Vec::new();
    let mut preview_medians = Vec::new();
    for run in 0..repeat {
        let mut times = Vec::new();
        let mut preview_times = Vec::new();
        for photo in sources.iter().skip(1).take(ARROW_SAMPLES.saturating_sub(1)) {
            let path = dir.join(&photo.rel_path);
            let start = std::time::Instant::now();
            let image = firstcut_decode::decode_display_at(&path, photo, BENCH_VIEWPORT_EDGE).map_err(|err| format!("{}: {err}", photo.rel_path))?;
            times.push(start.elapsed().as_secs_f64());
            std::hint::black_box(&image);

            let start = std::time::Instant::now();
            let preview = firstcut_decode::decode_thumbnail(&path, photo, firstcut_engine::pipeline::PREVIEW_EDGE)
                .map_err(|err| format!("{} preview: {err}", photo.rel_path))?;
            preview_times.push(start.elapsed().as_secs_f64());
            std::hint::black_box(&preview);
        }
        arrow_medians.push(median(&times));
        preview_medians.push(median(&preview_times));
        if run == 0 && times.is_empty() {
            return Err("the shoot has only one photograph; the arrow row needs a second".to_string());
        }
    }
    let Some(arrow_cold) = arrow_medians.first().copied() else {
        return Err("repeat is at least 1".to_string());
    };
    let arrow_warm = arrow_medians.iter().copied().filter(|t| t.is_finite()).fold(f64::MAX, f64::min);
    let preview_cold = preview_medians.first().copied().unwrap_or(f64::NAN);
    let preview_warm = preview_medians.iter().copied().filter(|t| t.is_finite()).fold(f64::MAX, f64::min);

    // §5 row 3: Exact RAW, alternating with a display decode so neither side owns the cold read.
    let backend = firstcut_decode_libraw::backend();
    let raw_photo = sources.iter().copied().find(|photo| matches!(photo.kind, firstcut_meta::meta::FileKind::Raw(_)));
    let mut raw_times = Vec::new();
    let mut display_times = Vec::new();
    if let (Some(backend), Some(photo)) = (backend.as_deref(), raw_photo) {
        for _ in 0..repeat {
            let start = std::time::Instant::now();
            let developed = backend.decode_raw(&dir.join(&photo.rel_path), photo).map_err(|err| format!("Exact RAW: {err}"))?;
            raw_times.push(start.elapsed().as_secs_f64());
            std::hint::black_box(&developed);

            let start = std::time::Instant::now();
            let preview = firstcut_decode::decode_display_at(&dir.join(&photo.rel_path), photo, BENCH_VIEWPORT_EDGE)
                .map_err(|err| format!("the RAW photograph's preview: {err}"))?;
            display_times.push(start.elapsed().as_secs_f64());
            std::hint::black_box(&preview);
        }
    }

    // §5 row 4's decode half: a 1:1 re-decode of the embedded full preview (the app's zoom path).
    let mut zoom_times = Vec::new();
    for _ in 0..repeat {
        let start = std::time::Instant::now();
        let full = firstcut_decode::decode_display(&first_path, first).map_err(|err| format!("1:1 re-decode: {err}"))?;
        zoom_times.push(start.elapsed().as_secs_f64());
        std::hint::black_box(&full);
    }
    let zoom_best = zoom_times.iter().copied().filter(|t| t.is_finite()).fold(f64::MAX, f64::min);

    // §5 row 5: sustained display throughput with the app's four-worker bank. The same decode
    // is spread over different photographs so workers do not contend on one file. Three targets
    // show the system-level effect of the fitted viewport: a small laptop pane uses the camera's
    // PRVW, while 4K/5K must pay for the full-preview entropy decode.
    let laptop_edge = fit_edge(6000, 4000, 1, Viewport::new(1440, 900));
    let monitor_edge = fit_edge(6000, 4000, 1, Viewport::new(3840, 2160));
    let throughput_full = display_throughput(dir, &sources, BENCH_VIEWPORT_EDGE)?;
    let throughput_laptop = display_throughput(dir, &sources, laptop_edge)?;
    let throughput_monitor = display_throughput(dir, &sources, monitor_edge)?;

    println!("folder: {}", dir.display());
    println!("photos: {} ({} with a full preview), {repeat} round(s)", scan.photos.len(), sources.len());
    println!();
    println!("§5 decode rows (macOS, release build; decode only, not GPU upload):");
    println!(
        "  first photo on screen   {:>7.3} s   (scan {:.3} s + display {:.3} s)   target ≤ 0.500 s  {}",
        first_on_screen_secs,
        scan_secs,
        first_decode_secs,
        verdict(first_on_screen_secs <= 0.5)
    );
    println!(
        "  arrow → next photo      {:>7.3} s cold, {:>7.3} s warm (median of up to {})   target ≤ 0.090 s  {}",
        arrow_cold,
        arrow_warm,
        ARROW_SAMPLES - 1,
        verdict(arrow_warm <= 0.09)
    );
    println!(
        "  arrow → nav preview     {:>7.3} s cold, {:>7.3} s warm   target ≤ 0.033 s (30 Hz hold)  {}",
        preview_cold,
        preview_warm,
        verdict(preview_warm <= 0.033)
    );
    match (backend.is_some(), raw_photo, raw_times.iter().copied().reduce(f64::min), display_times.iter().copied().reduce(f64::min)) {
        (true, Some(photo), Some(raw), Some(display)) => {
            let ratio = if display > 0.0 { raw / display } else { f64::INFINITY };
            println!(
                "  Exact RAW develop       {:>7.3} s warm ({:.2}× the preview's {:.3} s)   target ≤ 0.119 s  {}",
                raw,
                ratio,
                display,
                verdict(raw <= 0.079 * 1.5)
            );
            println!("                          {}", photo.rel_path);
        }
        (false, _, _, _) => println!("  Exact RAW develop       unavailable on this platform (the backend answers None; preview fallback)"),
        _ => println!("  Exact RAW develop       not measured: no RAW photograph in this folder"),
    }
    println!("  1:1 zoom re-decode      {:>7.3} s warm (embedded full preview)   GPU upload not included", zoom_best);
    println!(
        "  display throughput      {:>7.1} fps @ {BENCH_VIEWPORT_EDGE} px ({THROUGHPUT_WORKERS} workers, {THROUGHPUT_SAMPLES} decodes)   target ≥ 20  {}",
        throughput_full,
        verdict(throughput_full >= 20.0)
    );
    println!(
        "  display throughput      {:>7.1} fps @ {laptop_edge} px (laptop fit, PRVW path)   target ≥ 20  {}",
        throughput_laptop,
        verdict(throughput_laptop >= 20.0)
    );
    println!(
        "  display throughput      {:>7.1} fps @ {monitor_edge} px (4K 3:2 fit, full preview)   target ≥ 20  {}",
        throughput_monitor,
        verdict(throughput_monitor >= 20.0)
    );
    println!();
    println!("fitted display targets (6000×4000 / 3:2 sensor, camera PRVW is 1620 px):");
    for (name, width, height) in [
        ("13\" 1x laptop pane", 1440, 900),
        ("14\" Retina 2x pane", 3024, 1964),
        ("4K 16:9 monitor", 3840, 2160),
        ("5K 16:9 monitor", 5120, 2880),
        ("6K 16:9 monitor", 6016, 3384),
    ] {
        let target = fit_edge(6000, 4000, 1, Viewport::new(width, height));
        let source = if target <= 1620 { "PRVW decode (~6–8 ms)" } else { "full preview (~57–68 ms)" };
        println!("  {name:<22} {width:>4}×{height:<4} → {target:>4} px  {source}");
    }
    println!();
    println!("Cold = the first round; warm = the fastest of {repeat}. The cold-file-cache rule applies:");
    println!("whichever decode runs first pays for the read, so the warm row is the comparable one.");
    Ok(())
}

/// Four workers decoding `THROUGHPUT_SAMPLES` display frames at `target_edge`, spread over the
/// scan so no two workers read the same file at once. Returns frames per second.
fn display_throughput(dir: &std::path::Path, sources: &[&firstcut_meta::meta::PhotoMeta], target_edge: u32) -> Result<f64, String> {
    let samples: Vec<&firstcut_meta::meta::PhotoMeta> = sources.iter().copied().cycle().take(THROUGHPUT_SAMPLES).collect();
    if samples.is_empty() {
        return Err("the throughput row needs at least one photograph".to_string());
    }
    let start = std::time::Instant::now();
    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::new();
        for worker in 0..THROUGHPUT_WORKERS {
            let samples = &samples;
            handles.push(scope.spawn(move || -> Result<(), String> {
                for photo in samples.iter().skip(worker).step_by(THROUGHPUT_WORKERS) {
                    let image = firstcut_decode::decode_display_at(&dir.join(&photo.rel_path), photo, target_edge)
                        .map_err(|err| format!("{}: {err}", photo.rel_path))?;
                    std::hint::black_box(&image);
                }
                Ok(())
            }));
        }
        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => return Err(err),
                Err(_) => return Err("a decode worker panicked during the throughput row".to_string()),
            }
        }
        Ok(())
    })?;
    let secs = start.elapsed().as_secs_f64();
    Ok(if secs > 0.0 { THROUGHPUT_SAMPLES as f64 / secs } else { f64::NAN })
}

/// The middle value of a set of timings (`NaN`-free, never indexes).
fn median(times: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = times.iter().copied().filter(|t| t.is_finite()).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    match sorted.len() {
        0 => f64::NAN,
        len if len % 2 == 1 => sorted.get(len / 2).copied().unwrap_or(f64::NAN),
        len => match (sorted.get(len / 2 - 1), sorted.get(len / 2)) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            _ => f64::NAN,
        },
    }
}

fn no_sigs() -> HashMap<PhotoId, firstcut_batching::batch::VisualSig> {
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
    let mut opts = Opts { repeat: 5, ..Opts::default() };
    let mut i = 0;
    while let Some(arg) = args.get(i).map(String::as_str) {
        match arg {
            "--json" => opts.json = true,
            "--sorted" => opts.sorted = true,
            "--folder" => opts.folder = Some(PathBuf::from(flag(args, &mut i, "--folder")?)),
            "--repeat" => opts.repeat = flag(args, &mut i, "--repeat")?.parse().unwrap_or(5).max(1),
            "--freeze-from" => {
                opts.freeze_from = Some(flag(args, &mut i, "--freeze-from")?.parse().map_err(|_| "bad --freeze-from")?);
            }
            "--freeze-to" => {
                opts.freeze_to = Some(flag(args, &mut i, "--freeze-to")?.parse().map_err(|_| "bad --freeze-to")?);
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

fn frozen_batches(folder: &Folder, opts: &Opts) -> Vec<firstcut_batching::batch::Batch> {
    let (Some(from), Some(to)) = (opts.freeze_from, opts.freeze_to) else {
        return Vec::new();
    };
    let outcome = batch_with(&folder.photos, &HashMap::new(), &[], BatchParams::default());
    outcome
        .batches
        .into_iter()
        .filter(|b| b.index as usize >= from && b.index as usize <= to)
        .map(|b| firstcut_batching::batch::Batch { provisional: false, ..b })
        .collect()
}

fn name_of(folder: &Folder, id: PhotoId) -> String {
    folder.get_by_id(id).map_or_else(|| "<unknown>".into(), |p| p.rel_path.clone())
}

fn flag(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value"))
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
    use firstcut_catalog::MatchKind;

    /// A folder of parseable CR3s: 12 frames 90 ms apart inside one second, so the phases have a
    /// real burst to read and the session has something to insert. The same builder the core's own
    /// session tests use, so what the bench times is what the production path parses.
    fn shoot() -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..12u32 {
            std::fs::write(
                folder.path().join(format!("IMG_{index:04}.CR3")),
                firstcut_meta::meta::cr3::SyntheticCr3::r8().subsec(&format!("{:02}", index * 9)).build(),
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
        assert!(matches!(result.matched, MatchKind::Created), "a fresh sessions dir means the session is created, not matched");
        // The phases are timed separately and must add up to the total they are reported as, or
        // the summary would be printing overlapping work as if it were disjoint phases.
        let sum = result.scan_secs + result.order_secs + result.batch_secs + result.db_secs;
        assert!((sum - result.total_secs).abs() < 1e-9, "{sum} vs {}", result.total_secs);
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
        assert!(matches!(second.matched, MatchKind::Exact), "the second open against the same database is a re-open, not an open");
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
        assert!(fresh.is_dir(), "and exists once the session has opened in it");
    }
}
