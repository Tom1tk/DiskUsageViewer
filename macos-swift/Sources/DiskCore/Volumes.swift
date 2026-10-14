import Darwin
import Foundation

/// A volume the user can pick to scan.
public struct Volume: Identifiable, Equatable, Sendable {
    public var id: String { path }
    public let path: String
    public let name: String
    public let total: UInt64
    /// Space available to the user, not the raw free count.
    public let free: UInt64

    public var used: UInt64 { total > free ? total - free : 0 }
}

/// The local volumes a user would pick: the startup disk and anything mounted under `/Volumes`.
public func listVolumes() -> [Volume] {
    volumes(from: mountTable())
}

/// Filters a mount table to the pickable volumes. Split out so tests can pass a fixed table.
func volumes(from mounts: [MountInfo]) -> [Volume] {
    mounts
        .filter { $0.isLocal && $0.isBrowsable && isPickable($0.path) }
        .compactMap(makeVolume)
}

private func isPickable(_ path: String) -> Bool {
    path == "/" || path.hasPrefix("/Volumes/")
}

/// A volume that cannot be queried is not offered, since its size cannot be shown.
private func makeVolume(_ mount: MountInfo) -> Volume? {
    var info = statfs()
    guard statfs(mount.path, &info) == 0 else {
        return nil
    }
    let blockSize = UInt64(info.f_bsize)
    return Volume(
        path: mount.path,
        name: volumeName(mount.path),
        total: UInt64(info.f_blocks) * blockSize,
        free: UInt64(info.f_bavail) * blockSize
    )
}

private func volumeName(_ path: String) -> String {
    let url = URL(fileURLWithPath: path)
    if let name = try? url.resourceValues(forKeys: [.volumeLocalizedNameKey]).volumeLocalizedName,
       !name.isEmpty {
        return name
    }
    return path == "/" ? "Startup Disk" : url.lastPathComponent
}
