# Disk Usage Viewer, native Rust rewrite (option C): plan and spec

Goal: a single portable exe (no admin rights) that scans a chosen drive as fast as this PC allows and shows size per folder and file as a tree plus a treemap. Feature parity with `..\DiskUsageViewer\DiskUsageViewer.pyw`, with the fixes below. Scan speed is the top priority.

## Decisions (made; do not re-open)

1. Plain Win32 via `windows-sys` 0.61 for the GUI (no GUI framework). NT calls (`NtOpenFile`, `NtQueryDirectoryFile`, `NtClose`) declared in `nt.rs` with `#[link(name = "ntdll", kind = "raw-dylib")]`, the same way the benchmark did.
2. Scan: relative NT opens, parallel workers (LIFO stack, Mutex + Condvar, pending counter), `FileIdExtdDirectoryInformation` (class 60) so one listing returns attributes, EndOfFile, AllocationSize, reparse tag and file ID. Fallback to `FileDirectoryInformation` (class 1) if class 60 is rejected with STATUS_INVALID_INFO_CLASS (FAT/exFAT volumes).
3. Sizes are size on disk (AllocationSize), not logical size. Cloud-only OneDrive files therefore count as 0.
4. Reparse points: skip only name-surrogate tags (bit 0x2000_0000 set in the tag: symlink, junction, mount point). All other reparse entries are counted like normal entries.
5. Hard links: a file ID seen more than once is counted once, in the lowest entry id. Done by sorting file IDs after the scan (before totals). Only files that have an ID (class 60) take part.
6. Thread count: `min(16, available_parallelism)`. If the drive reports a seek penalty (spinning disk), use 2 threads. If the query fails, use the default. (Seek-penalty path is untested on a spinning disk.)
7. Results appear when the scan finishes (no progressive tree in v1). Progress shows counts and current folder during the scan.
8. Nothing is cached between runs (no saved results, no file watching) in v1.
9. Splitter between tree and treemap is fixed at 1:2 (no drag). Status text wraps; no window-size persistence.
10. Only `DiskUsageViewer.exe` is shipped. Static C runtime (`.cargo/config.toml`). `strip` on, LTO fat.

## Module ownership and contracts

Files are under `src/`. Each file stays under ~400 lines. Public items are exactly as listed; private helpers are free.

### nt.rs (+ scan.rs + tree.rs), owner: scanner agent
Port the NT enumeration from the benchmark `C:\Users\TPenrose\AppData\Local\Temp\claude\C--Users-TPenrose-Documents\5997c4dd-0c7c-48c3-a2aa-fa04c0e6c00f\scratchpad\fastscan\src\main.rs` (read it first), and change as listed here.

