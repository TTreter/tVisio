"""
tVisio — Construtor Automatizado do Executável Windows (.exe)
=============================================================================
Empacota o tVisio em um executável nativo para Windows (`tVisio.exe`) sem janela
de terminal (`--noconsole`), com ícone em alta definição (`assets/icon.ico`),
interface Translucent Glass UI, Bandeja do Sistema (`pystray`) e binários do
driver DirectShow embutidos para instalação 100% offline.

Uso:
  python build_exe.py             # Gera build rápida em diretório (dist/tVisio/tVisio.exe)
  python build_exe.py --onefile   # Gera executável portátil único (dist/tVisio.exe)
=============================================================================
"""

import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent


def ensure_offline_driver_dlls() -> Path:
    """
    Garante que as DLLs de 64-bit e 32-bit do driver DirectShow (UnityCapture)
    estejam presentes em `_local/driver/` antes da compilação para que sejam
    embutidas dentro do executável (permitindo ativação 100% offline).
    """
    driver_dir = BASE_DIR / "_local" / "driver"
    driver_dir.mkdir(parents=True, exist_ok=True)
    dll64 = driver_dir / "UnityCaptureFilter64.dll"
    dll32 = driver_dir / "UnityCaptureFilter32.dll"

    base_url = "https://raw.githubusercontent.com/schellingb/UnityCapture/master/Install"
    if not dll64.exists():
        print("[1/3] Baixando UnityCaptureFilter64.dll para embutir no executável...")
        urllib.request.urlretrieve(f"{base_url}/UnityCaptureFilter64.dll", dll64)
    if not dll32.exists():
        print("[1/3] Baixando UnityCaptureFilter32.dll para embutir no executável...")
        urllib.request.urlretrieve(f"{base_url}/UnityCaptureFilter32.dll", dll32)

    return driver_dir


def build_executable(onefile: bool = False) -> None:
    """Executa o PyInstaller com todos os recursos estáticos e hooks nativos."""
    driver_dir = ensure_offline_driver_dlls()
    sep = ";" if os.name == "nt" else ":"

    mode_flag = "--onefile" if onefile else "--onedir"
    exe_name = "tVisio"

    print(f"[2/3] Iniciando compilação PyInstaller ({mode_flag})...")

    cmd = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--noconfirm",
        "--clean",
        mode_flag,
        "--noconsole",
        f"--name={exe_name}",
        f"--icon={BASE_DIR / 'assets' / 'icon.ico'}",
        f"--add-data={BASE_DIR / 'assets'}{sep}assets",
        f"--add-data={BASE_DIR / 'src' / 'desktop'}{sep}src/desktop",
        f"--add-data={BASE_DIR / 'src' / 'mobile'}{sep}src/mobile",
        f"--add-data={driver_dir}{sep}driver",
        "--collect-all=pyvirtualcam",
        "--hidden-import=pystray._win32",
        "--hidden-import=aiohttp",
        "--hidden-import=aiortc",
        "--hidden-import=av",
        "--hidden-import=ifaddr",
        "--hidden-import=qrcode",
        str(BASE_DIR / "main.py"),
    ]

    subprocess.run(cmd, check=True)

    # Remove arquivo .spec temporário gerado na raiz para manter o repositório limpo
    spec_file = BASE_DIR / f"{exe_name}.spec"
    if spec_file.exists():
        spec_file.unlink()

    if onefile:
        out_path = BASE_DIR / "dist" / f"{exe_name}.exe"
    else:
        out_path = BASE_DIR / "dist" / exe_name / f"{exe_name}.exe"

    print(f"[3/3] Executável compilado com sucesso em: {out_path}")


if __name__ == "__main__":
    use_onefile = "--onefile" in [a.lower() for a in sys.argv[1:]]
    build_executable(onefile=use_onefile)
