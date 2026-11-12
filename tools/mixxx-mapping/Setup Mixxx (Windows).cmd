@echo off
rem Runs the set-up script next to this file. Double-click to start.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Setup-Mixxx-Windows.ps1" %*
echo.
pause
