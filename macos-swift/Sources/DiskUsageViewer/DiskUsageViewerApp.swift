import SwiftUI

/// The window app. `--autostart` starts a scan of the selected drive once the window appears.
struct DiskUsageViewerApp: App {
    @State private var model: ScanModel

    init() {
        let autostart = CommandLine.arguments.contains("--autostart")
        _model = State(initialValue: ScanModel(autostart: autostart))
    }

    var body: some Scene {
        WindowGroup("Disk Usage Viewer") {
            ContentView(model: model)
                .frame(minWidth: 900, minHeight: 560)
        }
        .defaultSize(width: 1200, height: 760)
    }
}
