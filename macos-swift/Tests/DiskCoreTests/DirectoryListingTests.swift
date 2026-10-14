import Darwin
import XCTest
@testable import DiskCore

final class DirectoryListingTests: XCTestCase {
    func testListingReportsFileIDsAndAllocatedSizes() throws {
        let fx = try Fixture()
        let file = try fx.makeFile("probe.bin", bytes: 9000)
        try FileManager.default.createDirectory(
            atPath: fx.root + "/nested", withIntermediateDirectories: false
        )
        var info = stat()
        XCTAssertEqual(lstat(file, &info), 0)

        let directory = try Directory.open(fx.root)
        var buffer = [UInt64](repeating: 0, count: 1024)
        // Copies the fields out: the name pointer is valid only inside the callback.
        var found: [String: (kind: UInt32, fileID: UInt64, linkCount: UInt32, allocatedSize: UInt64)] = [:]
        let finished = directory.forEachEntry(into: &buffer, isCancelled: { false }) { entry in
            let name = String(decoding: entry.name, as: UTF8.self)
            found[name] = (entry.kind, entry.fileID, entry.linkCount, entry.allocatedSize)
        }

        XCTAssertTrue(finished)
        XCTAssertEqual(Set(found.keys), ["probe.bin", "nested"])
        let probe = try XCTUnwrap(found["probe.bin"])
        XCTAssertEqual(probe.fileID, UInt64(info.st_ino))
        XCTAssertEqual(probe.allocatedSize, UInt64(info.st_blocks) * 512)
        XCTAssertEqual(probe.linkCount, 1)
        XCTAssertNotEqual(probe.kind, ObjectType.directory)
        XCTAssertEqual(try XCTUnwrap(found["nested"]).kind, ObjectType.directory)
    }

    func testOpeningAMissingPathThrowsTheErrno() {
        XCTAssertThrowsError(try Directory.open("/nonexistent-disk-core-test")) { error in
            XCTAssertEqual((error as? OpenFailure)?.code, ENOENT)
        }
    }
}
