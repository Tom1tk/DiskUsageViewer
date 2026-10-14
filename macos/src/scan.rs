//! Scans one folder tree into a `Tree`. Worker threads take folder jobs from one shared stack.
//! A job opens its folder by full path and lists it in batches with `getattrlistbulk`.
//!
//! Mount points are not entered and take no entry, so a scan of `/` does not count
//! `/System/Volumes/Data` a second time. Withheld folders (see `withheld.rs`) are recorded with
//! size zero and counted as skipped, without being opened.
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::{Relaxed, SeqCst}};
use std::sync::{Condvar, Mutex};

use crate::listing::{self, Directory, Entry};
use crate::mounts::mount_table;
use crate::tree::{self, NO_PARENT, Parts, Tree};
use crate::withheld::withheld_folders;

/// Bytes per `getattrlistbulk` call.
const BATCH_BYTES: usize = 64 * 1024;
/// Folders between progress flushes.
const FLUSH_EVERY: u32 = 256;
const MAX_THREADS: usize = 16;
/// Most idle workers woken when a folder queues new jobs.
const MAX_WAKE: usize = 64;

/// Live counts for the status line. Workers add to them in batches, not per entry.
#[derive(Default)]
pub struct Progress {
    pub folders: AtomicU64,
    pub files: AtomicU64,
    pub skipped: AtomicU64,
    /// The folder most recently started, for the status line.
    pub current: Mutex<String>,
    cancel: AtomicBool,
}

