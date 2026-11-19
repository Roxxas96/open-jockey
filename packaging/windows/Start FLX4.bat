@echo off
rem Starts Open Jockey with the DDJ-FLX4 mapping. Run from this folder.
cd /d "%~dp0"
open-jockey.exe --config config-flx4.toml
if errorlevel 1 pause
