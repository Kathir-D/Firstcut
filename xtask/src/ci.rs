//! `cargo xtask ci` (docs/v2-plan.md §6, M0): the full gate — fmt, clippy
//! (deny warnings), workspace tests, the layer check and the asset-attribution
//! check. Writes `target/ci/report.json` and `target/ci/summary.md` (a
//! PR-pasteable block); nothing is uploaded.
//!
//! Picks build jobs and test threads from the machine's CPUs and RAM so a
//! busy or small machine never OOMs (one build job per ~1.5 GB of available
//! RAM, at most one per CPU; test threads capped at 4). Explicit
//! `CARGO_BUILD_JOBS` and `RUST_TEST_THREADS` override these defaults and are
//! honoured, not replaced.

use std::{
    process::{Command, Stdio},
    time::Instant,
};

pub struct Step {
    name: &'static str,
    ok: bool,
    duration_s: f64,
    message: String,
}

pub fn run(summary_md_to_stdout: bool) -> i32 {
    let started = Instant::now();
    let mut machine = machine();
    machine.set_default_job_env();
    println!(
        "ci: machine = {} — build jobs {}, test threads {}",
        machine.summary,
        machine.build_jobs.map(|j| j.to_string()).unwrap_or("?".to_string()),
        machine.test_threads.map(|t| t.to_string()).unwrap_or("?".to_string())
    );

    let steps = vec![
        shell_step("fmt", machine.cargo_cmd().args(["fmt", "--all", "--check"])),
        shell_step("clippy", machine.cargo_cmd().args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])),
        fn_step("layers", crate::layers::run),
        fn_step("assets", check_assets),
        shell_step("test", machine.cargo_cmd().args(["test", "--workspace"])),
    ];

    let failed: Vec<&Step> = steps.iter().filter(|s| !s.ok).collect();
    let result = if failed.is_empty() { "pass" } else { "fail" };
    let total_s = started.elapsed().as_secs_f64();

    write_report(&machine, &steps, total_s, &failed);

    if let Some(md) = summary_markdown(&machine, &steps, total_s, &failed) {
        let path = std::path::Path::new("target/ci/summary.md");
        if let Err(e) = std::fs::write(path, md.clone()) {
            eprintln!("ci: could not write {}: {e}", path.display());
        }
        if summary_md_to_stdout {
            println!("{md}");
        }
    }
    println!("ci: result = {result} in {total_s:.1}s (target/ci/report.json, target/ci/summary.md)");
    if failed.is_empty() { 0 } else { 1 }
}

// ── steps ────────────────────────────────────────────────────────────────────

fn fn_step(name: &'static str, f: fn() -> Result<(), String>) -> Step {
    let started = Instant::now();
    match f() {
        Ok(()) => Step { name, ok: true, duration_s: started.elapsed().as_secs_f64(), message: "ok".to_string() },
        Err(msg) => Step { name, ok: false, duration_s: started.elapsed().as_secs_f64(), message: msg },
    }
}

fn shell_step(name: &'static str, cmd: &mut Command) -> Step {
    let started = Instant::now();
    let (ok, message) = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
        Ok(out) => {
            let ok = out.status.success();
            // Both streams matter here: clippy and rustc errors go to stderr, cargo test's
            // `failures:` report (test names + panic messages) goes to stdout. A failing step
            // echoes both, so the next reader never has to re-run CI to see what broke.
            let text = if ok {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
            };
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let tail = lines.last_page(250).join("\n");
            (ok, tail)
        }
        Err(e) => (false, format!("could not run: {e}")),
    };
    Step { name, ok, duration_s: started.elapsed().as_secs_f64(), message }
}

