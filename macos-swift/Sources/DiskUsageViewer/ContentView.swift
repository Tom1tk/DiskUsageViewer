import DiskCore
import SwiftUI

/// The main window: drive picker, Scan and Cancel, status line, then the folder tree beside the treemap.
struct ContentView: View {
    @Bindable var model: ScanModel

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            Text(model.status)
                .lineLimit(1)
                .truncationMode(.tail)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
            Divider()
            HSplitView {
                TreeOutlineView(
                    tree: model.tree,
                    generation: model.generation,
                    selection: model.selection,
                    onSelect: { model.selection = $0 },
                    onReveal: model.reveal
                )
                .frame(minWidth: 280, idealWidth: 380)
                TreemapPane(
                    tree: model.tree,
                    generation: model.generation,
                    focus: model.focus,
                    selection: model.selection,
                    onSelect: { model.selection = $0 }
                )
                .frame(minWidth: 320, maxWidth: .infinity)
            }
        }
        .task { model.startAutostartIfNeeded() }
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            Picker("Drive", selection: $model.selectedVolume) {
                ForEach(model.volumes) { volume in
                    Text(volumeLabel(volume)).tag(volume.path)
                }
            }
            .labelsHidden()
            .frame(maxWidth: 360)
            .disabled(model.isScanning || model.volumes.isEmpty)

            Button("Scan") { model.startScan() }
                .keyboardShortcut(.defaultAction)
                .disabled(model.isScanning || model.volumes.isEmpty)
            Button("Cancel") { model.cancelScan() }
                .keyboardShortcut(.cancelAction)
                .disabled(!model.isScanning)
            Button("Show in Finder") { model.revealSelection() }
                .disabled(model.tree == nil || model.selection == nil)
            Spacer()
        }
        .padding(12)
    }

    private func volumeLabel(_ volume: Volume) -> String {
        "\(volume.name)   \(humanSize(volume.free)) free of \(humanSize(volume.total))"
    }
}
