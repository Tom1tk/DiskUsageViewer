/// Parent id of the root.
let noParent = UInt32.max

/// Byte range of one name in the names arena.
struct NameSpan {
    var offset: UInt32
    var length: UInt32
}

/// One recorded entry. Ids are dense, 0..<count, and the root is 0.
struct EntryRecord {
    var id: UInt32
    var parent: UInt32
    var size: UInt64
    var isDir: Bool
    var name: NameSpan
}

/// A non-directory with a link count above one. Hard links share (volume, file) and count once.
struct FileRef: Equatable, Comparable {
    var volume: UInt64
    var file: UInt64
    var entry: UInt32

    static func < (a: FileRef, b: FileRef) -> Bool {
        (a.volume, a.file, a.entry) < (b.volume, b.file, b.entry)
    }
}

/// What the scanner hands to `buildTree`.
struct TreeParts {
    var names: [UInt8] = []
    var entries: [EntryRecord] = []
    var fileRefs: [FileRef] = []
    var skipped: UInt64 = 0
}

/// The scanned tree in struct-of-arrays form: one name arena, children as CSR lists, and folder
/// sizes summed in one reverse pass.
public struct Tree: Sendable {
    public let files: UInt64
    public let folders: UInt64
    public let skipped: UInt64
    private let names: [UInt8]
    private let nameSpans: [NameSpan]
    private let parents: [UInt32]
    private let sizes: [UInt64]
    private let directories: [Bool]
    private let childStart: [UInt32]
    private let children: [UInt32]

    init(
        files: UInt64, folders: UInt64, skipped: UInt64, names: [UInt8], nameSpans: [NameSpan],
        parents: [UInt32], sizes: [UInt64], directories: [Bool], childStart: [UInt32], children: [UInt32]
    ) {
        self.files = files
        self.folders = folders
        self.skipped = skipped
        self.names = names
        self.nameSpans = nameSpans
        self.parents = parents
        self.sizes = sizes
        self.directories = directories
        self.childStart = childStart
        self.children = children
    }

    public static let root: UInt32 = 0

    /// The root is named by the scanned path ("/", or "/Volumes/Backup"). Other entries by their name.
    public func name(_ id: UInt32) -> String {
        String(decoding: nameBytes(id), as: UTF8.self)
    }

    /// Size on disk. For a folder, the total of everything below it.
    public func size(_ id: UInt32) -> UInt64 {
        sizes[Int(id)]
    }

    public func isDirectory(_ id: UInt32) -> Bool {
        directories[Int(id)]
    }

    public func parent(_ id: UInt32) -> UInt32? {
        let par = parents[Int(id)]
        return par == noParent ? nil : par
    }

    /// Files and folders directly inside `id`. Zero for a file.
    public func childCount(_ id: UInt32) -> Int {
        let i = Int(id)
        return Int(childStart[i + 1] - childStart[i])
    }

    /// Largest size first; equal sizes by name, ignoring ASCII case.
    public func sortedChildren(_ id: UInt32) -> [UInt32] {
        let i = Int(id)
        let kids = children[Int(childStart[i])..<Int(childStart[i + 1])]
        return kids.sorted { a, b in
            let sa = sizes[Int(a)]
            let sb = sizes[Int(b)]
            return sa != sb ? sa > sb : nameLess(a, b)
        }
    }

    /// Full path with "/" separators. The root's name is used as it is, so "/" gives "/Users", not "//Users".
    public func path(_ id: UInt32) -> String {
        var chain: [UInt32] = []
        var cur = id
        while let par = parent(cur) {
            chain.append(cur)
            cur = par
        }
        var out = name(cur)
        for step in chain.reversed() {
            if !out.hasSuffix("/") {
                out.append("/")
            }
            out.append(name(step))
        }
        return out
    }

    private func nameBytes(_ id: UInt32) -> ArraySlice<UInt8> {
        let span = nameSpans[Int(id)]
        return names[Int(span.offset)..<Int(span.offset + span.length)]
    }

    private func nameLess(_ a: UInt32, _ b: UInt32) -> Bool {
        nameBytes(a).lexicographicallyPrecedes(nameBytes(b)) { asciiFolded($0) < asciiFolded($1) }
    }
}

private func asciiFolded(_ byte: UInt8) -> UInt8 {
    (65...90).contains(byte) ? byte + 32 : byte
}

/// Builds the tree: dedupes hard links, sums folder sizes, and lays out the child lists.
func buildTree(_ parts: TreeParts) -> Tree {
    let n = parts.entries.count
    var parents = [UInt32](repeating: noParent, count: n)
    var sizes = [UInt64](repeating: 0, count: n)
    var directories = [Bool](repeating: false, count: n)
    var spans = [NameSpan](repeating: NameSpan(offset: 0, length: 0), count: n)
    for entry in parts.entries {
        let i = Int(entry.id)
        parents[i] = entry.parent
        sizes[i] = entry.size
        directories[i] = entry.isDir
        spans[i] = entry.name
    }
    dedupe(parts.fileRefs, sizes: &sizes)
    // A child's id is always larger than its parent's, so each subtree is complete when reached.
    for id in stride(from: n - 1, through: 1, by: -1) where parents[id] != noParent {
        sizes[Int(parents[id])] += sizes[id]
    }
    var childStart = [UInt32](repeating: 0, count: n + 1)
    for id in 1..<n where parents[id] != noParent {
        childStart[Int(parents[id]) + 1] += 1
    }
    for i in 0..<n {
        childStart[i + 1] += childStart[i]
    }
    var cursor = childStart
    var children = [UInt32](repeating: 0, count: Int(childStart[n]))
    for id in 1..<n where parents[id] != noParent {
        let par = Int(parents[id])
        children[Int(cursor[par])] = UInt32(id)
        cursor[par] += 1
    }
    let folders = UInt64(directories.lazy.filter { $0 }.count)
    return Tree(
        files: UInt64(n) - folders,
        folders: folders,
        skipped: parts.skipped,
        names: parts.names,
        nameSpans: spans,
        parents: parents,
        sizes: sizes,
        directories: directories,
        childStart: childStart,
        children: children
    )
}

/// Keeps the size of the lowest entry id for each file and zeroes the others.
private func dedupe(_ refs: [FileRef], sizes: inout [UInt64]) {
    let sorted = refs.sorted()
    for (previous, current) in zip(sorted, sorted.dropFirst())
    where previous.volume == current.volume && previous.file == current.file {
        sizes[Int(current.entry)] = 0
    }
}