impl Progress {
    pub fn request_cancel(&self) {
        self.cancel.store(true, SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(SeqCst)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScanError {
    Cancelled,
    Failed(String),
}

/// Folders the scan does not enter. Mount points take no entry at all; withheld folders are
/// recorded with size zero and counted as skipped.
#[derive(Default)]
struct Exclusions {
    mounts: HashSet<String>,
    withheld: HashSet<String>,
}

/// Scans `root` (a folder such as `/` or `/Volumes/Backup`) and returns its tree.
pub fn scan_volume(root: &Path, progress: &Progress) -> Result<Tree, ScanError> {
    let home = std::env::var("HOME").unwrap_or_default();
    let withheld = if home.is_empty() { HashSet::new() } else { withheld_folders(&home) };
    let mounts = mount_table().into_iter().map(|m| m.path).collect();
    scan_with(root, progress, Exclusions { mounts, withheld })
}

fn scan_with(root: &Path, progress: &Progress, excluded: Exclusions) -> Result<Tree, ScanError> {
    Directory::open(root)
        .map_err(|err| ScanError::Failed(format!("cannot open {}: {err}", root.display())))?;
    let shared = Shared {
        queue: Mutex::new(Queue {
            jobs: vec![Job {
                id: 0,
                parent: NO_PARENT,
                path: root.to_path_buf(),
                name: root.to_string_lossy().into_owned(),
            }],
            pending: 1,
        }),
        wake: Condvar::new(),
        next_id: AtomicU32::new(1),
        excluded,
    };
    let outs = std::thread::scope(|s| {
        let workers: Vec<_> = (0..thread_count()).map(|_| s.spawn(|| worker(&shared, progress))).collect();
        workers.into_iter().map(|w| w.join().expect("scan worker panicked")).collect::<Vec<Out>>()
    });
    if progress.is_cancelled() {
        return Err(ScanError::Cancelled);
    }
    let count = shared.next_id.load(SeqCst) as usize;
    Ok(tree::build(merge(count, outs)))
}

/// A folder waiting to be listed. The root has no parent and is named by its path.
struct Job {
    id: u32,
    parent: u32,
    path: PathBuf,
    /// The entry's name in the tree.
    name: String,
}

struct Queue {
    jobs: Vec<Job>,
    /// Jobs queued or being listed. Zero means the scan is finished.
    pending: usize,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Entry ids are handed out as entries are recorded, so a child always has a larger id than
    /// its parent, and the ids stay dense (skipped entries take none).
    next_id: AtomicU32,
    excluded: Exclusions,
}

impl Shared {
    /// Waits for the next job. None when the scan is finished or cancelled.
    fn next_job(&self, progress: &Progress) -> Option<Job> {
        let mut q = self.queue.lock().expect("scan queue lock");
        loop {
            if progress.is_cancelled() {
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
    /// (volume and file ID, entry id) for files with more than one link, for hard-link dedupe.
    file_ids: Vec<(u128, u32)>,
    skipped: u64,
}

impl Out {
    fn push(&mut self, id: u32, parent: u32, name: &str, size: u64, is_dir: bool) {
        let off = self.names.len();
        self.names.extend(name.encode_utf16());
        let len = self.names.len() - off;
        self.recs.push(Rec { id, parent, size, is_dir, off, len });
    }
}

/// The folder being listed.
struct Folder<'a> {
    path: &'a Path,
    id: u32,
    volume: u64,
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
    fn process(&mut self, job: Job, buf: &mut [u8]) -> Vec<Job> {
        let Job { id, parent, path, name } = job;
        self.unflushed.folders += 1;
        self.since_flush += 1;
        if self.since_flush == FLUSH_EVERY {
            self.flush();
            if let Ok(mut current) = self.progress.current.try_lock() {
                *current = path.to_string_lossy().into_owned();
            }
        }
        // Folder size 0 here: tree::build adds every child's size, files included.
        if self.sh.excluded.withheld.contains(path.to_string_lossy().as_ref()) {
            self.skip();
            self.out.push(id, parent, &name, 0, true);
            return Vec::new();
        }
        let Ok(dir) = Directory::open(&path) else {
            // Unreadable: kept as a folder of size 0, and counted as skipped.
            self.skip();
            self.out.push(id, parent, &name, 0, true);
            return Vec::new();
        };
        let mut children = Vec::new();
        if !self.list(&dir, &path, id, &mut children, buf) {
            self.skip();
        }
        self.out.push(id, parent, &name, 0, true);
        children
    }

    /// Records the entries of one folder. False when the listing ended early (error or cancel).
    fn list(
        &mut self,
        dir: &Directory,
        path: &Path,
        folder_id: u32,
        children: &mut Vec<Job>,
        buf: &mut [u8],
    ) -> bool {
        let folder = Folder { path, id: folder_id, volume: dir.volume };
        let progress = self.progress;
        let listed = dir.for_each(buf, || progress.is_cancelled(), |entry| {
            self.record(&folder, &entry, children);
        });
        matches!(listed, Ok(true))
    }

    /// Records one entry: a subfolder (queued, with its id), a file (counted now), or nothing.
    fn record(&mut self, folder: &Folder<'_>, entry: &Entry<'_>, children: &mut Vec<Job>) {
        let name = String::from_utf8_lossy(entry.name);
        if entry.kind == listing::SYMLINK || name == "." || name == ".." {
            return;
        }
        if entry.kind == listing::DIRECTORY {
            let path = folder.path.join(name.as_ref());
            // A mount point is another volume, which is scanned on its own. It takes no entry.
            if self.sh.excluded.mounts.contains(path.to_string_lossy().as_ref()) {
                return;
            }
            let id = self.sh.next_id.fetch_add(1, Relaxed);
            children.push(Job { id, parent: folder.id, path, name: name.into_owned() });
            return;
        }
        let id = self.sh.next_id.fetch_add(1, Relaxed);
        self.out.push(id, folder.id, &name, entry.allocated, false);
        // A file ID of zero means the volume gave none, so such files cannot be deduped.
        if entry.link_count > 1 && entry.file_id != 0 {
            let key = u128::from(folder.volume) << 64 | u128::from(entry.file_id);
            self.out.file_ids.push((key, id));
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
    let mut buf = vec![0u8; BATCH_BYTES];
    while let Some(job) = sh.next_job(progress) {
        let children = w.process(job, &mut buf);
        sh.complete(children);
    }
    w.flush();
    w.out
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

/// One worker per core, up to `MAX_THREADS`. No seek-penalty query: on an SSD parallel reads help.
fn thread_count() -> usize {
    std::thread::available_parallelism().map_or(1, NonZeroUsize::get).min(MAX_THREADS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// A unique folder under the temp dir. Permissions are restored before it is removed.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("duv-scan-{tag}-{}", std::process::id()));
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
            let _ = fs::set_permissions(self.path("locked"), fs::Permissions::from_mode(0o755));
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file(path: &Path, len: usize) {
        fs::write(path, vec![7u8; len]).expect("write fixture file");
    }

    fn scan_dir(dir: &Path) -> (Tree, Progress) {
        let progress = Progress::default();
        let tree = scan_with(dir, &progress, Exclusions::default()).expect("scan fixture");
        (tree, progress)
    }

    fn child(tree: &Tree, parent: u32, name: &str) -> Option<u32> {
        tree.sorted_children(parent).into_iter().find(|&id| tree.name(id) == name)
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
        assert!(tree.size(one) >= 1 && tree.size(one) % 512 == 0, "allocation {}", tree.size(one));
        assert_eq!(tree.size(child(&tree, root, "empty.bin").expect("empty.bin")), 0);
        assert_eq!(tree.size(child(&tree, root, "big.bin").expect("big.bin")), 8192);
        let sub = child(&tree, root, "sub").expect("sub");
        let nested = child(&tree, sub, "nested.bin").expect("nested.bin");
        assert_eq!(tree.size(nested), 8192);
        assert_eq!(tree.size(sub), 8192);
        assert_eq!(tree.size(root), tree.size(one) + 16384);
        assert!(tree.is_dir(sub) && !tree.is_dir(nested));
        assert_eq!(tree.path(nested), format!("{}/sub/nested.bin", fx.0.display()));
    }

    #[test]
    fn symbolic_link_is_not_followed_or_counted() {
        let fx = Fixture::new("symlink");
        fs::create_dir_all(fx.path("sub")).unwrap();
        file(&fx.path("sub/data.bin"), 5000);
        std::os::unix::fs::symlink(fx.path("sub"), fx.path("link")).unwrap();
        let (tree, _) = scan_dir(&fx.0);
        assert!(child(&tree, tree.root(), "link").is_none());
        assert_eq!(tree.size(tree.root()), 8192, "data counted once");
        assert_eq!((tree.folders, tree.files, tree.skipped), (2, 1, 0));
    }

    #[test]
    fn hard_link_is_counted_once() {
        let fx = Fixture::new("hardlink");
        file(&fx.path("hl1.bin"), 5000);
        fs::hard_link(fx.path("hl1.bin"), fx.path("hl2.bin")).unwrap();
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
        fs::set_permissions(fx.path("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let (tree, progress) = scan_dir(&fx.0);
        let locked = child(&tree, tree.root(), "locked").expect("locked is recorded");
        assert!(tree.is_dir(locked));
        assert_eq!(tree.size(locked), 0);
        assert_eq!((tree.skipped, progress.skipped.load(SeqCst)), (1, 1));
        assert_eq!((tree.folders, tree.files), (2, 1));
    }

    #[test]
    fn withheld_folder_is_recorded_but_not_opened() {
        let fx = Fixture::new("withheld");
        fs::create_dir_all(fx.path("private")).unwrap();
        file(&fx.path("private/hidden.bin"), 5000);
        file(&fx.path("open.bin"), 5000);
        let excluded = Exclusions {
            withheld: HashSet::from([fx.path("private").to_string_lossy().into_owned()]),
            ..Exclusions::default()
        };
        let progress = Progress::default();
        let tree = scan_with(&fx.0, &progress, excluded).expect("scan");
        let private = child(&tree, tree.root(), "private").expect("private is recorded");
        assert_eq!((tree.size(private), tree.child_count(private)), (0, 0));
        assert_eq!((tree.skipped, tree.files), (1, 1));
        assert_eq!(tree.size(tree.root()), 8192);
    }

    #[test]
    fn mount_point_takes_no_entry_and_is_not_entered() {
        let fx = Fixture::new("mount");
        fs::create_dir_all(fx.path("other")).unwrap();
        file(&fx.path("other/inside.bin"), 5000);
        file(&fx.path("here.bin"), 5000);
        let excluded = Exclusions {
            mounts: HashSet::from([fx.path("other").to_string_lossy().into_owned()]),
            ..Exclusions::default()
        };
        let progress = Progress::default();
        let tree = scan_with(&fx.0, &progress, excluded).expect("scan");
        assert!(child(&tree, tree.root(), "other").is_none());
        assert_eq!((tree.folders, tree.files, tree.skipped), (1, 1, 0));
        assert_eq!(tree.size(tree.root()), 8192);
    }

    #[test]
    fn missing_root_fails_with_the_path_in_the_message() {
        let missing = std::env::temp_dir().join("duv-scan-no-such-folder");
        match scan_with(&missing, &Progress::default(), Exclusions::default()) {
            Err(ScanError::Failed(msg)) => assert!(msg.contains("duv-scan-no-such-folder"), "{msg}"),
            other => panic!("expected Failed, got {:?}", other.map(|t| t.folders)),
        }
    }

    #[test]
    fn cancel_before_scan_returns_cancelled() {
        let fx = Fixture::new("cancel");
        file(&fx.path("a.bin"), 1);
        let progress = Progress::default();
        progress.request_cancel();
        assert_eq!(scan_with(&fx.0, &progress, Exclusions::default()).err(), Some(ScanError::Cancelled));
    }
}
