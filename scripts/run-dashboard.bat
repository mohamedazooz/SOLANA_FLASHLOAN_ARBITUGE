@echo off
REM ============================================================
REM Solana Flash-Loan Arbitrage Dashboard Launcher (Windows)
REM ============================================================
REM This script starts the web dashboard for the arbitrage bot.
REM It can be run locally or on a Windows server.
REM ============================================================

cd /d "%~dp0\.."

chcp 65001 >nul
title Solana Flash-Loan Arbitrage Dashboard

echo.
echo ╔══════════════════════════════════════════════════════════════╗
echo ║  Solana Flash-Loan Arbitrage - Dashboard Launcher           ║
echo ║  Unified Control · V1 Transport · Kamino + Meteora DLMM     ║
echo ╚══════════════════════════════════════════════════════════════╝
echo.

REM Check if Node.js is installed
node --version >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Node.js is not installed or not in PATH.
    echo Please install Node.js 18+ from https://nodejs.org/
    pause
    exit /b 1
)

echo [INFO] Node.js version: 
node --version

REM Check if we're in the web directory
if not exist "web\package.json" (
    echo [ERROR] web\package.json not found.
    echo Please run this script from the project root directory.
    pause
    exit /b 1
)

cd web

REM Check if node_modules exists, if not install dependencies
if not exist "node_modules" (
    echo [INFO] Installing dependencies...
    npm install
    if %errorlevel% neq 0 (
        echo [ERROR] Failed to install dependencies.
        pause
        exit /b 1
    )
    echo [INFO] Dependencies installed successfully.
)

REM Build for production or start dev server
echo.
echo Select mode:
echo   1) Development server (with hot reload)
echo   2) Production build + preview
echo   3) Production build only
echo.
set /p choice="Enter choice [1-3]: "

if "%choice%"=="1" (
    echo [INFO] Starting development server...
    echo [INFO] Dashboard will be available at http://localhost:5173
    echo [INFO] Press Ctrl+C to stop
    echo.
    npm run dev
) else if "%choice%"=="2" (
    echo [INFO] Building for production...
    npm run build
    if %errorlevel% neq 0 (
        echo [ERROR] Build failed.
        pause
        exit /b 1
    )
    echo [INFO] Build successful. Starting preview server...
    echo [INFO] Dashboard will be available at http://localhost:4173
    echo [INFO] Press Ctrl+C to stop
    echo.
    npm run preview
) else if "%choice%"=="3" (
    echo [INFO] Building for production...
    npm run build
    if %errorlevel% neq 0 (
        echo [ERROR] Build failed.
        pause
        exit /b 1
    )
    echo [INFO] Build successful! Output in web/dist/
    echo [INFO] Deploy the 'dist' folder to your web server.
) else (
    echo [ERROR] Invalid choice.
    pause
    exit /b 1
)

cd ..
pause