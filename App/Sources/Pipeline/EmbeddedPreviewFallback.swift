import Foundation

/// The camera's own JPEG for each RAW, for the files ImageIO cannot open by itself.
///
/// macOS decodes most RAW formats, but not all of them: on the raw.pixls.us sample, Phase One IIQ,
/// Leaf MOS, Sigma X3F, Canon CRW, older Olympus ORF, Kodak KDC and Samsung NX SRW come back from
/// ImageIO as nothing (todo.md §8). The core reports the embedded JPEG of every non-CR3 RAW as
/// `PhotoMeta.preview`, and `DecodeEngine`'s decoders ask here **only after ImageIO has failed**, so
/// a file ImageIO reads is never shown from its smaller preview instead.
///
/// One table for the process rather than a field on every job: the thumbnail, display and
/// visual-signature reads all go through `DecodeEngine`'s static decoders, which are handed a URL
/// and nothing else.
final class EmbeddedPreviewFallback: @unchecked Sendable {
    static let shared = EmbeddedPreviewFallback()

    struct Entry {
        let range: ByteRange
        let orientation: UInt8
    }

    private let lock = NSLock()
    /// Folder → file → entry. Keyed by folder so two providers (or two tests) never wipe each
    /// other's: each one replaces only the folder it had open before.
    private var folders: [URL: [URL: Entry]] = [:]

    /// Every photo in `folder` that reported an embedded JPEG, in place of what `previous` held.
    func replace(folder: URL, photos: [PhotoMeta], previous: URL? = nil) {
        var files: [URL: Entry] = [:]
        for photo in photos {
            guard let preview = photo.preview else { continue }
            files[folder.appendingPathComponent(photo.relPath)] = Entry(
                range: preview.range, orientation: photo.orientation)
        }
        lock.lock()
        if let previous { folders[previous] = nil }
        folders[folder] = files.isEmpty ? nil : files
        lock.unlock()
    }

    func remove(folder: URL) {
        lock.lock()
        folders[folder] = nil
        lock.unlock()
    }

    func entry(for url: URL) -> Entry? {
        lock.lock()
        defer { lock.unlock() }
        for files in folders.values {
            if let entry = files[url] { return entry }
        }
        return nil
    }
}
