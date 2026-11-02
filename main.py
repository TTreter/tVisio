"""
tVisio — Desktop Application Entrypoint
Starts the Ultra-Low Latency Engine (WebRTC + Binary Turbo WS + 500Hz Anti-PSM + Virtual Cam)
and opens the native chromeless Liquid Glass Desktop App window.
"""

import asyncio
import os
import shutil
import subprocess
import sys
import webbrowser
from pathlib import Path

from src.backend.server import run_servers

BASE_DIR = Path(__file__).resolve().parent


def launch_desktop_window(url: str) -> None:
    """
    Launches a native chromeless Desktop App window (--app mode) on Windows
    using Edge or Chrome for zero-overhead native GPU rendering.
    """
    if os.environ.get("TVISIO_HEADLESS") == "1":
        return

    candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        shutil.which("msedge"),
        shutil.which("chrome"),
    ]

    for exe in candidates:
        if exe and Path(exe).exists():
            user_data_dir = BASE_DIR / "_local" / "app_profile"
            user_data_dir.mkdir(parents=True, exist_ok=True)
            subprocess.Popen(
                [
                    str(exe),
                    f"--app={url}",
                    "--window-size=1180,760",
                    f"--user-data-dir={user_data_dir}",
                    "--ignore-certificate-errors",
                    "--disable-features=Translate",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return

    webbrowser.open(url)


async def main() -> None:
    http_port = int(os.environ.get("TVISIO_HTTP_PORT", "8080"))
    https_port = int(os.environ.get("TVISIO_HTTPS_PORT", "8443"))

    # Launch native desktop window after server binds
    loop = asyncio.get_running_loop()
    loop.call_later(0.8, launch_desktop_window, f"http://127.0.0.1:{http_port}/desktop")

    await run_servers(http_port=http_port, https_port=https_port)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
