@echo off
title tVisio Launcher (Rust Native Engine)
cd /d "%~dp0"

if exist "target\release\tVisio.exe" (
    start "" "target\release\tVisio.exe"
    exit /b 0
)

echo [tVisio] Compilando executavel nativo otimizado em Rust (primeira execucao)...
cargo build --release
if exist "target\release\tVisio.exe" (
    start "" "target\release\tVisio.exe"
) else (
    echo [Erro] Certifique-se de ter o Rust instalado (https://rustup.rs) para compilar o tVisio.
    pause
)
