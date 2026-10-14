// Owner: ui.

import Testing

@testable import Firstcut

@Suite("Low-res preview detection")
struct LowResPreviewTests {
    /// A photo with the given sensor size and embedded previews. `preview` is the best preview
    /// (full over normal), so pass `fullPreview` in `preview` when there is no separate full.
    private static func meta(
        width: UInt32,
        height: UInt32,
        preview: EmbeddedPreview? = nil,
        fullPreview: EmbeddedPreview? = nil
    ) -> PhotoMeta {
        PhotoMeta(
            id: 1, relPath: "IMG.jpg", companions: [], kind: .jpeg,
            fileSize: 1024, captureTime: nil, shutterCount: nil, fileNumber: nil,
            cameraMake: nil, cameraModel: nil, cameraSerial: nil, lensModel: nil,
            focalLengthMm: nil, exposureTimeS: nil, fNumber: nil, iso: nil, exposureCompEv: nil,
            meteringMode: nil, driveMode: nil, shutterMode: nil, orientation: 1,
            width: width, height: height, af: nil,
            preview: preview, fullPreview: fullPreview, warnings: [])
    }

    @Test("Full-size preview is not low-res")
    func fullSizePreviewIsNotLowRes() {
        let range = ByteRange(offset: UInt64(0), len: UInt64(0))
        let preview = EmbeddedPreview(range: range, width: 6000, height: 4000)
        #expect(LowResPreview.isLowRes(Self.meta(width: 6000, height: 4000, preview: preview)) == false)
    }

    @Test("A preview far below the sensor is low-res")
    func smallPreviewIsLowRes() {
        let range = ByteRange(offset: UInt64(0), len: UInt64(0))
        let preview = EmbeddedPreview(range: range, width: 869, height: 593)
        #expect(LowResPreview.isLowRes(Self.meta(width: 4000, height: 3000, preview: preview)) == true)
    }

    @Test("No preview at all is not low-res")
    func noPreviewIsNotLowRes() {
        #expect(LowResPreview.isLowRes(Self.meta(width: 6000, height: 4000)) == false)
    }

    @Test("A zero-size sensor is not low-res")
    func zeroSensorIsNotLowRes() {
        #expect(LowResPreview.isLowRes(Self.meta(width: 0, height: 0)) == false)
    }
}
