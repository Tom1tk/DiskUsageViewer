import Darwin

/// Object types as `getattrlistbulk` reports them (the values of `enum vtype`).
enum ObjectType {
    static let directory: UInt32 = 2
    static let symlink: UInt32 = 5
}

/// The errno from a failed open.
struct OpenFailure: Error {
    let code: Int32
}

/// One directory entry. The name points into the listing buffer, so it is valid only inside the
/// callback that receives the entry.
struct DirectoryEntry {
    let name: UnsafeRawBufferPointer
    let kind: UInt32
    let fileID: UInt64
    let linkCount: UInt32
    /// Bytes allocated on disk. Zero for directories.
    let allocatedSize: UInt64
}

/// An open directory, read in batches with `getattrlistbulk`. Opened by path and closed on deinit.
final class Directory {
    private let fd: Int32
    /// The volume holding this directory (`f_fsid`). File IDs are only unique within one volume.
    let volume: UInt64

    private init(fd: Int32, volume: UInt64) {
        self.fd = fd
        self.volume = volume
    }

    deinit {
        Darwin.close(fd)
    }

    /// Opens a directory by path. Throws `OpenFailure` carrying the errno.
    static func open(_ path: String) throws(OpenFailure) -> Directory {
        let fd = Darwin.open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard fd >= 0 else {
            throw OpenFailure(code: errno)
        }
        var fs = statfs()
        guard fstatfs(fd, &fs) == 0 else {
            let code = errno
            Darwin.close(fd)
            throw OpenFailure(code: code)
        }
        let high = UInt64(UInt32(bitPattern: fs.f_fsid.val.0))
        let low = UInt64(UInt32(bitPattern: fs.f_fsid.val.1))
        return Directory(fd: fd, volume: high << 32 | low)
    }

    /// Calls `body` for every entry. Returns false when the listing stopped early: an error, or
    /// `isCancelled` became true. `buffer` is reused between calls to avoid allocating per folder.
    func forEachEntry(
        into buffer: inout [UInt64],
        isCancelled: () -> Bool,
        _ body: (DirectoryEntry) -> Void
    ) -> Bool {
        var request = attrlist()
        request.bitmapcount = UInt16(ATTR_BIT_MAP_COUNT)
        request.commonattr = bits(ATTR_CMN_RETURNED_ATTRS) | bits(ATTR_CMN_NAME)
            | bits(ATTR_CMN_OBJTYPE) | bits(ATTR_CMN_FILEID)
        request.fileattr = bits(ATTR_FILE_LINKCOUNT) | bits(ATTR_FILE_ALLOCSIZE)
        return buffer.withUnsafeMutableBytes { raw in
            guard let base = raw.baseAddress else {
                return false
            }
            while true {
                if isCancelled() {
                    return false
                }
                let count = getattrlistbulk(fd, &request, base, raw.count, 0)
                if count < 0 {
                    return false
                }
                if count == 0 {
                    return true
                }
                var offset = 0
                for _ in 0..<Int(count) {
                    let record = UnsafeRawPointer(base) + offset
                    body(decode(record))
                    offset += Int(record.loadUnaligned(as: UInt32.self))
                }
            }
        }
    }
}

private func bits<T: BinaryInteger>(_ value: T) -> UInt32 {
    UInt32(truncatingIfNeeded: value)
}

/// Reads one record. The layout follows the order the request asks for, and the returned-attributes
/// set says which of the file attributes are present:
///   0   length (u32)
///   4   returned set (5 × u32; fileattr at 16)
///   24  name reference (offset i32, length u32 including the NUL)
///   32  object type (u32)
///   36  file ID (u64, unaligned)
///   44  link count (u32) and allocated size (i64), files only
private func decode(_ record: UnsafeRawPointer) -> DirectoryEntry {
    let returnedFileAttrs = record.loadUnaligned(fromByteOffset: 16, as: UInt32.self)
    let nameOffset = record.loadUnaligned(fromByteOffset: 24, as: Int32.self)
    let nameLength = Int(record.loadUnaligned(fromByteOffset: 28, as: UInt32.self))
    let name = UnsafeRawBufferPointer(
        start: record + 24 + Int(nameOffset),
        count: max(0, nameLength - 1)
    )
    let kind = record.loadUnaligned(fromByteOffset: 32, as: UInt32.self)
    let fileID = record.loadUnaligned(fromByteOffset: 36, as: UInt64.self)
    var linkCount: UInt32 = 1
    var allocated: Int64 = 0
    var cursor = 44
    if returnedFileAttrs & bits(ATTR_FILE_LINKCOUNT) != 0 {
        linkCount = record.loadUnaligned(fromByteOffset: cursor, as: UInt32.self)
        cursor += 4
    }
    if returnedFileAttrs & bits(ATTR_FILE_ALLOCSIZE) != 0 {
        allocated = record.loadUnaligned(fromByteOffset: cursor, as: Int64.self)
    }
    return DirectoryEntry(
        name: name,
        kind: kind,
        fileID: fileID,
        linkCount: linkCount,
        allocatedSize: UInt64(max(0, allocated))
    )
}
