//! Native NT calls for the scanner: open a folder relative to its parent handle, list it with
//! NtQueryDirectoryFile, and read the records of each batch. Offsets follow the Windows SDK
//! structs FILE_ID_EXTD_DIR_INFORMATION (class 60) and FILE_DIRECTORY_INFORMATION (class 1).
use std::ffi::c_void;
use std::ptr::{null, null_mut};

type Handle = *mut c_void;

pub const ATTR_DIRECTORY: u32 = 0x10;
const ATTR_REPARSE_POINT: u32 = 0x400;
/// Set in the tag of symlinks, junctions and mount points. Cloud and dedup tags do not have it.
const TAG_NAME_SURROGATE: u32 = 0x2000_0000;
const FILE_LIST_DIRECTORY: u32 = 0x1;
const SYNCHRONIZE: u32 = 0x10_0000;
const FILE_SHARE_ALL: u32 = 0x7;
const FILE_DIRECTORY_FILE: u32 = 0x1;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x4000;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const STATUS_NO_MORE_FILES: i32 = 0x8000_0006_u32 as i32;
const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003_u32 as i32;
const STATUS_NAME_TOO_LONG: i32 = 0xC000_0106_u32 as i32;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: Handle,
    object_name: *const UnicodeString,
    attributes: u32,
    security_descriptor: *const c_void,
    security_qos: *const c_void,
}

#[repr(C)]
struct IoStatusBlock {
    status: usize,
    information: usize,
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenFile(h: *mut Handle, access: u32, oa: *const ObjectAttributes, iosb: *mut IoStatusBlock, share: u32, options: u32) -> i32;
    fn NtQueryDirectoryFile(h: Handle, event: Handle, apc: *const c_void, apc_ctx: *const c_void, iosb: *mut IoStatusBlock,
                            buf: *mut c_void, len: u32, class: u32, single: u8, mask: *const UnicodeString, restart: u8) -> i32;
    fn NtClose(h: Handle) -> i32;
}

/// An open directory handle, closed when dropped.
pub struct DirHandle(Handle);

// SAFETY: a directory handle is only passed to NT calls, which may be made from any thread.
unsafe impl Send for DirHandle {}
unsafe impl Sync for DirHandle {}

impl Drop for DirHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful NtOpenFile and is closed only here.
        unsafe { NtClose(self.0) };
    }
}

/// Opens `name` (UTF-16, no terminator) relative to `parent`, or as an absolute NT path when
/// `parent` is None. Returns the NTSTATUS on failure.
pub fn open_dir(parent: Option<&DirHandle>, name: &[u16]) -> Result<DirHandle, i32> {
    let Ok(bytes) = u16::try_from(name.len() * 2) else { return Err(STATUS_NAME_TOO_LONG) };
    let us = UnicodeString { length: bytes, maximum_length: bytes, buffer: name.as_ptr() };
    let oa = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: parent.map_or(null_mut(), |h| h.0),
        object_name: &us,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: null(),
        security_qos: null(),
    };
    let mut h: Handle = null_mut();
    let mut iosb = IoStatusBlock { status: 0, information: 0 };
    // SAFETY: `us`, `oa`, `name` and `iosb` are alive for the call, and `h` is a valid out-pointer.
    let status = unsafe {
        NtOpenFile(&mut h, FILE_LIST_DIRECTORY | SYNCHRONIZE, &oa, &mut iosb, FILE_SHARE_ALL,
                   FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT)
    };
    if status < 0 { Err(status) } else { Ok(DirHandle(h)) }
}

/// Listing class of a whole scan. Chosen once, before any folder is listed.
#[derive(Clone, Copy)]
pub enum Layout {
    /// FileIdExtdDirectoryInformation (class 60): attributes, reparse tag and 128-bit file ID.
    Extended,
    /// FileDirectoryInformation (class 1), for volumes that reject class 60 (FAT, exFAT).
    Basic,
}

impl Layout {
    fn class(self) -> u32 {
        match self {
            Layout::Extended => 60,
            Layout::Basic => 1,
        }
    }

    /// Offset of FileName in a record.
    fn name_offset(self) -> usize {
        match self {
            Layout::Extended => 88,
            Layout::Basic => 64,
        }
    }
}

/// Picks the layout for a scan from a probe of `dir`, the scan's root. The probe's batch is thrown
/// away; the first real listing restarts the enumeration.
pub fn detect_layout(dir: &DirHandle, buf: &mut [u64]) -> Layout {
    match query(dir, Layout::Extended, true, buf).0 {
        STATUS_INVALID_INFO_CLASS => Layout::Basic,
        _ => Layout::Extended,
    }
}

fn query(dir: &DirHandle, layout: Layout, restart: bool, buf: &mut [u64]) -> (i32, usize) {
    let mut iosb = IoStatusBlock { status: 0, information: 0 };
    // SAFETY: `buf` is writable for `buf.len() * 8` bytes and `iosb` is a valid out-pointer. No
    // event or APC is passed, so the call completes before it returns.
    let status = unsafe {
        NtQueryDirectoryFile(dir.0, null_mut(), null(), null(), &mut iosb, buf.as_mut_ptr().cast(),
                             (buf.len() * 8) as u32, layout.class(), 0, null(), u8::from(restart))
    };
    (status, iosb.information)
}

