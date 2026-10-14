//! Reads one folder's entries with `getattrlistbulk`, in batches. One call returns the name, type,
//! file ID, link count and allocated size of many entries, so the scanner makes few system calls.
use std::ffi::{CString, c_void};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc::{
    ATTR_BIT_MAP_COUNT, ATTR_CMN_FILEID, ATTR_CMN_NAME, ATTR_CMN_OBJTYPE, ATTR_CMN_RETURNED_ATTRS,
    ATTR_FILE_ALLOCSIZE, ATTR_FILE_LINKCOUNT, attrlist, statfs,
};

/// Object types as `getattrlistbulk` reports them (the values of `enum vtype`).
pub const DIRECTORY: u32 = 2;
pub const SYMLINK: u32 = 5;

/// One directory entry. `name` borrows the listing buffer, so it is valid only inside the
/// callback that receives the entry.
pub struct Entry<'a> {
    pub name: &'a [u8],
    pub kind: u32,
    pub file_id: u64,
    pub link_count: u32,
    /// Bytes allocated on disk. Zero for directories.
    pub allocated: u64,
}

/// An open folder. Closed when dropped.
pub struct Directory {
    fd: OwnedFd,
    /// The volume holding this folder (`f_fsid`). File IDs are only unique within one volume.
    pub volume: u64,
}

impl Directory {
    /// Opens a folder by path.
    pub fn open(path: &Path) -> io::Result<Directory> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
        // SAFETY: c_path is NUL-terminated and lives for the call.
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a fresh descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut info = MaybeUninit::<statfs>::uninit();
        // SAFETY: `fd` is open and `info` is a valid out-buffer for statfs.
        if unsafe { libc::fstatfs(std::os::fd::AsRawFd::as_raw_fd(&fd), info.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fstatfs returned 0, so every field is initialised.
        let info = unsafe { info.assume_init() };
        // libc keeps fsid_t's two words private. They are two i32s, which the size check proves.
        const _: () = assert!(std::mem::size_of::<libc::fsid_t>() == 2 * std::mem::size_of::<i32>());
        // SAFETY: fsid_t is two i32 words with no padding, as the assert above checks.
        let [low, high]: [i32; 2] = unsafe { std::mem::transmute(info.f_fsid) };
        let volume = u64::from(high as u32) << 32 | u64::from(low as u32);
        Ok(Directory { fd, volume })
    }

    /// Calls `visit` for every entry. Returns `Ok(true)` when the whole folder was listed and
    /// `Ok(false)` when `cancelled` stopped it. `buf` is reused between calls, so a scan does not
    /// allocate per folder.
    pub fn for_each(
        &self,
        buf: &mut [u8],
        cancelled: impl Fn() -> bool,
        mut visit: impl FnMut(Entry<'_>),
    ) -> io::Result<bool> {
        let mut request = attrlist {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE | ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: 0,
            fileattr: ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE,
            forkattr: 0,
        };
        let fd = std::os::fd::AsRawFd::as_raw_fd(&self.fd);
        loop {
            if cancelled() {
                return Ok(false);
            }
            // SAFETY: `request` and `buf` are valid for the call, and `buf.len()` bounds the write.
            let count = unsafe {
                libc::getattrlistbulk(
                    fd,
                    (&mut request as *mut attrlist).cast::<c_void>(),
                    buf.as_mut_ptr().cast::<c_void>(),
                    buf.len(),
                    0,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(true);
            }
            let mut offset = 0;
            for _ in 0..count {
                let record = buf.get(offset..).ok_or_else(bad_record)?;
                let length = read_u32(record, 0).ok_or_else(bad_record)? as usize;
                if length < 4 || length > record.len() {
                    return Err(bad_record());
                }
                let entry = decode(&record[..length]).ok_or_else(bad_record)?;
                visit(entry);
                offset += length;
            }
        }
    }
}

fn bad_record() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "unexpected getattrlistbulk record")
}

/// Reads one record. The layout follows the request in `for_each`:
///   0   length (u32)
///   4   returned attribute sets (5 × u32); the file set is at 16
///   24  name reference (offset i32 relative to 24, length u32 including the NUL)
///   32  object type (u32)
///   36  file ID (u64, unaligned)
///   44  link count (u32) if returned, then allocated size (i64) if returned
fn decode(record: &[u8]) -> Option<Entry<'_>> {
    let returned_file = read_u32(record, 16)?;
    let name_offset = read_i32(record, 24)?;
    let name_len = read_u32(record, 28)? as usize;
    let name_start = usize::try_from(24 + i64::from(name_offset)).ok()?;
    let name = record.get(name_start..name_start.checked_add(name_len.saturating_sub(1))?)?;
    let kind = read_u32(record, 32)?;
    let file_id = read_u64(record, 36)?;
    let mut cursor = 44;
    let mut link_count = 1;
    if returned_file & ATTR_FILE_LINKCOUNT != 0 {
        link_count = read_u32(record, cursor)?;
        cursor += 4;
    }
    let mut allocated = 0;
    if returned_file & ATTR_FILE_ALLOCSIZE != 0 {
        allocated = read_i64(record, cursor)?.max(0) as u64;
    }
    Some(Entry { name, kind, file_id, link_count, allocated })
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_i32(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn read_i64(bytes: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A unique folder under the temp dir, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("duv-listing-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create fixture");
            Fixture(dir)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn list(dir: &Directory) -> Vec<(String, u32, u64)> {
        let mut buf = vec![0u8; 64 * 1024];
        let mut seen = Vec::new();
        let complete = dir
            .for_each(&mut buf, || false, |e| {
                seen.push((String::from_utf8_lossy(e.name).into_owned(), e.kind, e.allocated));
            })
            .expect("list fixture");
        assert!(complete);
        seen
    }

    #[test]
    fn lists_names_types_and_allocated_sizes() {
        let fx = Fixture::new("basic");
        fs::create_dir(fx.0.join("sub")).unwrap();
        fs::write(fx.0.join("one.bin"), vec![1u8; 5000]).unwrap();
        fs::write(fx.0.join("empty.bin"), b"").unwrap();
        let dir = Directory::open(&fx.0).expect("open");
        let mut seen = list(&dir);
        seen.sort();
        let names: Vec<&str> = seen.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"sub") && names.contains(&"one.bin") && names.contains(&"empty.bin"));
        let sub = seen.iter().find(|(n, _, _)| n == "sub").unwrap();
        assert_eq!(sub.1, DIRECTORY);
        let one = seen.iter().find(|(n, _, _)| n == "one.bin").unwrap();
        assert!(one.2 >= 5000 && one.2 % 512 == 0, "allocated {}", one.2);
        assert_eq!(seen.iter().find(|(n, _, _)| n == "empty.bin").unwrap().2, 0);
    }