`scan.rs`:
```rust
pub struct Progress { pub folders: AtomicU64, pub files: AtomicU64, pub skipped: AtomicU64, pub current: Mutex<String>, pub cancel: AtomicBool }
impl Progress { pub fn new() -> Self; pub fn request_cancel(&self); }
pub enum ScanError { Cancelled, Failed(String) }
pub fn scan_drive(root: &str, progress: &Progress) -> Result<tree::Tree, ScanError>;
```
- `root` is a drive root such as `C:\`. Open it as `\??\C:\`. If it cannot be opened, return `Failed` with a message naming the path.
- Progress: each worker keeps local counters and adds them to the shared atomics every 256 folders and when it exits. `current` is updated with `try_lock` every 256 folders (never blocks a worker).
- Cancel is checked before each folder. A cancelled scan returns `Err(ScanError::Cancelled)`.
- Unreadable child folder: still recorded as a folder entry (size 0) and `skipped += 1`. Scan continues.
- Partial listing error (any status other than STATUS_NO_MORE_FILES 0x8000_0006): keep the entries read so far, `skipped += 1`.
- Entries named `.` and `..` are ignored.
- Reparse rule: skip an entry when attributes has 0x400 and the tag has bit 0x2000_0000 set. Skipped entries are not counted in `skipped`.
- Files: size = AllocationSize; file ID stored with the entry id for dedupe (class 60 only).
- Thread count: `disk::has_seek_penalty(root)` → `Some(true)` gives 2 threads; otherwise `available_parallelism().min(16)`.
- Calls `tree::build(...)` (or equivalent) after the workers finish. Deduplication runs before totals. Build time is measured in the benchmark and must stay small.

`tree.rs`:
```rust
pub struct Tree { pub files: u64, pub folders: u64, pub skipped: u64, /* private arrays */ }
impl Tree {
    pub fn root(&self) -> u32;                       // always 0
    pub fn name(&self, id: u32) -> String;           // root = "C:\"; others = entry name
    pub fn size(&self, id: u32) -> u64;              // size on disk; folders = subtree total
    pub fn is_dir(&self, id: u32) -> bool;
    pub fn parent(&self, id: u32) -> Option<u32>;    // None for root
    pub fn child_count(&self, id: u32) -> usize;     // files and folders; 0 for files
    pub fn sorted_children(&self, id: u32) -> Vec<u32>; // largest size first, ties by name
    pub fn path(&self, id: u32) -> String;           // "C:\Windows\System32"; no double backslash after "C:\"
}
```
- Storage: struct-of-arrays. One name arena (`Vec<u16>`) with (offset, length) per entry. Children stored as CSR (offsets + list), built by counting sort. No per-file heap objects.
- Entry ids are assigned from one atomic counter; a child's id is always larger than its parent's. Totals are one reverse pass: `size[parent] += size[id]` for id from n-1 down to 1.
- Dedupe: sort (file_id, entry id) pairs; for each run of equal file IDs, set size = 0 on all but the first (lowest entry id). Must run before the reverse pass.
- `sorted_children` allocates a new Vec each call (UI calls it only on focus change).

Tests (`#[cfg(test)]`, in the same files):
- tree: hand-built small tree, check totals, child order, path, dedupe.
- scan: fixture folder under `std::env::temp_dir()` with a unique name: nested folders, files of known length (1 byte, 5000 bytes, 0 bytes), an unreadable folder if creatable (skip the test if not), a junction via `cmd /c mklink /J` (skip if it fails). Assert: entry counts, every file's size equals its allocation (`(len + 4095) / 4096 * 4096` for NTFS with 4 KB clusters; assert `>= len` and `% 512 == 0` if unsure), junction not counted, folder totals equal sums.

