//! The main window: drive picker, Scan and Cancel, status line, folder tree and treemap.
//! `run` owns the `AppState` box. WM_NCCREATE stores its address in GWLP_USERDATA. Handlers
//! take `&AppState` only and never hold a RefCell borrow across a call that can send a message
//! back into this window.

use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{COLOR_BTNFACE, DEFAULT_GUI_FONT, GetStockObject, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    HTREEITEM, ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR,
    NMTREEVIEWW, TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem,
    GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBS_DROPDOWNLIST,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, IDC_ARROW, KillTimer,
    LoadCursorW, MSG, MoveWindow, PostQuitMessage, RegisterClassExW, SIZE_MINIMIZED,
    SW_SHOW, SetProcessDPIAware, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_NCCREATE, WM_NCDESTROY, WM_NOTIFY,
    WM_SETFONT, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP,
    WS_VISIBLE,
};

use crate::disk;
use crate::format::{grouped, human_size};
use crate::scan::{self, Progress, ScanError};
use crate::tree::Tree;
use crate::tree_view;
use crate::treemap_view::{self, WM_APP_REVEAL};
use crate::win::{self, message, post, scale, send, wide};

const TITLE: &str = "Disk Usage Viewer";
const CLASS_NAME: &str = "DiskUsageViewerMain";
const START_TEXT: &str =
    "Pick a drive, then press Scan. Folders Windows will not let you read are skipped.";
/// Posted by the scan thread once its result is in the slot.
const WM_APP_SCAN_DONE: u32 = WM_APP + 1;
/// Posted from WM_CREATE when `--autostart` was given.
const WM_APP_AUTOSTART: u32 = WM_APP + 3;
const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 150;
const TAIL_CHARS: usize = 70;
/// Static style: a "&" in a path shows literally instead of as an accelerator.
const SS_NOPREFIX: u32 = 0x80;
/// Static style: a one-line label is centred vertically in its box, to line up with the row's controls.
const SS_CENTERIMAGE: u32 = 0x200;

type Slot = Arc<Mutex<Option<Result<Tree, ScanError>>>>;

#[derive(Clone, Copy)]
struct Controls {
    label: HWND,
    combo: HWND,
    scan: HWND,
    cancel: HWND,
    status: HWND,
    tree: HWND,
    treemap: HWND,
}

struct Scan {
    root: String,
    progress: Arc<Progress>,
    result: Slot,
    started: Instant,
}

struct AppState {
    hinstance: HINSTANCE,
    autostart: bool,
    controls: Cell<Option<Controls>>,
    drives: RefCell<Vec<String>>,
    tree: RefCell<Option<Arc<Tree>>>,
    scan: RefCell<Option<Scan>>,
}

pub fn run(autostart: bool) {
    // SAFETY: a null name asks for this exe's own module handle. Nothing is retained.
    let hinstance = unsafe { GetModuleHandleW(null()) };
    set_dpi_awareness();
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TREEVIEW_CLASSES,
    };
    // SAFETY: `icc` is live for the call and its dwSize matches its type.
    if unsafe { InitCommonControlsEx(&icc) } == 0 {
        fail("Could not load the common controls.");
        return;
    }
    if !treemap_view::register(hinstance) {
        fail("Could not register the treemap window class.");
        return;
    }
    if !register_main_class(hinstance) {
        fail("Could not register the main window class.");
        return;
    }
    // SAFETY: plain query, no pointers.
    let dpi = unsafe { GetDpiForSystem() };
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale(1150, dpi),
        bottom: scale(720, dpi),
    };
    // SAFETY: `frame` is a live RECT that the call reads and rewrites.
    unsafe { AdjustWindowRectExForDpi(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0, dpi) };

    // Owned here so it outlives the window: the loop below ends only after the window is gone.
    let state = Box::new(AppState {
        hinstance,
        autostart,
        controls: Cell::new(None),
        drives: RefCell::new(Vec::new()),
        tree: RefCell::new(None),
        scan: RefCell::new(None),
    });
    let class = wide(CLASS_NAME);
    let title = wide(TITLE);
    // SAFETY: the class is registered and the strings are live for the call. The state pointer
    // is stored in GWLP_USERDATA and stays valid as described above.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            frame.bottom - frame.top,
            null_mut(),
            null_mut(),
            hinstance,
            (&*state as *const AppState).cast(),
        )
    };
    if hwnd.is_null() {
        fail("Could not create the main window.");
        return;
    }
    // SAFETY: `hwnd` is the window just created.
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    run_message_loop();
}

