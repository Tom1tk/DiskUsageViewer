# Disk Usage Viewer

A small Windows app that shows how much space each folder and file on a drive uses. The left pane is a tree you expand one folder at a time; the right pane is a treemap, where each block's area is proportional to its size.

It is a native Win32 and Rust rewrite of an earlier Python version. It ships as one portable `.exe`, with no installer and no administrator rights needed.

![Disk Usage Viewer showing sample data](docs/screenshot.png)

*Sample data from a demo build, not a real drive.*

## Use it

1. Download `DiskUsageViewer.exe` from the [Releases](https://github.com/Tom1tk/DiskUsageViewer/releases) page and run it.
2. Pick a drive and press **Scan**. **Cancel** stops a scan.
3. Expand folders in the tree. In the treemap, hover a block for its details, and click it to show it in the tree.

Folders that Windows will not let you read are skipped and counted in the status line.

Sizes are size on disk (the space allocated), not file length. Hard links count once. Symbolic links and junctions are not followed. Files that exist only in the cloud (for example OneDrive online-only files) count as 0.

### Command line

- `DiskUsageViewer.exe --autostart` starts a scan of the selected drive when the window opens.
- `DiskUsageViewer.exe --scan C:\` scans without a window and prints the scan time, the totals and the ten largest top-level entries to the parent console.

## Build

You need Windows 10 or 11, the Rust toolchain for the MSVC target (install with [rustup](https://rustup.rs)), and the Visual Studio C++ build tools.

- `build.bat` runs `cargo build --release` and copies the result to `DiskUsageViewer.exe` in this folder.
- `cargo test` runs the unit tests.

The C runtime is linked statically, so the exe runs without extra redistributables.

## How it scans

- It reads directories with the NT native API, so one listing returns names, attributes, sizes and file IDs.
- It scans in parallel: up to 16 threads, or 2 when the drive reports a seek penalty (a spinning disk).
- Results appear when the scan finishes.

## macOS version

The same app for macOS 14 or later, in [`macos/`](macos). It is written in Rust with [egui](https://github.com/emilk/egui) through eframe, with the same drive list, tree, treemap, hover and click behaviour and the same `--autostart` and `--scan` options. It shares the Windows `format.rs` and `treemap.rs` files, and the Windows code is unchanged.

### Build

You need the Rust toolchain from [rustup](https://rustup.rs) (verified with Rust 1.99.0 on Apple silicon). Then run:

```bash
macos/build-app.sh
```

This runs `cargo build --release` and creates `macos/DiskUsageViewer.app`, signed ad hoc (no developer account needed), so it runs on the Mac that built it. Open it with `open macos/DiskUsageViewer.app`. Run `cargo test` in `macos/` for the unit tests.

### Command line

- `DiskUsageViewer.app/Contents/MacOS/DiskUsageViewer --autostart` starts a scan of the selected drive when the window opens.
- `DiskUsageViewer.app/Contents/MacOS/DiskUsageViewer --scan /` scans without a window and prints the scan time, the totals and the ten largest top-level entries.

### How it scans

- The default drive is `/`, as in Finder. Your files live on the Data volume, which is reached through paths such as `/Users`, so a scan of `/` covers them. Other volumes under `/Volumes` appear in the drive list and are scanned on their own.
- Mount points are not entered, so the volumes under `/System/Volumes` are not counted twice.
- Some folders are never opened, because macOS asks for permission before it lets a program read them: Desktop, Documents, Downloads, Movies, Music and Pictures in your home folder, and several folders under `~/Library` such as Mail, Messages and Safari. They are counted as skipped and their sizes are left out, even if you grant Full Disk Access. The full list is in [`withheld.rs`](macos/src/withheld.rs).
- Sizes are size on disk. Hard links count once. Symbolic links are not followed. iCloud files that are not downloaded count as 0.

### Limits

- Because of the withheld folders, a typical account's total is lower than `du` or Finder report for the same disk.
- A folder whose open blocks (for example while macOS waits on a permission answer) stalls the scan, and Cancel is only checked between folders. There is no timeout yet.
- Built and tested on macOS 27 with Apple silicon only.

The design decisions and the verification log are in [tasks/macos-todo.md](tasks/macos-todo.md).

## Source

| File | Purpose |
|---|---|
| `src/main.rs` | Entry point and command-line options |
| `src/app.rs` | Main window, drive list, scan control |
| `src/scan.rs` | Parallel directory scanner |
| `src/nt.rs` | NT native directory-listing bindings |
| `src/tree.rs` | Compact size tree and totals |
| `src/tree_view.rs` | Lazy tree control |
| `src/treemap.rs` | Squarified treemap layout |
| `src/treemap_view.rs` | Treemap window (drawing and hover) |
| `src/disk.rs` | Drive list, free space, seek-penalty query |
| `src/format.rs` | Size and number formatting |
| `src/win.rs` | Small Win32 helpers |

The design decisions and the verification log are in [tasks/todo.md](tasks/todo.md).

## Known limits

- No progressive display: results appear when a scan finishes.
- Results are not saved between runs, and the tree does not update when files change.
- The seek-penalty thread choice has not been tested on a spinning disk.
- Cold-cache scan speed has not been measured.

## About this project

This was a test of Claude Haiku 5.5 working in Rust through Claude Code. The app is a native Win32 rewrite of an earlier Python version.
