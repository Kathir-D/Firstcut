// Owner: pipeline.
//
// Wave-1 spike: measures what the image pipeline needs to be built on top of. Standalone — no Rust,
// no app, no other Firstcut code.
//
//   swiftc -O -D SPIKE App/Sources/Pipeline/Spike/DecodeSpike.swift -o /tmp/firstcut-spike
//   FIRSTCUT_TEST_PHOTOS=~/Documents/testing /tmp/firstcut-spike            # full run
//   FIRSTCUT_TEST_PHOTOS=~/Documents/testing /tmp/firstcut-spike --quick    # small sample
//
// What it measures (task.md §7.2, §7.3):
//   1. Where the embedded full-resolution JPEG lives in a CR3 and how long it is.
//   2. Decode time per image at: full res, DCT-subsampled to viewport size, and ImageIO thumbnail
//      to viewport size — plus vImage Lanczos from the full decode as the "correct but slow" option.
//      The T4 exact-RAW arm needs a fallback: CIRAWFilter has no RAW decoder registered on this
//      macOS 27 build (its supportedDecoderVersions is [None] and outputImage is nil), so the
//      benchmark measures the full RAW decode through ImageIO instead and records that finding.
//   3. Parallel decode throughput at several worker counts, pinned to the performance cores.
//   4. T0 (256 px) thumbnail throughput for the whole-shoot pass.
//   5. Cost of reading the preview bytes off disk (T1 refill).
//   6. IOSurface allocation + pixel upload (bytes/s, ms/image).
//   7. Display swap: assigning an IOSurface-backed CGImage to a CALayer and getting it on screen.
//   8. VisualSig (dHash + histogram) cost per thumbnail, per the batching.md algorithm.
//
// The whole file is behind `#if SPIKE` so it compiles to nothing inside the app target.

#if SPIKE

import Accelerate
import AppKit
import CoreGraphics
import CoreImage
import Foundation
import ImageIO
import IOSurface
import QuartzCore

// MARK: - Config

let testPhotosEnv = "FIRSTCUT_TEST_PHOTOS"
let testPhotosRoot = URL(fileURLWithPath: ProcessInfo.processInfo.environment[testPhotosEnv]
    ?? NSString(string: "~/Documents/testing").expandingTildeInPath)

let args = CommandLine.arguments
let quick = args.contains("--quick")
let skipDisplay = args.contains("--no-display")

/// Viewport backing-pixel sizes worth knowing about: the loupe area of a maximized window on the
/// common Mac displays. All are Retina 2x.
let viewportSizes: [(name: String, px: CGSize)] = [
    ("1280x800 pt  (MBP 13\")", CGSize(width: 2560, height: 1600)),
    ("1440x900 pt  (MBP 14\")", CGSize(width: 2880, height: 1800)),
    ("1728x1117 pt (MBP 16\")", CGSize(width: 3456, height: 2234)),
]

// MARK: - Small utilities

@discardableResult
func note(_ s: String) -> String {
    print(s)
    fflush(stdout)
    return s
}

func ms(_ seconds: TimeInterval) -> String { String(format: "%.1f ms", seconds * 1000) }

/// `Date.timeIntervalSinceNow` returns a *negative* delta on this OS build (verified 2026-09-29),
/// so every measurement here goes through an explicit end stamp instead.
func elapsed(since start: Date) -> TimeInterval { Date().timeIntervalSince(start) }

/// Resident footprint of this process, in bytes. Used to show how much decoded pixels really cost.
func physFootprint() -> UInt64 {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
    let kr = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    return kr == KERN_SUCCESS ? UInt64(info.phys_footprint) : 0
}

func perfCoreCount() -> Int {
    var n = 0
    var size = MemoryLayout<Int>.size
    sysctlbyname("hw.perflevel0.logicalcpu", &n, &size, nil, 0)
    return n > 0 ? n : ProcessInfo.processInfo.activeProcessorCount
}

// MARK: - CR3 embedded preview location
//
// core-meta will hand us `PhotoMeta.preview.range` (docs/contracts/photo-meta.md). Until then the
// spike locates the JPEGs itself by scanning the head of the file for SOI markers, which is what a
// tolerant reader has to do for formats we haven't mapped yet.

struct EmbeddedJPEGRange {
    var offset: UInt64
    var length: UInt64
    var pixelSize: CGSize
}

enum CR3 {
    /// Bytes of the file we are willing to read while hunting for previews. The full-resolution
    /// JPEG in the test set starts ~350 KB in and is ~1.5 MB, so 8 MB always contains it whole.
    static let headerScanBytes = 8 << 20

    static func read(_ url: URL, offset: UInt64, length: UInt64) throws -> Data {
        let fd = try open(url.path, O_RDONLY)
        defer { close(fd) }
        var data = Data(count: Int(length))
        var got: UInt64 = 0
        try data.withUnsafeMutableBytes { raw in
            while got < length {
                let n = pread(fd, raw.baseAddress!.advanced(by: Int(got)), Int(length - got), off_t(offset + got))
                if n <= 0 { throw CocoaError(.fileReadCorruptFile) }
                got += UInt64(n)
            }
        }
        return data
    }

    /// Every JPEG whose start marker appears in the first `headerScanBytes` of the file, with the
    /// true byte length of its entropy-coded data.
    static func findEmbeddedJPEGs(in url: URL) throws -> [EmbeddedJPEGRange] {
        let head = try read(url, offset: 0, length: UInt64(min(headerScanBytes, fileSize(url))))
        var found: [EmbeddedJPEGRange] = []
        var i = 0
        let bytes = [UInt8](head)
        while i + 3 < bytes.count {
            if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF {
                if let sof = frameSize(bytes, at: i), let len = jpegLength(bytes, at: i), len > 1024 {
                    found.append(EmbeddedJPEGRange(offset: UInt64(i), length: UInt64(len), pixelSize: sof))
                }
                i += 3
            } else {
                i += 1
            }
        }
        return found.sorted { $0.pixelSize.width * $0.pixelSize.height > $1.pixelSize.width * $1.pixelSize.height }
    }

