import Darwin

/// One mounted filesystem, from `getmntinfo`.
public struct MountInfo: Equatable, Sendable {
    public let path: String
    public let fileSystem: String
    public let flags: UInt32

    public var isLocal: Bool { flags & UInt32(MNT_LOCAL) != 0 }
    public var isBrowsable: Bool { flags & UInt32(MNT_DONTBROWSE) == 0 }
}

/// Filesystems mounted right now. The result is a copy; the system's buffer is not kept.
public func mountTable() -> [MountInfo] {
    var buffer: UnsafeMutablePointer<statfs>?
    let count = getmntinfo(&buffer, MNT_NOWAIT)
    guard count > 0, let buffer else {
        return []
    }
    return (0..<Int(count)).map { index in
        let fs = buffer[index]
        return MountInfo(path: cString(fs.f_mntonname), fileSystem: cString(fs.f_fstypename), flags: fs.f_flags)
    }
}

/// Copies a fixed-size, NUL-terminated C char array (as imported from statfs) into a String.
private func cString<T>(_ field: T) -> String {
    withUnsafeBytes(of: field) { raw in
        String(cString: raw.baseAddress!.assumingMemoryBound(to: CChar.self))
    }
}
