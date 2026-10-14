import DiskCore
import SwiftUI

/// The focused folder as a squarified treemap, coloured like the Windows app. Hovering names a block;
/// clicking selects it, which focuses it when it is a folder.
struct TreemapPane: View {
    let tree: Tree?
    let generation: Int
    let focus: UInt32
    let selection: UInt32?
    let onSelect: (UInt32) -> Void

    @State private var cache = TreemapCache()
    @State private var hovered: Int?

    var body: some View {
        GeometryReader { geometry in
            let bounds = Rect(x: 0, y: 0, w: geometry.size.width, h: geometry.size.height)
            let blocks = layout(in: bounds)
            ZStack(alignment: .bottomLeading) {
                Canvas { context, _ in
                    drawBlocks(blocks, hovered: hovered, selection: selection, into: &context)
                }
                .onContinuousHover { phase in
                    switch phase {
                    case .active(let point):
                        hovered = blockIndex(at: point, in: blocks)
                    case .ended:
                        hovered = nil
                    }
                }
                .gesture(
                    SpatialTapGesture().onEnded { value in
                        guard let index = blockIndex(at: value.location, in: blocks),
                              let id = blocks[index].id else {
                            return
                        }
                        onSelect(id)
                    }
                )
                if blocks.isEmpty, tree != nil {
                    Text("Empty folder")
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                if let tree, let index = hovered, blocks.indices.contains(index) {
                    Text(hoverText(blocks[index], folderSize: tree.size(focus)))
                        .font(.callout)
                        .padding(6)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
                        .padding(8)
                }
            }
        }
    }

    private func layout(in bounds: Rect) -> [TreemapBlock] {
        guard let tree else {
            return []
        }
        return cache.blocks(tree: tree, generation: generation, folder: focus, bounds: bounds)
    }
}

/// Keeps the last treemap layout, so moving the mouse does not re-sort the folder each time.
@MainActor
final class TreemapCache {
    private struct Key: Equatable {
        let generation: Int
        let folder: UInt32
        let bounds: Rect
    }

    private var key: Key?
    private var blocks: [TreemapBlock] = []

    func blocks(tree: Tree, generation: Int, folder: UInt32, bounds: Rect) -> [TreemapBlock] {
        let next = Key(generation: generation, folder: folder, bounds: bounds)
        if key != next {
            key = next
            blocks = treemapBlocks(in: tree, folder: folder, bounds: bounds)
        }
        return blocks
    }
}

/// Palette from the Windows app. Folders take these in turn; files and the "more" block have their own.
private let palette: [Color] = [
    0xa8dadc, 0xf4a261, 0xe9c46a, 0x90be6d, 0xf9c74f,
    0xf8961e, 0xb5e48c, 0xc3aed6, 0xffb4a2, 0x9ad1d4,
].map(color(hex:))
private let fileColor = color(hex: 0xdfe3e8)
private let moreColor = color(hex: 0xeeeeee)
/// Blocks narrower or shorter than this get no label.
private let minLabelWidth: CGFloat = 60
private let minLabelHeight: CGFloat = 18

private func color(hex: UInt32) -> Color {
    Color(
        red: Double((hex >> 16) & 0xff) / 255,
        green: Double((hex >> 8) & 0xff) / 255,
        blue: Double(hex & 0xff) / 255
    )
}

private func frame(of block: TreemapBlock) -> CGRect {
    CGRect(x: block.rect.x, y: block.rect.y, width: block.rect.w, height: block.rect.h)
}

private func blockIndex(at point: CGPoint, in blocks: [TreemapBlock]) -> Int? {
    blocks.firstIndex { frame(of: $0).contains(point) }
}

private func fill(for kind: TreemapBlock.Kind) -> Color {
    switch kind {
    case .folder(let paletteIndex):
        return palette[paletteIndex % palette.count]
    case .file:
        return fileColor
    case .more:
        return moreColor
    }
}

/// Fills every block, outlines it, then marks the hovered and selected ones.
private func drawBlocks(
    _ blocks: [TreemapBlock],
    hovered: Int?,
    selection: UInt32?,
    into context: inout GraphicsContext
) {
    for (index, block) in blocks.enumerated() {
        let shape = Path(frame(of: block))
        context.fill(shape, with: .color(fill(for: block.kind)))
        context.stroke(shape, with: .color(.white), lineWidth: 1)
        drawLabel(block, into: &context)
        if index == hovered {
            context.stroke(shape, with: .color(.primary), lineWidth: 2)
        }
        if let id = block.id, id == selection {
            context.stroke(shape, with: .color(.accentColor), lineWidth: 3)
        }
    }
}

private func drawLabel(_ block: TreemapBlock, into context: inout GraphicsContext) {
    let rect = frame(of: block)
    guard rect.width > minLabelWidth, rect.height > minLabelHeight else {
        return
    }
    var clipped = context
    clipped.clip(to: Path(rect))
    clipped.draw(
        Text(block.name).font(.caption).foregroundStyle(.black),
        at: CGPoint(x: rect.minX + 4, y: rect.minY + 3),
        anchor: .topLeading
    )
}

private func hoverText(_ block: TreemapBlock, folderSize: UInt64) -> String {
    let share = "\(percent(block.size, of: folderSize)) of this folder"
    switch block.kind {
    case .more(let count):
        return "\(grouped(UInt64(count))) smaller items    \(humanSize(block.size))    \(share)"
    case .folder, .file:
        return "\(block.name)    \(humanSize(block.size))    \(share)"
    }
}
