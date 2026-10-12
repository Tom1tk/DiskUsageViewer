//! Scans one volume into a `Tree`. Worker threads take folder jobs from one shared stack. A job
//! opens its folder relative to the parent's handle and lists it in batches, so no full paths are built.
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::{Relaxed, SeqCst}};
use std::sync::{Arc, Condvar, Mutex};

use crate::nt::{self, Batch, DirHandle, Entry, Layout};
use crate::tree::{self, NO_PARENT, Parts, Tree};

/// Bytes per NtQueryDirectoryFile call.
const BATCH_BYTES: usize = 64 * 1024;
/// Folders between progress flushes.
const FLUSH_EVERY: u32 = 256;
const MAX_THREADS: usize = 16;
/// Workers on a spinning disk, where parallel seeks slow the scan down.
const SPINNING_THREADS: usize = 2;
/// Most idle workers woken when a folder queues new jobs.
const MAX_WAKE: usize = 64;

pub struct Progress {
    pub folders: AtomicU64,
    pub files: AtomicU64,
    pub skipped: AtomicU64,
    pub current: Mutex<String>,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Self {
        Progress {
            folders: AtomicU64::new(0),
            files: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            current: Mutex::new(String::new()),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, SeqCst);
    }
}

#[derive(Debug)]
pub enum ScanError {
    Cancelled,
    Failed(String),
}

/// Scans `root` (a drive root such as `C:\`, or a folder path) and returns its tree.
pub fn scan_drive(root: &str, progress: &Progress) -> Result<Tree, ScanError> {
    let display = display_name(root);
    let mut buf = vec![0u64; BATCH_BYTES / 8];
    let probe = nt::open_dir(None, &nt_path(&display)).map_err(|status| open_error(root, status))?;
    let layout = nt::detect_layout(&probe, &mut buf);
    drop(probe);
    let threads = thread_count(root);
    let shared = Shared {
        queue: Mutex::new(Queue {
            jobs: vec![Job { id: 0, parent: NO_PARENT, name: display, parent_dir: None }],
            pending: 1,
        }),
        wake: Condvar::new(),
        next_id: AtomicU32::new(1),
        layout,
    };
    let outs = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads).map(|_| s.spawn(|| worker(&shared, progress))).collect();
        workers.into_iter().map(|w| w.join().expect("scan worker panicked")).collect::<Vec<Out>>()
    });
    if progress.cancel.load(SeqCst) {
        return Err(ScanError::Cancelled);
    }
    let count = shared.next_id.load(SeqCst) as usize;
    Ok(tree::build(merge(count, outs)))
}

/// A folder waiting to be listed. The root has no parent handle and is named by its display name.
struct Job {
    id: u32,
    parent: u32,
    /// The entry's name; relative to `parent_dir` unless this is the root.
    name: Vec<u16>,
    /// The parent folder's handle, shared by its subfolders. None only for the root.
    parent_dir: Option<Arc<DirHandle>>,
}

struct Queue {
    jobs: Vec<Job>,
    /// Jobs queued or being listed. Zero means the scan is finished.
    pending: usize,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    next_id: AtomicU32,
    layout: Layout,
}

impl Shared {
    /// Waits for the next job. None when the scan is finished or cancelled.
    fn next_job(&self, progress: &Progress) -> Option<Job> {
        let mut q = self.queue.lock().expect("scan queue lock");
        loop {
            if progress.cancel.load(SeqCst) {
                self.wake.notify_all();
                return None;
            }
            if let Some(job) = q.jobs.pop() {
                return Some(job);
            }
            if q.pending == 0 {
                return None;
            }
            q = self.wake.wait(q).expect("scan queue wait");
        }
    }

    /// Retires the job just listed and queues the subfolders it found, under one lock.
    fn complete(&self, children: Vec<Job>) {
        let added = children.len();
        let mut q = self.queue.lock().expect("scan queue lock");
        q.pending = q.pending + added - 1;
        q.jobs.extend(children);
        let finished = q.pending == 0;
        drop(q);
        if finished {
            self.wake.notify_all();
        } else {
            for _ in 0..added.min(MAX_WAKE) {
                self.wake.notify_one();
            }
        }
    }
}

/// Entry counts not yet added to the shared `Progress`.
#[derive(Default)]
struct Counts {
    folders: u64,
    files: u64,
    skipped: u64,
}

/// One recorded entry; its name sits in `Out::names` at `off..off + len`.
struct Rec {
    id: u32,
    parent: u32,
    size: u64,
    is_dir: bool,
    off: usize,
    len: usize,
}

