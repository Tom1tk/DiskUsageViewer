import AppKit
import DiskCore
import SwiftUI

/// The folder tree as a native outline: name and size, largest first, like the Windows tree view.
struct TreeOutlineView: NSViewRepresentable {
    let tree: Tree?
    let generation: Int
    let selection: UInt32?
    let onSelect: (UInt32) -> Void
    let onReveal: (UInt32) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onSelect: onSelect, onReveal: onReveal)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let outline = NSOutlineView()
        let nameColumn = NSTableColumn(identifier: Coordinator.nameColumnID)
        nameColumn.title = "Name"
        nameColumn.width = 260
        nameColumn.minWidth = 120
        let sizeColumn = NSTableColumn(identifier: Coordinator.sizeColumnID)
        sizeColumn.title = "Size"
        sizeColumn.width = 110
        sizeColumn.minWidth = 70
        outline.addTableColumn(nameColumn)
        outline.addTableColumn(sizeColumn)
        outline.outlineTableColumn = nameColumn
        outline.allowsMultipleSelection = false
        outline.menu = context.coordinator.makeMenu()
        outline.dataSource = context.coordinator
        outline.delegate = context.coordinator
        context.coordinator.outline = outline

        let scroll = NSScrollView()
        scroll.documentView = outline
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        return scroll
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.onSelect = onSelect
        context.coordinator.onReveal = onReveal
        context.coordinator.update(tree: tree, generation: generation, selection: selection)
    }

    /// Owns the outline's data source and delegate. Child lists are cached until the tree changes.
    @MainActor
    final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
        static let nameColumnID = NSUserInterfaceItemIdentifier("name")
        static let sizeColumnID = NSUserInterfaceItemIdentifier("size")

        var onSelect: (UInt32) -> Void
        var onReveal: (UInt32) -> Void
        weak var outline: NSOutlineView?

        private var tree: Tree?
        private var generation = -1
        private var children: [UInt32: [UInt32]] = [:]
        private var isShowingSelection = false

        init(onSelect: @escaping (UInt32) -> Void, onReveal: @escaping (UInt32) -> Void) {
            self.onSelect = onSelect
            self.onReveal = onReveal
        }

        /// Reloads when the tree has changed, then makes the selection match the model.
        func update(tree: Tree?, generation: Int, selection: UInt32?) {
            if generation != self.generation {
                self.generation = generation
                self.tree = tree
                children = [:]
                outline?.reloadData()
                if tree != nil {
                    outline?.expandItem(NSNumber(value: Tree.root))
                }
            }
            showSelection(selection)
        }

        func makeMenu() -> NSMenu {
            let menu = NSMenu()
            let item = NSMenuItem(title: "Show in Finder", action: #selector(revealClickedRow), keyEquivalent: "")
            item.target = self
            menu.addItem(item)
            return menu
        }

        @objc private func revealClickedRow() {
            guard let outline, outline.clickedRow >= 0,
                  let id = entryID(outline.item(atRow: outline.clickedRow)) else {
                return
            }
            onReveal(id)
        }

        func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
            childIDs(of: item).count
        }

        func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
            NSNumber(value: childIDs(of: item)[index])
        }

        func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
            guard let tree, let id = entryID(item) else {
                return false
            }
            return tree.isDirectory(id) && tree.childCount(id) > 0
        }

        func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
            guard let tree, let id = entryID(item), let column = tableColumn else {
                return nil
            }
            let isSize = column.identifier == Coordinator.sizeColumnID
            let cell = outlineView.makeView(withIdentifier: column.identifier, owner: self) as? NSTableCellView
                ?? makeCell(identifier: column.identifier, rightAligned: isSize)
            cell.textField?.stringValue = isSize ? humanSize(tree.size(id)) : tree.name(id)
            return cell
        }

        func outlineViewSelectionDidChange(_ notification: Notification) {
            guard !isShowingSelection, let outline, let id = selectedID(in: outline) else {
                return
            }
            onSelect(id)
        }

        /// The entries shown under `item`. Nil is the invisible top level, whose only entry is the root.
        private func childIDs(of item: Any?) -> [UInt32] {
            guard let tree else {
                return []
            }
            guard let item else {
                return [Tree.root]
            }
            guard let id = entryID(item), tree.isDirectory(id) else {
                return []
            }
            if let known = children[id] {
                return known
            }
            let sorted = tree.sortedChildren(id)
            children[id] = sorted
            return sorted
        }

        /// Expands the path down to `id`, then selects its row and scrolls it into view.
        private func showSelection(_ id: UInt32?) {
            guard let outline, let tree, let id else {
                return
            }
            guard selectedID(in: outline) != id else {
                return
            }
            for ancestor in ancestors(of: id, in: tree).reversed() {
                outline.expandItem(NSNumber(value: ancestor))
            }
            let row = outline.row(forItem: NSNumber(value: id))
            guard row >= 0 else {
                return
            }
            isShowingSelection = true
            outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            outline.scrollRowToVisible(row)
            isShowingSelection = false
        }

        private func selectedID(in outline: NSOutlineView) -> UInt32? {
            guard outline.selectedRow >= 0 else {
                return nil
            }
            return entryID(outline.item(atRow: outline.selectedRow))
        }

        private func makeCell(identifier: NSUserInterfaceItemIdentifier, rightAligned: Bool) -> NSTableCellView {
            let cell = NSTableCellView()
            cell.identifier = identifier
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = .byTruncatingMiddle
            field.alignment = rightAligned ? .right : .left
            field.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(field)
            cell.textField = field
            NSLayoutConstraint.activate([
                field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
                field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
            return cell
        }
    }
}

/// The entry an outline item stands for. Items are `NSNumber`s holding the entry id.
private func entryID(_ item: Any?) -> UInt32? {
    (item as? NSNumber)?.uint32Value
}

/// The folders above `id`, nearest first, ending at the root.
private func ancestors(of id: UInt32, in tree: Tree) -> [UInt32] {
    var chain: [UInt32] = []
    var current = tree.parent(id)
    while let parent = current {
        chain.append(parent)
        current = tree.parent(parent)
    }
    return chain
}
