//! The workspace layer check (docs/v2-plan.md §4.1): reads `cargo metadata`
//! for the members and fails on any dependency edge that goes upward or
//! sideways in the crate ladder. The allowed edges live in exactly one
//! file — this one — not in prose.
//!
//! This task also enforces the unsafe carve-out rule (§4.5): exactly one crate
//! in the workspace may carry its own `[lints.rust]` table
//! (`firstcut-decode-libraw`, the approved LibRaw C++ bindings), because the
//! workspace forbids `unsafe_code` everywhere else and an override table is
//! the only way to re-allow it.

use std::process::Command;

use serde::Deserialize;

/// The ladder, top to bottom. A crate may depend only on crates ABOVE it;
/// the apps may depend on anything, and the exact edges they currently use
/// are listed so a new dep on something unconnected fails the check.
const LADDER: &[(&str, &[&str])] = &[
    ("firstcut-types", &[]),
    ("firstcut-meta", &["firstcut-types"]),
    ("firstcut-xmp", &["firstcut-types"]),
    ("firstcut-fileops", &["firstcut-types", "firstcut-meta", "firstcut-xmp"]),
    ("firstcut-batching", &["firstcut-types", "firstcut-meta"]),
    ("firstcut-catalog", &["firstcut-types"]),
    ("firstcut-decode", &["firstcut-types", "firstcut-meta"]),
    // Backend crates attach ABOVE `firstcut-decode` and implement its traits,
    // never baked in: the engine and the apps wire them in (docs/v2-plan.md §4.1).
    ("firstcut-decode-libraw", &["firstcut-types", "firstcut-meta", "firstcut-decode"]),
    (
        "firstcut-engine",
        &["firstcut-types", "firstcut-meta", "firstcut-xmp", "firstcut-fileops", "firstcut-batching", "firstcut-catalog", "firstcut-decode"],
    ),
    ("firstcut-ui", &["firstcut-engine", "firstcut-types"]),
    (
        "firstcut",
        &[
            "firstcut-types",
            "firstcut-meta",
            "firstcut-xmp",
            "firstcut-fileops",
            "firstcut-batching",
            "firstcut-catalog",
            "firstcut-decode",
            "firstcut-decode-libraw",
            "firstcut-engine",
            "firstcut-ui",
        ],
    ),
    (
        "firstcut-cli",
        &[
            "firstcut-types",
            "firstcut-meta",
            "firstcut-xmp",
            "firstcut-fileops",
            "firstcut-batching",
            "firstcut-catalog",
            "firstcut-decode",
            "firstcut-decode-libraw",
            "firstcut-engine",
        ],
    ),
    ("xtask", &[]),
];

/// The one crate allowed to re-enable `unsafe_code` (docs/v2-plan.md §4.5, D3).
const UNSAFE_CARVE_OUT: &str = "firstcut-decode-libraw";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
}
#[derive(Deserialize)]
struct MetadataPackage {
    name: String,
    dependencies: Vec<MetadataDependency>,
}
#[derive(Deserialize)]
struct MetadataDependency {
    name: String,
    optional: bool,
}

pub fn run() -> Result<(), String> {
    let metadata = cargo_metadata()?;

    let member_names: Vec<&str> = metadata.packages.iter().map(|p| p.name.as_str()).collect();
    let ladder_names: Vec<&str> = LADDER.iter().map(|(n, _)| *n).collect();

    let mut errors = Vec::new();
    for name in &member_names {
        if !ladder_names.contains(name) {
            errors.push(format!("unknown to the ladder: {name} (add it to xtask/src/layers.rs, in order)"));
        }
    }

    for package in &metadata.packages {
        let Some(allowed_deps) = LADDER.iter().find(|(n, _)| n == &package.name).map(|(_, d)| *d) else { continue };
        for dep in &package.dependencies {
            // Only workspace members are layered; external crates are not.
            if !ladder_names.contains(&dep.name.as_str()) || dep.optional {
                continue;
            }
            if !allowed_deps.contains(&dep.name.as_str()) {
                errors.push(format!("{} → {}: upward or sideways edge", package.name, dep.name));
            }
        }
    }

    check_unsafe_carve_out_is_unique(&mut errors);

    if errors.is_empty() {
        println!("layers: {} workspace edges match the ladder", member_names.len());
        Ok(())
    } else {
        errors.sort();
        for e in &errors {
            eprintln!("layers: {e}");
        }
        Err(format!("layers: {} crate edge problem{} (see above)", errors.len(), if errors.len() == 1 { "" } else { "s" }))
    }
}

fn cargo_metadata() -> Result<Metadata, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("failed to run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let parsed: Metadata = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata unparsable: {e}"))?;
    Ok(parsed)
}

/// Every workspace crate must inherit `[lints] workspace = true`; its own
/// `[lints.rust]` table is how a crate escapes the workspace `unsafe_code`
/// forbid, so exactly one table is permitted (the documented carve-out).
fn check_unsafe_carve_out_is_unique(errors: &mut Vec<String>) {
    for (name, _) in LADDER.iter() {
        let base = match *name {
            "xtask" => "xtask".to_string(),
            // the app binaries live under apps/
            "firstcut" | "firstcut-cli" => format!("apps/{name}"),
            _ => format!("crates/{name}"),
        };
        let manifest_path = format!("{base}/Cargo.toml");
        if let Ok(manifest) = std::fs::read_to_string(&manifest_path) {
            check_manifest(name, &manifest, errors);
        } else {
            errors.push(format!("{name}: manifest not found at {manifest_path}"));
        }
    }
}

fn check_manifest(name: &str, manifest: &str, errors: &mut Vec<String>) {
    if manifest.lines().any(|l| l.trim_start().starts_with("[lints.rust]")) && name != UNSAFE_CARVE_OUT {
        errors.push(format!("{name}: carries its own [lints.rust] table — only {UNSAFE_CARVE_OUT} may, the unsafe carve-out"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder must declare names uniquely and only allow edges to crates
    /// ABOVE the depender — this asserts the spec file itself, so a
    /// mistyped layer table fails here rather than shipping a broken check.
    #[test]
    fn ladder_is_topologically_consistent() {
        let mut seen = Vec::new();
        for (name, allowed) in LADDER.iter() {
            assert!(!seen.contains(name), "ladder declares {name} twice");
            for dep in allowed.iter() {
                assert!(seen.contains(dep), "ladder allows {name} -> {dep} but {dep} is not above {name} in the list");
            }
            seen.push(*name);
        }
        assert!(seen.contains(&UNSAFE_CARVE_OUT), "the unsafe carve-out crate must be in the ladder");
    }

    #[test]
    fn every_app_is_in_the_ladder() {
        for member in ["firstcut", "firstcut-cli"] {
            assert!(LADDER.iter().any(|(n, _)| *n == member), "the app {member} is missing from the layer table; add it so its edges are checked");
        }
    }

    #[test]
    fn a_manifests_other_lints_table_is_reported() {
        let mut errors = Vec::new();
        check_manifest("firstcut-types", "[lints.rust]\nunused = \"allow\"", &mut errors);
        assert_eq!(errors.len(), 1, "any own [lints.rust] table outside the carve-out must fail the check");

        errors.clear();
        check_manifest(UNSAFE_CARVE_OUT, "[lints.rust]\nunsafe_code = \"allow\"", &mut errors);
        assert!(errors.is_empty(), "the documented carve-out crate is allowed its override");
    }
}
