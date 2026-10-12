//! The folder tree on the left. Each item's lParam is its entry id, or `PLACEHOLDER`
//! for the dummy child that gives a folder its expand arrow.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Controls::{
    HTREEITEM, TVE_EXPAND, TVGN_CARET, TVGN_CHILD, TVGN_NEXT, TVGN_ROOT, TVI_LAST, TVI_ROOT,
    TVIF_PARAM, TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVITEMW, TVM_DELETEITEM,
    TVM_ENSUREVISIBLE, TVM_EXPAND, TVM_GETITEMW, TVM_GETNEXTITEM, TVM_INSERTITEMW,
    TVM_SELECTITEM, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, WM_SETREDRAW, WS_BORDER, WS_CHILD, WS_VISIBLE, WS_VSCROLL,
};

use crate::format::{human_size, percent};
use crate::tree::Tree;
use crate::win::{send, wide};

pub const PLACEHOLDER: u32 = u32::MAX;

pub fn create(parent: HWND, hinstance: HINSTANCE) -> HWND {
    let class = wide("SysTreeView32");
    let style = WS_CHILD
        | WS_VISIBLE
        | WS_BORDER
        | WS_VSCROLL
        | TVS_HASLINES
        | TVS_HASBUTTONS
        | TVS_LINESATROOT
        | TVS_SHOWSELALWAYS;
    // SAFETY: `class` is NUL-terminated and outlives the call. No window name or creation
    // parameter is passed. The size is 0 until the parent's layout moves the control.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            style,
            0,
            0,
            0,
            0,
            parent,
            null_mut(),
            hinstance,
            null(),
        )
    }
}

/// Replaces the whole tree with the root and its direct children. The root is expanded and selected.
pub fn show_root(hwnd: HWND, tree: &Tree) {
    clear(hwnd);
    set_redraw(hwnd, false);
    let root = tree.root();
    let root_text = format!("{}    {}", tree.name(root), human_size(tree.size(root)));
    let item = insert_item(hwnd, TVI_ROOT, TVI_LAST, &root_text, root as isize);
    insert_children(hwnd, tree, root, item);
    send(hwnd, TVM_EXPAND, TVE_EXPAND as usize, item);
    send(hwnd, TVM_SELECTITEM, TVGN_CARET as usize, item);
    set_redraw(hwnd, true);
}

pub fn clear(hwnd: HWND) {
    send(hwnd, TVM_DELETEITEM, 0, TVI_ROOT);
}

/// TVN_ITEMEXPANDING: replaces the placeholder child with the real children, once.
pub fn on_expanding(hwnd: HWND, tree: &Tree, item: HTREEITEM) {
    fill_children(hwnd, tree, item);
}

/// Expands the ancestors of `id` root-first, then selects it and scrolls it into view.
pub fn reveal(hwnd: HWND, tree: &Tree, id: u32) {
    let mut chain = Vec::new();
    let mut current = Some(id);
    while let Some(node) = current {
        chain.push(node);
        current = tree.parent(node);
    }
    // `chain` ends with the root, so the loop starts at the root's child.
    let mut item = send(hwnd, TVM_GETNEXTITEM, TVGN_ROOT as usize, 0);
    for &node in chain.iter().rev().skip(1) {
        expand(hwnd, tree, item);
        item = match find_child(hwnd, item, node) {
            Some(found) => found,
            None => return,
        };
    }
    if item != 0 {
        send(hwnd, TVM_SELECTITEM, TVGN_CARET as usize, item);
        send(hwnd, TVM_ENSUREVISIBLE, 0, item);
    }
}

pub fn item_id(hwnd: HWND, item: HTREEITEM) -> Option<u32> {
    item_lparam(hwnd, item).filter(|&id| id != PLACEHOLDER)
}

/// Like `item_id`, but also returns the placeholder marker. `None` when the item does not exist.
fn item_lparam(hwnd: HWND, item: HTREEITEM) -> Option<u32> {
    let mut tvi = TVITEMW {
        mask: TVIF_PARAM,
        hItem: item,
        state: 0,
        stateMask: 0,
        pszText: null_mut(),
        cchTextMax: 0,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };
    if send(hwnd, TVM_GETITEMW, 0, &mut tvi as *mut TVITEMW as isize) == 0 {
        return None;
    }
    Some(tvi.lParam as u32)
}

fn expand(hwnd: HWND, tree: &Tree, item: HTREEITEM) {
    fill_children(hwnd, tree, item);
    send(hwnd, TVM_EXPAND, TVE_EXPAND as usize, item);
}

/// Swaps a folder's placeholder child for its real children. Does nothing when there is no placeholder.
fn fill_children(hwnd: HWND, tree: &Tree, item: HTREEITEM) {
    let first = send(hwnd, TVM_GETNEXTITEM, TVGN_CHILD as usize, item);
    if first == 0 || item_lparam(hwnd, first) != Some(PLACEHOLDER) {
        return;
    }
    let Some(id) = item_id(hwnd, item) else {
        return;
    };
    set_redraw(hwnd, false);
    send(hwnd, TVM_DELETEITEM, 0, first);
    insert_children(hwnd, tree, id, item);
    set_redraw(hwnd, true);
}

fn insert_children(hwnd: HWND, tree: &Tree, parent_id: u32, parent_item: HTREEITEM) {
    let parent_size = tree.size(parent_id);
    for child in tree.sorted_children(parent_id) {
        let size = tree.size(child);
        let text = format!(
            "{}    {}    {}",
            tree.name(child),
            human_size(size),
            percent(size, parent_size)
        );
        let item = insert_item(hwnd, parent_item, TVI_LAST, &text, child as isize);
        if tree.is_dir(child) && tree.child_count(child) > 0 {
            insert_item(hwnd, item, TVI_LAST, "", PLACEHOLDER as isize);
        }
    }
}

fn insert_item(
    hwnd: HWND,
    parent: HTREEITEM,
    after: HTREEITEM,
    text: &str,
    lparam: isize,
) -> HTREEITEM {
    let mut buf = wide(text);
    let item = TVITEMW {
        mask: TVIF_TEXT | TVIF_PARAM,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: buf.as_mut_ptr(),
        cchTextMax: buf.len() as i32,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: lparam,
    };
    let insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: after,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };
    send(hwnd, TVM_INSERTITEMW, 0, &insert as *const TVINSERTSTRUCTW as isize)
}

fn find_child(hwnd: HWND, parent: HTREEITEM, id: u32) -> Option<HTREEITEM> {
    let mut item = send(hwnd, TVM_GETNEXTITEM, TVGN_CHILD as usize, parent);
    while item != 0 {
        if item_lparam(hwnd, item) == Some(id) {
            return Some(item);
        }
        item = send(hwnd, TVM_GETNEXTITEM, TVGN_NEXT as usize, item);
    }
    None
}

fn set_redraw(hwnd: HWND, on: bool) {
    send(hwnd, WM_SETREDRAW, usize::from(on), 0);
    if on {
        // SAFETY: invalidates the whole client area; no rectangle pointer is passed.
        unsafe { InvalidateRect(hwnd, null(), 1) };
    }
}
