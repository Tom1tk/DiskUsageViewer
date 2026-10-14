//! The treemap blocks for one folder: the layout of its children, with the rest folded into one
//! "more items" block. Port of the Windows app's `build_blocks` and `make_blocks`.
use crate::format::{grouped, human_size, percent};
use crate::tree::Tree;
use crate::treemap::{self, Rect};

/// Children drawn as their own blocks. The rest fold into one block.
pub const MAX_BLOCKS: usize = 150;
/// Folders take these colours in turn. Files and the "more" block have colours of their own.
pub const PALETTE: [u32; 10] = [
    0xa8dadc, 0xf4a261, 0xe9c46a, 0x90be6d, 0xf9c74f, 0xf8961e, 0xb5e48c, 0xc3aed6, 0xffb4a2, 0x9ad1d4,
];
pub const FILE_COLOUR: u32 = 0xdfe3e8;
pub const MORE_COLOUR: u32 = 0xeeeeee;

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Relative to the top-left of the bounds the blocks were laid out in.
    pub rect: Rect,
    /// None for the "more items" block, which cannot be opened.
    pub id: Option<u32>,
    pub name: String,
    pub size: u64,
    /// `0xRRGGBB`.
    pub colour: u32,
    pub hover_text: String,
}

/// A child of the focused folder that has a size.
struct Kid {
    id: u32,
    size: u64,
    is_dir: bool,
}

/// One layout slot: a listed child, or the rest of the children folded into one block.
enum Item<'a> {
    Kid(&'a Kid),
    More { count: usize, size: u64 },
}

impl Item<'_> {
    fn size(&self) -> u64 {
        match self {
            Item::Kid(kid) => kid.size,
            Item::More { size, .. } => *size,
        }
    }
}

/// The blocks for the children of `folder`, laid out in `bounds` (width, height).
pub fn folder_blocks(tree: &Tree, folder: u32, bounds: (f64, f64)) -> Vec<Block> {
    let (w, h) = bounds;
    if !(w > 0.0 && h > 0.0) {
        return Vec::new();
    }
    let kids: Vec<Kid> = tree
        .sorted_children(folder)
        .into_iter()
        .map(|id| Kid { id, size: tree.size(id), is_dir: tree.is_dir(id) })
        .filter(|kid| kid.size > 0)
        .collect();
    make_blocks(&kids, tree.size(folder), bounds, |id| tree.name(id))
}

/// `kids` are sorted largest first and have sizes above zero. The first `MAX_BLOCKS` become
/// blocks. The rest become one "more items" block, placed by size so the layout still sees the
/// sizes largest first.
fn make_blocks(
    kids: &[Kid],
    folder_size: u64,
    bounds: (f64, f64),
    name_of: impl Fn(u32) -> String,
) -> Vec<Block> {
    let (listed, rest) = kids.split_at(kids.len().min(MAX_BLOCKS));
    let mut items: Vec<Item> = listed.iter().map(Item::Kid).collect();
    if !rest.is_empty() {
        let rest_size: u64 = rest.iter().map(|kid| kid.size).sum();
        let at = listed.partition_point(|kid| kid.size >= rest_size);
        items.insert(at, Item::More { count: rest.len(), size: rest_size });
    }
    let sizes: Vec<f64> = items.iter().map(|item| item.size() as f64).collect();
    let (w, h) = bounds;
    let rects = treemap::layout(&sizes, Rect { x: 0.0, y: 0.0, w, h });
    let mut folders = 0usize;
    items
        .iter()
        .zip(rects)
        .map(|(item, rect)| match item {
            Item::Kid(kid) => {
                let colour = if kid.is_dir {
                    folders += 1;
                    PALETTE[(folders - 1) % PALETTE.len()]
                } else {
                    FILE_COLOUR
                };
                kid_block(kid, name_of(kid.id), colour, rect, folder_size)
            }
            Item::More { count, size } => more_block(*count, *size, rect, folder_size),
        })
        .collect()
}

