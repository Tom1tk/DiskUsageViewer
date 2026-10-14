/// Most blocks drawn for one folder. The rest fold into one "more items" block.
public let maxTreemapBlocks = 150

/// One drawn cell of the treemap, in the coordinates of the bounds it was laid out in.
public struct TreemapBlock: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        /// `paletteIndex` counts folders only, so folders take colours in order.
        case folder(paletteIndex: Int)
        case file
        /// Stands for `count` smaller children. It has no entry and cannot be opened.
        case more(count: Int)
    }

    /// The entry this block shows. Nil for the "more" block.
    public let id: UInt32?
    public let name: String
    public let size: UInt64
    public let kind: Kind
    public let rect: Rect
}

/// Blocks for the children of `folder`, laid out in `bounds`. Children with no size get no block.
///
/// The first `limit` children by size become blocks. The rest become one "more items" block, placed
/// by its combined size so the layout still sees the largest first.
public func treemapBlocks(
    in tree: Tree,
    folder: UInt32,
    bounds: Rect,
    limit: Int = maxTreemapBlocks
) -> [TreemapBlock] {
    guard bounds.w > 0, bounds.h > 0 else {
        return []
    }
    let kids = tree.sortedChildren(folder).filter { tree.size($0) > 0 }
    let slots = treemapSlots(kids, in: tree, limit: limit)
    let rects = layoutTreemap(slots.map { Double($0.size) }, in: bounds)
    var folders = 0
    return zip(slots, rects).map { slot, rect in
        switch slot {
        case .child(let id, _):
            let isFolder = tree.isDirectory(id)
            let kind: TreemapBlock.Kind = isFolder ? .folder(paletteIndex: folders) : .file
            folders += isFolder ? 1 : 0
            return TreemapBlock(id: id, name: tree.name(id), size: tree.size(id), kind: kind, rect: rect)
        case .more(let count, let size):
            return TreemapBlock(
                id: nil,
                name: "\(grouped(UInt64(count))) more items",
                size: size,
                kind: .more(count: count),
                rect: rect
            )
        }
    }
}

/// One layout slot: a listed child, or the rest of the children folded into one.
private enum Slot {
    case child(id: UInt32, size: UInt64)
    case more(count: Int, size: UInt64)

    var size: UInt64 {
        switch self {
        case .child(_, let size), .more(_, let size):
            return size
        }
    }
}

private func treemapSlots(_ kids: [UInt32], in tree: Tree, limit: Int) -> [Slot] {
    var slots = kids.prefix(limit).map { Slot.child(id: $0, size: tree.size($0)) }
    let rest = kids.dropFirst(limit)
    guard !rest.isEmpty else {
        return slots
    }
    let restSize = rest.reduce(UInt64(0)) { $0 + tree.size($1) }
    let at = slots.firstIndex { $0.size < restSize } ?? slots.count
    slots.insert(.more(count: rest.count, size: restSize), at: at)
    return slots
}