    #[test]
    fn file_ids_match_stat_and_hard_links_share_one() {
        use std::os::unix::fs::MetadataExt;
        let fx = Fixture::new("ids");
        fs::write(fx.0.join("a.bin"), vec![1u8; 10]).unwrap();
        fs::hard_link(fx.0.join("a.bin"), fx.0.join("b.bin")).unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        let dir = Directory::open(&fx.0).unwrap();
        let mut ids = Vec::new();
        let mut links = Vec::new();
        dir.for_each(&mut buf, || false, |e| {
            if e.kind != DIRECTORY {
                ids.push((e.name.to_vec(), e.file_id));
                links.push(e.link_count);
            }
        })
        .unwrap();
        assert_eq!(ids.len(), 2);
        let stat_id = fs::metadata(fx.0.join("a.bin")).unwrap().ino();
        assert!(ids.iter().all(|(_, id)| *id == stat_id), "{ids:?} vs {stat_id}");
        assert!(links.iter().all(|&n| n == 2), "{links:?}");
    }

    #[test]
    fn cancel_stops_the_listing_early() {
        let fx = Fixture::new("cancel");
        fs::write(fx.0.join("a.bin"), b"x").unwrap();
        let dir = Directory::open(&fx.0).unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        let complete = dir.for_each(&mut buf, || true, |_| panic!("no entry after cancel")).unwrap();
        assert!(!complete);
    }

    #[test]
    fn missing_folder_is_an_error() {
        let missing = std::env::temp_dir().join("duv-listing-no-such-folder");
        assert!(Directory::open(&missing).is_err());
    }
}
