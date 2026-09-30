# Changelog

All notable changes to Firstcut are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/).

## [0.1.0] - unreleased

The first release.

### Added

- Open a folder of photos, split it into bursts automatically, and cull batch by batch from the
  keyboard, with Lightroom-style shortcuts that can all be remapped.
- Burst batching from capture time (10 ms resolution), shutter count, orientation, focal length,
  exposure, and a perceptual hash of each frame, computed in the background and used to settle the
  boundaries that timing alone cannot decide. Single frames a few seconds apart are grouped.
- Two rating modes: Stars, and Keep / Not keep with green and red rings.
- Loupe, Grid and Compare views (2-, 3- and 4-up, with synchronized zoom). Pinch or click to zoom to
  100%, drag or scroll to pan, and zoom lock to compare a burst on the same detail.
- Autofocus point overlay, highlight and shadow clipping overlay, histogram, and a Lightroom-style
  info panel whose fields you can choose.
- Ratings are saved to a session database as you go and mirrored to `.xmp` sidecars that Lightroom
  and Capture One read. Undo and redo, resume where you left off, and import of ratings already in
  sidecars when a folder is opened for the first time.
- Finish Cull: move, trash, delete or mark the photos you did not keep, and copy, move or split the
  ones you did, with a dry-run preview, a typed confirmation for permanent deletion, and Undo Finish.
- A Settings window (General, Keyboard, Viewer, Metadata, Performance) with a shortcut editor.
- A Welcome window with recent folders and how far through each one you got.
- The folder is watched: new files appear and deleted files drop out while you cull.
- Metadata for JPEG, HEIC, PNG, TIFF and the TIFF-based RAW formats, and never skipping a photo that
  macOS can open.
- Homebrew cask and GitHub release, ad-hoc signed.

### Known limitations

See [Known limitations](README.md#known-limitations) in the README.
