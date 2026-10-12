//! Treemap pane: one window class that draws the header line, the blocks of one folder, the
//! hover text and the info line. It holds no tree of its own; the parent sets the model.

use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DEFAULT_PITCH, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_TOP, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE,
    FW_BOLD, FillRect, FrameRect, GetStockObject, GetTextMetricsW, HDC, HFONT, HGDIOBJ,
    InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode,
    SetTextColor, TEXTMETRICW, TRANSPARENT, WHITE_BRUSH,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetClientRect, GetParent, GetWindowLongPtrW,
    IDC_ARROW, LoadCursorW, PostMessageW, RegisterClassExW, SetWindowLongPtrW, WM_APP, WM_CREATE,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SIZE,
    WNDCLASSEXW, WS_CHILD, WS_VISIBLE,
};

use crate::format::{grouped, human_size, percent};
use crate::tree::Tree;
use crate::win::wide;
use crate::treemap::{self, Rect};

pub const WM_APP_REVEAL: u32 = WM_APP + 2;

const CLASS_NAME: &str = "DiskUsageTreemap";
const FONT_FACE: &str = "Segoe UI";
const HEADER_PT: i32 = 10;
const BASE_DPI: i32 = 96;
const MARGIN: i32 = 6;
const BAND: i32 = 24;
const LABEL_PAD_X: i32 = 5;
const LABEL_PAD_TOP: i32 = 4;
const LABEL_PAD_BOTTOM: i32 = 2;
const MIN_LABEL_W: i32 = 60;
const MIN_LABEL_H: i32 = 18;
const MIN_SIZE_LINE_H: i32 = 36;
const MAX_BLOCKS: usize = 150;
const HINT: &str = "Hover over a block for details. Click a block to open it in the tree.";
const EMPTY_TEXT: &str = "Empty folder";
const TEXT_COLOUR: COLORREF = rgb(0x1a1a1a);
const EMPTY_COLOUR: COLORREF = rgb(0x666666);
const FILE_COLOUR: COLORREF = rgb(0xdfe3e8);
const MORE_COLOUR: COLORREF = rgb(0xeeeeee);
const PALETTE: [COLORREF; 10] = [
    rgb(0xa8dadc),
    rgb(0xf4a261),
    rgb(0xe9c46a),
    rgb(0x90be6d),
    rgb(0xf9c74f),
    rgb(0xf8961e),
    rgb(0xb5e48c),
    rgb(0xc3aed6),
    rgb(0xffb4a2),
    rgb(0x9ad1d4),
];
const LINE_FLAGS: DRAW_TEXT_FORMAT =
    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
const LABEL_FLAGS: DRAW_TEXT_FORMAT =
    DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
const CENTRE_FLAGS: DRAW_TEXT_FORMAT = DT_CENTER | DT_VCENTER | DT_SINGLELINE;

/// #RRGGBB as a COLORREF, which GDI stores as 0x00BBGGRR.
const fn rgb(hex: u32) -> COLORREF {
    ((hex & 0xff) << 16) | (hex & 0xff00) | ((hex >> 16) & 0xff)
}

/// A child of the focused folder that has a size.
struct Kid {
    id: u32,
    size: u64,
    is_dir: bool,
}

struct Block {
    rect: RECT,
    /// None for the "more items" block, which cannot be opened.
    id: Option<u32>,
    size: u64,
    name: String,
    colour: COLORREF,
    hover_text: String,
}

struct Bands {
    header: RECT,
    area: RECT,
    info: RECT,
}

struct State {
    model: Option<Arc<Tree>>,
    focus: Option<u32>,
    blocks: Vec<Block>,
    hover: Option<usize>,
    tracking: bool,
    header_font: HFONT,
    dpi: u32,
}

impl State {
    fn new() -> Self {
        Self {
            model: None,
            focus: None,
            blocks: Vec::new(),
            hover: None,
            tracking: false,
            header_font: null_mut(),
            dpi: 0,
        }
    }
}

pub fn register(hinstance: HINSTANCE) -> bool {
    let class = wide(CLASS_NAME);
    // SAFETY: IDC_ARROW is a system cursor; a null instance is the documented way to load it.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_ARROW) };
    let class_def = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        hCursor: cursor,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // SAFETY: class_def and the class name it points to are alive for the call.
    unsafe { RegisterClassExW(&class_def) != 0 }
}

