//! `cargo xtask smoke`: build `apps/firstcut` and run it with `--smoke` — the
//! window opens, paints a few frames, and exits 0. This is where cross-OS
//! window creation is proven: the 3-OS CI matrix runs this task
//! (docs/v2-plan.md §6, M0). On Linux without a display the binary is wrapped
//! in `xvfb-run -a` when available.

use std::process::Command;

pub fn run() -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let build = Command::new(cargo).args(["build", "-p", "firstcut"]).status().map_err(|e| format!("could not run cargo build: {e}"))?;
    if !build.success() {
        return Err("cargo build -p firstcut failed".to_string());
    }

    let exe = target_binary("firstcut")?;
    let bin = std::path::Path::new(&exe);
    if !bin.exists() {
        return Err(format!("smoke: built binary missing at {}", bin.display()));
    }

    let status = launch(bin)?;
    if status.success() {
        println!("smoke: window opened, painted and closed cleanly ({})", bin.display());
        Ok(())
    } else {
        Err(format!("smoke: window app exited {}", status))
    }
}

fn launch(bin: &std::path::Path) -> Result<std::process::ExitStatus, String> {
    let needs_xvfb = cfg!(target_os = "linux") && std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none();
    if needs_xvfb {
        let mut xvfb = Command::new("xvfb-run");
        xvfb.arg("-a").arg(bin).arg("--smoke");
        xvfb.status()
            .map_err(|_| "smoke: no DISPLAY on Linux and xvfb-run is not installed (apt-get install xvfb libgl1 mesa-vulkan-drivers)".to_string())
    } else {
        Command::new(bin).arg("--smoke").status().map_err(|e| format!("could not run {}: {e}", bin.display()))
    }
}

fn target_binary(name: &str) -> Result<String, String> {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let ext = if cfg!(windows) { ".exe" } else { "" };
    Ok(format!("target/{profile}/{name}{ext}"))
}
