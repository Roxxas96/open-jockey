@echo off
rem Starts Open Jockey with the DDJ-GRV6 mapping. Run from this folder.
cd /d "%~dp0"
open-jockey.exe --config config-grv6.toml
if errorlevel 1 pause