Headless check (run by lead after build): `DiskUsageViewer.exe --scan C:\` prints `scan_ms`, totals and top-10 entries.

### disk.rs, format.rs, treemap.rs, owner: utilities agent
`disk.rs`:
```rust
pub struct Drive { pub root: String, pub total: u64, pub free: u64 }   // free = total free bytes (not caller quota)
impl Drive { pub fn used(&self) -> u64 { self.total - self.free } }
pub fn list_drives() -> Vec<Drive>;                 // drives that are ready, in drive-letter order
pub fn free_space(root: &str) -> Option<Drive>;     // one drive, same numbers
pub fn has_seek_penalty(root: &str) -> Option<bool>; // None when the query fails
```
- Use `GetLogicalDrives` bitmask, `GetDriveTypeW` (skip DRIVE_NO_ROOT_DIR = 1), `GetDiskFreeSpaceExW` (skip on failure, and skip when free > total).
- `has_seek_penalty`: `CreateFileW("\\.\C:", access 0, share all, OPEN_EXISTING)`, then `DeviceIoControl(IOCTL_STORAGE_QUERY_PROPERTY)` with `STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceSeekPenaltyProperty (7), QueryType: PropertyStandardQuery (0) }` and output `DEVICE_SEEK_PENALTY_DESCRIPTOR`; return `IncursSeekPenalty`. Close the handle on every path.

`format.rs`:
```rust
pub fn human_size(bytes: u64) -> String;   // same rules as Python human_size (see below)
pub fn percent(part: u64, whole: u64) -> String; // "12.3%"; whole == 0 gives "0.0%"
pub fn grouped(n: u64) -> String;          // "1,234,567"
```
- human_size: units B, KB, MB, GB (base 1024); B has no decimals (`"512 B"`), the others have one decimal (`"672.6 GB"`); beyond GB use TB with one decimal.

`treemap.rs` (port of `..\DiskUsageViewer\treemap.py`, squarified):
```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
pub fn layout(sizes: &[f64], bounds: Rect) -> Vec<Rect>; // one Rect per size, same order; sizes positive, largest first
```
- Empty input returns empty; non-positive total returns all zero rects.
- Tests: rects tile the bounds (areas sum to bounds area within 1e-6 relative), no overlaps, areas proportional to sizes, and an aspect ratio below 4 for a varied input of 20 sizes.

### treemap_view.rs, owner: treemap-control agent
The right-hand pane. A custom window class that draws the header, the blocks, the hover text and the bottom info line. It owns all of that drawing. It does not own the tree.

```rust
pub const WM_APP_REVEAL: u32 = WM_APP + 2; // sent (PostMessageW) to the parent; wParam = entry id (u32)
pub fn register(hinstance: HINSTANCE) -> bool;
pub fn create(parent: HWND, hinstance: HINSTANCE) -> HWND;
pub fn set_view(hwnd: HWND, tree: Option<Arc<Tree>>, focus: Option<u32>); // focus = folder whose children are drawn
```
- Store state in GWLP_USERDATA as a Box, freed on WM_NCDESTROY.
- `set_view` recomputes the block list (see Blocks) and invalidates the window.
- Header (top, bold Segoe UI 10 pt, created by the control): `"{path}    {size}    {n} items"` with `path` = `tree.path(focus)`, `size` = `format::human_size(tree.size(focus))`, `n` = `grouped(child_count)`. Empty when focus is None.
- Blocks: children with size > 0 in `sorted_children` order. Only the first 150 are drawn; the rest are one block labelled `"{count} more items"` (hover text `"{count} smaller items    {size}    {pct} of this folder"`). If no child has size > 0 draw "Empty folder" centred in grey.
- Colours: folder blocks take the palette in order (index counts folders only): `#a8dadc #f4a261 #e9c46a #90be6d #f9c74f #f8961e #b5e48c #c3aed6 #ffb4a2 #9ad1d4`. File blocks `#dfe3e8`. The "more" block `#eeeeee`. Block outline: white 1 px.
- Block label: name at (x+5, y+4) when w >= 60 and h >= 18, with `DT_END_ELLIPSIS | DT_NOPREFIX`. When h >= 36 also draw `format::human_size(size)` on a second line. Text colour `#1a1a1a`.
- Hover: a block under the mouse shows `"{name}    {size}    {pct} of this folder"` (pct = size / folder size, `format::percent`) in the info line. Without a block the info line shows `Hover over a block for details. Click a block to open it in the tree.`
- Click on a block that is a real entry: PostMessageW(parent, WM_APP_REVEAL, id, 0). The "more" block does nothing.
- Blocks are computed in `set_view` and on WM_SIZE, never on mouse move. Hover only changes the info line.
- Paint into a memory DC (double buffer) so there is no flicker. Background white.
- All pixel values scale with the window DPI (`GetDpiForWindow / 96`).

### tree_view.rs, app.rs, win.rs, owner: main-window agent (runs after treemap_view is done)
`win.rs`: small helpers only: `wide(&str) -> Vec<u16>` (NUL-terminated), `text_of(hwnd) -> String`, `set_text(hwnd, &str)`, `scale(px, dpi) -> i32`, `message(hwnd, title, text)`.

