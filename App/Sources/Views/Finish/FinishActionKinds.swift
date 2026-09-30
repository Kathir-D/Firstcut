// Owner: ui.
//
// `UnkeptAction` / `KeptAction` carry a folder name in some cases, so a Picker cannot compare them
// with `==` (the "Move to a subfolder" entry in `allCases` has a placeholder name, the current
// value has the user's). These compare by *kind* and swap the folder without changing the kind.
// Shared by the Finish sheet and Settings → General so the two cannot disagree.

import Foundation

extension UnkeptAction {
  func isSameKind(as other: UnkeptAction) -> Bool {
    switch (self, other) {
    case (.markRejectedInXmp, .markRejectedInXmp), (.moveToSubfolder, .moveToSubfolder),
      (.moveToTrash, .moveToTrash), (.deletePermanently, .deletePermanently), (.nothing, .nothing):
      true
    default: false
    }
  }

  /// The `allCases` element of this kind, which is what a Picker's tags hold.
  var pickerTag: UnkeptAction { UnkeptAction.allCases.first { $0.isSameKind(as: self) } ?? self }

  /// This kind with `folder`, for the kinds that take one.
  func with(folder: String) -> UnkeptAction {
    if case .moveToSubfolder = self { return .moveToSubfolder(folder.isEmpty ? "_Not kept" : folder) }
    return self
  }
}

extension KeptAction {
  func isSameKind(as other: KeptAction) -> Bool {
    switch (self, other) {
    case (.none, .none), (.copyTo, .copyTo), (.moveTo, .moveTo), (.splitByTier, .splitByTier),
      (.splitByStars, .splitByStars), (.writeList, .writeList):
      true
    default: false
    }
  }

  var pickerTag: KeptAction { KeptAction.allCases.first { $0.isSameKind(as: self) } ?? self }

  /// The text the action holds: a folder, or the file name for a list.
  var text: String {
    switch self {
    case .none: ""
    case .copyTo(let value), .moveTo(let value), .splitByTier(let value), .splitByStars(let value),
      .writeList(let value):
      value
    }
  }

  func with(folder: String) -> KeptAction {
    switch self {
    case .none: .none
    case .copyTo: .copyTo(folder)
    case .moveTo: .moveTo(folder)
    case .splitByTier: .splitByTier(folder)
    case .splitByStars: .splitByStars(folder)
    case .writeList: .writeList(folder.isEmpty ? "kept.txt" : folder)
    }
  }
}
