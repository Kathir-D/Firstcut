// Owner: ui.
import SwiftUI

/// The Go to Batch prompt (⌘L, plan 2.5): type a batch number and press Return to jump there.
struct GoToBatchSheet: View {
    let state: any CullViewState
    @State private var text = ""
    @State private var message: String?

    /// The batch number in `text`, or nil when it is not a whole number.
    static func number(from text: String) -> Int? {
        Int(text.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// What to tell the user about `text`, or nil when it names a real batch.
    static func message(for text: String, batchCount: Int) -> String? {
        guard let n = number(from: text) else { return "Type a batch number." }
        return (1...max(batchCount, 1)).contains(n) ? nil : "There are \(batchCount) batches."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Go to Batch")
                .font(.headline)
            Text("Batch 1 to \(state.batches.count). Currently in batch \(state.currentBatchIndex + 1).")
                .font(.system(size: 11))
                .foregroundStyle(Appearance.secondaryLabel)
            TextField("Batch number", text: $text)
                .textFieldStyle(.roundedBorder)
                .onChange(of: text) { _, _ in message = nil }
                .onSubmit { go() }
                .accessibilityLabel("Batch number")
            if let message {
                Text(message)
                    .font(.system(size: 11))
                    .foregroundStyle(Appearance.secondaryLabel)
            }
            HStack {
                Spacer()
                Button("Cancel") { state.dismissGoToBatch() }
                    .keyboardShortcut(.cancelAction)
                Button("Go") { go() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 300)
    }

    private func go() {
        if let problem = Self.message(for: text, batchCount: state.batches.count) {
            message = problem
        } else if let n = Self.number(from: text), !state.goToBatch(number: n) {
            message = "There are \(state.batches.count) batches."
        }
    }
}