    static func fileSize(_ url: URL) -> Int {
        (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) as? Int ?? 0
    }

    /// SOF0…SOF15 (excluding DHT/DAC/DRI), i.e. the frame header carrying the real dimensions.
    static func frameSize(_ b: [UInt8], at soi: Int) -> CGSize? {
        var i = soi + 2
        while i + 3 < b.count {
            guard b[i] == 0xFF else { return nil }
            let m = b[i + 1]
            if m == 0x01 || m == 0xD8 || (0xD0...0xD7).contains(m) { i += 2; continue }
            let segLen = (Int(b[i + 2]) << 8) | Int(b[i + 3])
            if segLen < 2 { return nil }
            let isSOF = (0xC0...0xCF).contains(m) && m != 0xC4 && m != 0xC8 && m != 0xCC
            if isSOF {
                guard i + 9 < b.count else { return nil }
                let h = (Int(b[i + 5]) << 8) | Int(b[i + 6])
                let w = (Int(b[i + 7]) << 8) | Int(b[i + 8])
                return CGSize(width: w, height: h)
            }
            if m == 0xDA { return nil }  // hit scan data before a frame header
            i += 2 + segLen
        }
        return nil
    }

    /// Byte length of the JPEG starting at `soi`, found by walking the markers and then the
    /// entropy-coded data up to EOI.
    static func jpegLength(_ b: [UInt8], at soi: Int) -> Int? {
        var i = soi + 2
        while i + 3 < b.count {
            guard b[i] == 0xFF else { return nil }
            let m = b[i + 1]
            if m == 0xD9 { return i + 2 - soi }
            if m == 0x01 || m == 0xD8 || (0xD0...0xD7).contains(m) { i += 2; continue }
            let segLen = (Int(b[i + 2]) << 8) | Int(b[i + 3])
            if segLen < 2 { return nil }
            if m == 0xDA {
                var j = i + 2 + segLen
                while j + 1 < b.count {
                    if b[j] == 0xFF && b[j + 1] == 0xD9 { return j + 2 - soi }
                    j += 1
                }
                return nil
            }
            i += 2 + segLen
        }
        return nil
    }
}

// MARK: - Decode variants

enum DecodeMode: String {
    case full = "full decode 6000x4000"
    case subsample4 = "subsample 4 (DCT /16)"
    case thumbnail = "thumbnail @ viewport (DCT)"
    case vimage = "full decode + vImage Lanczos"
}

enum Decoder {
    /// Full-resolution decode: what T3 (100% zoom) needs.
    ///
    /// `kCGImageSourceShouldCacheImmediately` is not optional here. Without it ImageIO hands back a
    /// *lazy* CGImage that decodes on first draw, so any timer around the call reports ~0.1 ms
    /// (verified 2026-09-29: lazy 0.1 ms vs. ShouldCacheImmediately 44 ms for the same frame).
    static func full(_ jpeg: Data) -> CGImage? {
        guard let src = CGImageSourceCreateWithData(jpeg as CFData, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(src, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
    }

    /// Decode with ImageIO's DCT subsampling. `factor` 2/4/8 → 1/2, 1/4, 1/8 scale.
    static func subsampled(_ jpeg: Data, factor: Int) -> CGImage? {
        guard let src = CGImageSourceCreateWithData(jpeg as CFData, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceSubsampleFactor: factor,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        return CGImageSourceCreateImageAtIndex(src, 0, opts as CFDictionary)
    }

    /// What T2 will use: the biggest DCT-scaled image whose longest edge fits `maxPixel`, with the
    /// EXIF orientation already applied.
    static func thumbnail(_ jpeg: Data, maxPixel: Int, withTransform: Bool = true) -> CGImage? {
        guard let src = CGImageSourceCreateWithData(jpeg as CFData, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: withTransform,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
        ]
        return CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary)
    }

    /// Full decode then Lanczos3 downscale via vImage — the reference for "what good looks like",
    /// and the cost we are trying to avoid by letting ImageIO do the DCT scaling.
    ///
    /// Returns raw BGRA pixels rather than a CGImage so the caller can diff them against the DCT
    /// path numerically.
    static func lanczosPixels(_ image: CGImage, maxPixel: Int) -> (data: Data, width: Int, height: Int)? {
        let space = CGColorSpaceCreateDeviceRGB()
        let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue
            | CGBitmapInfo.byteOrder32Little.rawValue)
        let sw = image.width, sh = image.height
        guard let sctx = CGContext(data: nil, width: sw, height: sh, bitsPerComponent: 8,
                                   bytesPerRow: 0, space: space, bitmapInfo: info.rawValue) else { return nil }
        sctx.interpolationQuality = .none
        sctx.draw(image, in: CGRect(x: 0, y: 0, width: sw, height: sh))
        guard let sdata = sctx.data else { return nil }
        let scale = Double(maxPixel) / Double(max(sw, sh))
        let dw = max(1, Int((Double(sw) * scale).rounded()))
        let dh = max(1, Int((Double(sh) * scale).rounded()))
        let drow = dw * 4
        let ddata = UnsafeMutableRawPointer.allocate(byteCount: drow * dh, alignment: 64)
        defer { ddata.deallocate() }
        var src = vImage_Buffer(data: sdata, height: vImagePixelCount(sh), width: vImagePixelCount(sw),
                                rowBytes: sctx.bytesPerRow)
        var dst = vImage_Buffer(data: ddata, height: vImagePixelCount(dh), width: vImagePixelCount(dw),
                                rowBytes: drow)
        guard vImageScale_ARGB8888(&src, &dst, nil, vImage_Flags(kvImageHighQualityResampling)) == kvImageNoError
        else { return nil }
        return (Data(bytes: ddata, count: drow * dh), dw, dh)
    }

    /// Same output geometry as `lanczosPixels`, via a plain CGContext draw (bilinear, what a naive
    /// implementation gets).
    static func contextPixels(_ image: CGImage, maxPixel: Int) -> (data: Data, width: Int, height: Int)? {
        let space = CGColorSpaceCreateDeviceRGB()
        let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue
            | CGBitmapInfo.byteOrder32Little.rawValue)
        let scale = Double(maxPixel) / Double(max(image.width, image.height))
        let dw = max(1, Int((Double(image.width) * scale).rounded()))
        let dh = max(1, Int((Double(image.height) * scale).rounded()))
        guard let ctx = CGContext(data: nil, width: dw, height: dh, bitsPerComponent: 8,
                                  bytesPerRow: 0, space: space, bitmapInfo: info.rawValue),
              let ddata = ctx.data else { return nil }
        ctx.interpolationQuality = .high
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: dw, height: dh))
        return (Data(bytes: ddata, count: ctx.bytesPerRow * dh), dw, dh)
    }

    static func vImageDownscaled(_ jpeg: Data, to maxPixel: Int) -> CGImage? {
        guard let big = full(jpeg), let px = lanczosPixels(big, maxPixel: maxPixel) else { return nil }
        return Pixels.cgImage(from: px)
    }
}

