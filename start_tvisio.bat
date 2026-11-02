@echo off
title tVisio — Ultra-Low Latency Virtual Webcam
cd /d "%~dp0"
if exist "dist\tVisio\tVisio.exe" (
    start "" "dist\tVisio\tVisio.exe"
) else (
    python main.py
)
