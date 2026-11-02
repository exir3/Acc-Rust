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

REM --- Force a rebuild when any source or UI file changed -----
REM  Cargo decides what to rebuild by file timestamps, not content.
REM  Some editors and copy/archive tools preserve very old stamps
REM  (even 1980), so a changed file can look older than the last
REM  build and cargo silently reuses the stale binary. Compare a
REM  content hash of every build input instead, and touch lib.rs
REM  (the crate that also embeds ui\) whenever that hash changes.
set "STAMP=%LOCAL_TARGET%\.run-inputs.sha256"
set "BUILD_STATE="
for /f "usebackq delims=" %%S in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$root='%ROOT%'; $files=@(); $files+=Get-ChildItem -LiteralPath (Join-Path $root 'src-tauri\src') -Recurse -File -Filter *.rs -ErrorAction SilentlyContinue; $files+=Get-Item -LiteralPath (Join-Path $root 'src-tauri\Cargo.toml'),(Join-Path $root 'src-tauri\build.rs'),(Join-Path $root 'src-tauri\tauri.conf.json') -ErrorAction SilentlyContinue; $files+=Get-ChildItem -LiteralPath (Join-Path $root 'src-tauri\capabilities') -Recurse -File -ErrorAction SilentlyContinue; $files+=Get-ChildItem -LiteralPath (Join-Path $root 'ui') -Recurse -File -ErrorAction SilentlyContinue; $sha=[System.Security.Cryptography.SHA256]::Create(); $sb=New-Object System.Text.StringBuilder; foreach($f in ($files|Sort-Object FullName)){ [void]$sb.Append($f.FullName.Substring($root.Length)).Append('|'); $content=[System.IO.File]::ReadAllBytes($f.FullName); [void]$sb.Append([System.BitConverter]::ToString($sha.ComputeHash($content)).Replace('-','')).Append([char]10) }; $hash=[System.BitConverter]::ToString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($sb.ToString()))).Replace('-','').ToLowerInvariant(); $stamp=Join-Path $root 'cargo-target\.run-inputs.sha256'; $old=''; if(Test-Path -LiteralPath $stamp){ $old=(Get-Content -LiteralPath $stamp -Raw).Trim() }; if($old -eq $hash){ Write-Output 'SAME' } else { $dir=Split-Path -Parent $stamp; if(-not (Test-Path -LiteralPath $dir)){ [void](New-Item -ItemType Directory -Path $dir -Force) }; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\src\lib.rs')).LastWriteTime=Get-Date; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\build.rs')).LastWriteTime=Get-Date; Set-Content -LiteralPath $stamp -Value $hash -NoNewline; Write-Output 'CHANGED' }"`) do set "BUILD_STATE=%%S"

if /i "!BUILD_STATE!"=="SAME" (
    echo [1/2] No input changes - using the existing build.
) else (
    echo [1/2] Input changes detected - rebuilding the app ...
)

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
REM --- Do not trust this build next time; retry a full rebuild ---
if defined STAMP if exist "!STAMP!" del /q "!STAMP!" >nul 2>nul
echo.
echo ============================================================
echo   RUN FAILED  (see messages above)
echo ============================================================
echo.
pause
exit /b 1
