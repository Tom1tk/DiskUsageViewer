//! The treemap pane: the focused folder as blocks, a header line, and a hover line. Hovering a
//! block shows its details; clicking one returns its id so the caller can reveal it in the tree.
//! Drawing and hit-testing only. The layout is in `blocks.rs`.
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::blocks::{Block, folder_blocks};
use crate::browser::Browser;
use crate::format::{grouped, human_size};
use crate::tree::Tree;

const HINT: &str = "Hover over a block for details. Click a block to open it in the tree.";
const EMPTY_TEXT: &str = "Empty folder";
const EMPTY_TEXT_SIZE: f32 = 14.0;
const EMPTY_COLOUR: Color32 = Color32::from_rgb(0x66, 0x66, 0x66);
/// Blocks narrower or shorter than this get no label.
const MIN_LABEL_WIDTH: f32 = 60.0;
const MIN_LABEL_HEIGHT: f32 = 18.0;
/// Blocks shorter than this get only the name, not the size on a second line.
const MIN_SIZE_LINE_HEIGHT: f32 = 36.0;
const LABEL_PAD: Vec2 = Vec2::new(5.0, 4.0);
const LABEL_LINE: f32 = 15.0;
const LABEL_FONT_SIZE: f32 = 12.0;
const TEXT_COLOUR: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);

/// The blocks of the last layout. Rebuilt only when the folder, the tree or the pane size changes.
#[derive(Default)]
pub struct BlockCache {
    key: Option<Key>,
    blocks: Vec<Block>,
}

/// Holds the tree by `Arc`, so a cached layout can never match a different tree at a reused address.
struct Key {
    tree: Arc<Tree>,
    focus: u32,
    size: Vec2,
}

impl BlockCache {
    fn blocks(&mut self, tree: &Arc<Tree>, focus: u32, size: Vec2) -> &[Block] {
        let fresh = self
            .key
            .as_ref()
            .is_some_and(|k| Arc::ptr_eq(&k.tree, tree) && k.focus == focus && k.size == size);
        if !fresh {
            self.blocks = folder_blocks(tree, focus, (f64::from(size.x), f64::from(size.y)));
            self.key = Some(Key { tree: Arc::clone(tree), focus, size });
        }
        &self.blocks
    }

    fn clear(&mut self) {
        self.key = None;
        self.blocks.clear();
    }
}

/// Draws the treemap for the browser's focused folder, or nothing before a scan. Returns the id of
/// the block the user clicked, which the caller reveals in the tree.
pub fn show(ui: &mut Ui, browser: Option<&Browser>, cache: &mut BlockCache) -> Option<u32> {
    let Some(browser) = browser else {
        cache.clear();
        return None;
    };
    let focus = browser.focus();
    ui.add(egui::Label::new(header_text(browser.tree(), focus)).truncate());
    let line = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
    let area_size = Vec2::new(ui.available_width(), (ui.available_height() - line).max(0.0));
    let (area, response) = ui.allocate_exact_size(area_size, Sense::click());
    let blocks = cache.blocks(browser.tree_handle(), focus, area_size);
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|pos| area.contains(*pos))
        .and_then(|pos| blocks.iter().position(|block| block_rect(area, block).contains(pos)));

    let painter = ui.painter_at(area);
    if blocks.is_empty() {
        let font = FontId::proportional(EMPTY_TEXT_SIZE);
        painter.text(area.center(), Align2::CENTER_CENTER, EMPTY_TEXT, font, EMPTY_COLOUR);
    }
    for block in blocks {
        draw_block(&painter, area, block);
    }

    let info = match hovered.and_then(|index| blocks.get(index)) {
        Some(block) => block.hover_text.as_str(),
        None => HINT,
    };
    ui.add(egui::Label::new(info).truncate());

    if response.clicked() {
        return hovered.and_then(|index| blocks[index].id);
    }
    None
}

fn header_text(tree: &Tree, focus: u32) -> String {
    format!(
        "{}    {}    {} items",
        tree.path(focus),
        human_size(tree.size(focus)),
        grouped(tree.child_count(focus) as u64)
    )
}

fn block_rect(area: Rect, block: &Block) -> Rect {
    let min = area.min + Vec2::new(block.rect.x as f32, block.rect.y as f32);
    Rect::from_min_size(min, Vec2::new(block.rect.w as f32, block.rect.h as f32))
}

fn draw_block(painter: &egui::Painter, area: Rect, block: &Block) {
    let rect = block_rect(area, block);
    painter.rect_filled(rect, CornerRadius::ZERO, colour(block.colour));
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    draw_label(painter, rect, block);
}

fn draw_label(painter: &egui::Painter, rect: Rect, block: &Block) {
    if rect.width() < MIN_LABEL_WIDTH || rect.height() < MIN_LABEL_HEIGHT {
        return;
    }
    let clipped = painter.with_clip_rect(rect);
    let origin = rect.min + LABEL_PAD;
    let font = FontId::proportional(LABEL_FONT_SIZE);
    clipped.text(origin, Align2::LEFT_TOP, &block.name, font.clone(), TEXT_COLOUR);
    if rect.height() < MIN_SIZE_LINE_HEIGHT {
        return;
    }
    let size_at = origin + Vec2::new(0.0, LABEL_LINE);
    clipped.text(size_at, Align2::LEFT_TOP, human_size(block.size), font, TEXT_COLOUR);
}

fn colour(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
