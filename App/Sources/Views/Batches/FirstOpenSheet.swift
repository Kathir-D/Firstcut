// Owner: ui.
import SwiftUI

/// Shown once, over the first folder opened: how to group photos and how to rate them. Both can be
/// changed later in Settings → General. Plan 2.1.
struct FirstOpenSheet: View {
    let state: any CullViewState
    @State private var batching: BatchingMode = .plays
    @State private var rating: RatingMode = .stars

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("How do you want to cull?")
                .font(.headline)
            Picker("Batch by", selection: $batching) {
                Text("Plays").tag(BatchingMode.plays)
                Text("Bursts").tag(BatchingMode.bursts)
            }
            .pickerStyle(.radioGroup)
            Text("Plays join bursts of the same moment into one batch; Bursts keeps each burst on its own.")
                .font(.system(size: 11))
                .foregroundStyle(Appearance.secondaryLabel)
            Picker("Rate with", selection: $rating) {
                Text("Stars").tag(RatingMode.stars)
                Text("Keep / Not keep").tag(RatingMode.keep)
            }
            .pickerStyle(.radioGroup)
            Text("You can change both later in Settings → General.")
                .font(.system(size: 11))
                .foregroundStyle(Appearance.secondaryLabel)
            HStack {
                Spacer()
                Button("Continue") {
                    state.completeFirstOpen(batchingMode: batching, ratingMode: rating)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 380)
        .onAppear { batching = state.batchingMode }
    }
}
