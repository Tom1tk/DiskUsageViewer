/// Folders the scanner records but does not open.
///
/// macOS guards the folders below with privacy prompts. An `open()` on one blocks until the user
/// answers the dialog, and a headless scan has nobody to answer it. `/home` is an automount point
/// whose listing also stalled in testing. Withheld folders show up with size zero and count as
/// skipped, the same as a folder that cannot be read.
public func withheldFolders(home: String) -> Set<String> {
    Set(protectedHomeFolders.map { home + "/" + $0 } + automountPoints)
}

/// Relative to the home folder. Users who grant Full Disk Access can still see these as skipped.
private let protectedHomeFolders = [
    "Desktop",
    "Documents",
    "Downloads",
    "Movies",
    "Music",
    "Pictures",
    "Library/Application Support/AddressBook",
    "Library/Calendars",
    "Library/Containers",
    "Library/Cookies",
    "Library/Group Containers",
    "Library/Mail",
    "Library/Messages",
    "Library/Mobile Documents",
    "Library/Safari",
]

private let automountPoints = ["/home", "/Network"]
