//! Drive enumeration and per-drive queries through Win32.

use std::mem::{offset_of, size_of};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetLogicalDrives, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_NO_ROOT_DIR;

use crate::win::wide;

pub struct Drive {
    pub root: String,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    pub fn used(&self) -> u64 {
        self.total - self.free
    }
}

pub fn list_drives() -> Vec<Drive> {
    let mask = logical_drive_mask();
    (0u8..26)
        .filter(|&bit| (mask & (1u32 << bit)) != 0)
        .filter_map(|bit| {
            let root = format!("{}:\\", char::from(b'A' + bit));
            if drive_type(&root) == DRIVE_NO_ROOT_DIR {
                None
            } else {
                free_space(&root)
            }
        })
        .collect()
}

pub fn free_space(root: &str) -> Option<Drive> {
    let wide_root = wide(root);
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    // SAFETY: `wide_root` is NUL-terminated and outlives the call. The caller-free-bytes
    // pointer is optional and passed as null. The other two point at live locals.
    let ok = unsafe { GetDiskFreeSpaceExW(wide_root.as_ptr(), null_mut(), &mut total, &mut free) };
    if ok == 0 || free > total {
        return None;
    }
    Some(Drive {
        root: root.to_owned(),
        total,
        free,
    })
}

/// `None` when the volume cannot be opened or the query fails.
pub fn has_seek_penalty(root: &str) -> Option<bool> {
    let volume = wide(&format!(r"\\.\{}", root.trim_end_matches('\\')));
    let handle = open_volume(&volume)?;
    let penalty = query_seek_penalty(handle);
    // SAFETY: `handle` came from a successful CreateFileW and is closed exactly once, on every path.
    unsafe { CloseHandle(handle) };
    penalty
}

fn logical_drive_mask() -> u32 {
    // SAFETY: GetLogicalDrives takes no pointers. A zero return means no drives (or failure).
    unsafe { GetLogicalDrives() }
}

fn drive_type(root: &str) -> u32 {
    let wide_root = wide(root);
    // SAFETY: `wide_root` is NUL-terminated and outlives the call.
    unsafe { GetDriveTypeW(wide_root.as_ptr()) }
}

/// Opens the volume with no access rights, which needs no administrator rights.
fn open_volume(volume: &[u16]) -> Option<HANDLE> {
    // SAFETY: `volume` is NUL-terminated and outlives the call. Null security attributes
    // and template handle are allowed for OPEN_EXISTING.
    let handle = unsafe {
        CreateFileW(
            volume.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

fn query_seek_penalty(handle: HANDLE) -> Option<bool> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut out = [0u8; size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()];
    let mut returned: u32 = 0;
    // SAFETY: the input and output buffers are valid for the sizes passed. `returned` is
    // non-null, which the call requires when no OVERLAPPED is given.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&query as *const STORAGE_PROPERTY_QUERY).cast(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            out.as_mut_ptr().cast(),
            out.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    // The descriptor's IncursSeekPenalty is a BOOLEAN (one byte). Reading the byte directly
    // avoids treating an unexpected driver value as a Rust bool.
    let flag_at = offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty);
    (ok != 0 && (returned as usize) > flag_at).then(|| out[flag_at] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_structs_match_sdk_layout() {
        assert_eq!(size_of::<STORAGE_PROPERTY_QUERY>(), 12);
        assert_eq!(size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>(), 12);
        assert_eq!(
            offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty),
            8
        );
    }

    #[test]
    fn list_drives_contains_c_and_free_fits_total() {
        let drives = list_drives();
        assert!(drives.iter().any(|d| d.root == r"C:\"));
        assert!(drives.iter().all(|d| d.free <= d.total));
    }

    #[test]
    fn free_space_of_c_is_some() {
        assert!(free_space(r"C:\").is_some());
    }

    #[test]
    fn seek_penalty_query_answers_for_c() {
        assert!(has_seek_penalty(r"C:\").is_some());
    }
}
