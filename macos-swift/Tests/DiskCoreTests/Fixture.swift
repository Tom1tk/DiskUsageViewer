import Darwin
import Foundation

/// A temporary folder for one test. `root` is a folder inside it, and `base` is removed on deinit.
final class Fixture {
    let base: URL

    var root: String {
        base.appendingPathComponent("root").path
    }

    init() throws {
        base = FileManager.default.temporaryDirectory
            .appendingPathComponent("DiskCoreTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: base.appendingPathComponent("root", isDirectory: true),
            withIntermediateDirectories: true
        )
    }

    deinit {
        try? FileManager.default.removeItem(at: base)
    }

    /// Writes a file of `bytes` zeros at `relative` (below `root`), creating folders as needed.
    func makeFile(_ relative: String, bytes: Int) throws -> String {
        let url = URL(fileURLWithPath: root).appendingPathComponent(relative)
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Data(count: bytes).write(to: url)
        return url.path
    }

    /// Bytes the file occupies on disk, as `stat` reports them.
    func allocatedBytes(_ path: String) -> UInt64 {
        var info = stat()
        guard lstat(path, &info) == 0 else {
            return 0
        }
        return UInt64(info.st_blocks) * 512
    }
}
