// Owner: ui.
import Foundation

/// Pure helpers for turning free-form path text into typed folder values.
enum PathInput {
    /// Trim surrounding whitespace/newlines, then resolve a single leading "~"
    /// to the user's home directory.
    static func expand(_ text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > 0 else { return "" }
        if trimmed == "~" || trimmed.hasPrefix("~/") {
            return NSHomeDirectory() + trimmed.dropFirst()
        }
        return trimmed
    }

    /// Classify the expanded path.
    enum Check: Equatable {
        case empty
        case notFound
        case notAFolder
        case folder(URL)
    }

    static func check(_ text: String) -> Check {
        let path = expand(text)
        guard !path.isEmpty else { return .empty }
        let fm = FileManager.default
        var isDir: ObjCBool = false
        let exists = fm.fileExists(atPath: path, isDirectory: &isDir)
        guard exists else { return .notFound }
        if isDir.boolValue {
            return .folder(URL(fileURLWithPath: path, isDirectory: true))
        }
        return .notAFolder
    }

    /// Localized message for a failed check, or `nil` when the check is fine.
    static func message(for check: Check) -> String? {
        switch check {
        case .empty:
            return nil
        case .notFound:
            return "No folder at that path."
        case .notAFolder:
            return "That is a file, not a folder."
        case .folder:
            return nil
        }
    }

    /// Suggest matching folder paths under the parent of the given text.
    static func suggestions(for text: String, limit: Int = 6) -> [String] {
        let trimmed = expand(text)
        guard !trimmed.isEmpty else { return [] }
        let fm = FileManager.default
        guard let slash = trimmed.lastIndex(of: "/") else { return [] }
        let rawParent = String(trimmed[trimmed.startIndex..<slash])
        let parent = rawParent.isEmpty ? "/" : rawParent
        let prefix = String(trimmed[trimmed.index(after: slash)...])
        let contents = (try? fm.contentsOfDirectory(atPath: parent)) ?? []
        let lower = prefix.lowercased()
        var result: [String] = []
        for name in contents where !name.hasPrefix(".") && name.lowercased().hasPrefix(lower) {
            let full = parent == "/" ? "/" + name : parent + "/" + name
            var isDir: ObjCBool = false
            if fm.fileExists(atPath: full, isDirectory: &isDir), isDir.boolValue {
                result.append(full)
            }
        }
        result.sort()
        if limit >= 0 {
            result = Array(result.prefix(limit))
        }
        return result
    }
}
