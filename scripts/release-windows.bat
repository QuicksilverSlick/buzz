@echo off
setlocal
REM ============================================================================
REM release-windows.bat <version> -- build and publish a Dreamforge Windows release
REM ============================================================================
REM One command turns a checkout into an auto-updating release:
REM   1. desktop\scripts\build-release-config.mjs writes src-tauri\tauri.release.conf.json
REM      (updater public key + endpoint, createUpdaterArtifacts). <version> is patched
REM      into that delta, so no tracked file changes.
REM   2. scripts\build-windows-installer.bat builds the sidecars and the NSIS installer;
REM      Tauri signs the .exe with the minisign key at %USERPROFILE%\.tauri\dreamforge.key.
REM      Create it once and keep it -- it is the root of trust for every future update:
REM        cd desktop && pnpm tauri signer generate --ci -w "%USERPROFILE%\.tauri\dreamforge.key"
REM   3. scripts\release-latest-json.py writes latest.json and refuses a version that is
REM      not newer than the one already published.
REM   4. gh uploads the .exe, its .sig and latest.json to the rolling GitHub release
REM      "dreamforge-desktop-latest" on QuicksilverSlick/buzz. Installed apps poll that
REM      latest.json and offer "Restart to update".
REM
REM Versions are plain x.y.z and only go up: the app updates only when the feed is
REM newer than what is installed. CARGO_TARGET_DIR is honoured when already set.
REM Same "(x86)" parenthesis trap as build-windows-installer.bat: branch with goto.
REM ============================================================================

if "%~1"=="" goto :usage
set "VERSION=%~1"
set "REPO_SLUG=QuicksilverSlick/buzz"
set "TAG=dreamforge-desktop-latest"
set "KEY=%USERPROFILE%\.tauri\dreamforge.key"
set "REPO_ROOT=%~dp0.."
set "BUZZ_UPDATER_ENDPOINT=https://github.com/%REPO_SLUG%/releases/download/%TAG%/latest.json"
set "EXE_NAME=Dreamforge_%VERSION%_x64-setup.exe"
set "EXE_URL=https://github.com/%REPO_SLUG%/releases/download/%TAG%/%EXE_NAME%"
if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%REPO_ROOT%\desktop\src-tauri\target"
set "BUNDLE=%CARGO_TARGET_DIR%\release\bundle\nsis"
set "EXE=%BUNDLE%\%EXE_NAME%"

if not exist "%KEY%.pub" goto :no_key
set /p BUZZ_UPDATER_PUBLIC_KEY=<"%KEY%.pub"
set "TAURI_SIGNING_PRIVATE_KEY=%KEY%"
set "TAURI_BUILD_ARGS=--ci --bundles nsis --config src-tauri/tauri.release.conf.json"

echo [release] %VERSION% against the published feed ...
python "%REPO_ROOT%\scripts\release-latest-json.py" --check "%VERSION%" "%BUZZ_UPDATER_ENDPOINT%"
if errorlevel 1 exit /b 1

pushd "%REPO_ROOT%\desktop" || exit /b 1
node scripts\build-release-config.mjs
if errorlevel 1 goto :config_failed
python -c "import json,sys; p='src-tauri/tauri.release.conf.json'; d=json.load(open(p)); d['version']=sys.argv[1]; json.dump(d, open(p,'w'), indent=2)" "%VERSION%"
if errorlevel 1 goto :config_failed
popd

call "%REPO_ROOT%\scripts\build-windows-installer.bat"
if errorlevel 1 exit /b 1
if not exist "%EXE%" goto :no_exe
if not exist "%EXE%.sig" goto :no_sig

for /f %%s in ('git -C "%REPO_ROOT%" rev-parse --short HEAD') do set "SHA=%%s"
python "%REPO_ROOT%\scripts\release-latest-json.py" "%VERSION%" "%EXE%.sig" "%EXE_URL%" "Dreamforge %VERSION% (%SHA%)" > "%BUNDLE%\latest.json"
if errorlevel 1 exit /b 1

gh release view "%TAG%" --repo "%REPO_SLUG%" >nul 2>&1
if errorlevel 1 gh release create "%TAG%" --repo "%REPO_SLUG%" --title "Dreamforge desktop (rolling)" --notes "Windows installer and the latest.json the in-app updater reads. scripts\release-windows.bat replaces latest.json and adds each new installer here."
if errorlevel 1 exit /b 1
gh release upload "%TAG%" "%EXE%" "%EXE%.sig" "%BUNDLE%\latest.json" --clobber --repo "%REPO_SLUG%"
if errorlevel 1 exit /b 1
echo [release] published %VERSION% ^(%SHA%^): https://github.com/%REPO_SLUG%/releases/tag/%TAG%
exit /b 0

:usage
echo usage: scripts\release-windows.bat ^<version^>   e.g. 0.5.21
exit /b 2
:no_key
echo [release] ERROR: no updater key at "%KEY%". Create it once:
echo   cd desktop ^&^& pnpm tauri signer generate --ci -w "%KEY%"
exit /b 1
:config_failed
popd
echo [release] ERROR: could not write src-tauri\tauri.release.conf.json
exit /b 1
:no_exe
echo [release] ERROR: installer not found at "%EXE%"
exit /b 1
:no_sig
echo [release] ERROR: no .sig next to the installer. Was TAURI_SIGNING_PRIVATE_KEY honoured?
exit /b 1
