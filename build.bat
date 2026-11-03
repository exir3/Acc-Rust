@echo off
REM ============================================================
REM  Hesabdari - build script
REM  Builds the Windows .exe (portable). The NSIS installer is
REM  optional and only built when explicitly requested.
REM
REM  Usage:
REM     build.bat            Portable exe only (default)
REM     build.bat portable   Portable exe only
REM     build.bat all        Portable exe + NSIS installer
REM     build.bat installer  NSIS installer only
REM     build.bat clean      Remove the build cache, then exit
REM ============================================================

setlocal enabledelayedexpansion
chcp 65001 >nul

REM --- Project root = folder of this script -------------------
set "ROOT=%~dp0"
set "ROOT=%ROOT:~0,-1%"
cd /d "%ROOT%"

REM --- Make cargo available even if not on PATH ---------------
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

REM --- Build cache location -----------------------------------
REM  Keep the build cache inside the project folder.
if not defined CARGO_TARGET_DIR (
    set "CARGO_TARGET_DIR=%ROOT%\cargo-target"
)
set "LOCAL_TARGET=%CARGO_TARGET_DIR%"
for %%I in ("%LOCAL_TARGET%") do set "LOCAL_TARGET=%%~fI"
set "CARGO_TARGET_DIR=%LOCAL_TARGET%"

REM --- Argument ----------------------------------------------
set "MODE=%~1"
if "%MODE%"=="" set "MODE=portable"
set "MADE_INSTALLER=0"

echo.
echo ============================================================
echo   Hesabdari - build
echo   Mode          : %MODE%
echo   Project       : %ROOT%
echo   Build cache   : %CARGO_TARGET_DIR%
echo ============================================================
echo.

REM --- Validate requested mode before checking build tools -----
if /i "%MODE%"=="clean" goto :do_clean
if /i "%MODE%"=="portable" goto :mode_valid
if /i "%MODE%"=="installer" goto :mode_valid
if /i "%MODE%"=="all" goto :mode_valid
echo [ERROR] Unknown mode: %MODE%
echo         Valid modes: portable, installer, all, clean
goto :fail

:mode_valid
REM --- Sanity checks -----------------------------------------
where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust from https://rustup.rs
    echo         then reopen this window and try again.
    goto :fail
)

REM --- Tauri CLI (installed as a cargo subcommand) -------------
set "TAURI=cargo tauri"
where cargo-tauri >nul 2>nul
if errorlevel 1 (
    echo [ERROR] Tauri CLI not found. Install it with:
    echo         cargo install tauri-cli --version "^2"
    goto :fail
)

REM --- Refresh the app crate before every build -----------------
REM  Cargo compares file timestamps, and some editors/copy tools
REM  keep very old timestamps on edited files. A changed file can
REM  then look older than the last build, so cargo silently reuses
REM  a stale binary. Touching these two files forces the crate and
REM  its embedded ui\ to be compiled from the current sources.
powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $root=$env:ROOT; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\src\lib.rs')).LastWriteTime=Get-Date; (Get-Item -LiteralPath (Join-Path $root 'src-tauri\build.rs')).LastWriteTime=Get-Date"
if errorlevel 1 (
    echo [ERROR] Could not refresh source timestamps.
    goto :fail
)

if /i "%MODE%"=="portable" goto :do_portable
if /i "%MODE%"=="installer" goto :do_installer
if /i "%MODE%"=="all" goto :do_all

:do_portable
echo [1/1] Building portable exe, release profile ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
if not exist "%EXE%" (
    echo [ERROR] Build finished but the exe was not found:
    echo         %EXE%
    goto :fail
)
set "MADE_INSTALLER=0"
goto :report

:do_installer
echo [1/2] Building release binary ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
if not exist "%EXE%" (
    echo [ERROR] Build finished but the exe was not found:
    echo         %EXE%
    goto :fail
)
echo [2/2] Building NSIS installer ...
call %TAURI% build --bundles nsis
if errorlevel 1 goto :fail
set "MADE_INSTALLER=1"
goto :report

:do_all
echo [1/3] Building release binary ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
if not exist "%EXE%" (
    echo [ERROR] Build finished but the exe was not found:
    echo         %EXE%
    goto :fail
)
echo [2/3] Building NSIS installer ...
call %TAURI% build --bundles nsis
if errorlevel 1 goto :fail
echo [3/3] Copying outputs ...
set "MADE_INSTALLER=1"

:report
echo.
echo ============================================================
echo   BUILD SUCCEEDED
echo ============================================================
if defined EXE if exist "%EXE%" echo   Portable exe : %EXE%
if "%MADE_INSTALLER%"=="1" if exist "%CARGO_TARGET_DIR%\release\bundle\nsis" (
    for %%F in ("%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe") do echo   Installer    : %%F
)

REM --- Copy results into dist\ for convenience ---------------
set "DIST=%ROOT%\dist"
if not exist "%DIST%" mkdir "%DIST%"
if defined EXE if exist "%EXE%" (
    copy /y "%EXE%" "%DIST%\Hesabdari.exe" >nul
    if errorlevel 1 (
        echo [ERROR] Could not copy the portable exe to %DIST%.
        goto :fail
    )
    echo   Copied to    : %DIST%\Hesabdari.exe
)
if "%MADE_INSTALLER%"=="1" if exist "%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe" (
    copy /y "%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe" "%DIST%\" >nul
    if errorlevel 1 (
        echo [ERROR] Could not copy the installer to %DIST%.
        goto :fail
    )
    echo   Installer copied to : %DIST%\
)

echo.
pause
goto :eof

:do_clean
echo Removing build cache: %CARGO_TARGET_DIR%
if exist "%CARGO_TARGET_DIR%" rmdir /s /q "%CARGO_TARGET_DIR%"
echo Done.
goto :end

:fail
echo.
echo ============================================================
echo   BUILD FAILED  (see messages above)
echo ============================================================
echo.
pause
exit /b 1

:end
endlocal