enum Pixels {
    static let space = CGColorSpaceCreateDeviceRGB()
    static let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue
        | CGBitmapInfo.byteOrder32Little.rawValue)

    static func cgImage(from px: (data: Data, width: Int, height: Int)) -> CGImage? {
        guard let provider = CGDataProvider(data: px.data as CFData) else { return nil }
        return CGImage(width: px.width, height: px.height, bitsPerComponent: 8, bitsPerPixel: 32,
                       bytesPerRow: px.width * 4, space: space, bitmapInfo: info,
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    static func raw(_ image: CGImage, width: Int, height: Int) -> Data? {
        guard let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                  bytesPerRow: width * 4, space: space, bitmapInfo: info.rawValue),
              let d = ctx.data else { return nil }
        ctx.interpolationQuality = .none
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return Data(bytes: d, count: width * 4 * height)
    }

    /// Mean absolute difference per channel, 0–255, over the RGB bytes.
    static func meanAbsDiff(_ a: Data, _ b: Data) -> Double {
        var sum = 0.0
        var n = 0
        let n8 = min(a.count, b.count)
        var i = 0
        while i + 3 < n8 {
            for c in 0..<3 { sum += abs(Double(a[i + c]) - Double(b[i + c])) }
            n += 3
            i += 4
        }
        return n > 0 ? sum / Double(n) : 0
    }

    /// Variance of the luma plane. A cheap proxy for "how much high-ISO grain survived the pipeline":
    /// a soft or noise-reduced downscale visibly loses this.
    static func lumaStdDev(_ data: Data, width: Int, height: Int) -> Double {
        var sum = 0.0, sumSq = 0.0
        var n = 0.0
        var i = 0
        let step = max(4, (width * height / 200_000) * 4) * 4  // sample at most ~200k pixels
        while i + 3 < data.count {
            let l = 0.299 * Double(data[i]) + 0.587 * Double(data[i + 1]) + 0.114 * Double(data[i + 2])
            sum += l; sumSq += l * l; n += 1
            i += step
        }
        guard n > 1 else { return 0 }
        let mean = sum / n
        return (sumSq / n - mean * mean).squareRoot()
    }

    /// Vertical gradient energy at high frequencies only: the classic "sharp vs soft" measure.
    /// A blurry resample loses most of this before JPEG noise can put it back.
    static func highFreqSharpness(_ data: Data, width: Int, height: Int) -> Double {
        var sum = 0.0
        var n = 0.0
        let rowBytes = width * 4
        for y in stride(from: 1, to: height, by: 3) {
            let o = y * rowBytes
            let p = o - rowBytes
            if o + rowBytes > data.count { break }
            var x = 4
            while x + 4 < width * 4 {
                let cur = luma(data, o + x), prev = luma(data, p + x), next = luma(data, o + x + 4)
                sum += abs(cur - 0.5 * (prev + next))
                n += 1
                x += 4
            }
        }
        return n > 0 ? sum / n : 0
    }

    private static func luma(_ d: Data, _ o: Int) -> Double {
        0.299 * Double(d[o]) + 0.587 * Double(d[o + 1]) + 0.114 * Double(d[o + 2])
    }
}

// MARK: - VisualSig (batching.md algorithm, pipeline implements it)

enum VisualSigSpike {
    /// Input: a 256 px-longest-edge thumbnail with orientation already applied (batching.md step 1).
    /// Returns the dHash + histogram. Spiking the cost only; core-batch exports the Rust reference
    /// `visual_sig` that the shipping implementation has to agree with bit for bit.
    static func sig(_ image: CGImage) -> (dhash: UInt64, hist: [UInt8]) {
        let (dhash, hist) = (dhash(image), histogram(image))
        return (dhash, hist)
    }

    /// Rec. 601 luma, resize to 9x8 with area averaging, bit `row*8+col` = px[row][col] > px[row][col+1].
    static func dhash(_ image: CGImage) -> UInt64 {
        let w = 9, h = 8
        var small = [UInt8](repeating: 0, count: w * h)
        if let cs = CGColorSpace(name: CGColorSpace.linearGray),
           let ctx = CGContext(data: &small, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w,
                               space: cs, bitmapInfo: CGImageAlphaInfo.none.rawValue) {
            ctx.interpolationQuality = .medium
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        }
        var bits: UInt64 = 0
        for row in 0..<h {
            for col in 0..<8 {
                if small[row * w + col] > small[row * w + col + 1] {
                    bits |= (1 << UInt64(row * 8 + col))
                }
            }
        }
        return bits
    }