fn kid_block(kid: &Kid, name: String, colour: u32, rect: Rect, folder_size: u64) -> Block {
    Block {
        rect,
        id: Some(kid.id),
        size: kid.size,
        hover_text: format!(
            "{name}    {}    {} of this folder",
            human_size(kid.size),
            percent(kid.size, folder_size)
        ),
        name,
        colour,
    }
}

fn more_block(count: usize, size: u64, rect: Rect, folder_size: u64) -> Block {
    let counted = grouped(count as u64);
    Block {
        rect,
        id: None,
        size,
        name: format!("{counted} more items"),
        colour: MORE_COLOUR,
        hover_text: format!(
            "{counted} smaller items    {}    {} of this folder",
            human_size(size),
            percent(size, folder_size)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NO_PARENT, Parts, build};

    /// A root with `count` children named `c<i>`. Child `i` is a folder when `i` is even, and
    /// has size `count - i`, so the sizes run largest first.
    fn wide_folder(count: usize) -> Tree {
        let n = count + 1;
        let mut p = Parts::new(n);
        let root: Vec<u16> = "/".encode_utf16().collect();
        p.set(0, NO_PARENT, &root, 0, true);
        for i in 0..count {
            let name: Vec<u16> = format!("c{i}").encode_utf16().collect();
            p.set(i as u32 + 1, 0, &name, (count - i) as u64, i % 2 == 0);
        }
        build(p)
    }

    #[test]
    fn small_folder_gets_one_block_per_child_with_no_more_block() {
        let tree = wide_folder(5);
        let blocks = folder_blocks(&tree, 0, (300.0, 200.0));
        assert_eq!(blocks.len(), 5);
        assert!(blocks.iter().all(|b| b.id.is_some()));
    }

    #[test]
    fn children_past_the_limit_fold_into_one_more_block() {
        let tree = wide_folder(MAX_BLOCKS + 30);
        let blocks = folder_blocks(&tree, 0, (800.0, 600.0));
        assert_eq!(blocks.len(), MAX_BLOCKS + 1);
        let more: Vec<&Block> = blocks.iter().filter(|b| b.id.is_none()).collect();
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].name, "30 more items");
        assert!(more[0].hover_text.starts_with("30 smaller items    "));
        assert_eq!(more[0].colour, MORE_COLOUR);
    }

    #[test]
    fn folders_take_palette_colours_in_turn_and_files_do_not() {
        let tree = wide_folder(6);
        let blocks = folder_blocks(&tree, 0, (300.0, 200.0));
        let folder_colours: Vec<u32> = blocks
            .iter()
            .filter(|b| tree.is_dir(b.id.unwrap()))
            .map(|b| b.colour)
            .collect();
        assert_eq!(folder_colours, vec![PALETTE[0], PALETTE[1], PALETTE[2]]);
        assert!(blocks
            .iter()
            .filter(|b| !tree.is_dir(b.id.unwrap()))
            .all(|b| b.colour == FILE_COLOUR));
    }

    #[test]
    fn hover_text_gives_size_and_share_of_the_folder() {
        let tree = wide_folder(4);
        let blocks = folder_blocks(&tree, 0, (300.0, 200.0));
        let first = blocks.iter().find(|b| b.name == "c0").expect("c0");
        // c0 has size 4 of the folder's total of 4 + 3 + 2 + 1 = 10.
        assert_eq!(first.hover_text, "c0    4 B    40.0% of this folder");
    }

    #[test]
    fn no_blocks_for_an_empty_or_flat_area() {
        let tree = wide_folder(3);
        assert!(folder_blocks(&tree, 0, (0.0, 200.0)).is_empty());
        // Node 2 is c1, a file (odd indexes are files), so it has no children to lay out.
        assert!(folder_blocks(&tree, 2, (300.0, 200.0)).is_empty(), "a file has no children");
    }
}
