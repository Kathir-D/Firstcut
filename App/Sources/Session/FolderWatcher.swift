// Owner: app-logic.
//
// Watches the session folder with FSEvents (task.md §11): new files appear, deleted ones drop out.
// FSEvents rather than a directory file descriptor because a card dump has subfolders
// (`DCIM/100CANON`) and only FSEvents reports what happens inside them.
//
// Events are coalesced by FSEvents itself (the stream latency) and filtered here to changes to
// *photo* files. Firstcut writes `.xmp` sidecars as you rate, and reacting to its own writes would
// rescan the folder on every keypress.

import CoreServices
import Foundation

@MainActor
final class FolderWatcher {
    /// What counts as a photo for the purposes of "did the shoot change". Mirrors the core's
    /// `IMAGE_EXTENSIONS`; a photo type missing here only means a delayed refresh, never a wrong one.
    static let photoExtensions: Set<String> = [
        "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "gpr", "heic", "heif", "iiq",
        "jpeg", "jpg", "kdc", "mef", "mos", "nef", "nrw", "orf", "pef", "png", "raf", "rw2", "rwl",
        "sr2", "srf", "srw", "tif", "tiff", "x3f",
    ]

    /// True when one of `paths` is a photo file.
    static func touchesPhotos(_ paths: [String]) -> Bool {
        paths.contains { path in
            photoExtensions.contains((path as NSString).pathExtension.lowercased())
        }
    }

    private let folder: URL
    private let onChange: @MainActor () -> Void
    private var stream: FSEventStreamRef?

    init(folder: URL, onChange: @escaping @MainActor () -> Void) {
        self.folder = folder
        self.onChange = onChange
    }

    func start() {
        guard stream == nil else { return }
        var context = FSEventStreamContext(
            version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil)
        let flags = UInt32(
            kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagFileEvents
                | kFSEventStreamCreateFlagNoDefer)
        guard
            let created = FSEventStreamCreate(
                nil, folderWatcherCallback, &context, [folder.path] as CFArray,
                FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
                // Latency: a copy of 1,500 files is one refresh, not fifteen hundred.
                1.5, flags)
        else { return }
        // The main queue, so the callback runs where the model lives.
        FSEventStreamSetDispatchQueue(created, DispatchQueue.main)
        FSEventStreamStart(created)
        stream = created
    }

    func stop() {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        self.stream = nil
    }

    fileprivate func handle(_ paths: [String]) {
        guard Self.touchesPhotos(paths) else { return }
        onChange()
    }
}

/// The C callback. It captures nothing, which is what lets it be a plain function pointer; the
/// watcher comes back through `info`. The stream is scheduled on the main queue, so this is already
/// on the main actor.
private let folderWatcherCallback: FSEventStreamCallback = { _, info, count, eventPaths, _, _ in
    guard let info else { return }
    let watcher = Unmanaged<FolderWatcher>.fromOpaque(info).takeUnretainedValue()
    let paths = (unsafeBitCast(eventPaths, to: NSArray.self) as? [String]) ?? []
    MainActor.assumeIsolated { watcher.handle(paths) }
}
