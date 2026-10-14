import Darwin
import XCTest
@testable import DiskCore

final class VolumesTests: XCTestCase {
    func testStartupDiskIsListedWithConsistentCapacity() throws {
        let startup = try XCTUnwrap(listVolumes().first { $0.path == "/" })

        XCTAssertFalse(startup.name.isEmpty)
        XCTAssertGreaterThan(startup.total, 0)
        XCTAssertLessThanOrEqual(startup.free, startup.total)
        XCTAssertEqual(startup.used, startup.total - startup.free)
    }

    func testOnlyLocalBrowsableRootAndVolumesPathsArePicked() {
        let mounts = [
            MountInfo(path: "/", fileSystem: "apfs", flags: UInt32(MNT_LOCAL)),
            MountInfo(path: "/System/Volumes/Data", fileSystem: "apfs", flags: UInt32(MNT_LOCAL)),
            MountInfo(path: "/Volumes/Recovery", fileSystem: "apfs", flags: UInt32(MNT_LOCAL | MNT_DONTBROWSE)),
            MountInfo(path: "/Volumes/Share", fileSystem: "smbfs", flags: 0),
        ]

        XCTAssertEqual(volumes(from: mounts).map(\.path), ["/"])
    }

    func testUsedIsZeroWhenFreeExceedsTotal() {
        let volume = Volume(path: "/x", name: "x", total: 10, free: 20)

        XCTAssertEqual(volume.used, 0)
    }
}
