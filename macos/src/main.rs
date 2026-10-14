//! Disk Usage Viewer for macOS. `DiskUsageViewer --scan <path>` scans without a window and prints
//! the scan time, the totals and the ten largest top-level entries. `--autostart` starts a scan of
//! the selected drive when the window opens.
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use disk_usage_viewer_macos::{app, format, scan};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|arg| arg == "--scan") {
        let root = args.get(pos + 1).map_or("/", String::as_str);
        return headless_scan(root);
    }
    let autostart = args.iter().any(|arg| arg == "--autostart");
    match app::run(autostart) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("could not open the window: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Scans `root` with no window and prints the totals and the ten largest top-level entries.
fn headless_scan(root: &str) -> ExitCode {
    let progress = scan::Progress::default();
    let start = Instant::now();
    match scan::scan_volume(Path::new(root), &progress) {
        Ok(tree) => {
            let root_id = tree.root();
            println!(
                "scan_ms={} on_disk_bytes={} files={} folders={} skipped={}",
                start.elapsed().as_millis(),
                tree.size(root_id),
                tree.files,
                tree.folders,
                tree.skipped
            );
            for id in tree.sorted_children(root_id).iter().take(10) {
                println!("{:>12}  {}", format::human_size(tree.size(*id)), tree.name(*id));
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("scan failed: {err:?}");
            ExitCode::FAILURE
        }
    }
}
