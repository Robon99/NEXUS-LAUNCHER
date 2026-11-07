@echo off
cd /d "%~dp0"
echo Checking NEXUS Launcher project...
cargo check --all-targets
if errorlevel 1 (
  echo.
  echo CHECK FAILED. Copy the full error output when reporting a problem.
  pause
  exit /b 1
)
echo.
echo CHECK PASSED. The Rust targets compile successfully.
pause
