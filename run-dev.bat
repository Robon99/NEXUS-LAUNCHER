@echo off
cd /d "%~dp0"
cargo run
if errorlevel 1 pause