    /// 16 bins per channel over all thumbnail pixels, each channel normalized so its largest bin is 255.
    static func histogram(_ image: CGImage) -> [UInt8] {
        let w = image.width, h = image.height
        var rgba = [UInt8](repeating: 0, count: w * h * 4)
        if let cs = CGColorSpace(name: CGColorSpace.sRGB),
           let ctx = CGContext(data: &rgba, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                               space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        }
        var bins = [[UInt32](repeating: 0, count: 16), [UInt32](repeating: 0, count: 16), [UInt32](repeating: 0, count: 16)]
        let sampleStride = max(1, w * h / 16384)
        for i in Swift.stride(from: 0, to: w * h, by: sampleStride) {
            let o = i * 4
            bins[0][Int(rgba[o]) * 16 / 256] += 1
            bins[1][Int(rgba[o + 1]) * 16 / 256] += 1
            bins[2][Int(rgba[o + 2]) * 16 / 256] += 1
        }
        var out = [UInt8](repeating: 0, count: 48)
        for c in 0..<3 {
            let peak = max(bins[c].max() ?? 1, 1)
            for b in 0..<16 {
                out[c * 16 + b] = UInt8(min(255, Int(Double(bins[c][b]) * 255 / Double(peak))))
            }
        }
        return out
    }
}

// MARK: - IOSurface upload

enum SurfaceUploader {
    /// 32-bit BGRA: the layout Core Animation accepts directly as `layer.contents`, and the same
    /// layout a decoded ImageIO CGImage already has, so the upload is a copy, not a conversion.
    static func makeSurface(pixelSize: CGSize) -> IOSurface? {
        IOSurfaceCreate([
            kIOSurfacePixelFormat: UInt32(0x42475241),  // 'BGRA'
            kIOSurfaceWidth: Int(pixelSize.width),
            kIOSurfaceHeight: Int(pixelSize.height),
            kIOSurfaceBytesPerElement: 4,
        ] as CFDictionary)
    }

    static func upload(_ image: CGImage, to surface: IOSurface) -> Bool {
        guard IOSurfaceLock(surface, [], nil) == kIOReturnSuccess else { return false }
        defer { _ = IOSurfaceUnlock(surface, [], nil) }
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let ctx = CGContext(data: IOSurfaceGetBaseAddress(surface),
                                  width: image.width, height: image.height,
                                  bitsPerComponent: 8, bytesPerRow: IOSurfaceGetBytesPerRow(surface),
                                  space: space,
                                  bitmapInfo: CGBitmapInfo.byteOrder32Little.rawValue
                                      | CGImageAlphaInfo.premultipliedFirst.rawValue) else { return false }
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return true
    }
}

// MARK: - Harness

struct Sample {
    var url: URL
    var jpeg: Data
    var fullSize: CGSize
}

/// Pull `count` CR3 files spread across the test games, read their full-resolution preview bytes
/// into memory once, so the decode measurements aren't measuring the disk.
func loadSamples(count: Int) throws -> [Sample] {
    let fm = FileManager.default
    let games = try fm.contentsOfDirectory(at: testPhotosRoot, includingPropertiesForKeys: nil)
        .filter { $0.hasDirectoryPath }
        .sorted { $0.lastPathComponent < $1.lastPathComponent }
    var urls: [URL] = []
    for game in games {
        let files = try fm.contentsOfDirectory(at: game, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension.lowercased() == "cr3" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        guard !files.isEmpty else { continue }
        let step = max(1, files.count / max(1, count / games.count + 1))
        for i in Swift.stride(from: 0, to: files.count, by: step) { urls.append(files[i]) }
        if urls.count >= count { break }
    }
    guard !urls.isEmpty else {
        throw NSError(domain: "spike", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "No CR3 files under \(testPhotosRoot.path). Set \(testPhotosEnv)."
        ])
    }
    return try urls.prefix(count).map { url in
        guard let full = try CR3.findEmbeddedJPEGs(in: url).first(where: { $0.pixelSize.width >= 4000 }) else {
            throw NSError(domain: "spike", code: 2, userInfo: [
                NSLocalizedDescriptionKey: "no full-resolution embedded preview in \(url.lastPathComponent)"
            ])
        }
        return Sample(url: url,
                      jpeg: try CR3.read(url, offset: full.offset, length: full.length),
                      fullSize: full.pixelSize)
    }
}

/// Time `body` over `items` with `workers` threads, each pinned to a performance core.
/// Returns the wall clock in seconds for the whole batch.
private final class WorkCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var index = 0
    func next(limit: Int) -> Int {
        lock.lock(); defer { lock.unlock() }
        guard index < limit else { return -1 }
        index += 1
        return index - 1
    }
}

func parallelTime<T>(_ items: [T], workers: Int, _ body: @escaping @Sendable (T) -> Void) -> TimeInterval {
    let counter = WorkCounter()
    let group = DispatchGroup()
    let start = DispatchTime.now()
    for _ in 0..<workers {
        DispatchQueue.global(qos: .userInitiated).async(group: group) {
            pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0)
            while true {
                let i = counter.next(limit: items.count)
                guard i >= 0 else { return }
                body(items[i])
            }
        }
    }
    group.wait()
    return Double(DispatchTime.now().uptimeNanoseconds - start.uptimeNanoseconds) / 1_000_000_000
}

extension CGSize {
    var megapixels: Double { Double(width * height) / 1e6 }
    var bytes: Int { Int(width * height * 4) }
    var text: String { "\(Int(width))x\(Int(height))" }
}

// MARK: - Display swap

