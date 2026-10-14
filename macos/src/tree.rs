//! The scanned tree in struct-of-arrays form: one name arena, children as CSR lists, and folder
//! sizes summed in one reverse pass. The scanner fills `Parts` and calls `build` once.
//! Copy of the Windows app's `src/tree.rs`, with `/` as the path separator.
use std::cmp::Ordering;

/// Parent id of the root.
pub(crate) const NO_PARENT: u32 = u32::MAX;

pub struct Tree {
    pub files: u64,
    pub folders: u64,
    pub skipped: u64,
    names: Vec<u16>,
    name_span: Vec<(u32, u32)>,
    parent: Vec<u32>,
    size: Vec<u64>,
    is_dir: Vec<bool>,
    child_start: Vec<u32>,
    children: Vec<u32>,
}

/// Entries indexed by id (dense, 0..n, root = 0), as the scanner found them.
pub(crate) struct Parts {
    pub names: Vec<u16>,
    pub name_span: Vec<(u32, u32)>,
    pub parent: Vec<u32>,
    pub size: Vec<u64>,
    pub is_dir: Vec<bool>,
    /// (volume and file ID, entry id) of each file with a size and an ID. Hard links share an ID.
    pub file_ids: Vec<(u128, u32)>,
    pub skipped: u64,
}

impl Parts {
    pub fn new(n: usize) -> Parts {
        Parts {
            names: Vec::new(),
            name_span: vec![(0, 0); n],
            parent: vec![NO_PARENT; n],
            size: vec![0; n],
            is_dir: vec![false; n],
            file_ids: Vec::new(),
            skipped: 0,
        }
    }

    pub fn set(&mut self, id: u32, parent: u32, name: &[u16], size: u64, is_dir: bool) {
        let i = id as usize;
        let off = u32::try_from(self.names.len()).expect("name arena over u32 units");
        self.names.extend_from_slice(name);
        self.name_span[i] = (off, name.len() as u32);
        self.parent[i] = parent;
        self.size[i] = size;
        self.is_dir[i] = is_dir;
    }
}

/// Keeps the size of the lowest entry id per file ID and zeroes the others.
fn dedupe(ids: &mut [(u128, u32)], size: &mut [u64]) {
    ids.sort_unstable();
    for pair in ids.windows(2) {
        if pair[0].0 == pair[1].0 {
            size[pair[1].1 as usize] = 0;
        }
    }
}

/// Dedupes hard links, sums folder sizes, and builds the child lists.
pub(crate) fn build(mut p: Parts) -> Tree {
    dedupe(&mut p.file_ids, &mut p.size);
    let n = p.parent.len();
    // A child's id is always larger than its parent's, so each subtree is complete when reached.
    for id in (1..n).rev() {
        let par = p.parent[id];
        if par != NO_PARENT {
            let s = p.size[id];
            p.size[par as usize] += s;
        }
    }
    let mut child_start = vec![0u32; n + 1];
    for id in 1..n {
        let par = p.parent[id];
        if par != NO_PARENT {
            child_start[par as usize + 1] += 1;
        }
    }
    for i in 0..n {
        child_start[i + 1] += child_start[i];
    }
    let mut cursor = child_start.clone();
    let mut children = vec![0u32; child_start[n] as usize];
    for id in 1..n {
        let par = p.parent[id];
        if par != NO_PARENT {
            let slot = &mut cursor[par as usize];
            children[*slot as usize] = id as u32;
            *slot += 1;
        }
    }
    let folders = p.is_dir.iter().filter(|&&d| d).count() as u64;
    Tree {
        files: n as u64 - folders,
        folders,
        skipped: p.skipped,
        names: p.names,
        name_span: p.name_span,
        parent: p.parent,
        size: p.size,
        is_dir: p.is_dir,
        child_start,
        children,
    }
}

impl Tree {
    pub fn root(&self) -> u32 {
        0
    }

    pub fn name(&self, id: u32) -> String {
        String::from_utf16_lossy(self.name_units(id))
    }

    fn name_units(&self, id: u32) -> &[u16] {
        let (off, len) = self.name_span[id as usize];
        &self.names[off as usize..][..len as usize]
    }

    pub fn size(&self, id: u32) -> u64 {
        self.size[id as usize]
    }

    pub fn is_dir(&self, id: u32) -> bool {
        self.is_dir[id as usize]
    }

    pub fn parent(&self, id: u32) -> Option<u32> {
        let par = self.parent[id as usize];
        (par != NO_PARENT).then_some(par)
    }

