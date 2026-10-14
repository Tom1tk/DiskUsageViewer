# macOS version: plan and verification

Goal: a native macOS app with the same features as the Windows build (drive list, Scan and Cancel, tree plus treemap, hover details, click to reveal, `--scan` and `--autostart`). Lives in `macos/`. The Windows code in `src/` is not changed.

## Decisions

1. **Rust, with egui through eframe.** The repo is Rust, so the macOS port is Rust too. The UI toolkit was chosen by the user (egui over AppKit or SwiftUI). An earlier Swift port was moved to `macos-swift/` and kept, not deleted. The lesson behind the change is in [tasks/lessons.md](lessons.md).
2. **Shared code by path.** `format.rs` and `treemap.rs` are included from `../../src` with `#[path]`, unchanged. `tree.rs` is a copy with `/` path separators, because the Windows tree joins with `\`.
3. **Scanner: `getattrlistbulk` on paths.** One call returns names, types, file IDs, link counts and allocated sizes for a batch of entries. Folders are opened by path, not by handle, as in the Swift port.
4. **Scan root is `/`.** On APFS the user's data lives on the Data volume and is reached through firmlinks (`/Users`, `/Applications`, ...). Scanning `/` covers both, as Finder does.
5. **Mount points are not entered.** A directory whose full path is in the mount table (`getmntinfo`) is skipped. `st_dev` cannot be used: `/` and the Data volume report the same device.
6. **Hard links** are deduplicated by (volume, file ID). The volume is the `f_fsid` of the folder being listed (`fstatfs` on its descriptor). Only non-directories with a link count above 1 are recorded.
7. **Symlinks are skipped silently**, as junctions are on Windows.
8. **No seek-penalty query.** Threads are `min(16, available_parallelism)`.
9. **Pending folder jobs keep full paths, not open handles.** A handle per pending job would exhaust macOS's default file-descriptor limit.
10. **Entry ids come from one counter, handed out under the queue lock, one per recorded entry.** A child is numbered after its parent. Skipped entries take no id, so ids are dense. The tree build relies on both properties.
11. **Withheld folders are never opened.** The list in `withheld.rs` (Desktop, Documents, Downloads, Movies, Music, Pictures, several `~/Library` folders, and the automount points `/home` and `/Network`) is recorded as skipped without an `open()`. macOS shows a privacy prompt on open, and a headless scan cannot answer it. Full Disk Access does not change this.
12. **The done line includes the Windows "used" comparison.** On APFS, `statfs` "used" is container-level, so on `/` it is larger than the scan total. The gap is the mount points that are skipped by design. The Swift port left this line out for that reason. The Rust port keeps it for parity with Windows.
13. **Treemap and tree match the Windows panes.** The same header, hover text, hint, empty text, label thresholds and 150-block cap. Hovering only changes the info line, as on Windows. Clicking a block reveals it in the tree.
14. **The window is one egui app.** The toolbar is a drive picker, Scan and Cancel, then a status line. The tree is on the left (resizable) and the treemap on the right. A scan runs on its own thread, and the window polls a channel every frame and refreshes the status every 150 ms.

## Module layout

- `macos/src/main.rs`: `--scan <path>` headless, `--autostart`, otherwise opens the window.
- `macos/src/lib.rs`: module list. `format` and `treemap` are the shared Windows files.
- `macos/src/app.rs`: the window, drive picker, scan control, status text, error dialog.
- `macos/src/tree_view.rs`: the tree pane. It draws only the visible rows, through `show_rows`.
- `macos/src/treemap_view.rs`: the treemap pane (drawing, hover, click).
- `macos/src/browser.rs`: what is open, selected and focused. Pure logic with tests.
- `macos/src/blocks.rs`: treemap blocks for one folder, with the "N more items" block.
- `macos/src/scan.rs`: parallel scanner, progress, cancel, hard-link handling.
- `macos/src/listing.rs`: `getattrlistbulk` reader.
- `macos/src/mounts.rs`: mount table, the volumes the drive picker offers, volume names.
- `macos/src/withheld.rs`: folders that are never opened.
- `macos/src/tree.rs`: size tree, totals, sorted children, paths.
- `macos/examples/screenshot.rs`: renders the real window to a BMP (see Review).
- `macos/build-app.sh`, `macos/Info.plist`: release build and `DiskUsageViewer.app`, ad hoc signed.

## Checklist

- [x] Rust toolchain installed (rustc 1.99.0, with the user's go-ahead)
- [x] Core modules: listing, mounts, withheld, tree, scan, blocks, browser, with tests
- [x] Window: app, tree pane, treemap pane
- [x] `cargo build` and `cargo build --release` with no warnings
- [x] `cargo test`: 48 passed
- [x] `--scan /usr` matches `du -sk -x /usr`
- [x] `--scan /Applications` and `--scan /opt` match `du -sk -x` exactly
- [x] `--scan /` completes (exit 0, 13.0 s)
- [x] Window rendered by egui and checked from the image: `/usr` as the root, tree and treemap, drive picker, buttons, status line
- [x] `--autostart` launched from the release binary and ran for 12 s with no crash (the scan result was not observed)
- [x] `build-app.sh` and `Info.plist`; README and `.gitignore` updated
- [ ] Hover details, click-to-reveal, Cancel and the scanning status line were not checked on screen
- [ ] `cargo clippy` not run: the clippy component is not installed for this toolchain

## Known limits

- Protected folders (Desktop, Documents, Mail, Safari, ...) are never opened, so they are counted as skipped and their sizes are left out, even with Full Disk Access.
- A folder open has no timeout. If an open blocks (for example while macOS waits on a privacy answer), the scan stalls. Cancel is checked between folders and between directory batches.
- iCloud "optimized" files count as 0, as OneDrive online-only files do on Windows.
- Compressed (APFS-compressed) files report their allocated size, which can be smaller than the logical size.
- Built and tested on macOS 27 with Apple silicon only.

## Review

Result: the crate builds with no warnings, its 48 unit tests pass, and the headless scans match `du` on every folder that was compared.

Measured in this run (release build, Rust):

- `cargo test`: 48 passed, 0 failed.
- `--scan /usr`: `on_disk_bytes` 1,279,352,832, 23,233 files, 2,374 folders, 1 skipped. `du -sk -x /usr` reports 1,249,368 KB, which is the same figure (×1024).
- `--scan /Applications`: `on_disk_bytes` 21,263,261,696, 205,780 files, 41,807 folders, 0 skipped. `du -sk -x /Applications` gives the same bytes.
- `--scan /opt`: `on_disk_bytes` 2,239,791,104, 49,190 files, 6,809 folders, 0 skipped. `du -sk -x /opt` gives the same bytes.
- `--scan /`: exit 0 in 13.0 s. `on_disk_bytes` 128,130,707,456; 1,046,787 files; 319,943 folders; 397 skipped. The top level reads System 49.9 GB, Users 27.3 GB, Applications 19.8 GB, Library 11.9 GB, private 7.2 GB, opt 2.1 GB, usr 1.2 GB.
- The Swift port's `--scan /` (earlier run) gave 124,512,296,960 bytes for 1,018,041 files and 397 skipped. The skipped count matches. The total is 3.6 GB higher here, and I did not trace the cause. The subtrees that were compared match `du`.
- `--autostart` from the release bundle binary ran for 12 s and was stopped by the timer, with no panic output.
- Window: `macos/examples/screenshot.rs` runs the real `App` over a scanned tree and asks egui for the frame (`ViewportCommand::Screenshot`). The frame was written to a BMP, converted with `sips`, and viewed. The screenshot showed the drive picker ("Macintosh HD   52.6 GB free of 228.3 GB"), Scan enabled and Cancel disabled, the status line, the tree with sizes and percentages, and the treemap with its header and hint. Screen Recording is not needed for this.

Open risks:

- No per-open timeout. A folder open that blocks would stall the scan.
- The Swift port's one full `/` run stalled for 9–11 minutes and was killed. Its cause is not known. The Rust runs so far did not stall (one full `/` run, 13.0 s).
- Privacy prompts from earlier test runs may still be on the user's screen. The user answers those.
- Not verified on screen: hover details, click-to-reveal, Cancel, and the scanning status text while a scan runs.