/// What one worker recorded.
#[derive(Default)]
struct Out {
    names: Vec<u16>,
    recs: Vec<Rec>,
    /// (file ID, entry id) for files with an ID, for hard-link dedupe.
    file_ids: Vec<(u128, u32)>,
    skipped: u64,
}

impl Out {
    fn push(&mut self, id: u32, parent: u32, name: impl Iterator<Item = u16>, size: u64, is_dir: bool) {
        let off = self.names.len();
        self.names.extend(name);
        let len = self.names.len() - off;
        self.recs.push(Rec { id, parent, size, is_dir, off, len });
    }
}

/// A folder being listed: its id, its handle, and the subfolder jobs found so far.
struct Folder {
    id: u32,
    dir: Arc<DirHandle>,
    children: Vec<Job>,
}

struct Worker<'a> {
    sh: &'a Shared,
    progress: &'a Progress,
    out: Out,
    unflushed: Counts,
    since_flush: u32,
}

impl Worker<'_> {
    /// Lists one folder and returns the subfolder jobs it found.
    fn process(&mut self, job: Job, buf: &mut [u64]) -> Vec<Job> {
        let Job { id, parent, name, parent_dir } = job;
        self.unflushed.folders += 1;
        self.since_flush += 1;
        if self.since_flush == FLUSH_EVERY {
            self.flush();
            if let Ok(mut current) = self.progress.current.try_lock() {
                *current = String::from_utf16_lossy(&name);
            }
        }
        let opened = open_folder(&name, parent_dir.as_deref());
        drop(parent_dir);
        let Ok(dir) = opened else {
            // Unreadable: kept as a folder of size 0, and counted as skipped.
            self.skip();
            self.out.push(id, parent, name.iter().copied(), 0, true);
            return Vec::new();
        };
        let mut folder = Folder { id, dir: Arc::new(dir), children: Vec::new() };
        if !self.list_folder(&mut folder, buf) {
            self.skip();
        }
        // Folder size 0 here: tree::build adds every child's size, files included.
        self.out.push(id, parent, name.iter().copied(), 0, true);
        folder.children
    }

    /// Records the entries of `folder`. False when the listing ended early (error, cancel, bad record).
    fn list_folder(&mut self, folder: &mut Folder, buf: &mut [u64]) -> bool {
        let mut restart = true;
        loop {
            if self.progress.cancel.load(Relaxed) {
                return false;
            }
            match nt::next_batch(&folder.dir, self.sh.layout, restart, buf) {
                Batch::End => return true,
                Batch::Error => return false,
                Batch::Entries(bytes) => {
                    restart = false;
                    if !self.record_batch(folder, bytes) {
                        return false;
                    }
                }
            }
        }
    }

    /// Records every entry in one batch. False when a record does not fit in the batch.
    fn record_batch(&mut self, folder: &mut Folder, bytes: &[u8]) -> bool {
        let mut at = 0;
        loop {
            let Some(entry) = nt::parse_entry(bytes, at, self.sh.layout) else {
                return false;
            };
            self.record(folder, &entry);
            if entry.next == 0 {
                return true;
            }
            at += entry.next;
        }
    }

    /// Records one entry: a subfolder (queued, with its id), a file (counted now), or nothing.
    fn record(&mut self, folder: &mut Folder, entry: &Entry<'_>) {
        // Skipped links are not counted in `skipped`; their targets are counted where they live.
        if nt::is_dot_entry(entry.name) || entry.is_skipped_link() {
            return;
        }
        let id = self.sh.next_id.fetch_add(1, Relaxed);
        if entry.is_dir() {
            let name: Vec<u16> = nt::name_units(entry.name).collect();
            folder.children.push(Job { id, parent: folder.id, name, parent_dir: Some(Arc::clone(&folder.dir)) });
            return;
        }
        self.out.push(id, folder.id, nt::name_units(entry.name), entry.allocation, false);
        // A zero ID means the volume gave none (some network shares); such files must not be deduped.
        if let Some(file_id) = entry.file_id.filter(|&f| f != 0) {
            self.out.file_ids.push((file_id, id));
        }
        self.unflushed.files += 1;
    }

    fn skip(&mut self) {
        self.out.skipped += 1;
        self.unflushed.skipped += 1;
    }

    /// Adds the unflushed counts to the shared progress.
    fn flush(&mut self) {
        let c = std::mem::take(&mut self.unflushed);
        self.progress.folders.fetch_add(c.folders, Relaxed);
        self.progress.files.fetch_add(c.files, Relaxed);
        self.progress.skipped.fetch_add(c.skipped, Relaxed);
        self.since_flush = 0;
    }
}

