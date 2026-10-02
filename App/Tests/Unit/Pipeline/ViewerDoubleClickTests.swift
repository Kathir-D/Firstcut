// Owner: pipeline + app-logic.
//
// The loupe's double-click: hop a whole batch, in the direction of the click (owner request,
// 2026-10-02).
//
// ## Why this is a host-level test and not a model test
//
// The model's half — `jumpBatch` moving exactly one batch in the right direction — is asserted in
// `SessionTests`. What this file pins is the half that has no test otherwise: that a real
// double-click on a real `CGImageViewerHost` reports the right **direction**, and that the clicks it
// must ignore really are ignored. The environment's wiring (the `onDoubleClick` closure that turns
// that into `state.send`) cannot be tested here because `AppEnvironment` is a live singleton, so
// this is deliberately the deepest layer that *can* be driven.
//
// The direction is the whole point of the feature: a double-click on the left of the picture means
// "back", and getting it wrong sends the photographer into a batch they did not ask for — the same
// class of bug as rotating a frame twice.

import AppKit
import CoreGraphics
import Foundation
import Testing

@testable import Firstcut

@MainActor
@Suite("The loupe's double-click")
struct ViewerDoubleClickTests {
    /// A host with a bitmap in it, wide enough that "left of centre" and "right of centre" are
    /// unambiguous.
    private func loadedHost(directionSink: @escaping (Int) -> Void) -> (CGImageViewerHost, CGRect) {
        let host = CGImageViewerHost(images: TestImageSource())
        host.onDoubleClick = directionSink
        host.frame = CGRect(x: 0, y: 0, width: 400, height: 300)
        // A flat image is enough: the click handling is about geometry, not about pixels.
        let context = CGContext(
            data: nil, width: 100, height: 80, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue)!
        context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.7, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 100, height: 80))
        host.setTestImage(context.makeImage()!)
        // Let the layout pass run so `imageRect` has a real frame to test against.
        host.layoutSubtreeIfNeeded()
        return (host, host.testImageRect)
    }

    /// Synthesised mouse events, because `NSEvent` cannot be constructed with a click count that
    /// the host would believe any other way.
    private func click(
        _ host: CGImageViewerHost, at point: CGPoint, clickCount: Int
    ) {
        let window = NSWindow(
            contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        let down = NSEvent.mouseEvent(
            with: .leftMouseDown, location: point, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, eventNumber: Int.random(in: 1...1_000_000), clickCount: clickCount,
            pressure: 1)!
        host.mouseDown(with: down)
        let up = NSEvent.mouseEvent(
            with: .leftMouseUp, location: point, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime + 0.01, windowNumber: window.windowNumber,
            context: nil, eventNumber: Int.random(in: 1...1_000_000), clickCount: clickCount,
            pressure: 0)!
        host.mouseUp(with: up)
    }

    @Test("A double-click on the right half of the picture means 'forward'")
    func rightHalfIsForward() {
        var seen: [Int] = []
        let (host, rect) = loadedHost { seen.append($0) }
        click(host, at: CGPoint(x: rect.maxX - 8, y: rect.midY), clickCount: 2)
        #expect(seen == [1], "the right half must be +1, got \(seen)")
    }

    @Test("A double-click on the left half of the picture means 'back'")
    func leftHalfIsBack() {
        var seen: [Int] = []
        let (host, rect) = loadedHost { seen.append($0) }
        click(host, at: CGPoint(x: rect.minX + 8, y: rect.midY), clickCount: 2)
        #expect(seen == [-1], "the left half must be -1, got \(seen)")
    }

    /// A single click is the existing zoom gesture and must stay that way. Reporting every click as
    /// a batch jump would make clicking to inspect a frame navigate away from it.
    @Test("A single click is the zoom gesture, not a batch jump")
    func singleClickDoesNotJump() {
        var seen: [Int] = []
        let (host, rect) = loadedHost { seen.append($0) }
        click(host, at: CGPoint(x: rect.midX, y: rect.midY), clickCount: 1)
        #expect(seen.isEmpty, "one click zoomed instead: \(seen)")
    }

    /// A drag is a pan, not a click. `mouseUp` after a drag must not be read as a double-click
    /// either, or a two-frame pan across a batch boundary would jump.
    @Test("A drag is a pan, not a double-click")
    func dragDoesNotJump() {
        var seen: [Int] = []
        let (host, rect) = loadedHost { seen.append($0) }
        let window = NSWindow(
            contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        let start = CGPoint(x: rect.minX + 10, y: rect.midY)
        func event(_ type: NSEvent.EventType, _ point: CGPoint, _ clicks: Int, _ pressure: Float)
            -> NSEvent
        {
            NSEvent.mouseEvent(
                with: type, location: point, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: Int.random(in: 1...1_000_000), clickCount: clicks,
                pressure: pressure)!
        }
        host.mouseDown(with: event(.leftMouseDown, start, 1, 1))
        // Past the drag slop, so `didDrag` latches.
        host.mouseDragged(with: event(.leftMouseDragged, CGPoint(x: start.x + 80, y: start.y), 1, 1))
        host.mouseUp(with: event(.leftMouseUp, CGPoint(x: start.x + 80, y: start.y), 2, 0))
        #expect(seen.isEmpty, "a drag must not be read as a double-click: \(seen)")
    }

    /// A click outside the photograph is a click on the background, which zooms to fit and has
    /// nothing to do with batches.
    @Test("A click on the background outside the picture does not jump")
    func backgroundClickDoesNotJump() {
        var seen: [Int] = []
        let (host, rect) = loadedHost { seen.append($0) }
        // Well outside the image rect but inside the view.
        click(host, at: CGPoint(x: 2, y: 2), clickCount: 2)
        #expect(seen.isEmpty, "the background is not the picture: \(seen)")
        #expect(rect.minX >= 0, "the image rect is inside the view for this test to mean anything")
    }
}

/// A `CullImageSource` that hands over one flat bitmap, so the host has an image without a decoder.
@MainActor
private final class TestImageSource: CullImageSource {
    var thumbnailProgress: Double { 1 }
    func thumbnail(for id: PhotoID, size: CGSize) -> CGImage? { nil }
    func displayImage(for id: PhotoID, minimumLongestEdge: Int) -> CGImage? { nil }
    func histogram(for id: PhotoID) -> CullHistogram? { nil }
}