pub fn create(parent: HWND, hinstance: HINSTANCE) -> HWND {
    let class = wide(CLASS_NAME);
    // SAFETY: the class name is alive for the call; the parent is the window that owns this pane.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
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

pub fn set_view(hwnd: HWND, tree: Option<Arc<Tree>>, focus: Option<u32>) {
    let Some(state) = state_mut(hwnd) else { return };
    state.model = tree;
    state.focus = focus;
    relayout(hwnd, state);
    invalidate_all(hwnd);
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            on_nccreate(hwnd);
            def_proc(hwnd, msg, wparam, lparam)
        }
        WM_NCDESTROY => {
            on_ncdestroy(hwnd);
            0
        }
        WM_CREATE => on_create(hwnd),
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            on_size(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, lparam);
            0
        }
        WM_MOUSELEAVE => {
            on_mouse_leave(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            on_click(hwnd, lparam);
            0
        }
        _ => def_proc(hwnd, msg, wparam, lparam),
    }
}

fn def_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: forwards an unhandled message unchanged to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn on_nccreate(hwnd: HWND) {
    let raw = Box::into_raw(Box::new(State::new()));
    // SAFETY: hwnd is the window being created; the Box is reclaimed in WM_NCDESTROY.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize) };
}

fn on_create(hwnd: HWND) -> LRESULT {
    let ok = state_mut(hwnd).is_some_and(|state| apply_dpi(hwnd, state));
    if ok { 0 } else { -1 }
}

fn on_ncdestroy(hwnd: HWND) {
    // SAFETY: clears the pointer first, so no later message can reach the State freed below.
    let raw = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut State;
    if raw.is_null() {
        return;
    }
    // SAFETY: raw came from Box::into_raw in on_nccreate and was just detached, so it is freed once.
    let state = unsafe { Box::from_raw(raw) };
    // SAFETY: no DC holds the font between paints, so it can be deleted (null is harmless).
    unsafe { DeleteObject(state.header_font) };
}

fn on_paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: ps is a valid out-pointer for the paint of this window.
    let dc = unsafe { BeginPaint(hwnd, &mut ps) };
    if dc.is_null() {
        return;
    }
    if let Some(state) = state_mut(hwnd) {
        paint_buffered(dc, client_rect(hwnd), state);
    }
    // SAFETY: pairs with the BeginPaint call above.
    unsafe { EndPaint(hwnd, &ps) };
}

fn on_size(hwnd: HWND) {
    let Some(state) = state_mut(hwnd) else { return };
    // A failed font rebuild keeps the old font, which is still valid at the old DPI.
    apply_dpi(hwnd, state);
    relayout(hwnd, state);
    invalidate_all(hwnd);
}

fn on_mouse_move(hwnd: HWND, lparam: LPARAM) {
    let Some(state) = state_mut(hwnd) else { return };
    if !state.tracking {
        state.tracking = request_leave(hwnd);
    }
    let (x, y) = point_of(lparam);
    let hover = state
        .blocks
        .iter()
        .position(|block| contains(&block.rect, x, y));
    if hover != state.hover {
        state.hover = hover;
        invalidate_info(hwnd, state.dpi);
    }
}

fn on_mouse_leave(hwnd: HWND) {
    let Some(state) = state_mut(hwnd) else { return };
    state.tracking = false;
    if state.hover.take().is_some() {
        invalidate_info(hwnd, state.dpi);
    }
}

fn on_click(hwnd: HWND, lparam: LPARAM) {
    let Some(state) = state_mut(hwnd) else { return };
    let (x, y) = point_of(lparam);
    let Some(id) = state
        .blocks
        .iter()
        .find(|block| contains(&block.rect, x, y))
        .and_then(|block| block.id)
    else {
        return;
    };
    // SAFETY: the parent is the window that owns this pane; the message is posted, not sent.
    unsafe {
        let parent = GetParent(hwnd);
        PostMessageW(parent, WM_APP_REVEAL, id as usize, 0);
    }
}

/// The State behind a window, or None before WM_NCCREATE and after WM_NCDESTROY.
fn state_mut<'a>(hwnd: HWND) -> Option<&'a mut State> {
    // SAFETY: GWLP_USERDATA holds the Box from on_nccreate, or 0 outside that window's lifetime.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    // SAFETY: raw is null or the live Box; messages for one window run on one thread.
    unsafe { raw.as_mut() }
}