fn worker(sh: &Shared, progress: &Progress) -> Out {
    let mut w = Worker { sh, progress, out: Out::default(), unflushed: Counts::default(), since_flush: 0 };
    let mut buf = vec![0u64; BATCH_BYTES / 8];
    while let Some(job) = sh.next_job(progress) {
        let children = w.process(job, &mut buf);
        sh.complete(children);
    }
    w.flush();
    w.out
}

/// Opens a folder under its parent's handle, or by absolute NT path for the root.
fn open_folder(name: &[u16], parent: Option<&DirHandle>) -> Result<DirHandle, i32> {
    match parent {
        Some(dir) => nt::open_dir(Some(dir), name),
        None => nt::open_dir(None, &nt_path(name)),
    }
}

/// Folds every worker's records into one table indexed by entry id (`count` ids in all).
fn merge(count: usize, outs: Vec<Out>) -> Parts {
    let mut parts = Parts::new(count);
    for out in outs {
        for r in &out.recs {
            parts.set(r.id, r.parent, &out.names[r.off..r.off + r.len], r.size, r.is_dir);
        }
        parts.file_ids.extend(out.file_ids);
        parts.skipped += out.skipped;
    }
    parts
}

/// Two workers on a spinning disk; otherwise one per core, up to `MAX_THREADS`.
fn thread_count(root: &str) -> usize {
    if crate::disk::has_seek_penalty(root) == Some(true) {
        return SPINNING_THREADS;
    }
    std::thread::available_parallelism().map_or(1, NonZeroUsize::get).min(MAX_THREADS)
}

/// The root's name in the tree: `C:\` for a drive, otherwise the path with no trailing backslash.
fn display_name(root: &str) -> Vec<u16> {
    let trimmed = root.trim_end_matches('\\');
    let text = if trimmed.ends_with(':') { format!("{trimmed}\\") } else { trimmed.to_owned() };
    text.encode_utf16().collect()
}

/// NT path of an absolute Win32 path, e.g. `\??\C:\Windows`.
fn nt_path(display: &[u16]) -> Vec<u16> {
    r"\??\".encode_utf16().chain(display.iter().copied()).collect()
}

