@echo off
setlocal

rem Rebuilds the frontend and the Rust backend in release mode, updating
rem the local exe in-place. Does not create installers/bundles (msi/nsis) -
rem just the exe, for fast local testing.
rem
rem Output: src-tauri\target\release\shuttle-sftp.exe

cd /d "%~dp0"

echo === Building frontend (npm run build) ===
call npm run build
if errorlevel 1 (
    echo Frontend build failed.
    exit /b 1
)

echo === Building backend (cargo build --release) ===
cd src-tauri
cargo build --release --features custom-protocol
if errorlevel 1 (
    echo Backend build failed.
    exit /b 1
)

echo.
echo Build succeeded: %~dp0src-tauri\target\release\shuttle-sftp.exe
