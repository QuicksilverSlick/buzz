@echo off
setlocal EnableDelayedExpansion
REM ============================================================================
REM dev-windows.bat -- Windows equivalent of `just dev`
REM ============================================================================
REM Usage: scripts\dev-windows.bat
REM
REM Starts the relay, the desktop Vite dev server, and the Tauri desktop app.
REM Assumes `just setup` has already been done once (see "One-time setup" below).
REM
REM Why this exists instead of `just dev`:
REM
REM   1. `just dev` prepends the repo's bin/ (Hermit shims) to PATH. Hermit is
REM      POSIX-only; on Windows those entries are checked out as plain text
REM      files containing the symlink target, so they are not executable.
REM      This script uses the natively installed toolchain instead.
REM
REM   2. `just dev`'s beforeDevCommand is Unix shell
REM      ("exec ./node_modules/.bin/vite ..."), which cmd.exe cannot run. Vite
REM      is started here as a separate process and Tauri is pointed at it.
REM
REM   3. rustc resolves a bare `link.exe` off PATH. Git for Windows ships
REM      /usr/bin/link.exe (GNU coreutils `link`), which shadows the MSVC
REM      linker and fails with "missing operand after '@...linker-arguments'".
REM      Calling vcvars64.bat puts the MSVC toolchain ahead of it.
REM      Do NOT set VCINSTALLDIR by hand to work around this: rustc then
REM      believes it is already inside a developer prompt, skips vswhere
REM      detection, and picks the Git linker again.
REM
REM Requirements:
REM   - Docker Desktop running. The compose services are `restart: unless-stopped`,
REM     so Postgres/Redis/MinIO/Adminer come back on their own once Docker is up;
REM     run `docker compose up -d` only if they are missing.
REM   - Visual Studio 2022 Build Tools with the C++ workload (MSVC v143).
REM     v142 (VS 2019) cannot link this project: the prebuilt ONNX Runtime that
REM     sherpa-onnx pulls in (huddle speech-to-text) references vectorized STL
REM     symbols (__std_find_trivial_*, __std_max_element_*, ...) that only exist
REM     in the v143 STL, producing ~41 unresolved externals.
REM   - .env must point the infra URLs at 127.0.0.1, not localhost. On hosts
REM     where localhost resolves to ::1 first, Docker's 127.0.0.1-only port
REM     bindings are unreachable and migrations fail with a pool timeout.
REM     RELAY_URL can stay localhost; seed-local-community.sh registers both.
REM
REM One-time setup (from Git Bash, once per clone):
REM   cp .env.example .env
REM   ./scripts/ensure-local-relay-key.sh .env
REM   # point DATABASE_URL / PGHOST / REDIS_URL / BUZZ_S3_ENDPOINT at 127.0.0.1
REM   docker compose up -d postgres redis minio minio-init adminer
REM   cargo run -p buzz-admin -- migrate
REM   ./scripts/seed-local-community.sh
REM   pnpm install
REM   cargo build -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
REM   # then copy those into desktop/src-tauri/binaries/<name>-x86_64-pc-windows-msvc.exe
REM ============================================================================

set "REPO_ROOT=%~dp0.."
pushd "%REPO_ROOT%" || exit /b 1

set "VITE_PORT=43137"
set "RELAY_URL_WS=ws://localhost:3000"

REM ---- MSVC environment (must precede any cargo invocation) ------------------

REM NOTE: paths here contain "(x86)". Never echo them inside a parenthesized
REM block -- the ")" closes the block and cmd fails with
REM "\Microsoft was unexpected at this time." Hence the goto-based branching.

set "VSDEVCMD=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
if exist "%VSDEVCMD%" goto :have_vs
echo [dev-windows] ERROR: VS 2022 Build Tools ^(MSVC v143^) not found at:
echo                "%VSDEVCMD%"
echo.
echo                Install the C++ workload with:
echo                  winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
popd
exit /b 1

