# Contributing to Firstcut

Thank you for wanting to help. Firstcut is a small project with one rule that matters more than the
rest: **it never modifies a photo, and it never loses a rating**. Anything that touches the session
database, sidecars, or the Finish step needs a test that proves it.

## Before you start

Open an issue for anything bigger than a small fix, so the direction can be agreed first. The plan,
the decisions already made, and what is still open are in [`todo.md`](todo.md); the module contracts
are in [`docs/contracts/`](docs/contracts/).

## Building

```sh
git clone https://github.com/Kathir-D/Firstcut.git && cd Firstcut
scripts/build-app.sh          # Rust core -> Swift bindings -> Xcode project -> dist/Firstcut.app
```

Needs Apple Silicon, Xcode, Rust (via rustup) and `brew install xcodegen`.

## Checks

These are exactly what CI runs; please run them before you push.

```sh
cargo fmt --manifest-path core/Cargo.toml --all --check
cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path core/Cargo.toml          # also runs on Linux

scripts/generate-project.sh
xcodebuild -project Firstcut.xcodeproj -scheme Firstcut \
  -destination 'platform=macOS,arch=arm64' test
```

The Rust core builds and tests on Linux, which is handy for the parsing, batching and file-operation
work. The Swift app needs a Mac.

## Tests that use real photos

Never commit a photo. Tests that need real files look in `FIRSTCUT_TEST_PHOTOS` and skip themselves
when it is not set. Because the app is ad-hoc signed, macOS asks for folder access again after every
rebuild; real-photo tests are therefore opt-in (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`,
`scripts/test-with-photos.sh`).

## Batching changes

Batching changes are judged against a hand-checked answer key in `tests/fixtures/ground-truth/`, not
against the batcher's own output. If you change a threshold, say what you looked at.

## Style

Match the code around you. Comments explain *why*, and the bug a rule exists to prevent, rather than
restating the code. Commit messages say what changed and why, in plain prose.

## License

By contributing you agree that your work is released under the
[GPL-3.0](LICENSE), like the rest of the project.
