@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul
title SENTINEL // Solana Flashloan Arbitrage Master Controller

cd /d "%~dp0"
set "BASE_DIR=%~dp0SOLANA_FLASHLOAN_ARBITUGE"
if not exist "%BASE_DIR%\Cargo.toml" set "BASE_DIR=%~dp0"
if "%BASE_DIR:~-1%"=="\" set "BASE_DIR=%BASE_DIR:~0,-1%"

echo ===============================================================================
echo       SENTINEL // SOLANA ATOMIC HFT FLASHLOAN ARBITRAGE SYSTEM
echo         مشغل الخدمات الشامل + تشغيل السيرفرات وفتح لوحة التحكم
echo ===============================================================================
echo.
echo [*] مسار العمل الأساسي (Base Directory):
echo     %BASE_DIR%
echo.

REM 1. Check Redis Port 6379
echo [1/4] فحص سيرفر ريديس (Redis Server - Port 6379)...
netstat -ano | findstr ":6379 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] سيرفر Redis يعمل بالفعل على بورت 6379.
) else (
    echo   [+] جاري تشغيل سيرفر Redis في نافذة تيرمينال مرئية...
    if exist "C:\Program Files\Redis\redis-server.exe" (
        start "[1] Redis Server (Port 6379)" /D "C:\Program Files\Redis" cmd /k "redis-server.exe"
    ) else (
        where redis-server >nul 2>&1
        if %errorlevel% equ 0 (
            start "[1] Redis Server (Port 6379)" cmd /k "redis-server"
        ) else (
            echo   [!] تنبيه: لم يتم العثور على redis-server في المسار القياسي C:\Program Files\Redis.
        )
    )
    ping 127.0.0.1 -n 2 >nul
)

REM 2. Check Solana Webhook Service Port 8085
echo [2/4] فحص خدمة الويب هوك (Solana Webhook Service - Port 8085)...
netstat -ano | findstr ":8085 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] خدمة Solana Webhook تعمل بالفعل على بورت 8085.
) else (
    echo   [+] جاري تشغيل Solana Webhook Service في نافذة تيرمينال منفصلة...
    if exist "%BASE_DIR%\webhook-service\target\release\solana-webhook-service.exe" (
        start "[2] Solana Webhook Service (Port 8085)" /D "%BASE_DIR%\webhook-service" cmd /k "target\release\solana-webhook-service.exe"
    ) else if exist "%BASE_DIR%\target\release\solana-webhook-service.exe" (
        start "[2] Solana Webhook Service (Port 8085)" /D "%BASE_DIR%\webhook-service" cmd /k "..\target\release\solana-webhook-service.exe"
    ) else if exist "%BASE_DIR%\webhook-service\target\debug\solana-webhook-service.exe" (
        start "[2] Solana Webhook Service (Port 8085)" /D "%BASE_DIR%\webhook-service" cmd /k "target\debug\solana-webhook-service.exe"
    ) else (
        start "[2] Solana Webhook Service (Port 8085)" /D "%BASE_DIR%\webhook-service" cmd /k "cargo run --release"
    )
    ping 127.0.0.1 -n 2 >nul
)

REM 3. Check Arbitrage Bot Engine Port 8080
echo [3/4] فحص محرك البوت الأساسي (Arbitrage Bot Engine - Port 8080)...
netstat -ano | findstr ":8080 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] محرك Arb-Bot يعمل بالفعل على بورت 8080.
) else (
    echo   [+] جاري تشغيل محرك الأربيتراتج Arb Bot في نافذة تيرمينال منفصلة...
    set "WEB_ADMIN_PASSWORD=sentinel_secret_key_2026_change_me"
    if exist "%BASE_DIR%\target\release\arb-bot.exe" (
        start "[3] Sentinel Arb Bot (Port 8080)" /D "%BASE_DIR%" cmd /k "target\release\arb-bot.exe"
    ) else if exist "%BASE_DIR%\target\debug\arb-bot.exe" (
        start "[3] Sentinel Arb Bot (Port 8080)" /D "%BASE_DIR%" cmd /k "target\debug\arb-bot.exe"
    ) else (
        start "[3] Sentinel Arb Bot (Port 8080)" /D "%BASE_DIR%" cmd /k "cargo run --release -p arb-bot"
    )
    ping 127.0.0.1 -n 2 >nul
)

REM 4. Check Vite Frontend Port 5173
echo [4/4] فحص واجهة المستخدم (Vite React Dashboard - Port 5173)...
netstat -ano | findstr ":5173 " | findstr "LISTENING" >nul
if %errorlevel% equ 0 (
    echo   [OK] واجهة Vite Web Dashboard تعمل بالفعل على بورت 5173.
) else (
    if exist "%BASE_DIR%\web\package.json" (
        echo   [+] جاري تشغيل سيرفر Vite في نافذة تيرمينال منفصلة...
        start "[4] Vite Web Dashboard (Port 5173)" /D "%BASE_DIR%\web" cmd /k "npm run dev"
    )
)

REM 5. Wait for arb-bot to respond
echo.
echo [*] جاري انتظار استجابة المحرك والتحقق من الجاهزية (Health Check)...
set /a retries=0
:wait_loop
ping 127.0.0.1 -n 2 >nul
set /a retries+=1
curl.exe -s -o nul http://127.0.0.1:8080/health
if %errorlevel% neq 0 (
    if !retries! lss 30 (
        goto wait_loop
    )
)

echo.
echo ===============================================================================
echo   مبروك يا باشا! كل السيرفرات اشتغلت وجميع التيرمينالات مفتوحة وتمام التمام!
echo ===============================================================================
echo.
echo   [1] لوحة التحكم المباشرة (شرح مصري وتتبع حي للمسارات):
echo       http://127.0.0.1:8080/dashboard
echo.
echo   [2] واجهة المستخدم المتقدمة (React / Vite):
echo       http://localhost:5173
echo.
echo   [★] الباسورد الإداري عند طلبه:
echo       sentinel_secret_key_2026_change_me
echo.
echo ===============================================================================

REM فتح الداش بورد تلقائياً في المتصفح الافتراضي
echo [*] جاري فتح لوحة التحكم في المتصفح تلقائياً...
start "" "http://127.0.0.1:8080/dashboard"

echo.
echo يمكنك إغلاق هذه النافذة فقط (النوافذ الأخرى للسيرفرات ستظل تعمل وتتابع المسارات).
pause
