import XCTest
@testable import DiskCore

final class TreemapTests: XCTestCase {
    private let bounds = Rect(x: 10, y: 20, w: 300, h: 200)
    private let sizes: [Double] = [
        100, 80, 60, 40, 30, 20, 15, 10, 8, 5, 4, 3, 2, 2, 1, 1, 1, 1, 1, 1,
    ]

    private func area(_ r: Rect) -> Double {
        r.w * r.h
    }

    private func overlap(_ a: Rect, _ b: Rect) -> Double {
        let w = min(a.x + a.w, b.x + b.w) - max(a.x, b.x)
        let h = min(a.y + a.h, b.y + b.h) - max(a.y, b.y)
        return max(w, 0) * max(h, 0)
    }

    func testEmptyInputGivesEmptyOutput() {
        XCTAssertEqual(layoutTreemap([], in: bounds), [])
    }

    func testZeroTotalGivesZeroRects() {
        XCTAssertEqual(layoutTreemap([0, 0, 0], in: bounds), Array(repeating: .zero, count: 3))
    }

    func testZeroBoundsGivesZeroRects() {
        let flat = Rect(x: 10, y: 20, w: 0, h: 200)
        XCTAssertEqual(layoutTreemap([3, 1], in: flat), Array(repeating: .zero, count: 2))
    }

    func testRectsTileTheBounds() {
        let rects = layoutTreemap(sizes, in: bounds)
        let sum = rects.reduce(0) { $0 + area($1) }
        XCTAssertEqual(sum, area(bounds), accuracy: area(bounds) * 1e-6)
        for r in rects {
            XCTAssertGreaterThanOrEqual(r.x, bounds.x - 1e-9)
            XCTAssertLessThanOrEqual(r.x + r.w, bounds.x + bounds.w + 1e-9)
            XCTAssertGreaterThanOrEqual(r.y, bounds.y - 1e-9)
            XCTAssertLessThanOrEqual(r.y + r.h, bounds.y + bounds.h + 1e-9)
        }
    }

    func testRectsDoNotOverlap() {
        let rects = layoutTreemap(sizes, in: bounds)
        for (i, a) in rects.enumerated() {
            for b in rects[(i + 1)...] {
                XCTAssertLessThan(overlap(a, b), 1e-9)
            }
        }
    }

    func testAreasAreProportionalToSizes() {
        let rects = layoutTreemap(sizes, in: bounds)
        let total = sizes.reduce(0, +)
        for (r, size) in zip(rects, sizes) {
            let expected = size / total * area(bounds)
            XCTAssertEqual(area(r), expected, accuracy: expected * 1e-6)
        }
    }

    func testAspectRatioStaysBelowFour() {
        for r in layoutTreemap(sizes, in: bounds) {
            XCTAssertLessThan(max(r.w / r.h, r.h / r.w), 4.0, "\(r)")
        }
    }

    func testMatchesReferenceLayout() {
        let expected: [(x: Double, y: Double, w: Double, h: Double)] = [
            (0.0, 0.0, 3.0, 2.0),
            (0.0, 2.0, 3.0, 2.0),
            (3.0, 0.0, 1.7142857142857142, 2.3333333333333335),
            (4.714285714285714, 0.0, 1.2857142857142856, 2.3333333333333335),
            (3.0, 2.3333333333333335, 1.2000000000000002, 1.6666666666666665),
            (4.2, 2.3333333333333335, 1.2000000000000002, 1.6666666666666665),
            (5.4, 2.3333333333333335, 0.5999999999999996, 1.6666666666666676),
        ]
        let rects = layoutTreemap(
            [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0],
            in: Rect(x: 0, y: 0, w: 6, h: 4)
        )
        XCTAssertEqual(rects.count, expected.count)
        for (r, e) in zip(rects, expected) {
            XCTAssertEqual(r.x, e.x, accuracy: 1e-9, "\(r)")
            XCTAssertEqual(r.y, e.y, accuracy: 1e-9, "\(r)")
            XCTAssertEqual(r.w, e.w, accuracy: 1e-9, "\(r)")
            XCTAssertEqual(r.h, e.h, accuracy: 1e-9, "\(r)")
        }
    }
}
