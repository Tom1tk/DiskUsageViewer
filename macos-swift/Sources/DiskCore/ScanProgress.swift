import Foundation

public enum ScanError: Error, Equatable, Sendable {
    case cancelled
    case failed(String)
}

/// Counters and the cancel flag, shared by the scan workers and the UI. Safe from any thread.
public final class ScanProgress: @unchecked Sendable {
    public struct Snapshot: Equatable, Sendable {
        public let folders: UInt64
        public let files: UInt64
        public let skipped: UInt64
        public let current: String
    }

    private let lock = NSLock()
    private let currentLock = NSLock()
    // Guarded by `lock`.
    private var folders: UInt64 = 0
    private var files: UInt64 = 0
    private var skipped: UInt64 = 0
    private var cancelled = false
    // Guarded by `currentLock`.
    private var current = ""

    public init() {}

    public func requestCancel() {
        lock.withLock { cancelled = true }
    }

    public var isCancelled: Bool {
        lock.withLock { cancelled }
    }

    public func snapshot() -> Snapshot {
        let counts = lock.withLock { (folders, files, skipped) }
        let path = currentLock.withLock { current }
        return Snapshot(folders: counts.0, files: counts.1, skipped: counts.2, current: path)
    }

    func add(folders added: UInt64, files addedFiles: UInt64, skipped addedSkipped: UInt64) {
        lock.withLock {
            folders += added
            files += addedFiles
            skipped += addedSkipped
        }
    }

    /// Never waits: a worker that finds the lock busy skips the update.
    func setCurrent(_ path: String) {
        guard currentLock.try() else {
            return
        }
        current = path
        currentLock.unlock()
    }
}
