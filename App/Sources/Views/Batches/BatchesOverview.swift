// Owner: ui.
import SwiftUI

/// What one cell of the Batches overview says about a batch. Pure, so it is testable without a view.
struct BatchSummary: Equatable {
    var number: Int
    var photoCount: Int
    var ratedCount: Int
    var isVisited: Bool
    /// Capture time of the batch's first photo, in Unix milliseconds. Shown in Plays mode, where a
    /// batch is a stretch of play rather than one burst.
    var startMs: Int64?

    init(number: Int, photos: [CullPhoto], isVisited: Bool) {
        self.number = number
        photoCount = photos.count
        ratedCount = photos.filter { !$0.rating.isNeutral }.count
        self.isVisited = isVisited
        startMs = photos.first?.meta.captureTime.map { Int64($0.unixMs) }
    }

    /// "12 photos · 5 rated".
    var detail: String {
        let noun = photoCount == 1 ? "photo" : "photos"
        return "\(photoCount) \(noun) · \(ratedCount) rated"
    }

    /// "Visited" or "Not visited yet".
    var status: String { isVisited ? "Visited" : "Not visited yet" }

    /// The start time as clock time, or nil when the photo has none.
    func startTime(timeZone: TimeZone = .current) -> String? {
        guard let startMs else { return nil }
        let formatter = DateFormatter()
        formatter.timeZone = timeZone
        formatter.dateFormat = "HH:mm:ss"
        return formatter.string(from: Date(timeIntervalSince1970: Double(startMs) / 1000))
    }
}

/// A grid of every batch: a cover thumbnail, how many photos, how many are rated, whether it has
/// been visited and, in Plays mode, when it started. Clicking a cell jumps there. Plan 2.5.
struct BatchesOverview: View {
    let state: any CullViewState
    private let columns = [GridItem(.adaptive(minimum: 130, maximum: 180), spacing: 10)]

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 10) {
                ForEach(state.batches) { batch in
                    let photos = state.photos(inBatch: batch.index)
                    let summary = BatchSummary(
                        number: batch.index + 1, photos: photos, isVisited: batch.isVisited)
                    Button {
                        _ = state.goToBatch(number: summary.number)
                    } label: {
                        cell(summary, cover: photos.first, isCurrent: batch.index == state.currentBatchIndex)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(
                        "Batch \(summary.number), \(summary.detail), \(summary.status)")
                }
            }
            .padding(2)
        }
        .frame(maxHeight: 320)
    }

    private func cell(_ summary: BatchSummary, cover: CullPhoto?, isCurrent: Bool) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            ZStack {
                RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color.black.opacity(0.35))
                if let cover,
                    let image = state.images.thumbnail(for: cover.id, size: CGSize(width: 240, height: 240))
                {
                    Image(decorative: image, scale: 1).resizable().scaledToFit().padding(2)
                } else {
                    ProgressView().controlSize(.small)
                }
            }
            .aspectRatio(3 / 2, contentMode: .fit)
            .overlay {
                if isCurrent {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .strokeBorder(Color.accentColor, lineWidth: 2)
                }
            }
            Text("Batch \(summary.number)").font(.caption).bold()
            Text(summary.detail).font(.system(size: 10)).foregroundStyle(Appearance.secondaryLabel)
            HStack(spacing: 4) {
                Circle()
                    .fill(summary.isVisited ? Appearance.keepGreen : Color.gray.opacity(0.5))
                    .frame(width: 6, height: 6)
                Text(summary.status).font(.system(size: 10)).foregroundStyle(Appearance.secondaryLabel)
                if state.batchingMode == .plays, let time = summary.startTime() {
                    Spacer(minLength: 0)
                    Text(time).font(.system(size: 10)).foregroundStyle(Appearance.secondaryLabel)
                }
            }
        }
    }
}
