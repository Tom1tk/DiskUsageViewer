//! The main window: a drive picker, Scan and Cancel, a status line, the folder tree on the left and
//! the treemap on the right. A scan runs on its own thread and sends its result over a channel,
//! which the window checks every frame.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Button, ComboBox, Ui, Vec2};

use crate::browser::Browser;
use crate::format::{grouped, human_size};
use crate::mounts::{self, Volume};
use crate::scan::{self, Progress, ScanError};
use crate::tree::Tree;
use crate::tree_view;
use crate::treemap_view::{self, BlockCache};

const TITLE: &str = "Disk Usage Viewer";
const START_TEXT: &str = "Pick a drive, then press Scan. Folders macOS will not let you read are skipped.";
const NO_DRIVES_TEXT: &str = "No drives found.";
const CANCELLING_TEXT: &str = "Cancelling...";
const CANCELLED_TEXT: &str = "Scan cancelled.";
const FAILED_TEXT: &str = "Scan failed.";
const NO_RESULT_TEXT: &str = "The scan ended without a result.";
/// How often a running scan refreshes its status line.
const REFRESH: Duration = Duration::from_millis(150);
const TAIL_CHARS: usize = 70;
const WINDOW_SIZE: [f32; 2] = [1150.0, 720.0];
const TREE_WIDTH: f32 = 340.0;
const TREE_MIN_WIDTH: f32 = 160.0;
const DRIVE_WIDTH: f32 = 360.0;
/// The drive the picker starts on, when the window opens.
const STARTUP_DISK: &str = "/";

/// A scan in progress. `outcome` yields one value, when the scan thread finishes.
struct Scan {
    root: String,
    progress: Arc<Progress>,
    outcome: Receiver<Result<Tree, ScanError>>,
    started: Instant,
}

pub struct App {
    volumes: Vec<Volume>,
    selected: usize,
    autostart: bool,
    status: String,
    /// A message for the dialog that a failed scan opens.
    error: Option<String>,
    browser: Option<Browser>,
    blocks: BlockCache,
    scan: Option<Scan>,
}

/// Opens the window. With `autostart`, a scan of the selected drive starts as soon as it opens.
pub fn run(autostart: bool) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(autostart)))),
    )
}

impl App {
    fn new(autostart: bool) -> Self {
        let volumes = mounts::list_volumes();
        let selected = volumes.iter().position(|v| v.path == STARTUP_DISK).unwrap_or(0);
        let status = if volumes.is_empty() { NO_DRIVES_TEXT } else { START_TEXT };
        App {
            volumes,
            selected,
            autostart,
            status: status.to_owned(),
            error: None,
            browser: None,
            blocks: BlockCache::default(),
            scan: None,
        }
    }

    /// A window that shows `tree` as if its scan of `root` had just finished. Used by
    /// `examples/screenshot.rs`, which renders the window without opening a scan.
    pub fn with_tree(tree: Tree, root: &str, secs: f64) -> Self {
        let mut app = App::new(false);
        app.status = done_text(&tree, root, secs);
        app.browser = Some(Browser::new(Arc::new(tree)));
        app
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let idle = self.scan.is_none();
        let can_scan = idle && !self.volumes.is_empty();
        let mut scan_clicked = false;
        let mut cancel_clicked = false;
        ui.horizontal(|ui| {
            ui.label("Drive:");
            ui.add_enabled_ui(idle, |ui| self.drive_picker(ui));
            scan_clicked = ui.add_enabled(can_scan, Button::new("Scan")).clicked();
            cancel_clicked = ui.add_enabled(!idle, Button::new("Cancel")).clicked();
        });
        if scan_clicked {
            self.start_scan();
        }
        if cancel_clicked {
            self.cancel_scan();
        }
    }