fn fail(text: &str) {
    message(null_mut(), TITLE, text);
}

fn set_dpi_awareness() {
    // SAFETY: takes a constant context value, no pointers.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        // SAFETY: takes no arguments.
        unsafe { SetProcessDPIAware() };
    }
}

fn register_main_class(hinstance: HINSTANCE) -> bool {
    let class = wide(CLASS_NAME);
    // SAFETY: loads a shared system cursor. Nothing is retained.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_ARROW) };
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: cursor,
        // The classic "(HBRUSH)(COLOR_BTNFACE + 1)" form of a system colour brush.
        hbrBackground: (COLOR_BTNFACE + 1) as isize as HBRUSH,
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
        hIconSm: null_mut(),
    };
    // SAFETY: `wc` and the class name are live for the call; Win32 copies both.
    unsafe { RegisterClassExW(&wc) != 0 }
}

fn run_message_loop() {
    // SAFETY: MSG is plain data. GetMessageW fills it before it is read.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `msg` is live. A null filter takes every message for this thread.
        let got = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        // 0 is WM_QUIT. A negative value is a failed call, which cannot happen with these arguments.
        if got <= 0 {
            break;
        }
        // SAFETY: `msg` was just filled by GetMessageW.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The AppState of a window, or None before WM_NCCREATE and after WM_NCDESTROY.
fn state<'a>(hwnd: HWND) -> Option<&'a AppState> {
    // SAFETY: GetWindowLongPtrW only reads the window's own slot.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const AppState;
    // SAFETY: a non-null value is the live AppState that run() owns (see the comment in run).
    unsafe { ptr.as_ref() }
}

fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: passes the message unchanged to the system default handler.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            // SAFETY: for WM_NCCREATE, lParam points at a CREATESTRUCTW that is valid for this message.
            let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
            // SAFETY: stores the AppState address that run() passed as the creation parameter.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
        }
        WM_NCDESTROY => {
            // SAFETY: clears the stored address. run() drops the AppState after the loop.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        }
        _ => {}
    }
    let Some(st) = state(hwnd) else {
        return default_proc(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_CREATE => on_create(hwnd, st),
        WM_SIZE => {
            if wparam != SIZE_MINIMIZED as usize {
                layout(
                    hwnd,
                    st,
                    (lparam & 0xffff) as i32,
                    ((lparam >> 16) & 0xffff) as i32,
                );
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID {
                on_timer(st);
            }
            0
        }
        WM_COMMAND => on_command(hwnd, st, wparam, lparam),
        WM_NOTIFY => on_notify(st, lparam),
        WM_APP_SCAN_DONE => {
            finish_scan(hwnd, st);
            0
        }
        WM_APP_AUTOSTART => {
            start_scan(hwnd, st);
            0
        }
        WM_APP_REVEAL => {
            on_reveal(st, wparam);
            0
        }
        WM_DESTROY => {
            // SAFETY: plain call, no arguments.
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => default_proc(hwnd, msg, wparam, lparam),
    }
}

fn child(parent: HWND, hinstance: HINSTANCE, class: &str, text: &str, style: u32) -> HWND {
    let (class, text) = (wide(class), wide(text));
    // SAFETY: `class` and `text` are NUL-terminated and live for the call. No creation data is passed.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
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

fn on_create(hwnd: HWND, st: &AppState) -> LRESULT {
    let hi = st.hinstance;
    let button = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
    let drives = disk::list_drives();
    let label = child(hwnd, hi, "STATIC", "Drive:", WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE);
    // The combo box is taller than its closed box (its drop list). WS_CLIPSIBLINGS keeps that
    // blank part from painting over the status line and the tree, which sit above it.
    let combo = child(
        hwnd,
        hi,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32,
    );
    let scan = child(hwnd, hi, "BUTTON", "Scan", button | BS_PUSHBUTTON as u32);
    let cancel = child(hwnd, hi, "BUTTON", "Cancel", button | BS_PUSHBUTTON as u32);
    let status = child(hwnd, hi, "STATIC", "", WS_CHILD | WS_VISIBLE | SS_NOPREFIX);
    let tree = tree_view::create(hwnd, hi);
    let treemap = treemap_view::create(hwnd, hi);
    let handles = [label, combo, scan, cancel, status, tree, treemap];
    if handles.iter().any(|h| h.is_null()) {
        return -1;
    }

    // SAFETY: DEFAULT_GUI_FONT is a stock object that Win32 owns and never deletes.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    for h in [label, combo, scan, cancel, status, tree] {
        send(h, WM_SETFONT, font as usize, 1);
    }

    for d in &drives {
        let text = wide(&format!(
            "{}   {} free of {}",
            d.root,
            human_size(d.free),
            human_size(d.total)
        ));
        send(combo, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }
    let roots: Vec<String> = drives.into_iter().map(|d| d.root).collect();
    let default = roots.iter().position(|r| r.eq_ignore_ascii_case(r"C:\")).unwrap_or(0);
    send(combo, CB_SETCURSEL, default, 0);

    set_status(status, if roots.is_empty() { "No drives found." } else { START_TEXT });
    win::enable(scan, !roots.is_empty());
    win::enable(cancel, false);

    st.controls.set(Some(Controls {
        label,
        combo,
        scan,
        cancel,
        status,
        tree,
        treemap,
    }));
    *st.drives.borrow_mut() = roots;
    if st.autostart {
        post(hwnd, WM_APP_AUTOSTART, 0, 0);
    }
    0
}

fn layout(hwnd: HWND, st: &AppState, width: i32, height: i32) {
    let Some(c) = st.controls.get() else { return };
    let dpi = dpi_of(hwnd);
    let margin = scale(10, dpi);
    let gap = scale(8, dpi);
    let row_h = scale(26, dpi);
    let status_top = margin + row_h + scale(6, dpi);
    let status_h = scale(36, dpi);
    let label_w = scale(48, dpi);
    let combo_w = scale(360, dpi);
    // The drop list's height. The closed box is sized by the font, and the rest overlaps the
    // status line and tree, which are created after it and so sit on top.
    let combo_h = scale(200, dpi);
    let btn_w = scale(80, dpi);

    let inner_w = (width - 2 * margin).max(0);
    let combo_x = margin + label_w + gap;
    let scan_x = combo_x + combo_w + gap;
    let cancel_x = scan_x + btn_w + gap;
    place(c.label, margin, margin, label_w, row_h);
    place(c.combo, combo_x, margin, combo_w, combo_h);
    place(c.scan, scan_x, margin, btn_w, row_h);
    place(c.cancel, cancel_x, margin, btn_w, row_h);
    place(c.status, margin, status_top, inner_w, status_h);

    let panes_top = status_top + status_h + gap;
    let panes_h = (height - margin - panes_top).max(0);
    let left_w = (inner_w - gap).max(0) / 3;
    let right_x = margin + left_w + gap;
    let right_w = (inner_w - gap - left_w).max(0);
    place(c.tree, margin, panes_top, left_w, panes_h);
    place(c.treemap, right_x, panes_top, right_w, panes_h);
}

fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    // SAFETY: `hwnd` is a child of the main window. MoveWindow takes plain integers.
    unsafe { MoveWindow(hwnd, x, y, w, h, 1) };
}

/// The window's DPI. Falls back to 96 (unscaled) when Win32 reports 0.
fn dpi_of(hwnd: HWND) -> u32 {
    // SAFETY: takes only the window handle.
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    }
}

fn on_command(hwnd: HWND, st: &AppState, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(c) = st.controls.get() else { return 0 };
    let notify = ((wparam >> 16) & 0xffff) as u32;
    let source = lparam as HWND;
    if notify == BN_CLICKED {
        if source == c.scan {
            start_scan(hwnd, st);
        } else if source == c.cancel {
            cancel_scan(st);
        }
    }
    0
}

fn on_notify(st: &AppState, lparam: LPARAM) -> LRESULT {
    let Some(c) = st.controls.get() else { return 0 };
    // SAFETY: for WM_NOTIFY, lParam points at a notification struct that starts with NMHDR.
    let header = unsafe { &*(lparam as *const NMHDR) };
    if header.hwndFrom != c.tree {
        return 0;
    }
    match header.code {
        TVN_ITEMEXPANDINGW => {
            // SAFETY: the TVN_ITEMEXPANDING code means lParam is an NMTREEVIEWW.
            let info = unsafe { &*(lparam as *const NMTREEVIEWW) };
            if let Some(tree) = current_tree(st) {
                tree_view::on_expanding(c.tree, &tree, info.itemNew.hItem);
            }
            // Returning 0 lets the expand proceed.
            0
        }
        TVN_SELCHANGEDW => {
            // SAFETY: the TVN_SELCHANGED code means lParam is an NMTREEVIEWW.
            let info = unsafe { &*(lparam as *const NMTREEVIEWW) };
            on_selection(c, st, info.itemNew.hItem);
            0
        }
        _ => 0,
    }
}

/// Focus follows the selection: a folder shows its own children, a file shows its parent.
fn on_selection(c: Controls, st: &AppState, item: HTREEITEM) {
    let Some(id) = tree_view::item_id(c.tree, item) else { return };
    let Some(tree) = current_tree(st) else { return };
    let focus = if tree.is_dir(id) { Some(id) } else { tree.parent(id) };
    treemap_view::set_view(c.treemap, Some(tree), focus);
}

fn on_reveal(st: &AppState, id: WPARAM) {
    let Some(c) = st.controls.get() else { return };
    let Some(tree) = current_tree(st) else { return };
    tree_view::reveal(c.tree, &tree, id as u32);
}

fn on_timer(st: &AppState) {
    let Some(c) = st.controls.get() else { return };
    let Some((root, progress)) = scan_handles(st) else { return };
    // "Cancelling..." stays until the scan reports back.
    if progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    set_status(c.status, &scanning_text(&root, &progress));
}

fn scanning_text(root: &str, progress: &Progress) -> String {
    let current = progress.current.lock().unwrap_or_else(|e| e.into_inner()).clone();
    format!(
        "Scanning {root}   {} files, {} folders, {} skipped   {}",
        grouped(progress.files.load(Ordering::Relaxed)),
        grouped(progress.folders.load(Ordering::Relaxed)),
        grouped(progress.skipped.load(Ordering::Relaxed)),
        tail(&current, TAIL_CHARS)
    )
}

/// The last `keep` characters of `text`, with "..." in front when something was cut.
fn tail(text: &str, keep: usize) -> String {
    let count = text.chars().count();
    if count <= keep {
        return text.to_owned();
    }
    let rest: String = text.chars().skip(count - keep).collect();
    format!("...{rest}")
}

/// Sets the status line only when the text changes, so the 150 ms timer does not repaint it.
fn set_status(status: HWND, text: &str) {
    if win::text_of(status) != text {
        win::set_text(status, text);
    }
}

fn current_tree(st: &AppState) -> Option<Arc<Tree>> {
    st.tree.borrow().clone()
}

fn scan_handles(st: &AppState) -> Option<(String, Arc<Progress>)> {
    st.scan
        .borrow()
        .as_ref()
        .map(|running| (running.root.clone(), Arc::clone(&running.progress)))
}

fn start_scan(hwnd: HWND, st: &AppState) {
    let Some(c) = st.controls.get() else { return };
    if st.scan.borrow().is_some() {
        return;
    }
    let index = send(c.combo, CB_GETCURSEL, 0, 0);
    let root = usize::try_from(index)
        .ok()
        .and_then(|i| st.drives.borrow().get(i).cloned());
    let Some(root) = root else { return };

    // Empty the views first, so no item or treemap block still refers to the old tree.
    tree_view::clear(c.tree);
    treemap_view::set_view(c.treemap, None, None);
    drop(st.tree.take());

    let progress = Arc::new(Progress::new());
    let result: Slot = Arc::new(Mutex::new(None));
    *st.scan.borrow_mut() = Some(Scan {
        root: root.clone(),
        progress: Arc::clone(&progress),
        result: Arc::clone(&result),
        started: Instant::now(),
    });
    win::enable(c.combo, false);
    win::enable(c.scan, false);
    win::enable(c.cancel, true);

    // The HWND crosses to the worker as a plain integer; the worker turns it back for the post.
    let hwnd_bits = hwnd as isize;
    let _ = thread::spawn(move || {
        let outcome = scan::scan_drive(&root, &progress);
        *result.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        post(hwnd_bits as HWND, WM_APP_SCAN_DONE, 0, 0);
    });
    // SAFETY: `hwnd` is our live window. The timer id is ours, and it is killed when the scan ends.
    unsafe { SetTimer(hwnd, TIMER_ID, TIMER_MS, None) };
    on_timer(st);
}

fn cancel_scan(st: &AppState) {
    let Some(c) = st.controls.get() else { return };
    let Some((_, progress)) = scan_handles(st) else { return };
    progress.request_cancel();
    set_status(c.status, "Cancelling...");
}

fn finish_scan(hwnd: HWND, st: &AppState) {
    let Some(c) = st.controls.get() else { return };
    let Some(running) = st.scan.take() else { return };
    // SAFETY: kills the timer that start_scan set. Harmless if it already stopped.
    unsafe { KillTimer(hwnd, TIMER_ID) };
    win::enable(c.combo, true);
    win::enable(c.scan, true);
    win::enable(c.cancel, false);

    let outcome = running.result.lock().unwrap_or_else(|e| e.into_inner()).take();
    match outcome {
        Some(Ok(tree)) => show_tree(c, st, &running, tree),
        Some(Err(ScanError::Cancelled)) => set_status(c.status, "Scan cancelled."),
        Some(Err(ScanError::Failed(text))) => {
            set_status(c.status, "Scan failed.");
            message(hwnd, TITLE, &text);
        }
        None => {
            set_status(c.status, "Scan failed.");
            message(hwnd, TITLE, "The scan ended without a result.");
        }
    }
}

/// The new tree is stored before the views fill, because filling them sends selection
/// notifications that read it.
fn show_tree(c: Controls, st: &AppState, running: &Scan, tree: Tree) {
    let tree = Arc::new(tree);
    let root = tree.root();
    *st.tree.borrow_mut() = Some(Arc::clone(&tree));
    tree_view::show_root(c.tree, &tree);
    treemap_view::set_view(c.treemap, Some(Arc::clone(&tree)), Some(root));
    let secs = running.started.elapsed().as_secs_f64();
    set_status(c.status, &done_text(&tree, &running.root, secs));
}

fn done_text(tree: &Tree, root: &str, secs: f64) -> String {
    let used = match disk::free_space(root) {
        Some(drive) => format!(" Windows reports {} used on {}.", human_size(drive.used()), root),
        None => String::new(),
    };
    format!(
        "Scanned {} on disk in {} files and {} folders ({secs:.0} s).{used} {} items could not be read and were skipped.",
        human_size(tree.size(tree.root())),
        grouped(tree.files),
        grouped(tree.folders),
        grouped(tree.skipped)
    )
}