fn client_rect(hwnd: HWND) -> RECT {
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: rc is a valid out-pointer; on failure it stays empty and no blocks are laid out.
    unsafe { GetClientRect(hwnd, &mut rc) };
    rc
}

fn invalidate_all(hwnd: HWND) {
    // SAFETY: a null rect invalidates the whole client area of this window.
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

fn invalidate_info(hwnd: HWND, dpi: u32) {
    let info = bands(client_rect(hwnd), dpi).info;
    // SAFETY: info is a valid rect in this window's client coordinates.
    unsafe { InvalidateRect(hwnd, &info, 0) };
}

fn request_leave(hwnd: HWND) -> bool {
    let mut request = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: request is a fully initialised TRACKMOUSEEVENT.
    unsafe { TrackMouseEvent(&mut request) != 0 }
}

/// Client coordinates from a mouse message. Decoded as signed, so a point left of or above the
/// client area (during capture) still reads correctly.
fn point_of(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam as usize;
    (
        (raw & 0xffff) as i16 as i32,
        ((raw >> 16) & 0xffff) as i16 as i32,
    )
}

fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Rebuilds the header font when the window DPI changes. False if the font could not be created.
fn apply_dpi(hwnd: HWND, state: &mut State) -> bool {
    // SAFETY: hwnd is a live window of this class.
    let reported = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if reported == 0 {
        BASE_DPI as u32
    } else {
        reported
    };
    if dpi == state.dpi {
        return true;
    }
    let header = create_font(dpi, HEADER_PT, FW_BOLD as i32);
    if header.is_null() {
        return false;
    }
    // SAFETY: no DC holds the old font between paints, so it can be deleted (null is harmless).
    unsafe { DeleteObject(state.header_font) };
    state.header_font = header;
    state.dpi = dpi;
    true
}

fn create_font(dpi: u32, points: i32, weight: i32) -> HFONT {
    let face = wide(FONT_FACE);
    // Negative height means character height. 72 points per inch; +36 rounds to nearest.
    let height = -((points * dpi as i32 + 36) / 72);
    // SAFETY: face is a NUL-terminated name that is alive for the call.
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

/// Recomputes blocks for the current model, focus and client size. Resets hover, since the
/// block indices change.
fn relayout(hwnd: HWND, state: &mut State) {
    let area = bands(client_rect(hwnd), state.dpi).area;
    let bounds = (
        (area.right - area.left) as f64,
        (area.bottom - area.top) as f64,
    );
    let blocks = match (&state.model, state.focus) {
        (Some(tree), Some(focus)) => build_blocks(tree, focus, bounds)
            .into_iter()
            .map(|block| Block {
                rect: shifted(block.rect, area.left, area.top),
                ..block
            })
            .collect(),
        _ => Vec::new(),
    };
    state.blocks = blocks;
    state.hover = None;
}

/// Blocks for the children of `focus`, in a space whose origin is the top-left of `bounds`.
fn build_blocks(tree: &Tree, focus: u32, bounds: (f64, f64)) -> Vec<Block> {
    let (w, h) = bounds;
    if !(w > 0.0 && h > 0.0) {
        return Vec::new();
    }
    let kids: Vec<Kid> = tree
        .sorted_children(focus)
        .into_iter()
        .map(|id| Kid {
            id,
            size: tree.size(id),
            is_dir: tree.is_dir(id),
        })
        .filter(|kid| kid.size > 0)
        .collect();
    make_blocks(&kids, tree.size(focus), bounds, |id| tree.name(id))
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

/// `kids` are sorted largest first and have sizes > 0. The first 150 become blocks; the rest
/// become one "more items" block, placed by size so the layout still sees largest first.
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
        items.insert(
            at,
            Item::More {
                count: rest.len(),
                size: rest_size,
            },
        );
    }
    let sizes: Vec<f64> = items.iter().map(|item| item.size() as f64).collect();
    let (w, h) = bounds;
    let rects = treemap::layout(
        &sizes,
        Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        },
    );
    let mut blocks = Vec::with_capacity(items.len());
    let mut folders = 0usize;
    for (item, rect) in items.iter().zip(&rects) {
        let rect = to_rect(*rect);
        blocks.push(match item {
            Item::Kid(kid) => {
                let colour = if kid.is_dir {
                    PALETTE[folders % PALETTE.len()]
                } else {
                    FILE_COLOUR
                };
                folders += usize::from(kid.is_dir);
                kid_block(kid, name_of(kid.id), colour, rect, folder_size)
            }
            Item::More { count, size } => more_block(*count, *size, rect, folder_size),
        });
    }
    blocks
}

