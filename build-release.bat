@echo off
cd /d "%~dp0"
cargo build --release
if errorlevel 1 (
  echo Build failed.
  pause
  exit /b 1
)
echo.
echo Built: target\release\nexus-launcher.exe
pause