/// Measures the real "photo on screen" latency: assign `layer.contents`, flush, and wait for the
/// next display callback. Runs on the main thread only (top-level code of the spike).
final class SwapProbe: NSObject {
    private var window: NSWindow!
    private var layer: CALayer!
    private var nextFrame: CFTimeInterval = 0
    private var frameIntervals: [CFTimeInterval] = []
    private var lastFrame: CFTimeInterval = 0
    private let target = ProbeTarget()

    final class ProbeTarget: NSObject {
        var onFrame: ((NSView) -> Void)?
        @objc func fired(_ view: NSView) { onFrame?(view) }
    }

    func run(surfaces: [IOSurface], cgImages: [CGImage], iterations: Int) -> [String: Double] {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        window = NSWindow(contentRect: CGRect(x: 100, y: 100, width: 900, height: 600),
                          styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        let view = NSView(frame: window.contentView!.bounds)
        view.wantsLayer = true
        view.layer?.backgroundColor = CGColor(red: 0.15, green: 0.15, blue: 0.15, alpha: 1)
        window.contentView = view
        window.orderFrontRegardless()
        app.activate()
        layer = CALayer()
        layer.contentsGravity = .resizeAspect
        view.layer?.addSublayer(layer)
        pump(0.5)

        let link = window.displayLink(target: target, selector: #selector(ProbeTarget.fired(_:)))
        link.add(to: .main, forMode: .common)
        target.onFrame = { [weak self] _ in
            guard let self else { return }
            let now = CACurrentMediaTime()
            if self.lastFrame > 0 { self.frameIntervals.append(now - self.lastFrame) }
            self.lastFrame = now
            self.nextFrame = now
        }
        pump(0.5)
        let intervals = frameIntervals.sorted()
        let medianFrameMs = intervals.isEmpty ? 0 : intervals[intervals.count / 2] * 1000

        var results: [String: Double] = ["display_interval_ms": medianFrameMs]
        results["display_hz"] = medianFrameMs > 0 ? 1000 / medianFrameMs : 0

        // IOSurface assigned straight to layer.contents: the real display path, no CGImage in between.
        measureFrameLatency(iterations: iterations, tag: "iosurface", results: &results) { [weak self] i in
            self?.layer.contents = surfaces[i % surfaces.count]
        }

        // Plain CGImage layer contents, for comparison.
        measureFrameLatency(iterations: iterations, tag: "cgimage", results: &results) { [weak self] i in
            self?.layer.contents = cgImages[i % cgImages.count]
        }

        link.invalidate()
        window.orderOut(nil)
        return results
    }

    private func measureFrameLatency(iterations: Int, tag: String,
                                     results: inout [String: Double],
                                     assign: (Int) -> Void) {
        var assigns: [Double] = []
        var frames: [Double] = []
        for i in 0..<iterations {
            nextFrame = 0
            let a = CACurrentMediaTime()
            assign(i)
            CATransaction.flush()
            let c = CACurrentMediaTime()
            pump(until: { self.nextFrame > 0 }, timeout: 0.25)
            let f = CACurrentMediaTime()
            assigns.append(c - a)
            frames.append(f - a)
        }
        assigns.sort(); frames.sort()
        results["\(tag)_assign_ms"] = assigns[assigns.count / 2] * 1000
        results["\(tag)_to_frame_ms"] = frames[frames.count / 2] * 1000
        results["\(tag)_to_frame_p95_ms"] = frames[Int(Double(frames.count - 1) * 0.95)] * 1000
        results["\(tag)_within_1_frame_pct"] = 100 * Double(frames.filter { $0 * 1000 <= medianFrame * 1.5 }.count) / Double(frames.count)
    }

    private var medianFrame: Double {
        let sorted = frameIntervals.sorted()
        return sorted.isEmpty ? 16.7 : sorted[sorted.count / 2] * 1000
    }

    private func pump(_ seconds: TimeInterval) {
        pump(until: { CACurrentMediaTime() >= 0 }, timeout: seconds)
    }

    private func pump(until condition: () -> Bool, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return }
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.001))
        }
    }
}

// MARK: - The run

note("""
Firstcut pipeline spike
  test photos: \(testPhotosRoot.path)
  machine:     \(perfCoreCount()) performance cores / \(ProcessInfo.processInfo.activeProcessorCount) logical, \
\(Int(ProcessInfo.processInfo.physicalMemory) / (1 << 30)) GB, macOS \(ProcessInfo.processInfo.operatingSystemVersionString)
""")

let sampleCount = quick ? 8 : 24
let samples: [Sample]
do {
    let t = Date()
    samples = try loadSamples(count: sampleCount)
    note("loaded \(samples.count) preview payloads in \(ms(elapsed(since: t))) "
        + "(\(samples.reduce(0) { $0 + $1.jpeg.count } / 1024) KB total, "
        + "\(samples.reduce(0) { $0 + Int($1.jpeg.count) } / samples.count / 1024) KB each)")
} catch {
    note("FAILED: \(error.localizedDescription)")
    exit(1)
}

// 1. Where the preview lives
note("\n## 1. CR3 embedded previews")
for s in samples.prefix(3) {
    let all = try! CR3.findEmbeddedJPEGs(in: s.url)
    let desc = all.map { "\($0.pixelSize.text) @\($0.offset)+\($0.length/1024)KB" }.joined(separator: ", ")
    note("  \(s.url.lastPathComponent): \(desc)")
}

