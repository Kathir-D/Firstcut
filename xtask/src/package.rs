//! `cargo xtask package`: build the release binary and wrap it in the artifact the
//! installer for this OS expects (docs/v2-plan.md §6, M6).
//!
//! The task runs on the host it is invoked on and produces the host's artifact:
//!
//! | host | artifact |
//! | --- | --- |
//! | macOS | `Firstcut.app` + `firstcut-<version>-macos-<arch>.zip` + a per-arch cask |
//! | Windows | `firstcut-<version>-windows-<arch>.zip` |
//! | Linux | AppImage (when `appimagetool` is present), `.deb` (when `dpkg-deb` is), and a `.tar.gz` |
//!
//! Every file gets a sibling `.sha256`. The release workflow uploads the directory;
//! tags and releases stay the owner's (AGENTS.md).
//!
//! Signing is deliberately optional: ad-hoc on macOS, untouched on Windows. A
//! release that has a Developer ID / Authenticode certificate sets
//! `FIRSTCUT_CODESIGN_IDENTITY` and the same task signs without a code change.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const APP_NAME: &str = "Firstcut";
const BINARY: &str = "firstcut";
const BUNDLE_ID: &str = "dev.firstcut.app";
const MIN_MACOS: &str = "12.0";

pub fn run() -> Result<(), String> {
    let version = version();
    let display_version = version.split('-').next().unwrap_or(version.as_str());
    let base = format!("firstcut-{version}-{}-{}", os_slug(), arch_slug());
    let out = PathBuf::from("target/package");
    std::fs::create_dir_all(&out).map_err(|e| format!("could not create {}: {e}", out.display()))?;

    build()?;
    let binary = target_binary()?;

    let artifacts = match std::env::consts::OS {
        "macos" => package_macos(&binary, &out, &base, display_version)?,
        "windows" => package_windows(&binary, &out, &base)?,
        "linux" => package_linux(&binary, &out, &base, &version)?,
        other => return Err(format!("packaging is not implemented for `{other}`")),
    };

    let mut sums = String::new();
    for artifact in &artifacts {
        let sum = sha256_file(artifact)?;
        let name = artifact.file_name().and_then(|name| name.to_str()).ok_or_else(|| format!("non-UTF-8 artifact name: {}", artifact.display()))?;
        let sidecar = artifact.with_file_name(format!("{name}.sha256"));
        std::fs::write(&sidecar, format!("{sum}  {name}\n")).map_err(|e| format!("could not write {}: {e}", sidecar.display()))?;
        let _ = writeln!(sums, "{sum}  {name}");
        println!("package: {} ({})", artifact.display(), human_size(artifact));
    }
    let sums_path = out.join(format!("SHA256SUMS-{}", base));
    std::fs::write(&sums_path, sums).map_err(|e| format!("could not write {}: {e}", sums_path.display()))?;
    println!("package: wrote {}", sums_path.display());

    if cfg!(target_os = "macos") {
        println!("package: cask stamp: version={display_version} artifact={base}.zip");
    }
    Ok(())
}

fn version() -> String {
    std::env::var("FIRSTCUT_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

fn os_slug() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn arch_slug() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    }
}

fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

fn target_binary() -> Result<PathBuf, String> {
    let name = if cfg!(windows) { format!("{BINARY}.exe") } else { BINARY.to_string() };
    let path = Path::new("target").join("release").join(name);
    if path.is_file() { Ok(path) } else { Err(format!("package: release binary missing at {}", path.display())) }
}

fn build() -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    run_cmd(&cargo, &["build", "--release", "-p", "firstcut"], None)
}

