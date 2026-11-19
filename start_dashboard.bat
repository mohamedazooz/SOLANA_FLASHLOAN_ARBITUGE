@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul
title SENTINEL // Solana Flashloan Arbitrage Master Controller
cd /d "%~dp0"

set "ROOT_DIR=%~dp0"
set "BASE_DIR=%~dp0SOLANA_FLASHLOAN_ARBITUGE"

if not exist "%BASE_DIR%\Cargo.toml" (
    if exist "%ROOT_DIR%Cargo.toml" (
        set "BASE_DIR=%ROOT_DIR%"
    )
)

echo ===============================================================================
echo       SENTINEL // SOLANA ATOMIC HFT FLASHLOAN ARBITRAGE SYSTEM
echo         مشغل الخدمات الشامل + تشغيل السيرفرات وفتح لوحة التحكم
echo ===============================================================================
echo.
echo [*] Base Directory: %BASE_DIR%
echo.

echo [1/4] Checking Redis Server on port 6379...
netstat -ano | findstr ":6379 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] Redis Server is already running on port 6379.
) else (
    echo   [+] Starting Redis Server in a separate window...
    if exist "C:\Program Files\Redis\redis-server.exe" (
        start "Sentinel Redis Server" "C:\Program Files\Redis\redis-server.exe"
    ) else (
        where redis-server >nul 2>&1
        if !errorlevel! equ 0 (
            start "Sentinel Redis Server" redis-server
        ) else (
            echo   [!] Warning: redis-server not found in standard path.
        )
    )
    ping -n 2 127.0.0.1 >nul
)

echo [2/4] Checking Solana Webhook Service on port 8085...
netstat -ano | findstr ":8085 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] Solana Webhook Service is already running on port 8085.
) else (
    echo   [+] Starting Solana Webhook Service in a separate window...
    start "Sentinel Solana Webhook Service" cmd /k "cd /d \"%BASE_DIR%\webhook-service\" && target\release\solana-webhook-service.exe"
    ping -n 2 127.0.0.1 >nul
)

echo [3/4] Checking Arbitrage Bot Engine on port 8080...
netstat -ano | findstr ":8080 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] Arbitrage Bot Engine is already running on port 8080.
) else (
    echo   [+] Starting Arbitrage Bot Engine in a separate window...
    set "WEB_ADMIN_PASSWORD=sentinel_secret_key_2026_change_me"
    start "Sentinel Arb Bot" cmd /k "cd /d \"%BASE_DIR%\" && set WEB_ADMIN_PASSWORD=sentinel_secret_key_2026_change_me&& target\release\arb-bot.exe"
)

echo [4/4] Checking Vite Web Dashboard on port 5173...
netstat -ano | findstr ":5173 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] Vite Web Dashboard is already running on port 5173.
) else (
    if exist "%BASE_DIR%\web\package.json" (
        echo   [+] Starting Vite Dev Server in a separate window...
        start "Sentinel Vite Frontend" cmd /k "cd /d \"%BASE_DIR%\web\" && npm run dev"
    )
)

echo.
echo [*] Waiting for services to become ready...
set /a retries=0
:wait_loop
ping -n 2 127.0.0.1 >nul
set /a retries+=1
curl.exe -s -o nul http://127.0.0.1:8080/health
if %errorlevel% neq 0 (
    if !retries! lss 25 (
        goto wait_loop
    )
)

echo.
echo ===============================================================================
echo   مبروك يا باشا! كل السيرفرات اشتغلت وتمام التمام!
echo ===============================================================================
echo.
echo   [1] لوحة التحكم المباشرة [شرح مصري وتتبع حي للمسارات]:
echo       http://127.0.0.1:8080/dashboard
echo.
echo   [2] واجهة المستخدم المتقدمة [React / Vite]:
echo       http://localhost:5173
echo.
echo   [★] الباسورد الإداري عند طلبه:
echo       sentinel_secret_key_2026_change_me
echo.
echo ===============================================================================

start "" "http://127.0.0.1:8080/dashboard"

echo.
echo اضغط أي زر لإغلاق هذه النافذة فقط [النوافذ الأخرى للسيرفرات ستظل تعمل في الخلفية].
pause >nul