// 2. Single-thread decode cost per mode
note("\n## 2. Single-thread decode cost (\(samples.count) images, warm cache)")
/// Keeps the optimiser from deleting decodes whose result nothing else reads. Printed at the end.
///
/// A `final class` rather than a top-level `var`: under Swift 6 a top-level `var` is implicitly
/// `@MainActor`, and it cannot then be mutated from a plain top-level func. A reference type with
/// `nonisolated(unsafe)` mutable state is the honest description -- this is a single-threaded
/// benchmark harness, and the value is read only by the code that wrote it.
final class Sink {
    nonisolated(unsafe) static var shared = Sink()
    nonisolated(unsafe) var pixels = 0
    nonisolated(unsafe) var sig = 0
}
for vp in viewportSizes {
    let maxPixel = Int(max(vp.px.width, vp.px.height))
    var rows: [String] = []
    let modes: [(String, () -> CGImage?)] = [
        ("full 6000x4000", { Decoder.full(samples[0].jpeg) }),
        ("subsample /4", { Decoder.subsampled(samples[0].jpeg, factor: 4) }),
        ("thumbnail DCT @\(maxPixel)", { Decoder.thumbnail(samples[0].jpeg, maxPixel: maxPixel) }),
        ("vImage Lanczos @\(maxPixel)", { Decoder.vImageDownscaled(samples[0].jpeg, to: maxPixel) }),
    ]
    for (name, run) in modes {
        // 5 timed repeats, median reported, so one cold-cache iteration cannot skew the table.
        var times: [Double] = []
        var img: CGImage?
        for _ in 0..<5 {
            let t = Date()
            img = run()
            times.append(elapsed(since: t) * 1000)
        }
        guard let img else { rows.append("    \(name)  FAILED"); continue }
        times.sort()
        Sink.shared.pixels &+= img.width &+ img.height
        rows.append(String(format: "    %-26@ %8.1f ms (median of 5, min %5.1f)  → %@", name as NSString,
                           times[2], times[0], "\(img.width)x\(img.height)" as NSString))
    }
    note("  viewport \(vp.name)  \(vp.px.text) px")
    for row in rows { note(row) }
}

// 3. Parallel throughput
note("\n## 3. Parallel decode throughput (workers pinned to P-cores, .userInitiated)")
for (modeName, maxPixel) in [("thumbnail DCT @3456 (T2 on a 16\" MBP)", 3456),
                             ("thumbnail DCT @256 (T0)", 256),
                             ("full 6000x4000 (T3)", 0),
                             ("vImage Lanczos @2880", -1)] {
    let rows = workersGrid(samples) { sample in
        let img: CGImage?
        switch maxPixel {
        case 0: img = Decoder.full(sample.jpeg)
        case -1: img = Decoder.vImageDownscaled(sample.jpeg, to: 2880)
        default: img = Decoder.thumbnail(sample.jpeg, maxPixel: maxPixel)
        }
        guard let img else { fatalError("decode failed") }
        Sink.shared.pixels &+= img.width &+ img.height
    }
    note("  \(modeName):")
    for row in rows { note(row) }
}

// 4. T0 thumbnails + sigs for the whole shoot
note("\n## 4. T0 (256 px) thumbnail + VisualSig, whole shoot projected")
func makeT0(_ sample: Sample) {
    guard let thumb = Decoder.thumbnail(sample.jpeg, maxPixel: 256) else { fatalError("no thumb") }
    let sig = VisualSigSpike.sig(thumb)
    Sink.shared.sig &+= Int(sig.dhash % 1021) + Int(sig.hist[24])
}
let t0Start = Date()
for s in samples { makeT0(s) }
let serialPer = Date().timeIntervalSince(t0Start) / Double(samples.count)
note(String(format: "  1 worker:   %6.2f ms/image → 1500 photos in %5.1f s, 2880 in %5.1f s",
             serialPer * 1000, serialPer * 1500, serialPer * 2880))
for workers in [2, 4, 6, 8] {
    let per = parallelTime(samples, workers: workers) { makeT0($0) } / Double(samples.count)
    note(String(format: "  %d workers:  %6.2f ms/image → 1500 photos in %5.1f s, 2880 in %5.1f s  (%.2fx)",
                 workers, per * 1000, per * 1500, per * 2880, serialPer / per))
}
if let first = Decoder.thumbnail(samples[0].jpeg, maxPixel: 256) {
    let thumbBytes = first.width * first.height * 4
    note(String(format: "  256 px thumbnail kept as decoded RGBA: %d bytes each → %.0f MB for 1500, %.0f MB for 2880",
                 thumbBytes, Double(thumbBytes * 1500) / 1e6, Double(thumbBytes * 2880) / 1e6))
}
note("  (sig sink: \(Sink.shared.sig))")

// 5. Reading preview bytes off disk (T1 refill)
note("\n## 5. Preview byte read from disk (T1 refill, no decode)")
let readBytes = samples.reduce(0) { $0 + $1.jpeg.count }
let tRead = Date()
var sink = 0
for s in samples {
    guard let full = try? CR3.findEmbeddedJPEGs(in: s.url).first(where: { $0.pixelSize.width >= 4000 }) else { continue }
    if let d = try? CR3.read(s.url, offset: full.offset, length: full.length) { sink += d.count }
}
let readElapsed = Date().timeIntervalSince(tRead)
note(String(format: "  %.1f ms/image for %.0f KB (%.0f MB/s)  [scan + read, worst case]",
             readElapsed / Double(samples.count) * 1000, Double(readBytes) / Double(samples.count) / 1024,
             Double(readBytes) / readElapsed / 1e6))
_ = sink

// 6 & 7. IOSurface + display swap
let viewport = viewportSizes[1]
let maxPixel = Int(max(viewport.px.width, viewport.px.height))
var decoded: [CGImage] = []
for s in samples.prefix(6) {
    if let img = Decoder.thumbnail(s.jpeg, maxPixel: maxPixel) { decoded.append(img) }
}
let memBefore = physFootprint()
var surfaces: [IOSurface] = []
let tAlloc = Date()
for img in decoded {
    if let surf = SurfaceUploader.makeSurface(pixelSize: CGSize(width: img.width, height: img.height)) {
        _ = SurfaceUploader.upload(img, to: surf)
        surfaces.append(surf)
    }
}
let allocElapsed = Date().timeIntervalSince(tAlloc)
let memAfter = physFootprint()

