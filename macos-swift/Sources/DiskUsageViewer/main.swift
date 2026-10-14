import DiskCore
import Foundation

/// `--scan <path>`: scans without a window and prints a summary. Returns the exit status.
func runHeadless(path: String) -> Int32 {
    let progress = ScanProgress()
    let started = Date()
    do {
        let tree = try scanVolume(root: path, progress: progress)
        let elapsed = Date().timeIntervalSince(started)
        print("scan_ms \(Int(elapsed * 1000))")
        print("on_disk_bytes \(tree.size(Tree.root))")
        print("files \(tree.files)")
        print("folders \(tree.folders)")
        print("skipped \(tree.skipped)")
        for id in tree.sortedChildren(Tree.root).prefix(10) {
            print("\(humanSize(tree.size(id)))\t\(tree.name(id))")
        }
        return 0
    } catch {
        switch error {
        case .cancelled:
            fputs("scan cancelled\n", stderr)
        case .failed(let message):
            fputs("error: \(message)\n", stderr)
        }
        return 1
    }
}

let arguments = CommandLine.arguments
if let flag = arguments.firstIndex(of: "--scan") {
    guard flag + 1 < arguments.count else {
        fputs("usage: DiskUsageViewer --scan <path>\n", stderr)
        exit(2)
    }
    exit(runHeadless(path: arguments[flag + 1]))
}

DiskUsageViewerApp.main()
