//! Folders the scanner records but does not open.
//!
//! macOS guards the folders below with privacy prompts. An open on one blocks until the user
//! answers the dialog, and a scan has nobody to answer it. `/home` is an automount point whose
//! listing stalled in testing. Withheld folders are recorded with size zero and count as skipped,
//! the same as a folder that cannot be read. Full Disk Access does not change this list.
use std::collections::HashSet;

/// Relative to the home folder.
const PROTECTED_HOME_FOLDERS: [&str; 15] = [
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
];

const AUTOMOUNT_POINTS: [&str; 2] = ["/home", "/Network"];

/// The full paths to withhold for the home folder `home`.
pub fn withheld_folders(home: &str) -> HashSet<String> {
    let home = home.trim_end_matches('/');
    let mut paths: HashSet<String> = PROTECTED_HOME_FOLDERS
        .iter()
        .map(|folder| format!("{home}/{folder}"))
        .collect();
    paths.extend(AUTOMOUNT_POINTS.iter().map(|point| (*point).to_owned()));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_folders_are_under_the_home_folder() {
        let paths = withheld_folders("/Users/tom");
        assert!(paths.contains("/Users/tom/Desktop"));
        assert!(paths.contains("/Users/tom/Library/Mail"));
        assert!(paths.contains("/home"));
        assert!(!paths.contains("/Users/tom/Applications"));
    }

    #[test]
    fn a_trailing_slash_on_home_does_not_double_up() {
        assert!(withheld_folders("/Users/tom/").contains("/Users/tom/Music"));
    }
}