trait LastPage {
    fn last_page(self, n: usize) -> Self;
}
impl LastPage for Vec<&str> {
    /// recent lines, like `tail`
    fn last_page(mut self, n: usize) -> Self {
        let len = self.len();
        if len > n {
            self.drain(0..len - n);
        }
        self
    }
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

// ── machine + adaptive jobs ─────────────────────────────────────────────────

pub struct Machine {
    pub summary: String,
    pub cpus: u32,
    pub ram_gib: f64,
    pub build_jobs: Option<u32>,
    pub test_threads: Option<u32>,
}

impl Machine {
    fn describe(cpus: u32, ram_gib: f64) -> Self {
        Self { summary: format!("{cpus} CPUs, {ram_gib:.1} GiB RAM"), cpus, ram_gib, build_jobs: None, test_threads: None }
    }
}

fn machine() -> Machine {
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
    let ram_bytes = read_ram_bytes().filter(|b| *b > 0);
    let ram_gib = ram_bytes.map_or(4.0, |b| b as f64 / (1024.0 * 1024.0 * 1024.0));
    Machine::describe(cpus, ram_gib)
}

/// Best-effort physical RAM in bytes; `None` means "could not read".
fn read_ram_bytes() -> Option<u64> {
    let out;
    if cfg!(target_os = "macos") {
        out = Command::new("sysctl").arg("-n").arg("hw.memsize").output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else if cfg!(target_os = "linux") {
        let s = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = s.lines().find(|l| l.starts_with("MemTotal "))?;
        line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
    } else if cfg!(windows) {
        out = Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        None
    }
}

impl Machine {
    /// Compute the job/thread defaults unless the operator set them explicitly.
    /// One build job per ~1.5 GB of RAM, at most one per CPU; test threads
    /// capped at 4. Runs at the start of `ci` and records what applies for the
    /// report; [`Machine::apply`] passes the resolved values to every spawned
    /// cargo command (setting process env vars is `unsafe` in edition 2024 and
    /// forbidden by the workspace lints — the explicit env pass-on is cleaner
    /// anyway). Operator settings are honoured, never replaced.
    fn set_default_job_env(&mut self) {
        if let Some(explicit) = std::env::var("CARGO_BUILD_JOBS").ok().and_then(|s| s.parse().ok()) {
            self.build_jobs = Some(explicit);
        } else {
            let by_ram = (self.ram_gib / 1.5).max(1.0) as u32;
            self.build_jobs = Some(self.cpus.min(by_ram));
        }
        if let Some(explicit) = std::env::var("RUST_TEST_THREADS").ok().and_then(|s| s.parse().ok()) {
            self.test_threads = Some(explicit);
        } else {
            self.test_threads = Some(self.cpus.min(4));
        }
    }

    /// Hand the resolved limits to a spawned cargo command.
    fn apply(&self, cmd: &mut Command) {
        if let Some(jobs) = self.build_jobs {
            cmd.env("CARGO_BUILD_JOBS", jobs.to_string());
        }
        if let Some(threads) = self.test_threads {
            cmd.env("RUST_TEST_THREADS", threads.to_string());
        }
    }

    fn cargo_cmd(&self) -> Command {
        let mut cmd = Command::new(cargo());
        self.apply(&mut cmd);
        cmd
    }
}

// ── asset attribution ───────────────────────────────────────────────────────

/// Every file under `assets/` other than ATTRIBUTION.md itself must be named in
/// ATTRIBUTION.md (docs/v2-plan.md §4.5: no proprietary assets ever).
fn check_assets() -> Result<(), String> {
    let path = std::path::Path::new("assets");
    if !path.exists() {
        return Ok(());
    }
    let attribution = std::fs::read_to_string(path.join("ATTRIBUTION.md")).map_err(|e| format!("assets: could not read ATTRIBUTION.md: {e}"))?;
    let mut unattributed = Vec::new();
    collect_assets(path, path, &attribution, &mut unattributed);
    if unattributed.is_empty() {
        Ok(())
    } else {
        Err(format!("assets: {} file(s) without an ATTRIBUTION.md entry: {}", unattributed.len(), unattributed.join(", ")))
    }
}

fn collect_assets(root: &std::path::Path, dir: &std::path::Path, attribution: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_assets(root, &p, attribution, out);
        } else if p.file_name().is_some_and(|n| n != "ATTRIBUTION.md")
            && let Some(rel) = p.strip_prefix(root).ok().and_then(|r| r.to_str())
        {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !attribution.contains(&name) {
                out.push(rel.to_string());
            }
        }
    }
}

// ── report ──────────────────────────────────────────────────────────────────

fn step_json(machine: &Machine, steps: &[Step], total_s: f64) -> serde_json::Value {
    let skipped = serde_json::json!([{
        "name": "real-photo corpus integration tests",
        "reason": "opt-in protocol (FIRSTCUT_ALLOW_PHOTO_TESTS=1); milestone-only, never in the CI gate (docs/v2-plan.md §9)"
    }]);
    serde_json::json!({
        "result": if steps.iter().all(|s| s.ok) { "pass" } else { "fail" },
        "total_seconds": total_s,
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpus": machine.cpus,
            "ram_gib": machine.ram_gib,
            "build_jobs": machine.build_jobs,
            "test_threads": machine.test_threads,
        },
        "steps": steps.iter().map(|s| serde_json::json!({
            "name": s.name,
            "ok": s.ok,
            "seconds": s.duration_s,
            "message": s.message,
        })).collect::<Vec<_>>(),
        "skips": skipped,
    })
}

fn write_report(machine: &Machine, steps: &[Step], total_s: f64, failed: &[&Step]) {
    let json = step_json(machine, steps, total_s);
    let dir = std::path::Path::new("target/ci");
    if std::fs::create_dir_all(dir).is_err() {
        eprintln!("ci: could not create target/ci/ — report skipped");
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&json)
        && let Err(e) = std::fs::write(dir.join("report.json"), text)
    {
        eprintln!("ci: could not write report.json: {e}");
    }
    if failed.is_empty() {
        println!("ci: all steps green");
    } else {
        for f in failed {
            eprintln!("ci: step failed — {} ({}s)\n{}", f.name, f.duration_s, f.message);
        }
    }
}

fn summary_markdown(machine: &Machine, steps: &[Step], total_s: f64, failed: &[&Step]) -> Option<String> {
    let mut md = format!(
        "## `cargo xtask ci` — {}\n\n- OS: {} ({}) — {} CPUs, {:.1} GiB RAM; build jobs {:?}, test threads {:?}\n- total: {:.1}s\n\n| step | result | s | note |\n| --- | --- | --- | --- |\n",
        if failed.is_empty() { "pass" } else { "fail" },
        std::env::consts::OS,
        std::env::consts::ARCH,
        machine.cpus,
        machine.ram_gib,
        machine.build_jobs,
        machine.test_threads,
        total_s,
    );
    for s in steps {
        let note = if s.message.len() > 60 { "see target/ci/report.json".to_string() } else { s.message.clone() };
        md += &format!("| {} | {} | {:.1} | {} |\n", s.name, if s.ok { "green" } else { "FAIL" }, s.duration_s, note);
    }
    md += "\nSkipped (stated, never silent): real-photo corpus is opt-in (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`), milestone-only by the tiered-test rule.\n";
    Some(md)
}
