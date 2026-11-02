"""
tVisio — Ponto de Entrada da Aplicação Desktop e Executável Windows (.exe)
=============================================================================
Inicializa o Motor de Transmissão de Vídeo de Ultra-Baixa Latência (Zero-Overhead)
combinando WebRTC nativo (H.264), Fallback Binário WebSocket com QoS WMM (DSCP EF),
Driver Virtual Cam DirectShow para Windows, Bandeja do Sistema (System Tray),
Mutex de Instância Única e interface Translucent Glass UI.

Desenvolvido por: Tiago Treter (tVisio Team)
Licença: GPLv3
=============================================================================
"""

import asyncio
import logging
from logging.handlers import RotatingFileHandler
import os
import sys
from pathlib import Path

from src.backend.settings_manager import get_data_dir
from src.backend.win_integration import check_single_instance, DesktopWindowManager
from src.backend.server import run_servers

# Diretório gravável de dados locais (_local)
BASE_DIR = get_data_dir()


def setup_runtime_logging() -> None:
    """
    Configura o sistema de logs. Quando empacotado como executável sem console (`--noconsole`),
    `sys.stdout` e `sys.stderr` são `None` no Windows — redirecionamos com segurança para
    `_local/tvisio.log` com rotação automática (máx. 1 MB).
    """
    log_dir = BASE_DIR / "_local"
    log_dir.mkdir(parents=True, exist_ok=True)
    log_file = log_dir / "tvisio.log"

    handlers: list[logging.Handler] = []
    try:
        file_handler = RotatingFileHandler(
            log_file, maxBytes=1_000_000, backupCount=1, encoding="utf-8"
        )
        handlers.append(file_handler)
    except Exception:
        pass

    if sys.stdout is not None and hasattr(sys.stdout, "write"):
        handlers.append(logging.StreamHandler(sys.stdout))
    else:
        # Evita falhas de bibliotecas que tentam escrever diretamente em sys.stdout/stderr
        devnull = open(os.devnull, "w", encoding="utf-8")
        sys.stdout = devnull
        sys.stderr = devnull

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        handlers=handlers,
        force=True,
    )


def launch_desktop_window(url: str) -> None:
    """Função auxiliar de compatibilidade para abrir a janela nativa `--app`."""
    mgr = DesktopWindowManager(desktop_url=url, data_dir=BASE_DIR)
    mgr.show_or_launch()


async def main() -> None:
    """
    Função principal assíncrona:
    1. Configura logs seguros para modo console e modo executável `--noconsole`.
    2. Verifica Mutex de instância única (se já estiver aberto na bandeja, foca a janela existente).
    3. Verifica se foi iniciado automaticamente com o Windows (`--startup` / `--minimized`).
    4. Inicia os servidores HTTP/HTTPS, Bandeja do Sistema e ciclo de vida da janela.
    """
    setup_runtime_logging()

    http_port = int(os.environ.get("TVISIO_HTTP_PORT", "8080"))
    https_port = int(os.environ.get("TVISIO_HTTPS_PORT", "8443"))

    # Garante instância única do aplicativo
    if not check_single_instance(http_port=http_port):
        sys.exit(0)

    # Detecta se foi lançado pelo Registro do Windows (--startup) ou flag --minimized
    args = [a.lower() for a in sys.argv[1:]]
    start_minimized_flag = "--startup" in args or "--minimized" in args

    await run_servers(
        http_port=http_port,
        https_port=https_port,
        start_minimized_override=start_minimized_flag,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        # Encerramento gracioso via Ctrl+C no terminal
        pass
