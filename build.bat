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

REM --- Sanity checks -----------------------------------------
where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust from https://rustup.rs
    echo         then reopen this window and try again.
    goto :fail
)

if /i "%MODE%"=="clean" (
    echo Removing build cache: %CARGO_TARGET_DIR%
    if exist "%CARGO_TARGET_DIR%" rmdir /s /q "%CARGO_TARGET_DIR%"
    echo Done.
    goto :end
)

REM --- Tauri CLI (installed as a cargo subcommand) -------------
set "TAURI=cargo tauri"
where cargo-tauri >nul 2>nul
if errorlevel 1 (
    echo [ERROR] Tauri CLI not found. Install it with:
    echo         cargo install tauri-cli --version "^2"
    goto :fail
)

if /i "%MODE%"=="portable" goto :do_portable
if /i "%MODE%"=="installer" goto :do_installer
goto :do_all

:do_portable
echo [1/1] Building portable exe, release profile ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
set "MADE_INSTALLER=0"
goto :report

:do_installer
echo [1/2] Building release binary ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
echo [2/2] Building NSIS installer ...
call %TAURI% build --bundles nsis
if errorlevel 1 goto :fail
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
set "MADE_INSTALLER=1"
goto :report

:do_all
echo [1/3] Building release binary ...
call %TAURI% build --no-bundle
if errorlevel 1 goto :fail
echo [2/3] Building NSIS installer ...
call %TAURI% build --bundles nsis
if errorlevel 1 goto :fail
echo [3/3] Copying outputs ...
set "EXE=%CARGO_TARGET_DIR%\release\Hesabdari.exe"
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
    echo   Copied to    : %DIST%\Hesabdari.exe
)
if "%MADE_INSTALLER%"=="1" if exist "%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe" (
    copy /y "%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe" "%DIST%\" >nul
    echo   Installer copied to : %DIST%\
)

echo.
pause
goto :eof

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