fn kid_block(kid: &Kid, name: String, colour: COLORREF, rect: RECT, folder_size: u64) -> Block {
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

fn more_block(count: usize, size: u64, rect: RECT, folder_size: u64) -> Block {
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

/// Rounds each edge rather than each width, so neighbouring blocks meet with no gap or overlap.
fn to_rect(r: Rect) -> RECT {
    RECT {
        left: r.x.round() as i32,
        top: r.y.round() as i32,
        right: (r.x + r.w).round() as i32,
        bottom: (r.y + r.h).round() as i32,
    }
}

fn shifted(r: RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: r.left + dx,
        top: r.top + dy,
        right: r.right + dx,
        bottom: r.bottom + dy,
    }
}

/// Pixel count at `dpi`, rounded to nearest.
fn scale(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32 + BASE_DPI / 2) / BASE_DPI
}

fn bands(client: RECT, dpi: u32) -> Bands {
    let margin = scale(MARGIN, dpi);
    let band = scale(BAND, dpi);
    let header_bottom = margin + band;
    let info_top = client.bottom - margin - band;
    Bands {
        header: RECT {
            left: margin,
            top: margin,
            right: client.right - margin,
            bottom: header_bottom,
        },
        area: RECT {
            left: margin,
            top: header_bottom + margin,
            right: client.right - margin,
            bottom: info_top - margin,
        },
        info: RECT {
            left: margin,
            top: info_top,
            right: client.right - margin,
            bottom: client.bottom - margin,
        },
    }
}

fn header_text(state: &State) -> String {
    match (&state.model, state.focus) {
        (Some(tree), Some(focus)) => format!(
            "{}    {}    {} items",
            tree.path(focus),
            human_size(tree.size(focus)),
            grouped(tree.child_count(focus) as u64)
        ),
        _ => String::new(),
    }
}

fn info_text(state: &State) -> &str {
    match state.hover.and_then(|i| state.blocks.get(i)) {
        Some(block) => block.hover_text.as_str(),
        None if state.model.is_some() => HINT,
        None => "",
    }
}

/// Draws into an off-screen bitmap and copies it once, so the window does not flicker.
fn paint_buffered(dc: HDC, client: RECT, state: &State) {
    let (w, h) = (client.right, client.bottom);
    if w <= 0 || h <= 0 {
        return;
    }
    // SAFETY: dc is a valid window DC; the memory DC is deleted below.
    let mem = unsafe { CreateCompatibleDC(dc) };
    if mem.is_null() {
        return;
    }
    // SAFETY: the bitmap is compatible with dc; it is deleted below after being deselected.
    let bitmap = unsafe { CreateCompatibleBitmap(dc, w, h) };
    if bitmap.is_null() {
        // SAFETY: mem is the DC created above, and nothing is selected into it.
        unsafe { DeleteDC(mem) };
        return;
    }
    // SAFETY: mem and bitmap are both valid; the previous bitmap is restored below.
    let old = unsafe { SelectObject(mem, bitmap) };
    draw_frame(mem, client, state);
    // SAFETY: copies the finished frame, then deselects the bitmap so it can be deleted.
    unsafe {
        BitBlt(dc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bitmap);
        DeleteDC(mem);
    }
}

fn draw_frame(dc: HDC, client: RECT, state: &State) {
    let pane = bands(client, state.dpi);
    // SAFETY: the stock white brush is never deleted; the background mode lasts for this DC only.
    unsafe {
        FillRect(dc, &client, GetStockObject(WHITE_BRUSH));
        SetBkMode(dc, TRANSPARENT as i32);
    }
    draw_line(dc, state.header_font, &header_text(state), &pane.header);
    draw_treemap(dc, pane.area, state);
    draw_line(dc, body_font(), info_text(state), &pane.info);
}

/// The system UI font for block labels and the info line. Stock, so it is never deleted.
fn body_font() -> HGDIOBJ {
    // SAFETY: GetStockObject takes only the stock object id.
    unsafe { GetStockObject(DEFAULT_GUI_FONT) }
}

