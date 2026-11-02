"""
tVisio — Ponto de Entrada da Aplicação Desktop (Windows)
=============================================================================
Inicializa o Motor de Transmissão de Vídeo de Ultra-Baixa Latência (Zero-Overhead)
combinando WebRTC nativo (H.264), Fallback Binário WebSocket com QoS WMM (DSCP EF),
Driver Virtual Cam DirectShow para Windows, e abre automaticamente a janela desktop
nativa no modo sem bordas (--app) com interface Translucent Glass UI.

Desenvolvido por: Tiago Treter (tVisio Team)
Licença: GPLv3
=============================================================================
"""

import asyncio
import os
import shutil
import subprocess
import sys
import webbrowser
from pathlib import Path

from src.backend.server import run_servers

# Diretório raiz da aplicação
BASE_DIR = Path(__file__).resolve().parent


def launch_desktop_window(url: str) -> None:
    """
    Abre a janela nativa sem bordas (--app mode) no Windows utilizando o Microsoft Edge
    ou Google Chrome para renderização GPU acelerada com zero overhead.
    
    Se nenhuma das instâncias baseadas em Chromium for encontrada, recorre ao navegador padrão.
    Pode ser desativado definindo a variável de ambiente TVISIO_HEADLESS=1.
    """
    if os.environ.get("TVISIO_HEADLESS") == "1":
        return

    # Ordem de prioridade de executáveis Chromium para modo aplicativo nativo
    candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        shutil.which("msedge"),
        shutil.which("chrome"),
    ]

    for exe in candidates:
        if exe and Path(exe).exists():
            # Diretório de perfil isolado para não misturar com o navegador do usuário
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

    # Fallback caso não encontre Edge nem Chrome
    webbrowser.open(url)


async def main() -> None:
    """
    Função principal assíncrona:
    1. Lê portas HTTP e HTTPS das variáveis de ambiente (padrão: 8080 e 8443).
    2. Agenda a abertura da janela desktop após inicialização dos sockets.
    3. Inicializa os servidores HTTP/HTTPS e o loop de eventos.
    """
    http_port = int(os.environ.get("TVISIO_HTTP_PORT", "8080"))
    https_port = int(os.environ.get("TVISIO_HTTPS_PORT", "8443"))

    # Abre a janela desktop nativa logo após os servidores estarem prontos para aceitar conexões
    loop = asyncio.get_running_loop()
    loop.call_later(0.8, launch_desktop_window, f"http://127.0.0.1:{http_port}/desktop")

    # Inicia servidores aiohttp (HTTP para desktop local e HTTPS auto-assinado para PWA mobile)
    await run_servers(http_port=http_port, https_port=https_port)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        # Encerramento gracioso via Ctrl+C no terminal
        pass
