// Owner: pipeline.
//
// The debug HUD: cache hits and misses, decode queue depth, memory by tier, and the signpost
// counters. Hidden unless Settings → Performance → "Debug HUD" is on, because it is a tool for
// proving todo.md §7.3 rather than part of the product.
//
// ## What it is for
//
// §7.1 makes one promise — nothing in the focus window is ever decoded on demand — and the number
// that decides it is `PipelineStats.focusMisses`. That counter is invisible without something to
// read it in, and a counter nobody can see is a counter nobody maintains. This is the readout, plus
// the two things that explain a *non-zero* miss when there is one: how deep the queue is, and whether
// the cache is under its budget (a cache thrashing at the budget looks exactly like a prefetch that
// is not keeping up).
//
// ## Where the numbers come from
//
// `ImageProvider.stats` is a snapshot taken under the engine's lock, so reading it cannot block a
// decode for long. The one thing this view must never do is compute anything on the main thread that
// it did not have to — so the formatting is done by the view and the *measurements* are all counters
// the engine maintains anyway.

import SwiftUI

struct DebugHUDView: View {
    let stats: PipelineStats
    let thumbnailProgress: Double
    let byteRangeDecodes: Int
    let containerDecodes: Int
    let focusSize: Int
    let viewportPixels: CGSize
    let budgetBytes: Int

    var body: some View {
      VStack(alignment: .leading, spacing: 3) {
        row("focus", "\(focusSize) photos", tint: stats.focusMisses == 0 ? .green : .red)
        row(
          "misses", "\(stats.focusMisses)",
          note: stats.focusMisses == 0 ? "must stay 0" : "todo.md §7.1 is broken",
          tint: stats.focusMisses == 0 ? .green : .red)
        row(
          "queue", "\(stats.queuedDecodes) queued · \(stats.decodesInProgress) decoding",
          // A queue many times the focus window means the prefetch is not keeping up with the
          // navigation, which is the thing §7.1 promises cannot happen.
          tint: stats.queuedDecodes > focusSize * 4 ? .orange : nil)
        Divider().opacity(0.4)
        row("thumbs", "\(stats.thumbnailDecodes) decoded · \(stats.thumbnailCacheHits) hits")
        row("display", "\(stats.displayDecodes) decoded · \(stats.displayCacheHits) hits")
        row(
          "from bytes", "\(byteRangeDecodes)",
          note: byteRangeDecodes == 0 && stats.displayDecodes > 0
            ? "optimisation unused" : "\(containerDecodes) from container",
          tint: byteRangeDecodes == 0 && stats.displayDecodes > 0 ? .orange : nil)
        row("histograms", "\(stats.histogramComputes) computed")
        Divider().opacity(0.4)
        row("thumb memory", bytes(stats.thumbnailBytes))
        row("display memory", bytes(stats.displayBytes))
        let used = stats.thumbnailBytes + stats.displayBytes
        row(
          "of budget", "\(percent(used, of: budgetBytes)) of \(bytes(budgetBytes))",
          tint: budgetBytes > 0 && used > budgetBytes ? .orange : nil)
        row("pressure sheds", "\(stats.pressureSheds)", tint: stats.pressureSheds > 0 ? .orange : nil)
        row("decode failures", "\(stats.decodeFailures)", tint: stats.decodeFailures > 0 ? .orange : nil)
        Divider().opacity(0.4)
        row("thumbnails", "\(Int((thumbnailProgress * 100).rounded()))%")
        row(
          "viewport",
          viewportPixels == .zero
            ? "not reported" : "\(Int(viewportPixels.width))×\(Int(viewportPixels.height)) px",
          // T2 is decoded at this size, so a zero here is why the viewer looks soft.
          tint: viewportPixels == .zero ? .orange : nil)
      }
      .font(.system(size: 10, design: .monospaced))
      .padding(8)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(.ultraThinMaterial)
      .clipShape(RoundedRectangle(cornerRadius: 8))
      .overlay(
        RoundedRectangle(cornerRadius: 8).strokeBorder(.white.opacity(0.12), lineWidth: 1)
      )
    }

    /// A label, a value, and optionally a note and a colour. Four arguments rather than one
    /// overloaded one, because a note is text and a tint is a colour and conflating them meant every
    /// call site had to pick which it meant.
    private func row(
      _ label: String, _ value: String, note: String? = nil, tint: Color? = nil
    ) -> some View {
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        Text(label)
          .foregroundStyle(.secondary)
          .frame(width: 86, alignment: .leading)
        Text(value)
          .foregroundStyle(tint ?? .primary)
        if let note {
          Text(note).foregroundStyle(tint ?? .secondary)
        }
      }
    }

    private func percent(_ part: Int, of total: Int) -> String {
      guard total > 0 else { return "—" }
      return "\(Int(Double(part) / Double(total) * 100))%"
    }

    private func bytes(_ count: Int) -> String {
      let value = Double(count)
      if value < 1024 { return "\(count) B" }
      if value < 1_048_576 { return String(format: "%.0f KB", value / 1024) }
      return String(format: "%.2f GB", value / 1_073_741_824)
    }
}
