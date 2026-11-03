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
set "STAMP="

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
set "LOCAL_TARGET=%CARGO_TARGET_DIR%"
for %%I in ("%LOCAL_TARGET%") do set "LOCAL_TARGET=%%~fI"
set "CARGO_TARGET_DIR=%LOCAL_TARGET%"

if /i "%~1"=="cleanup" goto :cleanup

REM --- Debug (default) or release -----------------------------
set "MODE=%~1"
if not defined MODE set "MODE=debug"
if /i not "%MODE%"=="debug" if /i not "%MODE%"=="release" (
    echo [ERROR] Unknown mode: %MODE%
    echo         Valid modes: debug, release, cleanup
    goto :fail
)
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

REM --- Stop only this build's running copy so the exe can be replaced ---
REM  The result is read from the command's output, not from errorlevel:
REM  PowerShell sets its exit code to 1 whenever the last statement left
REM  $? false (Get-Process writing a silenced "not found" error does
REM  that), even when nothing went wrong. So report OK/FAILED instead.
set "KILL_STATE="
for /f "usebackq delims=" %%K in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; try { $target=[System.IO.Path]::GetFullPath($env:EXE); $running=@(Get-Process -Name 'Hesabdari' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ([System.IO.Path]::GetFullPath($_.Path) -eq $target) }); foreach($p in $running){ try { Stop-Process -Id $p.Id -Force -ErrorAction Stop } catch { } }; $remaining=@(); if($running.Count -gt 0){ $deadline=(Get-Date).AddSeconds(10); do { [System.Threading.Thread]::Sleep(200); $remaining=@(Get-Process -Name 'Hesabdari' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ([System.IO.Path]::GetFullPath($_.Path) -eq $target) }) } while($remaining.Count -gt 0 -and (Get-Date) -lt $deadline) }; if($remaining.Count -gt 0){ Write-Output 'FAILED' } else { Write-Output 'OK' } } catch { Write-Output 'FAILED' }"`) do set "KILL_STATE=%%K"

if /i not "!KILL_STATE!"=="OK" (
    echo [ERROR] Could not close the running app. Close it manually and retry.
    goto :fail
)

REM --- Force a rebuild when any source or UI file changed -----
REM  Cargo decides what to rebuild by file timestamps, not content.
REM  Some editors and copy/archive tools preserve very old stamps
REM  (even 1980), so a changed file can look older than the last
REM  build and cargo silently reuses the stale binary. Compare a
REM  content hash of every build input instead, and touch lib.rs
REM  (the crate that also embeds ui\) whenever that hash changes.
set "STAMP=%LOCAL_TARGET%\.run-inputs.sha256"
set "BUILD_STATE="
for /f "usebackq delims=" %%S in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $root=$env:ROOT; $files=@(); $files+=Get-ChildItem -LiteralPath (Join-Path $root 'src-tauri\src') -Recurse -File -Filter *.rs; $files+=Get-Item -LiteralPath (Join-Path $root 'src-tauri\Cargo.toml'),(Join-Path $root 'src-tauri\build.rs'),(Join-Path $root 'src-tauri\tauri.conf.json'); $files+=Get-ChildItem -LiteralPath (Join-Path $root 'src-tauri\capabilities') -Recurse -File; $files+=Get-ChildItem -LiteralPath (Join-Path $root 'ui') -Recurse -File; $sha=[System.Security.Cryptography.SHA256]::Create(); $sb=New-Object System.Text.StringBuilder; foreach($f in ($files|Sort-Object FullName)){ [void]$sb.Append($f.FullName.Substring($root.Length)).Append('|'); $content=[System.IO.File]::ReadAllBytes($f.FullName); [void]$sb.Append([System.BitConverter]::ToString($sha.ComputeHash($content)).Replace('-','')).Append([char]10) }; $hash=[System.BitConverter]::ToString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($sb.ToString()))).Replace('-','').ToLowerInvariant(); $stamp=Join-Path $env:LOCAL_TARGET '.run-inputs.sha256'; $old=''; if(Test-Path -LiteralPath $stamp){ $old=(Get-Content -LiteralPath $stamp -Raw).Trim() }; if($old -eq $hash){ Write-Output 'SAME' } else { $dir=Split-Path -Parent $stamp; if(-not (Test-Path -LiteralPath $dir)){ [void](New-Item -ItemType Directory -Path $dir -Force) }; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\src\lib.rs')).LastWriteTime=Get-Date; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\build.rs')).LastWriteTime=Get-Date; Set-Content -LiteralPath $stamp -Value $hash -NoNewline; Write-Output 'CHANGED' }"`) do set "BUILD_STATE=%%S"

if /i not "!BUILD_STATE!"=="SAME" if /i not "!BUILD_STATE!"=="CHANGED" (
    echo [ERROR] Could not verify the current source files.
    goto :fail
)

if /i "!BUILD_STATE!"=="SAME" (
    echo [1/2] No input changes detected.
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
if errorlevel 1 (
    echo [ERROR] Could not start the newly built app.
    goto :fail
)

echo.
echo App started from the latest build. Run this script again after editing sources.
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
