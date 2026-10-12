//! Disk Usage Viewer (native build). Shows how much space each folder and file on a drive uses,
//! as a tree and a treemap. `DiskUsageViewer.exe --scan C:\` scans without a window and prints
//! the totals, for timing and checks. `--autostart` starts a scan of the selected drive at launch.
#![windows_subsystem = "windows"]

mod app;
mod disk;
mod format;
mod nt;
mod scan;
mod tree;
mod tree_view;
mod treemap;
mod treemap_view;
mod win;

use std::time::Instant;

use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|arg| arg == "--scan") {
        let root = args.get(pos + 1).cloned().unwrap_or_else(|| "C:\\".to_string());
        headless_scan(&root);
        return;
    }
    app::run(args.iter().any(|arg| arg == "--autostart"));
}

/// Scans `root` with no window and prints the totals and the ten largest top-level entries.
fn headless_scan(root: &str) {
    // The exe is a windows-subsystem app, so it has no console of its own; reuse the parent's.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    let progress = scan::Progress::new();
    let start = Instant::now();
    match scan::scan_drive(root, &progress) {
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
        }
        Err(err) => println!("scan failed: {err:?}"),
    }
}
