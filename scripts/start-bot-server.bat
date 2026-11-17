@echo off
REM ============================================================
REM Solana Flash-Loan Arbitrage Bot Server Launcher (Windows)
REM ============================================================
REM This script starts the Rust arbitrage bot on Windows.
REM For production, use the Linux VPS with systemd service.
REM ============================================================

cd /d "%~dp0\.."

chcp 65001 >nul
title Solana Flash-Loan Arbitrage Bot Server

echo.
echo ╔══════════════════════════════════════════════════════════════╗
echo ║  Solana Flash-Loan Arbitrage - Bot Server                   ║
echo ║  V1 Transport · Kamino Flash Loans · Meteora DLMM           ║
echo ╚══════════════════════════════════════════════════════════════╝
echo.

REM Check if Rust is installed
cargo --version >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Rust is not installed or not in PATH.
    echo Please install Rust from https://rustup.rs/
    pause
    exit /b 1
)

echo [INFO] Rust version:
cargo --version

REM Check if we're in the project root
if not exist "Cargo.toml" (
    echo [ERROR] Cargo.toml not found.
    echo Please run this script from the project root directory.
    pause
    exit /b 1
)

REM Check for .env file
if not exist ".env" (
    echo [WARN] .env file not found. Copying from .env.example...
    if exist ".env.example" (
        copy .env.example .env
        echo [INFO] Created .env from .env.example
        echo [WARN] Please edit .env with your configuration before running!
        pause
    ) else (
        echo [ERROR] .env.example not found.
        pause
        exit /b 1
    )
)

REM Build the bot
echo.
echo [INFO] Building bot in release mode...
cargo build --release --bin arb-bot
if %errorlevel% neq 0 (
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

echo [INFO] Build successful!

REM Check for required environment variables
echo.
echo [INFO] Checking configuration...
set ARB_LIVE=0
set DRY_RUN=true
set RUST_LOG=info

REM Start the bot
echo.
echo [INFO] Starting bot in SHADOW mode (safe for testing)...
echo [INFO] Set ARB_LIVE=1 in .env for live mode (REAL FUNDS!)
echo [INFO] Press Ctrl+C to stop
echo.

cargo run --release --bin arb-bot

echo.
echo [INFO] Bot stopped.
pause