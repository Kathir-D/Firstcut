//! `cargo xtask` — the workspace's own automation (docs/v2-plan.md §6, M0):
//! `layers` forbids upward/sideways crate edges, `ci` is the full milestone
//! gate, `smoke` proves the app opens a native window on the CI runners.

mod ci;
mod layers;
mod package;
mod smoke;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code: i32 = match args.first().map(String::as_str) {
        Some("ci") if args.iter().any(|a| a == "--summary-md") => ci::run(true),
        Some("ci") => ci::run(false),
        Some("layers") => layers::run().ok().map_or(1, |()| 0),
        Some("package") => package::run().map_or_else(
            |error| {
                eprintln!("package: {error}");
                1
            },
            |()| 0,
        ),
        Some("smoke") => smoke::run().ok().map_or(1, |()| 0),
        _ => {
            print_help();
            0
        }
    };
    std::process::ExitCode::from(code as u8)
}

fn print_help() {
    println!(
        "cargo xtask — the Firstcut workspace automation

TASKS:
    ci            full gate: fmt, clippy (-D warnings), layers, assets,
                  workspace tests; writes target/ci/report.json + summary.md
                  [--summary-md] also prints the PR-pasteable block
    layers        fail on any upward/sideways crate edge (xtask/src/layers.rs)
                  and on any lints-table override outside the LibRaw carve-out
    smoke         build apps/firstcut and run it with --smoke (window opens,
                  paints, exits 0); wraps in xvfb-run on headless Linux"
    );
}