    pub fn child_count(&self, id: u32) -> usize {
        let i = id as usize;
        (self.child_start[i + 1] - self.child_start[i]) as usize
    }

    fn children_of(&self, id: u32) -> &[u32] {
        let i = id as usize;
        &self.children[self.child_start[i] as usize..self.child_start[i + 1] as usize]
    }

    /// Largest size first; equal sizes by name.
    pub fn sorted_children(&self, id: u32) -> Vec<u32> {
        let mut kids = self.children_of(id).to_vec();
        kids.sort_by(|&a, &b| self.size(b).cmp(&self.size(a)).then_with(|| self.cmp_names(a, b)));
        kids
    }

    fn cmp_names(&self, a: u32, b: u32) -> Ordering {
        // macOS volumes are case-insensitive by default. Folding ASCII only avoids an allocation per compare.
        let fold = |c: &u16| if *c < 128 { u16::from((*c as u8).to_ascii_lowercase()) } else { *c };
        self.name_units(a).iter().map(fold).cmp(self.name_units(b).iter().map(fold))
    }

    /// The full path. Under the root `/`, children are `/name`, not `//name`.
    pub fn path(&self, id: u32) -> String {
        let mut chain = Vec::new();
        let mut cur = id;
        while let Some(par) = self.parent(cur) {
            chain.push(cur);
            cur = par;
        }
        let mut out = self.name(cur);
        for &step in chain.iter().rev() {
            if !out.ends_with('/') {
                out.push('/');
            }
            out.push_str(&self.name(step));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// `/` holds Windows/ (a.bin 300, link1.bin 1000), B.txt 100, x.dat 300, alpha.txt 300,
    /// link2.bin 1000 (same file as link1.bin), and Zeta/ (empty).
    fn sample() -> Tree {
        let mut p = Parts::new(9);
        p.set(0, NO_PARENT, &units("/"), 0, true);
        p.set(1, 0, &units("Windows"), 0, true);
        p.set(2, 1, &units("a.bin"), 300, false);
        p.set(3, 0, &units("B.txt"), 100, false);
        p.set(4, 0, &units("x.dat"), 300, false);
        p.set(5, 0, &units("Zeta"), 0, true);
        p.set(6, 0, &units("alpha.txt"), 300, false);
        p.set(7, 1, &units("link1.bin"), 1000, false);
        p.set(8, 0, &units("link2.bin"), 1000, false);
        p.file_ids = vec![(42, 8), (42, 7)];
        p.skipped = 2;
        build(p)
    }

    #[test]
    fn totals_counts_and_dedupe() {
        let t = sample();
        assert_eq!((t.files, t.folders, t.skipped), (6, 3, 2));
        assert_eq!((t.size(7), t.size(8)), (1000, 0), "lowest id keeps the size");
        assert_eq!(t.size(1), 1300);
        assert_eq!(t.size(0), 2000);
    }

    #[test]
    fn children_sorted_by_size_then_name() {
        let t = sample();
        assert_eq!(t.sorted_children(0), vec![1, 6, 4, 3, 8, 5]);
        assert_eq!(t.sorted_children(1), vec![7, 2]);
        assert!(t.sorted_children(2).is_empty());
        assert_eq!((t.child_count(0), t.child_count(1), t.child_count(2)), (6, 2, 0));
    }

    #[test]
    fn path_parent_and_names() {
        let t = sample();
        assert_eq!(t.path(0), "/");
        assert_eq!(t.path(2), "/Windows/a.bin");
        assert_eq!(t.path(5), "/Zeta");
        assert_eq!(t.parent(0), None);
        assert_eq!(t.parent(2), Some(1));
        assert_eq!(t.name(6), "alpha.txt");
        assert!(t.is_dir(5) && !t.is_dir(2));
    }

    #[test]
    fn path_has_no_double_slash_under_a_folder_root() {
        let mut p = Parts::new(2);
        p.set(0, NO_PARENT, &units("/Users/tom/fixture"), 0, true);
        p.set(1, 0, &units("a.bin"), 1, false);
        assert_eq!(build(p).path(1), "/Users/tom/fixture/a.bin");
    }

    #[test]
    fn path_under_a_volume_root_has_one_slash() {
        let mut p = Parts::new(2);
        p.set(0, NO_PARENT, &units("/Volumes/Backup"), 0, true);
        p.set(1, 0, &units("Photos"), 0, true);
        assert_eq!(build(p).path(1), "/Volumes/Backup/Photos");
    }
}