:have_vs
call "%VSDEVCMD%" >nul
if errorlevel 1 goto :vs_failed
goto :vs_ok

:vs_failed
echo [dev-windows] ERROR: vcvars64.bat failed.
popd
exit /b 1

:vs_ok

REM CMake is needed by the audiopus_sys build script (Opus codec, huddle audio).
REM VS 2022 ships one; only prepend if cmake is not already resolvable.
where /q cmake.exe || set "PATH=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;%PATH%"

REM ---- Preflight -------------------------------------------------------------

if exist ".env" goto :have_env
echo [dev-windows] ERROR: .env not found. See "One-time setup" in this script.
popd
exit /b 1

:have_env
docker info >nul 2>&1
if errorlevel 1 goto :no_docker
goto :have_docker

:no_docker
echo [dev-windows] ERROR: Docker daemon not reachable. Start Docker Desktop and retry.
popd
exit /b 1

:have_docker

REM ---- Relay -----------------------------------------------------------------
REM Load .env into this process (the relay inherits it), mirroring the
REM `set -o allexport; source .env` that the just recipes do. `eol=#` skips
REM comment lines; `tokens=1,* delims==` splits on the first `=` only, so
REM values containing `=` survive intact.

echo [dev-windows] Loading .env ...
for /f "usebackq eol=# tokens=1,* delims==" %%a in ("%REPO_ROOT%\.env") do (
    if not "%%~a"=="" set "%%~a=%%~b"
)

if exist "target\debug\buzz-relay.exe" goto :have_relay
echo [dev-windows] ERROR: target\debug\buzz-relay.exe not found. Build it first:
echo                  cargo build -p buzz-relay
popd
exit /b 1

:have_relay

echo [dev-windows] Starting relay on %RELAY_URL_WS% ...
start "buzz-relay" cmd /c "cd /d "%REPO_ROOT%" && target\debug\buzz-relay.exe"

REM ---- Vite ------------------------------------------------------------------

echo [dev-windows] Starting Vite dev server on http://localhost:%VITE_PORT% ...
start "buzz-vite" cmd /c "cd /d "%REPO_ROOT%\desktop" && pnpm exec vite --port %VITE_PORT% --strictPort"

REM ---- Desktop app -----------------------------------------------------------
REM Wait for the relay's readiness probe before launching, mirroring `just dev`,
REM so the app does not come up against a relay that is still migrating.

echo [dev-windows] Waiting for relay readiness ...
set "RELAY_READY="
for /l %%i in (1,1,120) do (
    if not defined RELAY_READY (
        curl --silent --fail --max-time 1 http://127.0.0.1:8080/_readiness >nul 2>&1 && set "RELAY_READY=1"
        if not defined RELAY_READY ping -n 2 127.0.0.1 >nul
    )
)
if defined RELAY_READY goto :relay_ready
echo [dev-windows] ERROR: relay did not become ready within 60s; refusing to launch desktop.
echo                Check the "buzz-relay" window for the failure.
popd
exit /b 1

:relay_ready
echo [dev-windows] Relay ready.

set "TAURI_CONFIG=%TEMP%\buzz-dev-config.json"
> "%TAURI_CONFIG%" echo {"build":{"devUrl":"http://localhost:%VITE_PORT%","beforeDevCommand":""},"identifier":"xyz.block.buzz.app.dev","productName":"Buzz Dev"}

set "BUZZ_RELAY_URL=%RELAY_URL_WS%"
set "BUZZ_DEV_KEYRING_SERVICE=buzz-desktop-dev.main"

cd /d "%REPO_ROOT%\desktop"
echo [dev-windows] Launching desktop app ^(relay %BUZZ_RELAY_URL%, vite %VITE_PORT%^) ...
call pnpm exec tauri dev --config "%TAURI_CONFIG%"
set "EXIT_CODE=%ERRORLEVEL%"

popd
exit /b %EXIT_CODE%
