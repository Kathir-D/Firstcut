// Owner: pipeline.
//
// The J clipping overlay (todo.md §9.2) had no tests at all: `ClippingMask` was a hard-coded pair of
// byte comparisons with nothing asserting what they paint, and it had **two** real bugs that only a
// pixel-level test could have found (see `ClippingMask.make`): the buffer's byte order was not
// pinned, so the comparison read the alpha channel and every pixel of an opaque photograph counted
// as a highlight; and the buffer was premultiplied, so an unpremultiply turned premultiplied black
// into transparent black — the exact pixel the shadow overlay exists to find. It also made the
// Settings thresholds (which used to default to 0.0/1.0, meaning "everything clips") reachable.
//
// The images here are **single-colour**, deliberately: one colour gives one verdict, so the tests
// assert what a threshold means without depending on which row a pixel landed in.

import CoreGraphics
import Foundation
import Testing

@testable import Firstcut

@Suite("The clipping overlay (todo.md §9.2)")
struct ClippingMaskTests {
    /// The layout §7.1 forces on every cached bitmap, so the mask's draw is a straight copy of it.
    private static let layout =
        CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue

    /// A 4×4 image of one colour, in the layout above (memory is B, G, R, A).
    private func solid(_ colour: (UInt8, UInt8, UInt8)) -> CGImage {
        let side = 4
        var bytes = [UInt8](repeating: 0, count: side * side * 4)
        for index in stride(from: 0, to: bytes.count, by: 4) {
            bytes[index] = colour.2
            bytes[index + 1] = colour.1
            bytes[index + 2] = colour.0
            bytes[index + 3] = 255
        }
        let info = CGBitmapInfo(rawValue: Self.layout)
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(
            width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: side * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info, provider: provider, decode: nil,
            shouldInterpolate: false, intent: .defaultIntent)!
    }

    /// The mask's first pixel, as `(red, green, blue, alpha)` in the mask's own layout
    /// (`premultipliedLast | byteOrder32Little`, memory R, G, B, A), or nil if it cannot be read.
    private func firstPixel(_ mask: CGImage) -> (UInt8, UInt8, UInt8, UInt8)? {
        guard
            let provider = mask.dataProvider,
            let data = provider.data as Data?,
            data.count >= 4
        else { return nil }
        return (
            data[data.startIndex], data[data.startIndex + 1], data[data.startIndex + 2],
            data[data.startIndex + 3]
        )
    }

    /// True when the mask paints its first pixel at all (alpha above zero).
    private func paints(_ mask: CGImage) -> Bool {
        (firstPixel(mask)?.3 ?? 0) > 0
    }

    /// The first pixel as a string, because a tuple is not `Equatable` and these tests assert on
    /// four bytes at a time.
    private func describe(_ mask: CGImage) -> String {
        guard let pixel = firstPixel(mask) else { return "unreadable" }
        return "(\(pixel.0),\(pixel.1),\(pixel.2),\(pixel.3))"
    }

    @Test("A channel at or past the highlight point is painted red")
    func highlightsPaintRed() {
        // Fully white clips; 249 in every channel is one step below the default point.
        let white = ClippingMask.make(from: solid((255, 255, 255)))!
        #expect(describe(white) == "(178,0,0,178)", "premultiplied red at ~70%")

        let justUnder = ClippingMask.make(from: solid((249, 249, 249)))!
        #expect(paints(justUnder) == false, "249 is below the default 250 point")

        // Any *one* channel at the point is enough, which is the whole rule: a blown sky is one
        // channel over, not three.
        let redOnly = ClippingMask.make(from: solid((255, 200, 200)))!
        #expect(paints(redOnly), "one channel at the point is a highlight")
    }

