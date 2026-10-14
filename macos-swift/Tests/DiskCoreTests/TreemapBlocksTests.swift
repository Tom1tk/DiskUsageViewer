import XCTest
@testable import DiskCore

final class TreemapBlocksTests: XCTestCase {
    private let bounds = Rect(x: 0, y: 0, w: 400, h: 300)

    func testEmptyFolderHasNoBlocks() throws {
        let fx = try Fixture()
        try FileManager.default.createDirectory(atPath: fx.root + "/empty", withIntermediateDirectories: false)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())
        let empty = try XCTUnwrap(tree.sortedChildren(Tree.root).first)

        XCTAssertEqual(treemapBlocks(in: tree, folder: empty, bounds: bounds), [])
    }

    func testChildrenWithNoSizeGetNoBlock() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("a.bin", bytes: 10_000)
        _ = try fx.makeFile("empty.txt", bytes: 0)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        let blocks = treemapBlocks(in: tree, folder: Tree.root, bounds: bounds)

        XCTAssertEqual(blocks.map(\.name), ["a.bin"])
        XCTAssertEqual(blocks.first?.kind, .file)
    }

    func testFolderBlocksTakePaletteIndexesInSizeOrder() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("small/s.bin", bytes: 1_000)
        _ = try fx.makeFile("big/b.bin", bytes: 50_000)
        _ = try fx.makeFile("loose.bin", bytes: 1)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        let blocks = treemapBlocks(in: tree, folder: Tree.root, bounds: bounds)

        // "loose.bin" and "small" tie on size, so the name decides. Files do not use up palette slots.
        XCTAssertEqual(blocks.map(\.name), ["big", "loose.bin", "small"])
        XCTAssertEqual(blocks.map(\.kind), [
            .folder(paletteIndex: 0), .file, .folder(paletteIndex: 1),
        ])
    }

    func testChildrenPastTheLimitFoldIntoOneMoreBlockPlacedBySize() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("big.bin", bytes: 20_000)
        _ = try fx.makeFile("b.bin", bytes: 1)
        _ = try fx.makeFile("c.bin", bytes: 1)
        _ = try fx.makeFile("d.bin", bytes: 1)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())
        let oneBlock = fx.allocatedBytes(fx.root + "/b.bin")

        let blocks = treemapBlocks(in: tree, folder: Tree.root, bounds: bounds, limit: 2)

        XCTAssertEqual(blocks.map(\.name), ["big.bin", "2 more items", "b.bin"])
        XCTAssertEqual(blocks[1].kind, .more(count: 2))
        XCTAssertNil(blocks[1].id)
        XCTAssertEqual(blocks[1].size, oneBlock * 2)
    }

    func testBlocksTileTheBoundsWhenTheyAreAllDrawn() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("one.bin", bytes: 30_000)
        _ = try fx.makeFile("two.bin", bytes: 20_000)
        _ = try fx.makeFile("three.bin", bytes: 10_000)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        let blocks = treemapBlocks(in: tree, folder: Tree.root, bounds: bounds)

        let area = blocks.reduce(0.0) { $0 + $1.rect.w * $1.rect.h }
        XCTAssertEqual(area, bounds.w * bounds.h, accuracy: 1e-6)
    }

    func testZeroBoundsGiveNoBlocks() throws {
        let fx = try Fixture()
        _ = try fx.makeFile("a.bin", bytes: 10)
        let tree = try scanVolume(root: fx.root, progress: ScanProgress())

        XCTAssertEqual(treemapBlocks(in: tree, folder: Tree.root, bounds: Rect(x: 0, y: 0, w: 0, h: 100)), [])
    }
}
