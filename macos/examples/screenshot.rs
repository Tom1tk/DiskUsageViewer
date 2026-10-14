//! Renders the real window to an image, so the layout can be checked without Screen Recording
//! permission: egui draws the frame and returns its pixels. Usage:
//!   cargo run --example screenshot -- <scan-root> <out.bmp>
//! The output is a BMP. Convert it to PNG with `sips -s format png <out.bmp> --out <out.png>`.
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use disk_usage_viewer_macos::app::App;
use disk_usage_viewer_macos::scan::{self, Progress};
use eframe::egui::{self, ViewportCommand};

/// Frames drawn before the screenshot is requested, so the layout has settled.
const FRAMES_BEFORE_SHOT: u32 = 20;
const WINDOW_SIZE: [f32; 2] = [1150.0, 720.0];
/// Pixels per row in a BMP header (72 dpi).
const PIXELS_PER_METRE: i32 = 2835;

struct Shot {
    app: App,
    frames: u32,
    requested: bool,
    out: String,
}

impl eframe::App for Shot {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.app, ui, frame);
        let ctx = ui.ctx().clone();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = image {
            write_bmp(Path::new(&self.out), &image).expect("write screenshot");
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.frames += 1;
        if self.frames >= FRAMES_BEFORE_SHOT && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let root = args.next().unwrap_or_else(|| "/usr".to_owned());
    let out = args.next().unwrap_or_else(|| "screenshot.bmp".to_owned());

    let start = Instant::now();
    let tree = scan::scan_volume(Path::new(&root), &Progress::default()).expect("scan the root");
    let secs = start.elapsed().as_secs_f64();
    let app = App::with_tree(tree, &root, secs);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Disk Usage Viewer")
            .with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        "Disk Usage Viewer",
        options,
        Box::new(move |_cc| Ok(Box::new(Shot { app, frames: 0, requested: false, out }))),
    )
}

/// Writes 24-bit BMP. Rows run bottom-up and are padded to four bytes.
fn write_bmp(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [width, height] = image.size;
    let row_bytes = width * 3;
    let padding = (4 - row_bytes % 4) % 4;
    let data_len = (row_bytes + padding) * height;
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"BM")?;
    out.write_all(&((54 + data_len) as u32).to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&54u32.to_le_bytes())?;
    out.write_all(&40u32.to_le_bytes())?;
    out.write_all(&(width as i32).to_le_bytes())?;
    out.write_all(&(height as i32).to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&24u16.to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&(data_len as u32).to_le_bytes())?;
    out.write_all(&PIXELS_PER_METRE.to_le_bytes())?;
    out.write_all(&PIXELS_PER_METRE.to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    let pad = [0u8; 3];
    for y in (0..height).rev() {
        for x in 0..width {
            let c = image.pixels[y * width + x];
            out.write_all(&[c.b(), c.g(), c.r()])?;
        }
        out.write_all(&pad[..padding])?;
    }
    out.flush()
}
