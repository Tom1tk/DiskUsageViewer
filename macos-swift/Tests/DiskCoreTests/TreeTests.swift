import XCTest
@testable import DiskCore

final class TreeTests: XCTestCase {
    /// Builds parts from rows given in id order. Row 0 must be the root.
    private func parts(
        _ rows: [(name: String, parent: UInt32, size: UInt64, isDir: Bool)],
        refs: [FileRef] = []
    ) -> TreeParts {
        var names: [UInt8] = []
        var entries: [EntryRecord] = []
        for (index, row) in rows.enumerated() {
            let bytes = Array(row.name.utf8)
            let span = NameSpan(offset: UInt32(names.count), length: UInt32(bytes.count))
            names += bytes
            entries.append(EntryRecord(
                id: UInt32(index), parent: row.parent, size: row.size, isDir: row.isDir, name: span
            ))
        }
        return TreeParts(names: names, entries: entries, fileRefs: refs)
    }

    /// The sample tree used by most tests:
    ///   /            (root)
    ///   ├── Users/
    ///   │   ├── a.txt    100
    ///   │   └── B.txt    100 (hard link to 5, so it keeps the size)
    ///   ├── big.bin      300
    ///   └── b-link.txt   100 (hard link to 4, counted at 4 instead)
    private func sampleTree() -> Tree {
        buildTree(parts([
            ("/", noParent, 0, true),
            ("Users", 0, 0, true),
            ("big.bin", 0, 300, false),
            ("a.txt", 1, 100, false),
            ("B.txt", 1, 100, false),
            ("b-link.txt", 0, 100, false),
        ], refs: [
            FileRef(volume: 1, file: 9, entry: 4),
            FileRef(volume: 1, file: 9, entry: 5),
        ]))
    }

    func testCountsSplitFilesAndFolders() {
        let tree = sampleTree()
        XCTAssertEqual(tree.folders, 2)
        XCTAssertEqual(tree.files, 4)
        XCTAssertEqual(tree.skipped, 0)
    }

    func testFolderSizesAreTotalsOfTheirContents() {
        let tree = sampleTree()
        XCTAssertEqual(tree.size(1), 200)
        XCTAssertEqual(tree.size(Tree.root), 500)
    }

    func testHardLinkKeepsSizeOnlyAtLowestEntry() {
        let tree = sampleTree()
        XCTAssertEqual(tree.size(4), 100)
        XCTAssertEqual(tree.size(5), 0)
    }

    func testChildrenAreSortedBySizeThenNameIgnoringCase() {
        let tree = sampleTree()
        XCTAssertEqual(tree.sortedChildren(Tree.root), [2, 1, 5])
        XCTAssertEqual(tree.sortedChildren(1), [3, 4])
    }

    func testChildCountAndParent() {
        let tree = sampleTree()
        XCTAssertEqual(tree.childCount(Tree.root), 3)
        XCTAssertEqual(tree.childCount(2), 0)
        XCTAssertNil(tree.parent(Tree.root))
        XCTAssertEqual(tree.parent(3), 1)
    }

    func testPathsUseSlashAndDoNotDoubleTheRoot() {
        let tree = sampleTree()
        XCTAssertEqual(tree.path(Tree.root), "/")
        XCTAssertEqual(tree.path(1), "/Users")
        XCTAssertEqual(tree.path(3), "/Users/a.txt")
    }

    func testNamesAreReadFromTheArena() {
        let tree = sampleTree()
        XCTAssertEqual(tree.name(Tree.root), "/")
        XCTAssertEqual(tree.name(4), "B.txt")
        XCTAssertTrue(tree.isDirectory(1))
        XCTAssertFalse(tree.isDirectory(2))
    }
}
