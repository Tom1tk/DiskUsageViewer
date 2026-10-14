import Foundation

/// Listing buffer size in 8-byte words: 64 KB per `getattrlistbulk` call.
private let listingWords = 8 * 1024
/// Upper bound on scan threads. Past this, the disk is the limit, not the CPU.
private let maxThreads = 16
/// Folders between updates of the "current folder" text, so the UI shows a moving path.
private let currentPathInterval = 256

/// Scans `root` and everything below it, staying on the volume that holds it.
///
/// Folders on other volumes are skipped by full path. `mountPoints` defaults to every mount point
/// in the mount table, which keeps a Data volume reached through `/System/Volumes/Data` from being
/// counted twice. `withheld` folders are recorded but not opened; it defaults to `withheldFolders`
/// for the home folder. Symlinks are not followed. Folders that cannot be read or are withheld are
/// kept with size zero and counted in `Tree.skipped`.
public func scanVolume(
    root: String,
    progress: ScanProgress,
    mountPoints: Set<String>? = nil,
    withheld: Set<String>? = nil
) throws(ScanError) -> Tree {
    do {
        _ = try Directory.open(root)
    } catch {
        throw ScanError.failed("cannot open \(root): \(String(cString: strerror(error.code)))")
    }
    let excluded = mountPoints ?? Set(mountTable().map(\.path))
    let withheldPaths = withheld ?? withheldFolders(home: NSHomeDirectory())
    let queue = ScanQueue(root: ScanJob(id: Tree.root, path: root), progress: progress)
    let results = WorkerResults()
    let group = DispatchGroup()
    let threadCount = max(1, min(maxThreads, ProcessInfo.processInfo.activeProcessorCount))
    progress.add(folders: 1, files: 0, skipped: 0)
    for _ in 0..<threadCount {
        group.enter()
        Thread {
            let worker = ScanWorker(
                queue: queue, progress: progress, mountPoints: excluded, withheld: withheldPaths
            )
            results.add(worker.run())
            group.leave()
        }.start()
    }
    group.wait()
    if progress.isCancelled {
        throw ScanError.cancelled
    }
    return buildTree(merge([rootParts(name: root)] + results.all))
}

/// The root's record. The root is named by the path it was scanned from.
private func rootParts(name: String) -> TreeParts {
    let bytes = Array(name.utf8)
    let span = NameSpan(offset: 0, length: UInt32(bytes.count))
    return TreeParts(
        names: bytes,
        entries: [EntryRecord(id: Tree.root, parent: noParent, size: 0, isDir: true, name: span)]
    )
}

/// Joins the workers' records. Each name offset moves by the size of the arena before it.
private func merge(_ parts: [TreeParts]) -> TreeParts {
    var merged = TreeParts()
    for part in parts {
        let base = UInt32(merged.names.count)
        merged.names += part.names
        merged.entries += part.entries.map { entry in
            var moved = entry
            moved.name.offset += base
            return moved
        }
        merged.fileRefs += part.fileRefs
        merged.skipped += part.skipped
    }
    return merged
}

/// Each worker hands its records here when it finishes.
private final class WorkerResults: @unchecked Sendable {
    private let lock = NSLock()
    private var finished: [TreeParts] = []

    func add(_ parts: TreeParts) {
        lock.withLock { finished.append(parts) }
    }

    var all: [TreeParts] {
        lock.withLock { finished }
    }
}

/// One thread's share of the scan. Records stay here until the thread finishes, so no lock is needed
/// on the hot path.
private final class ScanWorker {
    private let queue: ScanQueue
    private let progress: ScanProgress
    private let mountPoints: Set<String>
    private let withheld: Set<String>
    private var parts = TreeParts()
    private var listed = 0

    init(queue: ScanQueue, progress: ScanProgress, mountPoints: Set<String>, withheld: Set<String>) {
        self.queue = queue
        self.progress = progress
        self.mountPoints = mountPoints
        self.withheld = withheld
    }

    func run() -> TreeParts {
        var buffer = [UInt64](repeating: 0, count: listingWords)
        while let job = queue.nextJob() {
            queue.complete(list(job, buffer: &buffer))
        }
        return parts
    }

    /// Records the entries of one folder and returns the subfolders still to visit.
    private func list(_ job: ScanJob, buffer: inout [UInt64]) -> [ScanJob] {
        listed += 1
        if listed % currentPathInterval == 0 {
            progress.setCurrent(job.path)
        }
        guard let directory = try? Directory.open(job.path) else {
            parts.skipped += 1
            progress.add(folders: 0, files: 0, skipped: 1)
            return []
        }
        var children: [ScanJob] = []
        var folders: UInt64 = 0
        var files: UInt64 = 0
        var withheldHere: UInt64 = 0
        let finished = directory.forEachEntry(into: &buffer, isCancelled: { progress.isCancelled }) { entry in
            guard !isDotEntry(entry.name) else {
                return
            }
            switch entry.kind {
            case ObjectType.symlink:
                return
            case ObjectType.directory:
                let path = childPath(job.path, entry.name)
                guard !mountPoints.contains(path) else {
                    return
                }
                let id = queue.allocateID()
                record(id: id, parent: job.id, name: entry.name, size: 0, isDir: true)
                folders += 1
                if withheld.contains(path) {
                    withheldHere += 1
                } else {
                    children.append(ScanJob(id: id, path: path))
                }
            default:
                let id = queue.allocateID()
                record(id: id, parent: job.id, name: entry.name, size: entry.allocatedSize, isDir: false)
                if entry.linkCount > 1 && entry.fileID != 0 {
                    parts.fileRefs.append(FileRef(volume: directory.volume, file: entry.fileID, entry: id))
                }
                files += 1
            }
        }
        let skippedHere = (finished ? 0 : 1) + withheldHere
        parts.skipped += skippedHere
        progress.add(folders: folders, files: files, skipped: skippedHere)
        return children
    }

    private func record(id: UInt32, parent: UInt32, name: UnsafeRawBufferPointer, size: UInt64, isDir: Bool) {
        let offset = UInt32(parts.names.count)
        parts.names.append(contentsOf: name)
        let span = NameSpan(offset: offset, length: UInt32(name.count))
        parts.entries.append(EntryRecord(id: id, parent: parent, size: size, isDir: isDir, name: span))
    }
}

/// `.` and `..`, which are not entries.
private func isDotEntry(_ name: UnsafeRawBufferPointer) -> Bool {
    name.count <= 2 && name.allSatisfy { $0 == UInt8(ascii: ".") }
}

/// The path of an entry inside `parent`. A parent of "/" is not doubled.
private func childPath(_ parent: String, _ name: UnsafeRawBufferPointer) -> String {
    let separator = parent.hasSuffix("/") ? "" : "/"
    return parent + separator + String(decoding: name, as: UTF8.self)
}