`tree_view.rs`:
```rust
pub const PLACEHOLDER: u32 = u32::MAX;  // lParam of the dummy child that gives a folder its expand arrow
pub fn create(parent: HWND, hinstance: HINSTANCE) -> HWND;   // SysTreeView32, TVS_HASLINES | TVS_HASBUTTONS | TVS_LINESATROOT | TVS_SHOWSELALWAYS, WS_BORDER | WS_VSCROLL
pub fn show_root(hwnd: HWND, tree: &Tree);     // clear; insert root (expanded, selected) with its children one level down
pub fn clear(hwnd: HWND);
pub fn on_expanding(hwnd: HWND, tree: &Tree, item: HTREEITEM); // TVN_ITEMEXPANDING: replace the placeholder with real children, once
pub fn reveal(hwnd: HWND, tree: &Tree, id: u32);            // expand ancestors root-first, select id, ensure visible
pub fn item_id(hwnd: HWND, item: HTREEITEM) -> Option<u32>; // lParam, None for placeholder
```
- Item text: `"{name}    {size}    {share}"`, share = `percent(size, parent size)`; no share for the root.
- Children inserted in `sorted_children` order. Folders get one placeholder child when they have children.
- Insert with `WM_SETREDRAW` off during bulk insert.
- Lookup by id (reveal): walk the child items and compare lParam. No HashMap.

`app.rs`: the main window. `pub fn run(autostart: bool)`.
- Window title `Disk Usage Viewer`. Client size 1150 x 720 at 96 DPI, scaled. Per-monitor-aware-v2 DPI awareness (`SetProcessDpiAwarenessContext(-4)`, fall back to `SetProcessDPIAware`).
- Layout (client coordinates, all scaled, margin 10, gap 8):
  - Row 1 (height 26, top 10): static `Drive:`, combo (CBS_DROPDOWNLIST, width 360), `Scan` button (80), `Cancel` button (80).
  - Row 2: status static, full width, height 36 (two lines, wraps), 6 px below row 1.
  - Below: left column = tree view, width `(client_w - 2*margin - gap) / 3`; right column = treemap control, remaining width. Both fill down to the bottom margin.
