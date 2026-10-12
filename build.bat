@echo off
cd /d "%~dp0"
cargo build --release
if errorlevel 1 exit /b 1
copy /Y "target\release\disk_usage_viewer.exe" "DiskUsageViewer.exe"
if errorlevel 1 exit /b 1
echo Built DiskUsageViewer.exe
