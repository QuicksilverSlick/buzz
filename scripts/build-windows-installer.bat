@echo off
setlocal
REM ============================================================================
REM build-windows-installer.bat -- produce an installable Buzz.exe for Windows
REM ============================================================================
REM Usage: scripts\build-windows-installer.bat
REM
REM Builds the release sidecars, copies them where Tauri expects, then runs
REM `pnpm tauri build` to produce an NSIS installer (and MSI where WiX is
REM available). Output lands in:
REM   desktop\src-tauri\target\release\bundle\nsis\Buzz_<version>_x64-setup.exe
REM
REM Takes considerably longer than a debug build (full optimization across the
REM whole dependency graph). Unattended once started.
REM
REM See scripts\dev-windows.bat for why vcvars64 is required (Git's
REM /usr/bin/link.exe otherwise shadows the MSVC linker) and for the VS 2022
REM v143 requirement. The same "(x86)" parenthesis trap applies here, so this
REM script also branches with goto rather than parenthesized if-blocks.
REM
REM Note: bundling reads tauri.conf.json merged with tauri.windows.conf.json.
REM The latter drops the buzz-backend-kubernetes sidecar, which is not built on
REM Windows -- so only the five sidecars below are needed.
REM ============================================================================

set "REPO_ROOT=%~dp0.."
pushd "%REPO_ROOT%" || exit /b 1

set "TARGET=x86_64-pc-windows-msvc"

set "VSDEVCMD=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
if exist "%VSDEVCMD%" goto :have_vs
echo [build-installer] ERROR: VS 2022 Build Tools not found at:
echo                   "%VSDEVCMD%"
popd
exit /b 1

:have_vs
call "%VSDEVCMD%" >nul
if errorlevel 1 goto :vs_failed
goto :vs_ok

:vs_failed
echo [build-installer] ERROR: vcvars64.bat failed.
popd
exit /b 1

:vs_ok
where /q cmake.exe || set "PATH=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;%PATH%"

REM ---- Release sidecars ------------------------------------------------------

echo [build-installer] Building release sidecars ...
cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
if errorlevel 1 goto :sidecar_failed
goto :sidecars_ok

:sidecar_failed
echo [build-installer] ERROR: sidecar build failed.
popd
exit /b 1

:sidecars_ok
if not exist "desktop\src-tauri\binaries" mkdir "desktop\src-tauri\binaries"
for %%b in (buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz) do (
    copy /y "target\release\%%b.exe" "desktop\src-tauri\binaries\%%b-%TARGET%.exe" >nul
    if errorlevel 1 echo [build-installer] WARNING: could not copy %%b.exe
)

REM ---- Bundle ----------------------------------------------------------------

cd /d "%REPO_ROOT%\desktop"
echo [build-installer] Running tauri build ^(this is the long part^) ...
call pnpm tauri build
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" goto :done
echo.
echo [build-installer] Installer(s) written to:
dir /b /s "%REPO_ROOT%\desktop\src-tauri\target\release\bundle\*.exe" 2>nul
dir /b /s "%REPO_ROOT%\desktop\src-tauri\target\release\bundle\*.msi" 2>nul

:done
popd
exit /b %EXIT_CODE%
