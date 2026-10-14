import AppKit
import DiskCore
import Foundation
import Observation

/// Shown before any scan. Same wording as the Windows app, naming macOS.
private let startText = "Pick a drive, then press Scan. Folders macOS will not let you read are skipped."
/// Characters of the current folder's path shown at the end of the status line.
private let pathTailLength = 70
/// How often the status line is refreshed while a scan runs.
private let refreshInterval = Duration.milliseconds(150)

/// The window's state: the chosen drive, the scan in progress or finished, and the selected entry.
@MainActor
@Observable
final class ScanModel {
    let volumes: [Volume]
    var selectedVolume: String
    private(set) var tree: Tree?
    /// Bumped each time `tree` changes, so the views know to reload.
    private(set) var generation = 0
    var selection: UInt32?
    private(set) var status: String
    private(set) var isScanning = false

    @ObservationIgnored private var progress: ScanProgress?
    @ObservationIgnored private var refreshTask: Task<Void, Never>?
    @ObservationIgnored private var scanStarted = Date()
    @ObservationIgnored private var autostartPending: Bool

    init(autostart: Bool) {
        let volumes = listVolumes()
        self.volumes = volumes
        selectedVolume = volumes.first { $0.path == "/" }?.path ?? volumes.first?.path ?? "/"
        status = volumes.isEmpty ? "No drives found." : startText
        autostartPending = autostart
    }

    /// The folder the treemap shows: the selected folder, or the folder that holds the selected file.
    var focus: UInt32 {
        guard let tree, let selection else {
            return Tree.root
        }
        if tree.isDirectory(selection) {
            return selection
        }
        return tree.parent(selection) ?? Tree.root
    }

    /// `--autostart`: starts the scan once, when the window first appears.
    func startAutostartIfNeeded() {
        guard autostartPending else {
            return
        }
        autostartPending = false
        startScan()
    }

    func startScan() {
        guard !isScanning, volumes.contains(where: { $0.path == selectedVolume }) else {
            return
        }
        let root = selectedVolume
        let progress = ScanProgress()
        self.progress = progress
        isScanning = true
        scanStarted = Date()
        setTree(nil)
        selection = nil
        status = scanningText(root: root, progress: progress)
        startRefreshing(root: root)
        Task {
            let outcome = await Task.detached { runScan(root: root, progress: progress) }.value
            finishScan(outcome)
        }
    }

    func cancelScan() {
        guard isScanning, let progress else {
            return
        }
        progress.requestCancel()
        status = "Cancelling..."
    }

    /// Shows an entry in Finder, with it selected in its folder.
    func reveal(_ id: UInt32) {
        guard let tree else {
            return
        }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: tree.path(id))])
    }

    func revealSelection() {
        guard let selection else {
            return
        }
        reveal(selection)
    }

    private func startRefreshing(root: String) {
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: refreshInterval)
                guard !Task.isCancelled else {
                    return
                }
                self?.refreshScanningStatus(root: root)
            }
        }
    }

    /// Leaves "Cancelling..." on screen once a cancel has been asked for.
    private func refreshScanningStatus(root: String) {
        guard let progress, !progress.isCancelled else {
            return
        }
        status = scanningText(root: root, progress: progress)
    }

    private func finishScan(_ outcome: Result<Tree, ScanError>) {
        refreshTask?.cancel()
        refreshTask = nil
        progress = nil
        isScanning = false
        switch outcome {
        case .success(let scanned):
            setTree(scanned)
            selection = Tree.root
            status = doneText(scanned, seconds: Date().timeIntervalSince(scanStarted))
        case .failure(.cancelled):
            status = "Scan cancelled."
        case .failure(.failed(let message)):
            status = "Scan failed: \(message)"
        }
    }

    private func setTree(_ newTree: Tree?) {
        tree = newTree
        generation += 1
    }

    private func scanningText(root: String, progress: ScanProgress) -> String {
        let snapshot = progress.snapshot()
        let counts = "\(grouped(snapshot.files)) files, \(grouped(snapshot.folders)) folders, "
            + "\(grouped(snapshot.skipped)) skipped"
        return "Scanning \(root)   \(counts)   \(tail(snapshot.current, pathTailLength))"
    }

    private func doneText(_ scanned: Tree, seconds: Double) -> String {
        "Scanned \(humanSize(scanned.size(Tree.root))) on disk in \(grouped(scanned.files)) files and "
            + "\(grouped(scanned.folders)) folders (\(Int(seconds.rounded())) s). "
            + "\(grouped(scanned.skipped)) items could not be read and were skipped."
    }
}

/// Runs one scan away from the main thread. The result is a value, so it can cross back.
private func runScan(root: String, progress: ScanProgress) -> Result<Tree, ScanError> {
    do {
        return .success(try scanVolume(root: root, progress: progress))
    } catch {
        return .failure(error)
    }
}

/// The last `length` characters of `text`, with "..." in front when something was cut.
private func tail(_ text: String, _ length: Int) -> String {
    guard text.count > length else {
        return text
    }
    return "...\(text.suffix(length))"
}