    @Test("Every channel at or below the shadow point is painted blue")
    func shadowsPaintBlue() {
        // Black is the pixel the shadow overlay exists for, and the one an unpremultiplied buffer
        // used to lose entirely.
        let black = ClippingMask.make(from: solid((0, 0, 0)))!
        #expect(describe(black) == "(0,0,178,178)", "premultiplied blue at ~70%")

        let justOver = ClippingMask.make(from: solid((6, 6, 6)))!
        #expect(paints(justOver) == false, "6 is above the default 5 point")

        // All three have to be at the bottom: one dark channel in a colourful pixel is not a shadow.
        let oneDark = ClippingMask.make(from: solid((0, 120, 200)))!
        #expect(paints(oneDark) == false, "one dark channel is not a clipped shadow")
    }

    @Test("A threshold that includes a value includes it, and one that excludes it does not")
    func thresholdsAreHonoured() {
        let grey = solid((200, 200, 200))
        #expect(paints(ClippingMask.make(from: grey, highlight: 250, shadow: 5)!) == false)
        #expect(paints(ClippingMask.make(from: grey, highlight: 200, shadow: 5)!))

        // The comparison is `<=`, so a shadow point of n includes values up to n and no further:
        // (3, 3, 3) is a shadow at 3 and above, and not one at 2 — which is the whole difference
        // between the default 5 and the 0.0 the settings model used to hold, where only pure black
        // could ever have been a shadow.
        let nearBlack = solid((3, 3, 3))
        for point in UInt8(3)...5 {
            #expect(
                paints(ClippingMask.make(from: nearBlack, highlight: 250, shadow: point)!),
                "3 is a shadow at this point: \(point)")
        }
        for point in UInt8(0)...2 {
            #expect(
                paints(ClippingMask.make(from: nearBlack, highlight: 250, shadow: point)!) == false,
                "3 is not a shadow at this point: \(point)")
        }
        // ...and 5 is the last value that includes 5.
        #expect(paints(ClippingMask.make(from: solid((5, 5, 5)), highlight: 250, shadow: 5)!))
        #expect(
            paints(ClippingMask.make(from: solid((5, 5, 5)), highlight: 250, shadow: 4)!) == false)
    }

    @Test("The defaults are the numbers the overlay has always used")
    func defaultsAreUnchanged() {
        // The regression test for the settings defaults: `make(from:)` with no thresholds must
        // still mean 250/5 whatever the model says, and it must still paint both ends.
        for colour in [
            (UInt8(255), UInt8(255), UInt8(255)), (0, 0, 0), (128, 128, 128),
            (249, 249, 249), (6, 6, 6), (250, 0, 0),
        ] {
            let implicit = ClippingMask.make(from: solid(colour))!
            let explicit = ClippingMask.make(from: solid(colour), highlight: 250, shadow: 5)!
            #expect(
                describe(implicit) == describe(explicit),
                "\(colour): \(describe(implicit)) vs \(describe(explicit))")
        }
    }

    @Test("Settings fractions become byte clip points, clamped and ordered")
    func settingsFractionsResolve() {
        // The model's defaults, written as the fractions they are.
        #expect(
            ClippingMask.thresholds(highlight: 250.0 / 255.0, shadow: 5.0 / 255.0) == (250, 5),
            "the settings defaults reproduce the mask's own defaults")

        // Out-of-range values are what a hand-edited settings.json contains. Clamped, not honoured.
        #expect(ClippingMask.thresholds(highlight: 4, shadow: -3).highlight == 255)
        #expect(ClippingMask.thresholds(highlight: 4, shadow: -3).shadow == 0)

        // An inverted pair would paint every pixel one colour if the highlight could end up below
        // the shadow: the shadow branch is the `else` of the highlight branch, so a highlight point
        // under the shadow point means every pixel in between gets both verdicts.
        let inverted = ClippingMask.thresholds(highlight: 0, shadow: 1)
        #expect(
            inverted.highlight >= inverted.shadow,
            "the highlight can never sit below the shadow: \(inverted)")

        // The worst inverted pair a hand-edited file can contain — both ends at 1.0 — must not
        // overflow the byte arithmetic (`lower + 1` on 255 traps). This is the crash this test found.
        let worst = ClippingMask.thresholds(highlight: 1, shadow: 1)
        #expect(worst.shadow == 255, "the shadow is honoured")
        #expect(worst.highlight == 255, "and the highlight cannot go past the byte it lives in")
    }
}