fn open_error(root: &str, status: i32) -> ScanError {
    ScanError::Failed(format!("cannot open {root} (NTSTATUS {status:#010X})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A unique folder under the temp dir, removed on drop. The test junction and the deny ACE
    /// are removed first, because remove_dir_all would fail on the deny ACE.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("dv-scan-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create fixture");
            Fixture(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = Command::new("icacls").arg(self.path("locked")).args(["/remove:d", "*S-1-1-0"]).output();
            let _ = Command::new("cmd").args(["/C", "rmdir"]).arg(self.path("jct")).output();
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file(path: &Path, len: usize) {
        fs::write(path, vec![7u8; len]).expect("write fixture file");
    }

    /// `cmd /C mklink <flag> link target`. False when mklink fails (e.g. no NTFS).
    fn mklink(flag: &str, link: &Path, target: &Path) -> bool {
        Command::new("cmd")
            .args(["/C", "mklink", flag])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn scan_dir(dir: &Path) -> (Tree, Progress) {
        let progress = Progress::new();
        let tree = scan_drive(&dir.to_string_lossy(), &progress).expect("scan fixture");
        (tree, progress)
    }

    fn child(tree: &Tree, parent: u32, name: &str) -> Option<u32> {
        tree.sorted_children(parent).into_iter().find(|&id| tree.name(id) == name)
    }

    fn name_of(root: &str) -> String {
        String::from_utf16_lossy(&display_name(root))
    }

    #[test]
    fn root_names_have_no_trailing_backslash_except_drives() {
        assert_eq!(name_of("C:"), r"C:\");
        assert_eq!(name_of(r"C:\"), r"C:\");
        assert_eq!(name_of(r"C:\Users\x\"), r"C:\Users\x");
    }

    #[test]
    fn nested_tree_counts_sizes_and_paths() {
        let fx = Fixture::new("nested");
        fs::create_dir_all(fx.path("sub/deeper")).unwrap();
        file(&fx.path("one.bin"), 1);
        file(&fx.path("empty.bin"), 0);
        file(&fx.path("big.bin"), 5000);
        file(&fx.path("sub/nested.bin"), 5000);
        let (tree, progress) = scan_dir(&fx.0);
        let root = tree.root();
        assert_eq!((tree.files, tree.folders, tree.skipped), (4, 3, 0));
        assert_eq!(progress.folders.load(SeqCst), 3);
        assert_eq!(progress.files.load(SeqCst), 4);
        let one = child(&tree, root, "one.bin").expect("one.bin");
        // Observed value is recorded in the report; 1 byte must occupy some allocation.
        assert!(tree.size(one) >= 1 && tree.size(one) % 8 == 0, "one.bin allocation {}", tree.size(one));
        assert_eq!(tree.size(child(&tree, root, "empty.bin").expect("empty.bin")), 0);
        assert_eq!(tree.size(child(&tree, root, "big.bin").expect("big.bin")), 8192);
        let sub = child(&tree, root, "sub").expect("sub");
        let nested = child(&tree, sub, "nested.bin").expect("nested.bin");
        assert_eq!(tree.size(nested), 8192);
        assert_eq!(tree.size(sub), 8192);
        assert_eq!(tree.size(root), tree.size(one) + 16384);
        assert!(tree.is_dir(sub) && !tree.is_dir(nested));
        assert_eq!(tree.path(nested), format!(r"{}\sub\nested.bin", fx.0.display()));
    }

    #[test]
    fn junction_is_not_followed_or_counted_twice() {
        let fx = Fixture::new("junction");
        fs::create_dir_all(fx.path("sub")).unwrap();
        file(&fx.path("sub/data.bin"), 5000);
        if !mklink("/J", &fx.path("jct"), &fx.path("sub")) {
            eprintln!("junction test skipped: mklink /J failed");
            return;
        }
        let (tree, _) = scan_dir(&fx.0);
        assert!(child(&tree, tree.root(), "jct").is_none());
        assert_eq!(tree.size(tree.root()), 8192, "data counted once");
        assert_eq!((tree.folders, tree.files, tree.skipped), (2, 1, 0));
    }

    #[test]
    fn hard_link_is_counted_once() {
        let fx = Fixture::new("hardlink");
        file(&fx.path("hl1.bin"), 5000);
        if !mklink("/H", &fx.path("hl2.bin"), &fx.path("hl1.bin")) {
            eprintln!("hard link test skipped: mklink /H failed");
            return;
        }
        let (tree, _) = scan_dir(&fx.0);
        let root = tree.root();
        let a = tree.size(child(&tree, root, "hl1.bin").expect("hl1.bin"));
        let b = tree.size(child(&tree, root, "hl2.bin").expect("hl2.bin"));
        assert!((a == 0) != (b == 0), "exactly one name keeps the size: {a} and {b}");
        assert_eq!(a + b, 8192);
        assert_eq!((tree.files, tree.size(root)), (2, 8192));
    }

    #[test]
    fn unreadable_folder_is_kept_with_size_zero_and_skipped() {
        let fx = Fixture::new("denied");
        fs::create_dir_all(fx.path("locked")).unwrap();
        file(&fx.path("locked/secret.bin"), 5000);
        file(&fx.path("open.bin"), 5000);
        let denied = Command::new("icacls")
            .arg(fx.path("locked"))
            .args(["/deny", "*S-1-1-0:(OI)(CI)(F)"])
            .output();
        if !denied.is_ok_and(|o| o.status.success()) {
            eprintln!("unreadable folder test skipped: icacls deny failed");
            return;
        }
        let (tree, progress) = scan_dir(&fx.0);
        let locked = child(&tree, tree.root(), "locked").expect("locked is recorded");
        assert!(tree.is_dir(locked));
        assert_eq!(tree.size(locked), 0);
        assert_eq!((tree.skipped, progress.skipped.load(SeqCst)), (1, 1));
        assert_eq!((tree.folders, tree.files), (2, 1));
    }

    #[test]
    fn missing_root_fails_with_the_path_in_the_message() {
        let missing = std::env::temp_dir().join("dv-scan-no-such-folder");
        match scan_drive(&missing.to_string_lossy(), &Progress::new()) {
            Err(ScanError::Failed(msg)) => assert!(msg.contains("dv-scan-no-such-folder"), "{msg}"),
            other => panic!("expected Failed, got {:?}", other.map(|t| t.folders)),
        }
    }

    #[test]
    fn cancel_before_scan_returns_cancelled() {
        let fx = Fixture::new("cancel");
        file(&fx.path("a.bin"), 1);
        let progress = Progress::new();
        progress.request_cancel();
        assert!(matches!(scan_drive(&fx.0.to_string_lossy(), &progress), Err(ScanError::Cancelled)));
    }
}
