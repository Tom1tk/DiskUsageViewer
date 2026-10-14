//! The folder tree on the left: the browser's visible rows in a scroll area. A folder with children
//! has a triangle that opens or closes it. Clicking a row selects it, and the treemap follows.
use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, TextStyle, Ui, Vec2};

use crate::browser::{Browser, Row};
use crate::format::{human_size, percent};
use crate::tree::Tree;

const ROW_PAD_Y: f32 = 3.0;
/// Horizontal space per depth level.
const INDENT: f32 = 16.0;
/// The slot left of each name that holds the open/close triangle.
const TOGGLE_WIDTH: f32 = 14.0;
const TRIANGLE_HALF: f32 = 4.0;
const TEXT_GAP: f32 = 2.0;

/// What a click on a row does. Applied after the rows are drawn, since drawing only reads the browser.
enum Action {
    Toggle(u32),
    Select(u32),
}

/// Draws the tree. A reveal sets the scroll offset for one frame, so the selected row is centred.
pub fn show(ui: &mut Ui, browser: &mut Browser) {
    browser.refresh();
    let row_height = ui.text_style_height(&TextStyle::Body) + ROW_PAD_Y * 2.0;
    let step = row_height + ui.spacing().item_spacing.y;
    let rows: Vec<Row> = browser.rows().to_vec();
    let selected = browser.selection();
    let scroll_row = if browser.take_scroll_request() { browser.selected_row() } else { None };

    let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(index) = scroll_row {
        let offset = index as f32 * step - ui.available_height() / 2.0 + row_height / 2.0;
        scroll = scroll.vertical_scroll_offset(offset.max(0.0));
    }
    let picked = scroll
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            let mut picked = None;
            for &row in &rows[range] {
                let expanded = browser.is_expanded(row.id);
                let is_selected = selected == Some(row.id);
                if let Some(found) = draw_row(ui, browser.tree(), row, is_selected, expanded, row_height) {
                    picked = Some(found);
                }
            }
            picked
        })
        .inner;
    match picked {
        Some(Action::Toggle(id)) => browser.toggle(id),
        Some(Action::Select(id)) => browser.select(id),
        None => {}
    }
}

/// Draws one row and returns the click on it, if any.
fn draw_row(
    ui: &mut Ui,
    tree: &Tree,
    row: Row,
    selected: bool,
    expanded: bool,
    row_height: f32,
) -> Option<Action> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::click());
    let visuals = ui.visuals();
    let (selection_fill, selection_text, hover_fill, plain_text) = (
        visuals.selection.bg_fill,
        visuals.selection.stroke.color,
        visuals.widgets.hovered.weak_bg_fill,
        visuals.text_color(),
    );
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, CornerRadius::ZERO, selection_fill);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, hover_fill);
    }
    let text_colour = if selected { selection_text } else { plain_text };

    let x = rect.left() + row.depth as f32 * INDENT;
    let toggle = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(TOGGLE_WIDTH, row_height));
    let can_open = tree.is_dir(row.id) && tree.child_count(row.id) > 0;
    if can_open {
        draw_triangle(&painter, toggle.center(), expanded, text_colour);
    }
    let font = TextStyle::Body.resolve(ui.style());
    let label_at = Pos2::new(x + TOGGLE_WIDTH + TEXT_GAP, rect.center().y);
    painter.text(label_at, Align2::LEFT_CENTER, label(tree, row.id), font, text_colour);

    if !response.clicked() {
        return None;
    }
    let on_toggle = response.interact_pointer_pos().is_some_and(|pos| toggle.contains(pos));
    Some(if can_open && on_toggle { Action::Toggle(row.id) } else { Action::Select(row.id) })
}

/// A right-pointing triangle when closed, down-pointing when open.
fn draw_triangle(painter: &egui::Painter, centre: Pos2, expanded: bool, colour: Color32) {
    let h = TRIANGLE_HALF;
    let points = if expanded {
        vec![centre + Vec2::new(-h, -h / 2.0), centre + Vec2::new(h, -h / 2.0), centre + Vec2::new(0.0, h / 2.0)]
    } else {
        vec![centre + Vec2::new(-h / 2.0, -h), centre + Vec2::new(-h / 2.0, h), centre + Vec2::new(h / 2.0, 0.0)]
    };
    painter.add(Shape::convex_polygon(points, colour, Stroke::NONE));
}

/// The row text: name, size, and share of the parent folder. The root has no parent to compare with.
fn label(tree: &Tree, id: u32) -> String {
    let size = tree.size(id);
    match tree.parent(id) {
        Some(parent) => format!(
            "{}    {}    {}",
            tree.name(id),
            human_size(size),
            percent(size, tree.size(parent))
        ),
        None => format!("{}    {}", tree.name(id), human_size(size)),
    }
}
