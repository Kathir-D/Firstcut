// Owner: ui.

import Foundation

enum PhotoFormatter {
  static func shutter(_ seconds: Float?) -> String {
    guard let seconds, seconds > 0 else { return "—" }
    if seconds >= 1 { return String(format: "%.1f s", seconds) }
    let denominator = Int((1 / seconds).rounded())
    guard denominator > 0 else { return String(format: "%.4f s", seconds) }
    return "1/\(denominator)"
  }

  static func aperture(_ value: Float?) -> String {
    guard let value, value > 0 else { return "—" }
    return String(format: "f/%.1f", value)
  }

  static func iso(_ value: UInt32?) -> String {
    guard let value, value > 0 else { return "—" }
    return "\(value)"
  }

  static func focalLength(_ value: Float?) -> String {
    guard let value, value > 0 else { return "—" }
    if abs(value.rounded() - value) < 0.01 { return "\(Int(value)) mm" }
    return String(format: "%.1f mm", value)
  }

  static func exposureCompensation(_ value: Float?) -> String {
    guard let value else { return "—" }
    if abs(value) < 0.005 { return "0 EV" }
    return String(format: "%+.1f EV", value)
  }

  static func fileSize(_ bytes: UInt64) -> String {
    let formatter = ByteCountFormatter()
    formatter.countStyle = .file
    formatter.allowedUnits = [.useMB, .useGB]
    return formatter.string(fromByteCount: Int64(bytes))
  }

  static func dimensions(width: UInt32, height: UInt32) -> String {
    "\(width) × \(height)"
  }

  static func captureTime(_ capture: CaptureTime?) -> String {
    guard let capture else { return "—" }
    let date = Date(timeIntervalSince1970: Double(capture.unixMs) / 1000)
    let base = date.formatted(date: .abbreviated, time: .standard)
    guard capture.subsecResolutionMs < 1000, capture.subsecResolutionMs > 1 else { return base }
    let fraction = (abs(capture.unixMs) % 1000) / max(1, Int64(capture.subsecResolutionMs))
    let digits = capture.subsecResolutionMs == 10 ? 2 : 3
    return "\(base).\(String(format: "%0\(digits)d", fraction))"
  }

  static func elapsed(_ interval: TimeInterval) -> String {
    let total = Int(interval.rounded())
    let hours = total / 3600
    let minutes = (total % 3600) / 60
    let seconds = total % 60
    if hours > 0 { return String(format: "%d:%02d:%02d", hours, minutes, seconds) }
    return String(format: "%d:%02d", minutes, seconds)
  }
}