/// One listing batch: the record bytes, the end of the folder, or a listing error.
pub enum Batch<'a> {
    Entries(&'a [u8]),
    End,
    Error,
}

/// Lists the next batch of `dir` into `buf`. `restart` starts the enumeration over.
pub fn next_batch<'a>(dir: &DirHandle, layout: Layout, restart: bool, buf: &'a mut [u64]) -> Batch<'a> {
    let (status, valid) = query(dir, layout, restart, buf);
    match status {
        STATUS_NO_MORE_FILES => Batch::End,
        s if s < 0 => Batch::Error,
        _ => Batch::Entries(bytes_of(buf, valid)),
    }
}

fn bytes_of(words: &[u64], valid: usize) -> &[u8] {
    let len = valid.min(words.len() * 8);
    // SAFETY: `len` is within the `words.len() * 8` bytes of `words`, and every byte pattern is a
    // valid u8. The alignment of u8 is 1, so any pointer is aligned for it.
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), len) }
}

/// One directory record. `name` borrows the batch; `next` is the offset of the following record, or 0.
pub struct Entry<'a> {
    pub next: usize,
    pub attributes: u32,
    pub allocation: u64,
    pub reparse_tag: u32,
    pub file_id: Option<u128>,
    pub name: &'a [u8],
}

impl Entry<'_> {
    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }

    /// Symlinks, junctions and mount points. Their target is counted where it really is, so they are
    /// skipped here. Other reparse points (cloud placeholders, dedup) are counted like normal entries.
    pub fn is_skipped_link(&self) -> bool {
        self.attributes & ATTR_REPARSE_POINT != 0 && self.reparse_tag & TAG_NAME_SURROGATE != 0
    }
}

/// Reads the record at `at` in a batch. Returns None when the record does not fit in the batch.
pub fn parse_entry(bytes: &[u8], at: usize, layout: Layout) -> Option<Entry<'_>> {
    let name_len = u32_at(bytes, at + 60)? as usize;
    let name_start = at + layout.name_offset();
    let (reparse_tag, file_id) = match layout {
        Layout::Extended => (u32_at(bytes, at + 68)?, Some(u128::from_le_bytes(array_at(bytes, at + 72)?))),
        // Basic records have no reparse tag. Treating every reparse point as a name surrogate skips
        // it, as the benchmark did.
        Layout::Basic => (TAG_NAME_SURROGATE, None),
    };
    Some(Entry {
        next: u32_at(bytes, at)? as usize,
        attributes: u32_at(bytes, at + 56)?,
        allocation: u64::from_le_bytes(array_at(bytes, at + 48)?),
        reparse_tag,
        file_id,
        name: bytes.get(name_start..name_start + name_len)?,
    })
}

fn array_at<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at + N)?.try_into().ok()
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    array_at(bytes, at).map(u32::from_le_bytes)
}

/// `.` and `..` as UTF-16LE.
pub fn is_dot_entry(name: &[u8]) -> bool {
    matches!(name, [b'.', 0] | [b'.', 0, b'.', 0])
}

/// Converts a UTF-16LE name from a record into code units.
pub fn name_units(name: &[u8]) -> impl Iterator<Item = u16> + '_ {
    name.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a record of the given layout with `name` (ASCII) at its name offset.
    fn record(layout: Layout, attributes: u32, tag: u32, name: &str) -> Vec<u8> {
        let mut b = vec![0u8; layout.name_offset()];
        b[56..60].copy_from_slice(&attributes.to_le_bytes());
        b[60..64].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        if let Layout::Extended = layout {
            b[68..72].copy_from_slice(&tag.to_le_bytes());
            b[72..88].copy_from_slice(&7u128.to_le_bytes());
        }
        b.extend(name.bytes().flat_map(|c| [c, 0]));
        b
    }

    #[test]
    fn basic_record_has_name_at_64_and_every_reparse_point_is_a_link() {
        let plain = record(Layout::Basic, 0x20, 0, "file.txt");
        let entry = parse_entry(&plain, 0, Layout::Basic).expect("record fits");
        assert_eq!(name_units(entry.name).collect::<Vec<_>>(), "file.txt".encode_utf16().collect::<Vec<_>>());
        assert!(!entry.is_skipped_link());
        let reparse = record(Layout::Basic, 0x420, 0, "link");
        assert!(parse_entry(&reparse, 0, Layout::Basic).expect("record fits").is_skipped_link());
    }

    #[test]
    fn extended_record_reads_tag_and_file_id() {
        let rec = record(Layout::Extended, 0x420, 0xA000_000C, "link");
        let entry = parse_entry(&rec, 0, Layout::Extended).expect("record fits");
        assert_eq!(entry.file_id, Some(7));
        assert!(entry.is_skipped_link());
        let cloud = record(Layout::Extended, 0x420, 0x9000_001A, "cloud");
        assert!(!parse_entry(&cloud, 0, Layout::Extended).expect("record fits").is_skipped_link());
    }
}
