import Darwin
import XCTest
@testable import DiskCore

final class ScannerTests: XCTestCase {
    func testCountsFilesFoldersAndAllocatedBytes() throws {
        let fx = try Fixture()
        let big = try fx.makeFile("sub/a.bin", bytes: 5000)
        let small = try fx.makeFile("top.bin", bytes: 1)
        let empty = try fx.makeFile("empty.txt", bytes: 0)

        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(tree.folders, 2)
        XCTAssertEqual(tree.files, 3)
        XCTAssertEqual(tree.skipped, 0)
        let expected = [big, small, empty].reduce(UInt64(0)) { $0 + fx.allocatedBytes($1) }
        XCTAssertEqual(tree.size(Tree.root), expected)
        let sub = try XCTUnwrap(tree.sortedChildren(Tree.root).first { tree.name($0) == "sub" })
        XCTAssertEqual(tree.size(sub), fx.allocatedBytes(big))
        XCTAssertGreaterThanOrEqual(fx.allocatedBytes(big), 5000)
        XCTAssertEqual(tree.name(Tree.root), fx.root)
        XCTAssertEqual(tree.path(sub), fx.root + "/sub")
    }

    func testHardLinkIsCountedOnce() throws {
        let fx = try Fixture()
        let original = try fx.makeFile("data.bin", bytes: 20_000)
        XCTAssertEqual(link(original, fx.root + "/alias.bin"), 0)

        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(tree.size(Tree.root), fx.allocatedBytes(original))
        XCTAssertEqual(tree.files, 2)
    }

    func testSymlinksAreNotFollowed() throws {
        let fx = try Fixture()
        let keep = try fx.makeFile("keep.bin", bytes: 1)
        let outside = fx.base.appendingPathComponent("outside.bin").path
        FileManager.default.createFile(atPath: outside, contents: Data(count: 50_000))
        try FileManager.default.createSymbolicLink(atPath: fx.root + "/file-link", withDestinationPath: outside)
        // A link back to the parent would loop forever if it were followed.
        try FileManager.default.createSymbolicLink(atPath: fx.root + "/dir-link", withDestinationPath: fx.base.path)

        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(tree.files, 1)
        XCTAssertEqual(tree.folders, 1)
        XCTAssertEqual(tree.skipped, 0)
        XCTAssertEqual(tree.size(Tree.root), fx.allocatedBytes(keep))
    }

    func testUnreadableFolderIsKeptWithZeroSizeAndCountedAsSkipped() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("locked/secret.bin", bytes: 4096)
        let locked = fx.root + "/locked"
        XCTAssertEqual(chmod(locked, 0), 0)
        defer { _ = chmod(locked, 0o755) }

        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(tree.skipped, 1)
        XCTAssertEqual(tree.folders, 2)
        XCTAssertEqual(tree.files, 0)
        let folder = try XCTUnwrap(tree.sortedChildren(Tree.root).first)
        XCTAssertEqual(tree.size(folder), 0)
    }

    func testMountPointsAreNotEntered() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("mounted/inner.bin", bytes: 10_000)
        let plain = try fx.makeFile("plain.bin", bytes: 1)

        let tree = try scanVolume(
            root: fx.root,
            progress: ScanProgress(),
            mountPoints: [fx.root + "/mounted"]
        )

        XCTAssertEqual(tree.folders, 1)
        XCTAssertEqual(tree.files, 1)
        XCTAssertEqual(tree.skipped, 0)
        XCTAssertEqual(tree.size(Tree.root), fx.allocatedBytes(plain))
    }

    func testWithheldFolderIsRecordedButNotOpened() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("private/secret.bin", bytes: 4096)
        let plain = try fx.makeFile("plain.bin", bytes: 1)
        let progress = ScanProgress()

        let tree = try scanVolume(
            root: fx.root,
            progress: progress,
            mountPoints: [],
            withheld: [fx.root + "/private"]
        )

        XCTAssertEqual(tree.folders, 2)
        XCTAssertEqual(tree.files, 1)
        XCTAssertEqual(tree.skipped, 1)
        XCTAssertEqual(progress.snapshot().skipped, 1)
        XCTAssertEqual(tree.size(Tree.root), fx.allocatedBytes(plain))
        let folder = try XCTUnwrap(tree.sortedChildren(Tree.root).first { tree.name($0) == "private" })
        XCTAssertTrue(tree.isDirectory(folder))
        XCTAssertEqual(tree.size(folder), 0)
        XCTAssertEqual(tree.childCount(folder), 0)
    }

    func testDefaultWithheldListCoversProtectedHomeFoldersAndAutomounts() {
        let withheld = withheldFolders(home: "/Users/someone")
        XCTAssertTrue(withheld.contains("/Users/someone/Desktop"))
        XCTAssertTrue(withheld.contains("/Users/someone/Library/Mail"))
        XCTAssertTrue(withheld.contains("/home"))
        XCTAssertFalse(withheld.contains("/Users/someone/Projects"))
    }

    func testMissingRootThrowsFailed() throws {
        let fx = try Fixture()
        XCTAssertThrowsError(try scanVolume(root: fx.root + "/missing", progress: ScanProgress())) { error in
            guard case ScanError.failed(let message) = error else {
                return XCTFail("unexpected error \(error)")
            }
            XCTAssertTrue(message.hasPrefix("cannot open"), message)
        }
    }

    func testCancelledScanThrowsCancelled() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("a.bin", bytes: 1)
        let progress = ScanProgress()
        progress.requestCancel()

        XCTAssertThrowsError(try scanVolume(root: fx.root, progress: progress)) { error in
            XCTAssertEqual(error as? ScanError, .cancelled)
        }
    }

    func testParallelScanCountsEveryEntry() throws {
        let fx = try Fixture()
        for folder in 0..<40 {
            for file in 0..<25 {
                _ = try fx.makeFile("d\(folder)/f\(file).bin", bytes: 100)
            }
        }
        let perFolder = fx.allocatedBytes(fx.root + "/d0/f0.bin") * 25

        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(tree.folders, 41)
        XCTAssertEqual(tree.files, 1000)
        XCTAssertEqual(tree.skipped, 0)
        let children = tree.sortedChildren(Tree.root)
        XCTAssertEqual(children.count, 40)
        for child in children {
            XCTAssertEqual(tree.size(child), perFolder)
        }
        XCTAssertEqual(tree.size(Tree.root), perFolder * 40)
    }
}