note("\n## 6. IOSurface allocation + upload")
let realPixelSize = decoded.first.map { CGSize(width: $0.width, height: $0.height) } ?? viewport.px
note(String(format: "  surface %dx%d BGRA: alloc+upload %.2f ms/image (%d surfaces), footprint +%.1f MB",
             Int(realPixelSize.width), Int(realPixelSize.height),
             allocElapsed / Double(max(1, decoded.count)) * 1000,
             surfaces.count, Double(memAfter - memBefore) / 1e6))
let uploadBytes = Double(decoded.reduce(0) { $0 + $1.width * $1.height * 4 })
let tUpload = Date()
for i in 0..<60 {
    let img = decoded[i % decoded.count]
    _ = SurfaceUploader.upload(img, to: surfaces[i % surfaces.count])
}
let uploadElapsed = Date().timeIntervalSince(tUpload)
note(String(format: "  pure upload: %.2f ms/image, %.1f GB/s",
             uploadElapsed / 60 * 1000, uploadBytes / uploadElapsed / 1e9))

if !skipDisplay {
    note("\n## 7. Display swap (real CALayer in a real window)")
    let probe = SwapProbe()
    var results = probe.run(surfaces: surfaces, cgImages: decoded, iterations: 40)
    for k in results.keys.sorted() {
        let v = results[k]!
        note(String(format: "  %-20@ %@", k as NSString,
                     k.hasSuffix("_ms") || k.hasSuffix("_hz") ? String(format: "%.2f", v) : String(format: "%.4f", v) as NSString))
    }
    _ = results
} else {
    note("\n## 7. Display swap: skipped (--no-display)")
}

// 8. Quality benchmark (task.md §7.2)
note("\n## 8. Quality: embedded preview vs CIRAWFilter, and DCT vs Lanczos")
qualityBenchmark(samples)

// 9. Memory per tier

note("\n## 9. Decoded pixel cost per tier")
for (name, px) in [("T2 @1280x800pt", CGSize(width: 2560, height: 1600)),
                   ("T2 @1728x1117pt", CGSize(width: 3456, height: 2234)),
                   ("T3 6000x4000", CGSize(width: 6000, height: 4000))] {
    let b = px.bytes
    note(String(format: "  %-16@ %@ px  %6.1f MB each  → %5d in a 6.4 GB budget",
                name as NSString, px.text as NSString, Double(b) / 1e6, Int(6.4e9 / Double(b))))
}

note("\n(sink: \(Sink.shared.pixels) — nothing above was optimised away)")
note("\ndone.")

/// Sweep worker counts for one decode mode and print ms/image, images/s, and the time projected
/// for a 1,500-photo shoot.
func workersGrid(_ samples: [Sample], decode: @escaping @Sendable (Sample) -> Void) -> [String] {
    var out = ["    workers   ms/image   images/s   1500 photos  speedup"]
    let baseline: Double = {
        let t = Date()
        for s in samples { decode(s) }
        return elapsed(since: t) / Double(samples.count)
    }()
    out.append(String(format: "    %7d  %8.1f  %8.1f  %8.1f s   1.00x  (baseline)",
                      1, baseline * 1000, 1 / baseline, baseline * 1500))
    for workers in [2, 3, 4, 5, 6, 8, 10] {
        let wall = parallelTime(samples, workers: workers, decode)
        let per = wall / Double(samples.count)
        out.append(String(format: "    %7d  %8.1f  %8.1f  %8.1f s   %.2fx",
                          workers, per * 1000, 1 / per, per * 1500, baseline / per))
    }
    return out
}


