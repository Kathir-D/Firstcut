# Packaging

`cargo xtask package` builds the release app **on the host it runs on** and wraps it in the
installer that host expects. It never cross-compiles: the 3-OS release workflow runs the same
command on macOS, Windows and Linux and attaches the results to one Release.

```sh
cargo xtask package                       # this OS, workspace version (e.g. 0.2.0-dev)
FIRSTCUT_VERSION=0.2.0 cargo xtask package # a release build with a clean version
```

Every artifact gets a sibling `.sha256` and a `SHA256SUMS-firstcut-<version>-<os>-<arch>` file.
The release workflow uploads the checksums with the installers; nothing is signed by a secret
that lives in this repository.

## Artifacts

| Host | Files |
| --- | --- |
| macOS | `Firstcut.app` inside `firstcut-<version>-macos-<arch>.zip`, plus a per-arch Homebrew cask (`*.cask.rb`) and its `sha256` |
| Windows | `firstcut-<version>-windows-x64.zip` containing `firstcut.exe`, the `.ico`, `LICENSE.txt` and `README.md` |
| Linux | `firstcut-<version>-linux-x86_64.AppImage` when `appimagetool` is installed; `firstcut-<version>-linux-x86_64.deb` when `dpkg-deb` is installed; otherwise a `.tar.gz` of the AppDir |

`Firstcut.app` carries `NSDocumentsFolderUsageDescription`, `NSDesktopFolderUsageDescription`
and `NSDownloadsFolderUsageDescription`. Without them macOS refuses to show the document-folder
prompt, and the native picker that M5 added cannot grant access to the shoot.

## Signing (optional)

- **macOS**: ad-hoc (`codesign --sign -`) by default, which is enough for a local run. A real
  Developer ID sets `FIRSTCUT_CODESIGN_IDENTITY="Developer ID Application: …"`; notarisation is
  the owner's release step.
- **Windows**: the zip is unsigned unless `FIRSTCUT_SIGNTOOL` (e.g. `signtool.exe`) and
  `FIRSTCUT_SIGN_CERT` are set. Windows shows its one-time SmartScreen warning for an unsigned
  download, which is expected for a v2 build without a certificate.
- **Linux**: AppImage and `.deb` are unsigned; checksums are the integrity record.

## Linux floor

The `.deb` depends on `libc6 (>= 2.35)` (Ubuntu 22.04), plus `libxkbcommon0`,
`libwayland-client0`, `libgl1` and `libvulkan1`. The AppImage bundles the app but not the
graphics driver; an old machine without a Vulkan driver can run it with
`FIRSTCUT_WGPU_BACKEND=gl firstcut` (or the AppImage) and wgpu takes the GL path.

## Cask stamp

The macOS run prints the version, artifact name and checksum and writes a per-arch cask:

```sh
FIRSTCUT_VERSION=2.0.0 FIRSTCUT_RELEASE_TAG=v2.0.0 cargo xtask package
# target/package/firstcut-2.0.0-macos-arm64.cask.rb
```

Both architectures are built by running the task on each Mac; the two casks differ only in
`depends_on arch` and the artifact name.

## Release workflow

`.github/workflows/v2-release.yml` reacts to an owner-pushed `v2*` (or `v0.2*`) tag, or a
manual run. It runs the 3-OS package matrix, uploads each OS's artifacts, and on a tag attaches
them to the GitHub Release. Tags and Releases are the owner's; agents do not create them.
