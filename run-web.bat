@echo off
REM ============================================================
REM  Hesabdari - local web mode
REM  Builds the loopback web server from the current sources and
REM  starts it. The browser opens automatically.
REM
REM  Usage:
REM     run-web.bat            Build (debug) and start
REM     run-web.bat release    Build (release) and start
REM
REM  The server listens on 127.0.0.1 only (default port 4174).
REM  Close this window or press Ctrl+C to stop the server.
REM ============================================================

setlocal enabledelayedexpansion
chcp 65001 >nul
title Hesabdari - local web server - close this window to stop

REM --- Project root = folder of this script -------------------
set "ROOT=%~dp0"
set "ROOT=%ROOT:~0,-1%"
cd /d "%ROOT%"

REM --- Make cargo available even if not on PATH ---------------
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

REM --- Debug (default) or release -----------------------------
set "MODE=%~1"
if not defined MODE set "MODE=debug"
if /i not "%MODE%"=="debug" if /i not "%MODE%"=="release" (
    echo [ERROR] Unknown mode: %MODE%
    echo         Valid modes: debug, release
    goto :fail
)
if /i "%MODE%"=="release" (
    set "CARGO_ARGS=--release"
    set "PROFILE_NAME=release"
) else (
    set "CARGO_ARGS="
    set "PROFILE_NAME=debug"
)

set "PORT=4174"
if defined HESABDARI_WEB_PORT set "PORT=%HESABDARI_WEB_PORT%"
set "URL=http://127.0.0.1:%PORT%/"

echo.
echo ============================================================
echo   Hesabdari - local web mode
echo   Address : %URL%
echo   Profile : %PROFILE_NAME%
echo   Project : %ROOT%
echo ============================================================
echo.

REM --- If this server is already up, just open it -------------
REM  Building while the running exe is still open would fail on
REM  Windows, so an existing healthy server is reused. Only a
REM  health payload from this app counts; another service that
REM  happens to answer on the same port must not be treated as
REM  ours (exit code 10).
powershell -NoProfile -ExecutionPolicy Bypass -Command "$reuse = $false; $foreign = $false; for ($i = 0; $i -lt 8; $i++) { try { $response = Invoke-WebRequest -UseBasicParsing -Uri '%URL%api/health' -TimeoutSec 1; if ($response.StatusCode -eq 200) { $health = $response.Content | ConvertFrom-Json; if ($health.ok -eq $true -and $health.mode -eq 'local-web') { $reuse = $true } else { $foreign = $true }; break } } catch { }; Start-Sleep -Milliseconds 400 }; if ($reuse) { exit 0 }; if ($foreign) { exit 10 }; exit 1"
if errorlevel 10 (
    echo [ERROR] Port %PORT% is already used by a different service.
    echo         Its /api/health response is not the Hesabdari local web
    echo         server, so this script will not reuse or replace it.
    echo         Close that service, or set HESABDARI_WEB_PORT to a free
    echo         port and run this script again.
    goto :fail
)
if not errorlevel 1 (
    echo [1/1] A Hesabdari local web server is already running.
    echo       Address: %URL%
    echo       If the app was rebuilt, close that server window and run
    echo       this script again so the new build is served.
    if not defined HESABDARI_NO_OPEN powershell -NoProfile -ExecutionPolicy Bypass -File "%ROOT%\scripts\open-web-ui.ps1" "%URL%?v=%RANDOM%%RANDOM%"
    echo.
    echo Close the server window to stop it.
    goto :end
)

REM --- The desktop app shares the same database ----------------
tasklist /FI "IMAGENAME eq Hesabdari.exe" 2>nul | find /I "Hesabdari.exe" >nul
if not errorlevel 1 (
    echo [WARN] The desktop app Hesabdari.exe is running.
    echo        Desktop and local web modes share one SQLite file and
    echo        must not use it at the same time.
    echo        Close the desktop app, then press any key to continue.
    echo.
    pause >nul
)

where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust from https://rustup.rs
    echo         then reopen this window and try again.
    goto :fail
)

echo [1/2] Building the web server from the current sources ...
echo       The first build can take a few minutes.
echo.
cargo build --manifest-path "%ROOT%\src-tauri\Cargo.toml" --features web --bin hesabdari-web %CARGO_ARGS%
if errorlevel 1 goto :fail

if defined CARGO_TARGET_DIR (
    set "TARGET_DIR=%CARGO_TARGET_DIR%"
) else (
    set "TARGET_DIR=%ROOT%\src-tauri\target"
)
for %%I in ("%TARGET_DIR%") do set "TARGET_DIR=%%~fI"
set "EXE=%TARGET_DIR%\%PROFILE_NAME%\hesabdari-web.exe"

if not exist "%EXE%" (
    echo [ERROR] Build finished but the web server was not found:
    echo         %EXE%
    goto :fail
)

echo.
echo [2/2] Starting the server. Close this window or press Ctrl+C to stop.
echo       The browser opens automatically.
echo.
"%EXE%"

echo.
echo The local web server has stopped.
goto :end

:fail
echo.
echo ============================================================
echo   LOCAL WEB MODE FAILED  (see messages above)
echo ============================================================
echo   If the build could not replace hesabdari-web.exe, close the
echo   server window that is already open and run this script again.
echo.
pause
exit /b 1

:end
endlocal
