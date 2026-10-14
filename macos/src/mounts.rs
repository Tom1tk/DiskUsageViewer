//! The mount table, and the volumes the drive picker offers: the startup disk and anything
//! mounted under `/Volumes`. Other mounts (`/System/Volumes/*`, `/dev`, network shares) are not
//! offered, and the scanner does not enter them.
use std::ffi::{CStr, CString, c_void};
use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr;

use libc::{ATTR_BIT_MAP_COUNT, ATTR_VOL_NAME, MNT_DONTBROWSE, MNT_LOCAL, MNT_NOWAIT, attrlist, statfs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountInfo {
    pub path: String,
    pub local: bool,
    pub browsable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub path: String,
    pub name: String,
    pub total: u64,
    /// Space available to the user, not the raw free count.
    pub free: u64,
}

impl Volume {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

/// Every mount, as the kernel reports it. `MNT_NOWAIT` returns the cached table without asking
/// the file systems, so a stuck network mount cannot hang this call.
pub fn mount_table() -> Vec<MountInfo> {
    let mut raw: *mut statfs = ptr::null_mut();
    // SAFETY: getmntinfo writes a pointer to its own static array into `raw`. The array is read
    // below before any other call that could replace it.
    let count = unsafe { libc::getmntinfo(&mut raw, MNT_NOWAIT) };
    if count <= 0 || raw.is_null() {
        return Vec::new();
    }
    // SAFETY: getmntinfo returned `count` valid entries at `raw`.
    let entries = unsafe { std::slice::from_raw_parts(raw, count as usize) };
    entries.iter().map(mount_info).collect()
}

fn mount_info(entry: &statfs) -> MountInfo {
    // SAFETY: the kernel fills f_mntonname with a NUL-terminated path.
    let path = unsafe { CStr::from_ptr(entry.f_mntonname.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let flags = entry.f_flags;
    MountInfo {
        path,
        local: flags & MNT_LOCAL as u32 != 0,
        browsable: flags & MNT_DONTBROWSE as u32 == 0,
    }
}

/// The volumes the drive picker offers.
pub fn list_volumes() -> Vec<Volume> {
    volumes_from(&mount_table())
}

/// The volumes for a mount table. Split out so tests can pass a fixed table.
pub fn volumes_from(mounts: &[MountInfo]) -> Vec<Volume> {
    offered_mounts(mounts).filter_map(make_volume).collect()
}

/// The mounts the drive picker may offer, before checking that each one can be queried.
fn offered_mounts(mounts: &[MountInfo]) -> impl Iterator<Item = &MountInfo> {
    mounts
        .iter()
        .filter(|m| m.local && m.browsable && (m.path == "/" || m.path.starts_with("/Volumes/")))
}

/// A volume that cannot be queried is not offered, since its size cannot be shown.
fn make_volume(mount: &MountInfo) -> Option<Volume> {
    let c_path = CString::new(mount.path.as_str()).ok()?;
    let mut info = MaybeUninit::<statfs>::uninit();
    // SAFETY: c_path is NUL-terminated and `info` is a valid out-buffer for statfs.
    if unsafe { libc::statfs(c_path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs returned 0, so every field of `info` is initialised.
    let info = unsafe { info.assume_init() };
    let block = u64::from(info.f_bsize);
    Some(Volume {
        path: mount.path.clone(),
        name: volume_name(&mount.path),
        total: info.f_blocks * block,
        free: info.f_bavail * block,
    })
}

/// The name shown in Finder ("Macintosh HD"), else the startup disk or the folder name.
fn volume_name(path: &str) -> String {
    let fallback = if path == "/" {
        "Startup Disk".to_owned()
    } else {
        Path::new(path)
            .file_name()
            .map_or_else(|| path.to_owned(), |name| name.to_string_lossy().into_owned())
    };
    read_volume_name(path).filter(|name| !name.is_empty()).unwrap_or(fallback)
}

/// Asks the volume for `ATTR_VOL_NAME`. The buffer is a length, then an attribute reference
/// (offset relative to the reference itself, and a length that includes the NUL), then the name.
fn read_volume_name(path: &str) -> Option<String> {
    let c_path = CString::new(path).ok()?;
    let mut request = attrlist {
        bitmapcount: ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: ATTR_VOL_NAME,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buf = [0u8; 1024];
    // SAFETY: `request`, `c_path` and `buf` are valid for the call, and `buf.len()` bounds the write.
    let rc = unsafe {
        libc::getattrlist(
            c_path.as_ptr(),
            (&mut request as *mut attrlist).cast::<c_void>(),
            buf.as_mut_ptr().cast::<c_void>(),
            buf.len(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let reference = 4;
    let offset = i32::from_ne_bytes(buf.get(reference..reference + 4)?.try_into().ok()?);
    let len = u32::from_ne_bytes(buf.get(reference + 4..reference + 8)?.try_into().ok()?) as usize;
    let start = usize::try_from(reference as i64 + i64::from(offset)).ok()?;
    let bytes = buf.get(start..start.checked_add(len)?)?;
    let text = bytes.split(|&b| b == 0).next()?;
    Some(String::from_utf8_lossy(text).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(path: &str, local: bool, browsable: bool) -> MountInfo {
        MountInfo { path: path.to_owned(), local, browsable }
    }

    #[test]
    fn only_local_browsable_root_and_volumes_are_offered() {
        let table = [
            mount("/", true, true),
            mount("/System/Volumes/Data", true, false),
            mount("/Volumes/Backup", true, true),
            mount("/Volumes/Network", false, true),
            mount("/Volumes/Hidden", true, false),
            mount("/private/tmp/ram", true, true),
        ];
        let paths: Vec<&str> = offered_mounts(&table).map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["/", "/Volumes/Backup"]);
    }

    #[test]
    fn name_falls_back_to_the_folder_name_for_a_missing_volume() {
        assert_eq!(volume_name("/no/such/volume"), "volume");
        assert!(!volume_name("/").is_empty(), "the root always has a name");
    }

    #[test]
    fn used_is_total_minus_free() {
        let v = Volume { path: "/".into(), name: "x".into(), total: 100, free: 30 };
        assert_eq!(v.used(), 70);
        let full = Volume { free: 200, ..v };
        assert_eq!(full.used(), 0);
    }

    #[test]
    fn the_root_volume_reports_a_size() {
        let root = volumes_from(&mount_table()).into_iter().find(|v| v.path == "/");
        let root = root.expect("the root volume is offered");
        assert!(root.total > 0 && root.free <= root.total);
    }
}