    fn drive_picker(&mut self, ui: &mut Ui) {
        let shown = self.volumes.get(self.selected).map(volume_label).unwrap_or_default();
        ComboBox::from_id_salt("drive")
            .width(DRIVE_WIDTH)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (index, volume) in self.volumes.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, index, volume_label(volume));
                }
            });
    }

    fn start_scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let Some(volume) = self.volumes.get(self.selected) else { return };
        let root = volume.path.clone();
        // The old tree goes first, so no tree row or treemap block still refers to it.
        self.browser = None;

        let progress = Arc::new(Progress::default());
        let (sender, outcome) = mpsc::channel();
        let worker = Arc::clone(&progress);
        let path = PathBuf::from(&root);
        thread::spawn(move || {
            // The receiver is gone only when the window has closed, so a failed send has no one to tell.
            let _ = sender.send(scan::scan_volume(&path, &worker));
        });
        self.status = scanning_text(&root, &progress);
        self.scan = Some(Scan { root, progress, outcome, started: Instant::now() });
    }

    fn cancel_scan(&mut self) {
        let Some(scan) = self.scan.as_ref() else { return };
        scan.progress.request_cancel();
        self.status = CANCELLING_TEXT.to_owned();
    }

    /// Checks for the scan's result. While the scan runs, refreshes the status line.
    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(scan) = self.scan.as_ref() else { return };
        let outcome = match scan.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => {
                self.status = if scan.progress.is_cancelled() {
                    CANCELLING_TEXT.to_owned()
                } else {
                    scanning_text(&scan.root, &scan.progress)
                };
                ctx.request_repaint_after(REFRESH);
                return;
            }
            Err(TryRecvError::Disconnected) => None,
        };
        self.finish_scan(outcome);
    }

    /// `None` means the scan thread ended without sending a result.
    fn finish_scan(&mut self, outcome: Option<Result<Tree, ScanError>>) {
        let Some(scan) = self.scan.take() else { return };
        match outcome {
            Some(Ok(tree)) => self.show_tree(&scan, tree),
            Some(Err(ScanError::Cancelled)) => self.status = CANCELLED_TEXT.to_owned(),
            Some(Err(ScanError::Failed(text))) => {
                self.status = FAILED_TEXT.to_owned();
                self.error = Some(text);
            }
            None => {
                self.status = FAILED_TEXT.to_owned();
                self.error = Some(NO_RESULT_TEXT.to_owned());
            }
        }
    }

    fn show_tree(&mut self, scan: &Scan, tree: Tree) {
        let secs = scan.started.elapsed().as_secs_f64();
        self.status = done_text(&tree, &scan.root, secs);
        self.browser = Some(Browser::new(Arc::new(tree)));
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let Some(text) = self.error.as_deref() else { return };
        let mut close = false;
        egui::Window::new(TITLE)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(text);
                close = ui.button("OK").clicked();
            });
        if close {
            self.error = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.autostart {
            self.autostart = false;
            self.start_scan();
        }
        self.poll_scan(ui.ctx());

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("status").show(ui, |ui| {
            ui.label(self.status.as_str());
        });
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(TREE_WIDTH)
            .min_size(TREE_MIN_WIDTH)
            .show(ui, |ui| {
                if let Some(browser) = self.browser.as_mut() {
                    tree_view::show(ui, browser);
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(id) = treemap_view::show(ui, self.browser.as_ref(), &mut self.blocks) {
                if let Some(browser) = self.browser.as_mut() {
                    browser.reveal(id);
                }
            }
        });
        self.error_dialog(ui.ctx());
    }
}

fn volume_label(volume: &Volume) -> String {
    format!(
        "{}   {} free of {}",
        volume.name,
        human_size(volume.free),
        human_size(volume.total)
    )
}

fn scanning_text(root: &str, progress: &Progress) -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let current = progress.current.lock().unwrap_or_else(|e| e.into_inner()).clone();
    format!(
        "Scanning {root}   {} files, {} folders, {} skipped   {}",
        grouped(progress.files.load(Relaxed)),
        grouped(progress.folders.load(Relaxed)),
        grouped(progress.skipped.load(Relaxed)),
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

fn done_text(tree: &Tree, root: &str, secs: f64) -> String {
    let used = mounts::list_volumes()
        .into_iter()
        .find(|v| v.path == root)
        .map(|v| v.used());
    let used = match used {
        Some(bytes) => format!(" macOS reports {} used on {}.", human_size(bytes), root),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_of_long_text() {
        assert_eq!(tail("short", 70), "short");
        assert_eq!(tail("abcdef", 3), "...def");
    }
}
