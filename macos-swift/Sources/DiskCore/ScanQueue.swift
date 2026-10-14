import Foundation

/// A folder waiting to be listed.
struct ScanJob: Sendable {
    /// The folder's entry id. Its children take ids from the queue.
    let id: UInt32
    /// Full path, used to open the folder. Jobs hold paths, not handles, so the queue cannot run
    /// the process out of file descriptors.
    let path: String
}

/// Folder jobs for the workers, newest first, and the count that says when the scan is finished.
final class ScanQueue: @unchecked Sendable {
    private let condition = NSCondition()
    private let progress: ScanProgress
    // All guarded by `condition`.
    private var jobs: [ScanJob]
    /// Jobs queued or being listed. Zero means nothing is left to do.
    private var pending: Int
    private var nextID: UInt32 = 1

    init(root: ScanJob, progress: ScanProgress) {
        self.progress = progress
        jobs = [root]
        pending = 1
    }

    /// Waits for a job. Returns nil when the scan is finished or cancelled.
    func nextJob() -> ScanJob? {
        condition.lock()
        defer { condition.unlock() }
        while true {
            if progress.isCancelled {
                return nil
            }
            if let job = jobs.popLast() {
                return job
            }
            if pending == 0 {
                return nil
            }
            // A timeout, not just a signal, so a cancel is noticed even while every worker is waiting.
            _ = condition.wait(until: Date(timeIntervalSinceNow: 0.1))
        }
    }

    /// Retires the job just listed and queues the subfolders it found, under one lock.
    func complete(_ children: [ScanJob]) {
        condition.lock()
        defer { condition.unlock() }
        pending += children.count - 1
        jobs.append(contentsOf: children)
        if !children.isEmpty || pending == 0 {
            condition.broadcast()
        }
    }

    /// The next entry id. Ids come from one counter under the lock, so a child is always numbered
    /// after its parent.
    func allocateID() -> UInt32 {
        condition.lock()
        defer { condition.unlock() }
        let id = nextID
        nextID += 1
        return id
    }
}