fn run_cmd(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let status = command.status().map_err(|e| format!("could not run `{program}`: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("`{program} {}` exited {status}", args.join(" "))) }
}

fn package_macos(binary: &Path, out: &Path, base: &str, display_version: &str) -> Result<Vec<PathBuf>, String> {
    let app = out.join(format!("{APP_NAME}.app"));
    let _ = std::fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    copy(binary, &macos.join(BINARY))?;
    copy(Path::new("assets/branding/firstcut.icns"), &resources.join("AppIcon.icns"))?;
    copy(Path::new("LICENSE"), &resources.join("LICENSE.txt"))?;
    copy(Path::new("README.md"), &resources.join("README.md"))?;
    std::fs::write(contents.join("Info.plist"), info_plist(display_version)).map_err(|e| format!("could not write Info.plist: {e}"))?;

    // Ad-hoc signing is what makes Gatekeeper's first launch coherent on a machine that
    // downloads the zip. A Developer ID, when present, turns the same call into a
    // notarizable signature.
    let identity = std::env::var("FIRSTCUT_CODESIGN_IDENTITY").unwrap_or_else(|_| "-".to_string());
    if std::env::var_os("FIRSTCUT_SKIP_CODESIGN").is_none()
        && let Err(error) = run_cmd("codesign", &["--force", "--deep", "--sign", &identity, app.to_str().unwrap_or("")], None)
    {
        eprintln!("package: ad-hoc codesign skipped ({error}); the app still runs locally");
    }

    let zip = out.join(format!("{base}.zip"));
    let _ = std::fs::remove_file(&zip);
    run_cmd("ditto", &["-c", "-k", "--sequesterRsrc", "--keepParent", app.to_str().unwrap_or(""), zip.to_str().unwrap_or("")], None)?;

    let cask_path = out.join(format!("{base}.cask.rb"));
    std::fs::write(&cask_path, cask(display_version, base, &sha256_file(&zip)?))
        .map_err(|e| format!("could not write {}: {e}", cask_path.display()))?;
    Ok(vec![zip, cask_path])
}

fn info_plist(display_version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>{BINARY}</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>{APP_NAME}</string>
    <key>CFBundleDisplayName</key><string>{APP_NAME}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{display_version}</string>
    <key>CFBundleVersion</key><string>{display_version}</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.photography</string>
    <key>LSMinimumSystemVersion</key><string>{MIN_MACOS}</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
    <!-- M5: a native folder picker is the grant TCC records. Without these strings macOS
         refuses to show the prompt and the app silently loses the folder. -->
    <key>NSDocumentsFolderUsageDescription</key><string>Firstcut opens the shoot folder you choose so it can cull the photographs inside.</string>
    <key>NSDesktopFolderUsageDescription</key><string>Firstcut opens the shoot folder you choose so it can cull the photographs inside.</string>
    <key>NSDownloadsFolderUsageDescription</key><string>Firstcut opens the shoot folder you choose so it can cull the photographs inside.</string>
</dict>
</plist>
"#
    )
}

fn cask(version: &str, base: &str, sha: &str) -> String {
    let arch = if cfg!(target_arch = "aarch64") { "arm" } else { "intel" };
    let tag = std::env::var("FIRSTCUT_RELEASE_TAG").unwrap_or_else(|_| format!("v{version}"));
    format!(
        r#"cask "firstcut" do
  version "{version}"
  sha256 "{sha}"

  url "https://github.com/Kathir-D/Firstcut/releases/download/{tag}/{base}.zip"
  name "Firstcut"
  desc "Cull a shoot without leaving the keyboard"
  homepage "https://github.com/Kathir-D/Firstcut"

  depends_on macos: ">= :monterey"
  depends_on arch: :{arch}

  app "Firstcut.app"
end
"#
    )
}

fn package_windows(binary: &Path, out: &Path, base: &str) -> Result<Vec<PathBuf>, String> {
    let dir = out.join(base);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    copy(binary, &dir.join(format!("{BINARY}.exe")))?;
    copy(Path::new("assets/branding/firstcut.ico"), &dir.join("Firstcut.ico"))?;
    copy(Path::new("LICENSE"), &dir.join("LICENSE.txt"))?;
    copy(Path::new("README.md"), &dir.join("README.md"))?;

    let zip = out.join(format!("{base}.zip"));
    let _ = std::fs::remove_file(&zip);
    // Windows ships bsdtar; `-a` picks zip from the extension.
    run_cmd("tar", &["-a", "-c", "-f", zip.to_str().unwrap_or(""), "-C", dir.to_str().unwrap_or(""), "."], None)?;

    // Optional Authenticode: the certificate stays out of the repo, in the release
    // environment. Without it the zip is simply unsigned, which Windows warns about once.
    if let (Ok(tool), Ok(cert)) = (std::env::var("FIRSTCUT_SIGNTOOL"), std::env::var("FIRSTCUT_SIGN_CERT")) {
        let exe = dir.join(format!("{BINARY}.exe"));
        if let Err(error) = run_cmd(&tool, &["sign", "/fd", "SHA256", "/f", &cert, exe.to_str().unwrap_or("")], None) {
            eprintln!("package: Authenticode signing skipped ({error})");
        }
    }
    Ok(vec![zip])
}

fn package_linux(binary: &Path, out: &Path, base: &str, version: &str) -> Result<Vec<PathBuf>, String> {
    let appdir = out.join(format!("{APP_NAME}.AppDir"));
    let _ = std::fs::remove_dir_all(&appdir);
    let bin_dir = appdir.join("usr/bin");
    let desktop_dir = appdir.join("usr/share/applications");
    let icon_dir = appdir.join("usr/share/icons/hicolor/512x512/apps");
    for dir in [&bin_dir, &desktop_dir, &icon_dir] {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    copy(binary, &bin_dir.join(BINARY))?;
    copy(Path::new("assets/branding/firstcut-512.png"), &icon_dir.join("firstcut.png"))?;
    copy(Path::new("assets/branding/firstcut-512.png"), &appdir.join("firstcut.png"))?;
    copy(Path::new("LICENSE"), &appdir.join("LICENSE.txt"))?;
    std::fs::write(desktop_dir.join("firstcut.desktop"), desktop_entry()).map_err(|e| format!("could not write .desktop: {e}"))?;
    // appimagetool reads the desktop file from the AppDir root, not only from the installed
    // applications directory; without this it aborts with "Desktop file not found".
    std::fs::write(appdir.join("firstcut.desktop"), desktop_entry()).map_err(|e| format!("could not write the AppDir .desktop: {e}"))?;
    let app_run = appdir.join("AppRun");
    std::fs::write(&app_run, "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/firstcut\" \"$@\"\n")
        .map_err(|e| format!("could not write AppRun: {e}"))?;
    make_executable(&app_run)?;

    let mut artifacts = Vec::new();

    let appimage = out.join(format!("{base}.AppImage"));
    let _ = std::fs::remove_file(&appimage);
    let arch_env = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    if command_exists("appimagetool") {
        let appdir_s = appdir.to_str().unwrap_or("");
        let out_s = appimage.to_str().unwrap_or("");
        let first = Command::new("appimagetool").args([appdir_s, out_s]).env("ARCH", arch_env).status();
        let ok = first.is_ok_and(|status| status.success());
        if !ok {
            // The downloaded appimagetool is itself an AppImage and needs FUSE or the
            // extract-and-run flag; retry the portable form before giving up.
            run_cmd("appimagetool", &["--appimage-extract-and-run", appdir_s, out_s], None)?;
        }
        artifacts.push(appimage);
    } else {
        eprintln!("package: appimagetool not found; writing a .tar.gz of the AppDir instead");
        let tarball = out.join(format!("{base}.tar.gz"));
        let _ = std::fs::remove_file(&tarball);
        run_cmd("tar", &["-czf", tarball.to_str().unwrap_or(""), "-C", out.to_str().unwrap_or(""), &format!("{APP_NAME}.AppDir")], None)?;
        artifacts.push(tarball);
    }

    if command_exists("dpkg-deb") {
        let deb = out.join(format!("{base}.deb"));
        let _ = std::fs::remove_file(&deb);
        let root = out.join("debroot");
        let _ = std::fs::remove_dir_all(&root);
        for dir in [root.join("DEBIAN"), root.join("usr/bin"), root.join("usr/share/applications"), root.join("usr/share/icons/hicolor/512x512/apps")]
        {
            std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        copy(binary, &root.join("usr/bin").join(BINARY))?;
        copy(Path::new("assets/branding/firstcut-512.png"), &root.join("usr/share/icons/hicolor/512x512/apps/firstcut.png"))?;
        std::fs::write(root.join("usr/share/applications/firstcut.desktop"), desktop_entry())
            .map_err(|e| format!("could not write .desktop: {e}"))?;
        std::fs::write(root.join("DEBIAN/control"), deb_control(version)).map_err(|e| format!("could not write control: {e}"))?;
        run_cmd("dpkg-deb", &["--build", "--root-owner-group", root.to_str().unwrap_or(""), deb.to_str().unwrap_or("")], None)?;
        artifacts.push(deb);
    } else {
        eprintln!("package: dpkg-deb not found; the .deb is built on the Linux release runner");
    }

    Ok(artifacts)
}

fn desktop_entry() -> &'static str {
    "[Desktop Entry]\nName=Firstcut\nComment=Cull a shoot without leaving the keyboard\nExec=firstcut %F\nIcon=firstcut\nTerminal=false\nType=Application\nCategories=Graphics;Photography;\nMimeType=image/x-canon-cr3;image/x-canon-cr2;image/jpeg;image/tiff;\n"
}

fn deb_control(version: &str) -> String {
    format!(
        "Package: firstcut\nVersion: {version}\nSection: graphics\nPriority: optional\nArchitecture: {}\nMaintainer: Kathir Dev <kathir@users.noreply.github.com>\nDepends: libc6 (>= 2.35), libxkbcommon0, libwayland-client0, libgl1, libvulkan1\nDescription: Cull a shoot without leaving the keyboard\n Firstcut opens one folder, orders the frames into bursts, and lets a\n photographer rate, reject and finish the shoot with the keyboard.\n",
        deb_arch()
    )
}

fn command_exists(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok() || Command::new("which").arg(name).output().is_ok_and(|output| output.status.success())
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map_err(|e| format!("could not copy {} to {}: {e}", from.display(), to.display()))?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path).map_err(|e| format!("could not stat {}: {e}", path.display()))?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).map_err(|e| format!("could not chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("could not hash {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).ok_or_else(|| "package: short read while hashing".to_string())?;
        hasher.update(chunk);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn human_size(path: &Path) -> String {
    let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / (1 << 10) as f64)
    } else {
        format!("{bytes} B")
    }
}
