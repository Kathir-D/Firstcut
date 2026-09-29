// Owner: ui.
//
// Right-side glass inspector (task.md §9.5). Field selection is a settings choice owned by
// app-logic; every field the panel shows comes straight off `PhotoMeta`.

import SwiftUI

struct InfoPanel: View {
  let state: any CullViewState

  var body: some View {
    GlassBackground {
      VStack(alignment: .leading, spacing: 0) {
        header
        Divider().overlay(Appearance.separator)
        if let photo = state.currentPhoto {
          ScrollView {
            VStack(alignment: .leading, spacing: 0) {
              RatingSummary(rating: photo.rating, tier: photo.tier, mode: state.ratingMode)
              rows(for: photo)
              if let histogram = state.images.histogram(for: photo.id) {
                HistogramView(histogram: histogram)
                  .padding(.horizontal, 14)
                  .padding(.top, 12)
              }
            }
            .padding(.bottom, 14)
          }
        } else {
          Text("No photo selected")
            .font(.system(size: 12))
            .foregroundStyle(Appearance.secondaryLabel)
            .padding(14)
          Spacer()
        }
      }
    }
    .frame(width: Appearance.infoPanelWidth)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("Photo info")
  }

  private var header: some View {
    HStack(spacing: 6) {
      Text(state.folderName)
        .font(.system(size: 11, weight: .semibold))
        .foregroundStyle(Appearance.secondaryLabel)
        .lineLimit(1)
        .truncationMode(.middle)
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 9)
  }

  private func rows(for photo: CullPhoto) -> some View {
    let meta = photo.meta
    let position =
      "Batch \(state.currentBatchIndex + 1) of \(state.batches.count) · photo \(state.currentPhotoIndex + 1) of \(state.photosInCurrentBatch.count)"
    let camera = [meta.cameraMake, meta.cameraModel].compactMap { $0 }.joined(separator: " ")
    let folder = (meta.relPath as NSString).deletingLastPathComponent
    let entries: [(String, String)] = [
      ("File", photo.fileName),
      ("Captured", PhotoFormatter.captureTime(meta.captureTime)),
      ("Camera", camera.isEmpty ? "—" : camera),
      ("Serial", meta.cameraSerial ?? "—"),
      ("Lens", meta.lensModel ?? "—"),
      ("Focal length", PhotoFormatter.focalLength(meta.focalLengthMm)),
      ("Shutter", PhotoFormatter.shutter(meta.exposureTimeS)),
      ("Aperture", PhotoFormatter.aperture(meta.fNumber)),
      ("ISO", PhotoFormatter.iso(meta.iso)),
      ("Exposure comp.", PhotoFormatter.exposureCompensation(meta.exposureCompEv)),
      ("Metering", meta.meteringMode ?? "—"),
      ("AF area", meta.af?.areaMode ?? "—"),
      ("AF points in focus", afPoints(meta)),
      ("Drive", meta.driveMode ?? "—"),
      ("Shutter mode", meta.shutterMode ?? "—"),
      ("Shutter count", meta.shutterCount.map(String.init) ?? "—"),
      ("Dimensions", PhotoFormatter.dimensions(width: meta.width, height: meta.height)),
      ("File size", PhotoFormatter.fileSize(meta.fileSize)),
      ("Folder", folder.isEmpty ? "—" : folder),
      ("Position", position),
    ]
    return VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(entries.enumerated()), id: \.offset) { index, entry in
        InfoRow(title: entry.0, value: entry.1)
        if index < entries.count - 1 {
          Divider().overlay(Appearance.separator.opacity(0.4))
        }
      }
      if !meta.warnings.isEmpty {
        Text(meta.warnings.joined(separator: "\n"))
          .font(.system(size: 10))
          .foregroundStyle(Appearance.rejectRed)
          .padding(.horizontal, 14)
          .padding(.top, 10)
      }
    }
    .padding(.top, 6)
  }

  private func afPoints(_ meta: PhotoMeta) -> String {
    guard let af = meta.af else { return "—" }
    return "\(af.points.filter(\.inFocus).count) of \(af.points.count)"
  }
}

struct InfoRow: View {
  let title: String
  let value: String

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 10) {
      Text(title)
        .font(.system(size: 11))
        .foregroundStyle(Appearance.secondaryLabel)
        .frame(width: 108, alignment: .leading)
      Text(value)
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(Appearance.primaryLabel)
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 5)
    .accessibilityElement(children: .combine)
  }
}

struct HistogramView: View {
  let histogram: CullHistogram

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Text("Histogram")
        .font(.system(size: 10, weight: .semibold))
        .foregroundStyle(Appearance.secondaryLabel)
      GeometryReader { geometry in
        let width = geometry.size.width
        let height = geometry.size.height
        ZStack(alignment: .bottomLeading) {
          channel(histogram.luminance, .white.opacity(0.55), width: width, height: height)
          channel(histogram.red, Appearance.rejectRed, width: width, height: height)
          channel(histogram.green, .green, width: width, height: height)
          channel(histogram.blue, .blue, width: width, height: height)
        }
        .background(Color.white.opacity(0.06))
        .clipShape(.rect(cornerRadius: 3))
      }
      .frame(height: 74)
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("Histogram")
  }

  private func channel(_ values: [Double], _ color: Color, width: CGFloat, height: CGFloat)
    -> some View
  {
    let peak = max(values.max() ?? 0, 0.0001)
    return Path { path in
      guard values.count > 1 else { return }
      for (index, value) in values.enumerated() {
        let x = width * CGFloat(index) / CGFloat(values.count - 1)
        let y = height * CGFloat(value / peak)
        if index == 0 {
          path.move(to: CGPoint(x: x, y: y))
        } else {
          path.addLine(to: CGPoint(x: x, y: y))
        }
      }
    }
    .stroke(color, lineWidth: 1)
  }
}

struct RatingSummary: View {
  let rating: Rating
  let tier: Tier
  let mode: RatingMode

  var body: some View {
    HStack(spacing: 8) {
      if mode == .stars {
        HStack(spacing: 1) {
          // REV-76: exactly `stars` filled stars, no empty outlines, so the row reads as the
          // rating rather than as a ratio. Unrated draws nothing.
          ForEach(0..<RatingVisuals.starCount(for: rating), id: \.self) { _ in
            Image(systemName: "star.fill")
              .font(.system(size: 10))
              .foregroundStyle(Color.yellow)
          }
        }
      } else {
        Circle()
          .fill(rating.keep ? Appearance.keepGreen : Appearance.rejectRed)
          .frame(width: 8, height: 8)
        Text(rating.keep ? "Keep" : "Not keep")
          .font(.system(size: 10))
      }
      if let label = rating.label {
        Circle()
          .fill(RatingColor.color(for: label))
          .frame(width: 8, height: 8)
      }
      Spacer(minLength: 0)
      Text(tier.title)
        .font(.system(size: 10, weight: .semibold))
        .foregroundStyle(Appearance.secondaryLabel)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 8)
    .accessibilityElement(children: .combine)
    .accessibilityLabel("Rating \(tier.title)")
  }
}
