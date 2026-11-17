@echo off
title Solana Arbitrage Bot - Live VPS Terminal Monitor
color 0A
echo =====================================================================
echo  Solana Flash Loan Arbitrage Bot - VPS Live Monitor
echo  Host: 102.206.132.212 (User: machsol)
echo  Command: sudo journalctl -u arb-bot -f
echo =====================================================================
echo.
ssh -t -o StrictHostKeyChecking=accept-new machsol@102.206.132.212 "sudo journalctl -u arb-bot -f"
echo.
echo =====================================================================
echo  Session ended. Press any key to exit...
echo =====================================================================
pause >nul
