// Owner: app-logic + core-meta.
//
// The first-photo fast path (todo.md §7.3, "folder open → first photo on screen < 1 s"), in Swift.
//
// The core has two exports for this and they are the whole trick:
//
//     firstPhotoName(path:)  → "IMG_0001.CR3", from a directory listing alone
//     readPhoto(folder:relPath:) → that file's metadata, from one header read
//
// So a photograph is on screen after two small reads instead of after 2,880 of them, and the full
// scan, the batching and the T2 prefetch follow in parallel behind it. The picture the app shows
// immediately is byte-for-byte the one the scan returns for that file — `tests/cr3_exiftool.rs`
// asserts that on the real photos, over a stride of a whole shoot — so the photograph does not
// change when the scan lands, only the *order* does.
//
// Not a capability check. `FirstcutCoreBridge.hasSessionAPI` exists because a *class* can be missing
// from an old build of the generated bindings; these are free functions the app calls by name, so a
// bindings file without them is a compile error, which is a better failure than `false`.

import FirstcutCore
import Foundation

public enum CoreFirstPhoto {
    /// One photograph from a folder, or nil when the folder has none this core can read.
    ///
    /// Blocking and off the main actor: two `pread`s, and a directory listing of a shoot can be
    /// 2,880 entries. The caller decides which thread.
    public static func read(_ folder: URL) -> PhotoMeta? {
        guard let name = firstPhotoName(path: folder.path) else { return nil }
        return readPhoto(folder: folder.path, relPath: name)
    }
}
