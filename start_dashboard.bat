@echo off
setlocal
cd /d "%~dp0"

REM ============================================================
REM  SOLANA_FLASHLOAN_ARBITUGE - start server + open dashboard
REM  Dashboard: http://127.0.0.1:8080  (login: WEB_ADMIN_PASSWORD)
REM ============================================================

where cargo >nul 2>&1
if errorlevel 1 (
    echo [ERROR] cargo not found in PATH. Install Rust: https://rustup.rs
    pause
    exit /b 1
)

REM If the bot is already running, just open the dashboard.
curl -s -o nul --max-time 2 http://127.0.0.1:8080/health
if not errorlevel 1 (
    echo [start] Server already running on http://127.0.0.1:8080
    start "" http://127.0.0.1:8080
    exit /b 0
)

REM Build if the release binary is missing.
if not exist "target\release\arb-bot.exe" (
    echo [start] Building arb-bot ^(release^) - first run may take several minutes...
    cargo build --release -p arb-bot
    if errorlevel 1 (
        echo [ERROR] Build failed.
        pause
        exit /b 1
    )
)

REM Dashboard login password: taken from WEB_ADMIN_PASSWORD or .env if set,
REM otherwise a random one is generated for this session and printed.
if not "%WEB_ADMIN_PASSWORD%"=="" goto :havepass
if exist ".env" (
    findstr /B /C:"WEB_ADMIN_PASSWORD=" .env >nul 2>&1
    if not errorlevel 1 goto :havepass
)
set "WEB_ADMIN_PASSWORD=local-%RANDOM%%RANDOM%%RANDOM%"
echo [start] Generated dashboard login password for this session: %WEB_ADMIN_PASSWORD%
echo         (set WEB_ADMIN_PASSWORD in .env to use a fixed one)
:havepass

echo [start] Starting arb-bot in a separate window...
start "arb-bot" cmd /k "set WEB_ADMIN_PASSWORD=%WEB_ADMIN_PASSWORD%&& target\release\arb-bot.exe"

REM Wait for the health endpoint, then open the dashboard.
set /a tries=0
:wait
timeout /t 2 /nobreak >nul
curl -s -o nul --max-time 2 http://127.0.0.1:8080/health
if errorlevel 1 (
    set /a tries+=1
    if %tries% geq 60 (
        echo [ERROR] Server did not become healthy within 120s. Check the arb-bot window for errors.
        pause
        exit /b 1
    )
    goto :wait
)

echo [start] Server healthy. Opening dashboard: http://127.0.0.1:8080
start "" http://127.0.0.1:8080
echo [start] Done. Close the arb-bot window to stop the server.
pause