- Combo entries: `"{root}   {format::human_size(free)} free of {format::human_size(total)}"`, one per `list_drives()` entry. Selected by default: the entry for `C:\`, else the first. No drives: status `No drives found.` and Scan disabled.
- Status texts (exact):
  - start: `Pick a drive, then press Scan. Folders Windows will not let you read are skipped.`
  - scanning (every 150 ms via WM_TIMER id 1): `Scanning {root}   {grouped(files)} files, {grouped(folders)} folders, {grouped(skipped)} skipped   {tail(current, 70)}` where `tail` keeps the last 70 chars with `...` in front when longer.
  - cancelling: `Cancelling...`
  - cancelled: `Scan cancelled.`
  - failed: `Scan failed.` plus a MessageBox with the error text.
  - done: `Scanned {human_size(on_disk)} on disk in {grouped(files)} files and {grouped(folders)} folders ({secs:.0} s). Windows reports {human_size(used)} used on {root}. {grouped(skipped)} items could not be read and were skipped.` `used` comes from `disk::free_space(root)`, and is omitted when it returns None.
- While a scan runs: Scan and the combo are disabled, Cancel is enabled. Afterwards the reverse.
- Scan runs on a `std::thread`. It takes an `Arc<Progress>`, stores the result in `Arc<Mutex<Option<Result<Tree, ScanError>>>>`, then PostMessageW(hwnd, WM_APP_SCAN_DONE, 0, 0). The UI thread never blocks on it.
- Done: `tree_view::show_root`, `treemap_view::set_view(Some(arc), Some(root))`, final status. Cancelled/failed: clear both views, status text.
- Tree notifications: TVN_ITEMEXPANDING → `tree_view::on_expanding`; TVN_SELCHANGEDW → focus = id if folder else parent; `treemap_view::set_view(..., focus)`.
- WM_APP_REVEAL from the treemap → `tree_view::reveal`.
- `--autostart`: after the window is shown, start a scan of the selected drive (post a message from WM_CREATE, not a direct call).
- WM_DESTROY → PostQuitMessage(0).

## Verification (lead)

1. `cargo test` passes (all unit tests).
2. `cargo build --release` has no warnings. Copy `target\release\disk_usage_viewer.exe` to `DiskUsageViewer.exe`.
3. `DiskUsageViewer.exe --scan C:\` completes. Compare the file and folder counts with the benchmark (about 1.93M files, 319k folders, 449 skipped, before the reparse change), and check the on-disk total is below the 721.6 GB Windows reports used.
4. `DiskUsageViewer.exe --autostart`: screenshot via PowerShell (no input sent to the desktop); close the window afterwards.
5. Static C runtime check: `dumpbin /dependents` is not available, so check that the exe runs with PATH limited to System32 and Windows.

## Known gaps (state these in the final answer)

- No progressive display; no saved results; no ReadDirectoryChangesW updates.
- Seek-penalty thread selection not tested on a spinning disk.
- Dedupe of hard links and class-60 listing cost measured only by the final scan time.
- Cold-cache performance not measured.
- No splitter drag; no DPI test on a second monitor.

## Progress

- [x] Cargo project, windows-sys 0.61 fetched, release profile, static CRT
- [x] main.rs entry points (`--scan`, `--autostart`)
- [x] nt.rs, scan.rs, tree.rs (+ tests): audited by lead; 7 scan tests and 4 tree/nt tests pass, and no test printed a skip message
- [x] disk.rs, format.rs, treemap.rs (+ tests): 4 + 6 + 8 tests pass
- [x] treemap_view.rs: paint checked by screenshot (1 unit test)
- [x] tree_view.rs, app.rs, win.rs: checked by screenshot (no unit tests, per spec)
- [x] verify: cargo test (32 pass), release build (0 warnings), headless scan, GUI launch and close, autostart screenshot, PATH-limited run, exe copy
- [x] build.bat: cargo build --release, then copy. Run in the earlier session; this session ran the same steps by hand.
- [x] Review section (below)

## Review

Verified:
- `cargo test --release`: 32 passed, 0 failed.
- `cargo build --release`: 0 warnings. `DiskUsageViewer.exe` copied from `target\release`.
- Crash fixed. The first paint with no model called `DrawTextW` with an empty string. An empty `Vec<u16>` has a dangling pointer (fault address 0x2). `draw_text` in `src\treemap_view.rs` now returns early on empty text. The temporary vectored exception handler in `src\main.rs` is removed.
- Launch with no args: still running at 6 s. `CloseMainWindow` exits with code 0. No new Application event log faults.
- `--autostart`: three runs. Two completed (5 s and 9 s, from the status line). One was still scanning at 12 s and was closed mid-scan (exit code 0). The last run was captured with PrintWindow after setting per-monitor DPI awareness, because the display runs at 125% (no input sent). It shows the header, tree, treemap, status line and info line, with even margins. Exit code 0, no faults.
- Headless `--scan C:\`: run 1 (earlier session) `scan_ms=3371`, 449 skipped. Four later runs: 10,601 / 10,927 / 10,277 / 7,944 ms (median 10.5 s), 446 skipped. The 3.4 s run did not repeat. Cause not isolated: the reference was 3.0–4.8 s, and Windows' used figure rose from 730.8 to 739.2 GB during the session, so background disk activity is a possible cause (not checked). No cold-cache figure.
- PATH limited to System32 and Windows: exit 0 (static CRT).

Threshold (verification step 3): `--scan` prints raw bytes; the GUI and spec use base-1024 units. The final GUI run scanned 683.4 GB against 739.2 GB Windows reports used, in the same units. 683.4 is below both 721.6 (the spec figure, now stale) and 739.2. Passes. Not checked: the ~56 GB between the scan and Windows' used figure (possibly filesystem metadata and folders that need admin).

Not verified:
- No fresh Python baseline. Counts differ from the spec's benchmark (1.93M files, 319k folders) by under 1%. That is data drift, not checked.
- Dropdown not opened in the GUI test.
- Known gaps above still apply (cold cache, spinning-disk thread choice, second-monitor DPI).

Deviations from spec:
- Combo window is 200 px tall (spec row height 26) so the drop-down list has room. The closed box looks normal in the screenshot.
- `app.rs` 615 lines and `treemap_view.rs` 775 lines. Over the spec's ~400 guideline, under the 800 max in coding-style.md. Not split.
