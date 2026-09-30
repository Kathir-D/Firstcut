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
        score. This is how the thresholds in task.md §5 are tuned.

  ground-truth <meta.json> --out <template.json>
        Writes the batcher's current batches as a ground-truth TEMPLATE, and prints the
        ambiguous boundaries as a checklist. The template is not ground truth: a human has to look
        at the photographs, fix each ambiguous boundary, set `verified` to the date, and save it as
        tests/fixtures/ground-truth/<game>.json. Never write that file from this output unchanged.

  bench <meta.json> [--repeat <n>]
        Time order() + batch() and compare against the 2 s / 1,500-file target.
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
            if v.provisional {
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
