@echo off
REM ============================================================
REM  Hesabdari - run script
REM  Builds the app from the current sources and starts it, so it
REM  can be used after every change.
REM
REM  Usage:
REM     run.bat            Build (debug) and start   <- normal use
REM     run.bat release    Build (release) and start <- slower, finished build
REM     run.bat cleanup    Remove old incremental/duplicate build caches
REM ============================================================

setlocal enabledelayedexpansion
chcp 65001 >nul

REM --- Project root = folder of this script -------------------
set "ROOT=%~dp0"
set "ROOT=%ROOT:~0,-1%"
cd /d "%ROOT%"

REM --- Make cargo available even if not on PATH ---------------
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

REM --- Build cache location (same as build.bat) --------------
if not defined CARGO_TARGET_DIR (
    set "CARGO_TARGET_DIR=%ROOT%\cargo-target"
)
set "LOCAL_TARGET=%ROOT%\cargo-target"

REM --- Keep dev builds from accumulating large incremental caches ---
REM  This can make recompiling Rust code slower, but does not affect app runtime.
set "CARGO_INCREMENTAL=0"

if /i "%~1"=="cleanup" goto :cleanup

REM --- Debug (default) or release -----------------------------
set "MODE=%~1"
if /i "%MODE%"=="release" (
    set "CARGO_ARGS=--release"
    set "PROFILE_NAME=release"
) else (
    set "CARGO_ARGS="
    set "PROFILE_NAME=debug"
)
set "EXE=%CARGO_TARGET_DIR%\%PROFILE_NAME%\Hesabdari.exe"

echo.
echo ============================================================
echo   Hesabdari - run
echo   Profile : %PROFILE_NAME%
echo   Project : %ROOT%
echo   Target  : %CARGO_TARGET_DIR%
echo ============================================================
echo.

where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust from https://rustup.rs
    echo         then reopen this window and try again.
    goto :fail
)

REM --- Stop the running copy so the exe can be replaced -------
taskkill /f /im Hesabdari.exe >nul 2>nul

REM --- Re-embed the UI when ui\ is newer than the last build --
REM  The frontend files are embedded into the binary at compile
REM  time, so the app crate must be refreshed when ui\ changed.
if exist "!EXE!" (
    powershell -NoProfile -ExecutionPolicy Bypass -Command "$exe = Get-Item -LiteralPath '%EXE%'; $newest = Get-ChildItem -LiteralPath '%ROOT%\ui' -Recurse -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1; if ($newest -and ($newest.LastWriteTime -gt $exe.LastWriteTime)) { (Get-Item -LiteralPath '%ROOT%\src-tauri\src\lib.rs').LastWriteTime = Get-Date }"
)

echo [1/2] Building ...
cargo build --manifest-path "%ROOT%\src-tauri\Cargo.toml" %CARGO_ARGS%
if errorlevel 1 goto :fail

if not exist "!EXE!" (
    echo [ERROR] Build finished but the exe was not found:
    echo         !EXE!
    goto :fail
)

echo [2/2] Starting ...
start "" "!EXE!"

echo.
echo App started. Close this window or run the script again after changes.
timeout /t 3 >nul
goto :eof

:cleanup
echo This removes only generated build caches:
echo   %LOCAL_TARGET%\debug\incremental
echo   %ROOT%\src-tauri\target
set "CONFIRM="
set /p "CONFIRM=Continue? [y/N]: "
if /i not "!CONFIRM!"=="Y" (
    echo Cleanup cancelled.
    goto :eof
)
if exist "%LOCAL_TARGET%\debug\incremental" rmdir /s /q "%LOCAL_TARGET%\debug\incremental"
if exist "%ROOT%\src-tauri\target" rmdir /s /q "%ROOT%\src-tauri\target"
echo Build caches removed. The next build may take longer once.
goto :eof

:fail
echo.
echo ============================================================
echo   RUN FAILED  (see messages above)
echo ============================================================
echo.
pause
exit /b 1