/// One line of text in `font`, vertically centred in `rect`.
fn draw_line(dc: HDC, font: HGDIOBJ, text: &str, rect: &RECT) {
    // SAFETY: font is a live GDI object; the DC's previous font is restored below.
    let old = unsafe { SelectObject(dc, font) };
    draw_text(dc, text, rect, LINE_FLAGS, TEXT_COLOUR);
    // SAFETY: restores the font that was selected before this call.
    unsafe { SelectObject(dc, old) };
}

fn draw_treemap(dc: HDC, area: RECT, state: &State) {
    // SAFETY: the stock font is never deleted; the previous font is restored below.
    let old = unsafe { SelectObject(dc, body_font()) };
    let has_area = area.right > area.left && area.bottom > area.top;
    let shows_empty =
        has_area && state.blocks.is_empty() && state.model.is_some() && state.focus.is_some();
    if shows_empty {
        draw_text(dc, EMPTY_TEXT, &area, CENTRE_FLAGS, EMPTY_COLOUR);
    }
    for block in &state.blocks {
        draw_block(dc, block, state.dpi);
    }
    // SAFETY: restores the font that was selected before this call.
    unsafe { SelectObject(dc, old) };
}

fn draw_block(dc: HDC, block: &Block, dpi: u32) {
    let rect = block.rect;
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    // SAFETY: a solid brush for this block only; it is deleted below and never selected.
    let brush = unsafe { CreateSolidBrush(block.colour) };
    if brush.is_null() {
        return;
    }
    // SAFETY: rect is valid; the brush and the stock white brush are both live here.
    unsafe {
        FillRect(dc, &rect, brush);
        FrameRect(dc, &rect, GetStockObject(WHITE_BRUSH));
        DeleteObject(brush);
    }
    draw_label(dc, block, dpi);
}

fn draw_label(dc: HDC, block: &Block, dpi: u32) {
    let rect = block.rect;
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    if w < scale(MIN_LABEL_W, dpi) || h < scale(MIN_LABEL_H, dpi) {
        return;
    }
    let name_rect = RECT {
        left: rect.left + scale(LABEL_PAD_X, dpi),
        top: rect.top + scale(LABEL_PAD_TOP, dpi),
        right: rect.right - scale(LABEL_PAD_X, dpi),
        bottom: rect.bottom - scale(LABEL_PAD_BOTTOM, dpi),
    };
    draw_text(dc, &block.name, &name_rect, LABEL_FLAGS, TEXT_COLOUR);
    if h < scale(MIN_SIZE_LINE_H, dpi) {
        return;
    }
    let Some(line) = line_height(dc) else { return };
    let size_rect = RECT {
        top: name_rect.top + line,
        ..name_rect
    };
    draw_text(
        dc,
        &human_size(block.size),
        &size_rect,
        LABEL_FLAGS,
        TEXT_COLOUR,
    );
}

fn line_height(dc: HDC) -> Option<i32> {
    let mut metrics = TEXTMETRICW::default();
    // SAFETY: metrics is a valid out-pointer; dc has a font selected.
    let ok = unsafe { GetTextMetricsW(dc, &mut metrics) };
    (ok != 0).then_some(metrics.tmHeight)
}

fn draw_text(dc: HDC, text: &str, rect: &RECT, flags: DRAW_TEXT_FORMAT, colour: COLORREF) {
    // DrawTextW reads the pointer even when the count is 0, and an empty Vec's pointer is dangling.
    if text.is_empty() {
        return;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut bounds = *rect;
    // SAFETY: units and bounds are alive for the call, and the length matches units.
    unsafe {
        SetTextColor(dc, colour);
        DrawTextW(dc, units.as_ptr(), units.len() as i32, &mut bounds, flags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_with_400_children_gives_150_blocks_and_a_more_block() {
        let kids: Vec<Kid> = (0..400)
            .map(|id| Kid {
                id,
                size: 1000 - u64::from(id),
                is_dir: id % 3 == 0,
            })
            .collect();
        let total: u64 = kids.iter().map(|kid| kid.size).sum();
        let blocks = make_blocks(&kids, total, (800.0, 600.0), |id| format!("item {id}"));
        assert_eq!(blocks.len(), MAX_BLOCKS + 1);
        // The rest sum (about 180,000) is larger than any listed child (at most 1000).
        assert!(blocks[0].id.is_none());
        assert_eq!(blocks.iter().filter(|block| block.id.is_none()).count(), 1);
    }
}