/// §7.2 benchmark. Answers three questions with numbers:
///   a) Does the embedded preview look like a RAW decode, at fit and at 100%?
///   b) Is ImageIO's DCT-scaled thumbnail good enough, or does it need the vImage Lanczos pass?
///   c) How expensive is `CIRAWFilter` (T4)?
func qualityBenchmark(_ samples: [Sample]) {
    // task.md §7.2 asks for CIRAWFilter specifically. Probe it, and say so out loud if we are
    // measuring something else instead of quietly substituting.
    let rawViaFilter: Bool = {
        guard let f = CIFilter(name: "CIRAWFilter") else { return false }
        f.setValue(samples[0].url, forKey: "imageURL")
        return f.outputImage != nil
    }()
    if rawViaFilter {
        note("  RAW arm: CIRAWFilter")
    } else {
        let versions = (CIFilter(name: "CIRAWFilter")?.value(forKey: "supportedDecoderVersions") as? [Any])?
            .map { String(describing: $0) }.joined(separator: ",") ?? "unavailable"
        note("  RAW arm: CIRAWFilter produced no output on this build (supportedDecoderVersions = [\(versions)]),")
        note("          so T4 is measured through ImageIO's full RAW decode of the CR3 instead.")
    }
    let isoTable = loadISOTable()
    func iso(_ url: URL) -> Int { isoTable[url.lastPathComponent] ?? 0 }
    let isoCutoff = 8000  // the genuinely grainy frames; at ISO 800 the grain argument is moot
    var highIso = samples.filter { iso($0.url) >= isoCutoff }
    if highIso.isEmpty { highIso = samples.sorted { iso($0.url) > iso($1.url) }.prefix(6).map { $0 } }
    note("  sample: \(highIso.count) frames, ISO \(highIso.map { iso($0.url) }.sorted())")

    for (label, maxPixel) in [("fit (2880 px longest edge)", 2880), ("100% (6000 px, native)", 6000)] {
        var rows: [String] = []
        var dctVsLanczos: [Double] = []
        var previewVsRaw: [Double] = []
        var dctSharp: [Double] = [], lanczosSharp: [Double] = [], rawSharp: [Double] = []
        var dctGrain: [Double] = [], lanczosGrain: [Double] = [], rawGrain: [Double] = []

        for s in highIso.prefix(6) {
            guard let dct = Decoder.thumbnail(s.jpeg, maxPixel: maxPixel) else { continue }

            // arm A: DCT-scaled embedded preview — what T2 will use
            let tA = Date()
            let dctA = Decoder.thumbnail(s.jpeg, maxPixel: maxPixel)!
            let tAms = Date().timeIntervalSince(tA) * 1000
            guard let dctPx = Pixels.raw(dctA, width: dct.width, height: dct.height) else { continue }

            // arm B: full preview decode + vImage Lanczos — the "high quality" alternative
            let tB = Date()
            let lanczos = maxPixel >= 6000 ? Decoder.full(s.jpeg) : Decoder.vImageDownscaled(s.jpeg, to: maxPixel)
            let tBms = Date().timeIntervalSince(tB) * 1000
            guard let lanczos, let lanczosPx = Pixels.raw(lanczos, width: dct.width, height: dct.height) else { continue }

            // arm C: the exact-RAW decode (T4). CIRAWFilter is unusable on this OS build, so this
            // is ImageIO's full RAW decode of the CR3, resampled to the comparison size.
            let tC = Date()
            var rawPx = Data()
            var tCms = 0.0
            if let rawFull = rawDecode(s.url) {
                tCms = Date().timeIntervalSince(tC) * 1000
                rawPx = Pixels.raw(rawFull, width: dct.width, height: dct.height) ?? Data()
            } else {
                tCms = Date().timeIntervalSince(tC) * 1000
            }

            let madDL = Pixels.meanAbsDiff(dctPx, lanczosPx)
            let madPR = rawPx.isEmpty ? -1 : Pixels.meanAbsDiff(lanczosPx, rawPx)
            dctSharp.append(Pixels.highFreqSharpness(dctPx, width: dct.width, height: dct.height))
            lanczosSharp.append(Pixels.highFreqSharpness(lanczosPx, width: dct.width, height: dct.height))
            dctGrain.append(Pixels.lumaStdDev(dctPx, width: dct.width, height: dct.height))
            lanczosGrain.append(Pixels.lumaStdDev(lanczosPx, width: dct.width, height: dct.height))
            if !rawPx.isEmpty {
                rawSharp.append(Pixels.highFreqSharpness(rawPx, width: dct.width, height: dct.height))
                rawGrain.append(Pixels.lumaStdDev(rawPx, width: dct.width, height: dct.height))
                previewVsRaw.append(madPR)
            }
            dctVsLanczos.append(madDL)

            rows.append(String(format: "    %-10@ ISO%-6d  DCT %6.1f ms  Lanczos %6.1f ms  RAW %7.1f ms  |  Δ(DCT,Lanczos) %5.1f  Δ(preview,RAW) %5.1f",
                                s.url.deletingPathExtension().lastPathComponent as NSString, iso(s.url),
                                tAms, tBms, tCms, madDL, madPR))
        }
        note("  \(label):")
        for r in rows { note(r) }
        guard !dctSharp.isEmpty else { continue }
        let avg = { (a: [Double]) -> Double in a.isEmpty ? 0 : a.reduce(0, +) / Double(a.count) }
        let dctS = avg(dctSharp), lancS = avg(lanczosSharp), rawS = avg(rawSharp)
        let dctG = avg(dctGrain), lancG = avg(lanczosGrain), rawG = avg(rawGrain)
        note(String(format: "    Δ(DCT,Lanczos) mean %.1f / 255   |   acutance vs RAW: DCT %.0f%%, Lanczos %.0f%%   |   grain (luma σ): DCT %.1f, Lanczos %.1f, RAW %.1f",
                     avg(dctVsLanczos), rawS > 0 ? 100 * dctS / rawS : 0, rawS > 0 ? 100 * lancS / rawS : 0,
                     dctG, lancG, rawG))
        if !previewVsRaw.isEmpty {
            note(String(format: "    Δ(embedded preview, RAW decode) mean %.1f / 255 over %d frames — that is the gap T4 exists to close",
                         avg(previewVsRaw), previewVsRaw.count))
        }
    }
}

/// ISO per file name, read from the committed exiftool dumps in `tests/fixtures/exiftool/`.
///
/// ImageIO does not expose ISO for `com.canon.cr3-raw-image` (it returns no EXIF ISO on this
/// machine), and the Canon MakerNote is core-meta's job to parse. The exiftool dumps are the
/// ground truth both of them will be checked against, so use them.
func loadISOTable() -> [String: Int] {
    var table: [String: Int] = [:]
    let fixtures = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // Spike/
        .deletingLastPathComponent()   // Pipeline/
        .deletingLastPathComponent()   // Sources/
        .deletingLastPathComponent()   // App/
        .deletingLastPathComponent()   // <repo root>
        .appendingPathComponent("tests/fixtures/exiftool")
    guard let names = try? FileManager.default.contentsOfDirectory(at: fixtures, includingPropertiesForKeys: nil) else {
        return table
    }
    for file in names where file.pathExtension == "json" {
        guard let data = try? Data(contentsOf: file),
              let rows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { continue }
        for row in rows {
            guard let name = row["FileName"] as? String else { continue }
            table[name] = (row["ISO"] as? Int) ?? 0
        }
    }
    return table
}

/// Full RAW decode of a file through ImageIO. This is the T4 path that actually works on this OS
/// build (see `qualityBenchmark`): ImageIO routes `com.canon.cr3-raw-image` through its own RAW
/// decoder, ~140 ms for a 24 MP frame.
func rawDecode(_ url: URL) -> CGImage? {
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(src, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
}
#else

// Compiled to nothing inside the app target. See the header comment for how to run the spike.
enum PipelineSpikePlaceholder {}

#endif
