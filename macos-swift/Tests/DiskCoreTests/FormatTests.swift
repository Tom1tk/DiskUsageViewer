import XCTest
@testable import DiskCore

final class FormatTests: XCTestCase {
    func testHumanSizeSmallValuesHaveNoDecimals() {
        XCTAssertEqual(humanSize(0), "0 B")
        XCTAssertEqual(humanSize(512), "512 B")
        XCTAssertEqual(humanSize(1023), "1023 B")
    }

    func testHumanSizeKBMBGBHaveOneDecimal() {
        XCTAssertEqual(humanSize(1024), "1.0 KB")
        XCTAssertEqual(humanSize(1536), "1.5 KB")
        XCTAssertEqual(humanSize(1024 * 1024 * 3 / 2), "1.5 MB")
        XCTAssertEqual(humanSize(724_000_000_000), "674.3 GB")
    }

    func testHumanSizeTBAndEdgeCases() {
        XCTAssertEqual(humanSize(1 << 40), "1.0 TB")
        XCTAssertEqual(humanSize(1280), "1.2 KB")
        XCTAssertEqual(humanSize(1792), "1.8 KB")
        XCTAssertEqual(humanSize(1_048_575), "1024.0 KB")
        XCTAssertEqual(humanSize((1 << 40) - 1), "1024.0 GB")
        XCTAssertEqual(humanSize(UInt64.max), "16777216.0 TB")
    }

    func testPercentOfWhole() {
        XCTAssertEqual(percent(1, of: 8), "12.5%")
        XCTAssertEqual(percent(0, of: 8), "0.0%")
    }

    func testPercentOfZeroWholeIsZero() {
        XCTAssertEqual(percent(5, of: 0), "0.0%")
    }

    func testGroupedInsertsCommas() {
        XCTAssertEqual(grouped(0), "0")
        XCTAssertEqual(grouped(999), "999")
        XCTAssertEqual(grouped(1000), "1,000")
        XCTAssertEqual(grouped(1_933_720), "1,933,720")
        XCTAssertEqual(grouped(UInt64.max), "18,446,744,073,709,551,615")
    }
}
