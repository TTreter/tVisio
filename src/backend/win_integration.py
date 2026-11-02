"""
tVisio — Integração Nativa com o Sistema Operacional Windows
=============================================================================================
Gerencia todos os recursos nativos de aplicativo desktop no Windows:
1. Mutex de Instância Única (`CreateMutexW`) com ativação da janela principal via IPC HTTP.
2. Inicialização automática com o Windows via Registro (`HKCU\\...\\CurrentVersion\\Run`).
3. Bandeja do Sistema (`System Tray` via `pystray`) com menu de contexto dinâmico em português.
4. Monitoramento inteligente da janela nativa (`HWND` via `user32.dll`) para:
   - Minimizar para a bandeja (ocultando da barra de tarefas com `SW_HIDE`).
   - Fechar (`X`) para a bandeja (liberando 100% da memória gráfica do navegador e mantendo
     o servidor WebRTC + Virtual Camera rodando em segundo plano).
   - Restaurar ou relançar a janela ao clicar no ícone da bandeja.
5. Liberação automática de portas no Firewall do Windows (`netsh advfirewall` via UAC).
=============================================================================================
"""

import asyncio
import ctypes
import logging
import os
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING, Optional

from PIL import Image

if TYPE_CHECKING:
    from src.backend.server import TVisioServer

logger = logging.getLogger("tVisio.WinIntegration")

# Referência global do Mutex para evitar coleta pelo Garbage Collector durante a execução
_MUTEX_HANDLE: Optional[int] = None

# Constantes da API Win32 (user32.dll)
SW_HIDE = 0
SW_SHOWNORMAL = 1
SW_SHOW = 5
SW_RESTORE = 9
WM_CLOSE = 0x0010
ERROR_ALREADY_EXISTS = 183

WINDOW_TITLE = "tVisio — Ultra-Low Latency Virtual Webcam"
RUN_REG_PATH = r"Software\Microsoft\Windows\CurrentVersion\Run"
RUN_APP_NAME = "tVisio"


def check_single_instance(http_port: int = 8080) -> bool:
    """
    Garante que apenas uma instância do tVisio rode por vez no Windows.
    Se já houver outra instância ativa (mesmo oculta na bandeja do sistema),
    envia um sinal local para abrir a janela existente e retorna False.
    """
    global _MUTEX_HANDLE
    if os.name != "nt":
        return True

    try:
        kernel32 = ctypes.windll.kernel32
        _MUTEX_HANDLE = kernel32.CreateMutexW(None, False, "Local\\tVisio_SingleInstance_Mutex_v1")
        last_error = kernel32.GetLastError()
        if last_error == ERROR_ALREADY_EXISTS:
            # Acorda a janela da instância que já está rodando em segundo plano
            try:
                req = urllib.request.Request(
                    f"http://127.0.0.1:{http_port}/api/window/show",
                    method="POST",
                    data=b"{}",
                    headers={"Content-Type": "application/json"},
                )
                urllib.request.urlopen(req, timeout=1.5)
            except Exception:
                pass
            return False
    except Exception as e:
        logger.debug(f"Falha ao verificar Mutex de instância única: {e}")

    return True


def get_startup_command() -> str:
    """
    Monta o comando exato para iniciar o tVisio silenciosamente no login do Windows (`--startup`).
    Funciona tanto no executável compilado (`.exe`) quanto em ambiente Python de desenvolvimento.
    """
    if getattr(sys, "frozen", False):
        exe_path = Path(sys.executable).resolve()
        return f'"{exe_path}" --startup'

    # Em modo script Python, prefere pythonw.exe para não abrir janela de console
    py_dir = Path(sys.executable).resolve().parent
    pythonw = py_dir / "pythonw.exe"
    interpreter = pythonw if pythonw.exists() else Path(sys.executable).resolve()
    main_script = Path(__file__).resolve().parent.parent.parent / "main.py"
    return f'"{interpreter}" "{main_script}" --startup'


def get_start_with_windows() -> bool:
    """Verifica no Registro do Windows (`HKCU\\...\\Run`) se o tVisio está configurado para iniciar com o sistema."""
    if os.name != "nt":
        return False
    try:
        import winreg

        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_REG_PATH, 0, winreg.KEY_READ) as key:
            val, _ = winreg.QueryValueEx(key, RUN_APP_NAME)
            return bool(val)
    except FileNotFoundError:
        return False
    except Exception:
        return False


def set_start_with_windows(enabled: bool) -> bool:
    """
    Ativa ou desativa a inicialização automática do tVisio com o Windows
    no registro do usuário atual (`HKEY_CURRENT_USER`), sem exigir privilégios de Administrador.
    """
    if os.name != "nt":
        return False
    try:
        import winreg

        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_REG_PATH, 0, winreg.KEY_SET_VALUE) as key:
            if enabled:
                cmd = get_startup_command()
                winreg.SetValueEx(key, RUN_APP_NAME, 0, winreg.REG_SZ, cmd)
                logger.info(f"Inicialização com o Windows ativada: {cmd}")
            else:
                try:
                    winreg.DeleteValue(key, RUN_APP_NAME)
                    logger.info("Inicialização com o Windows removida do Registro.")
                except FileNotFoundError:
                    pass
        return True
    except Exception as e:
        logger.error(f"Erro ao configurar inicialização com o Windows: {e}")
        return False


def register_firewall_rules(http_port: int = 8080, https_port: int = 8443) -> bool:
    """
    Solicita elevação UAC para adicionar regras de entrada no Firewall do Windows
    liberando as portas TCP (8080/8443) e UDP (WebRTC) para conexão imediata do celular.
    """
    if os.name != "nt":
        return False
    try:
        commands = (
            f'netsh advfirewall firewall delete rule name="tVisio Virtual Webcam" >nul 2>&1 & '
            f'netsh advfirewall firewall add rule name="tVisio Virtual Webcam" dir=in action=allow '
            f"protocol=TCP localport={http_port},{https_port} profile=any & "
            f'netsh advfirewall firewall add rule name="tVisio Virtual Webcam" dir=in action=allow '
            f"protocol=UDP localport=1024-65535 profile=private,domain"
        )
        ret = ctypes.windll.shell32.ShellExecuteW(
            None, "runas", "cmd.exe", f"/c {commands}", None, SW_HIDE
        )
        return int(ret) > 32
    except Exception as e:
        logger.error(f"Erro ao configurar regra de Firewall: {e}")
        return False


def open_local_data_folder(folder_path: Path) -> bool:
    """Abre a pasta de dados locais (`_local`) diretamente no Windows Explorer."""
    try:
        folder_path.mkdir(parents=True, exist_ok=True)
        if os.name == "nt":
            os.startfile(str(folder_path))
        else:
            subprocess.Popen(["xdg-open", str(folder_path)])
        return True
    except Exception as e:
        logger.error(f"Erro ao abrir diretório de dados: {e}")
        return False


class DesktopWindowManager:
    """
    Gerencia a janela nativa `--app` (Edge/Chrome) e monitora seu Handle Win32 (`HWND`)
    para implementar 'Minimizar para a Bandeja' e 'Fechar para a Bandeja'.
    """

    def __init__(self, desktop_url: str, data_dir: Path) -> None:
        self.desktop_url = desktop_url
        self.profile_dir = data_dir / "_local" / "app_profile"
        self.hwnd: int = 0
        self.hidden_to_tray: bool = False
        self.window_ever_found: bool = False
        self.last_launch_ts: float = 0.0
        self._proc: Optional[subprocess.Popen] = None

    def find_hwnd(self) -> int:
        """Localiza o Handle (`HWND`) da janela do tVisio via API nativa `user32.FindWindowW`."""
        if os.name != "nt":
            return 0
        try:
            user32 = ctypes.windll.user32
            hwnd = user32.FindWindowW(None, WINDOW_TITLE)
            if hwnd:
                self.hwnd = int(hwnd)
                return self.hwnd

            # Fallback caso o navegador adicione sufixo ao título da janela
            found_hwnd = 0
            WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)

            def _enum_cb(h: int, _lparam: int) -> bool:
                nonlocal found_hwnd
                length = user32.GetWindowTextLengthW(h)
                if length > 0:
                    buf = ctypes.create_unicode_buffer(length + 1)
                    user32.GetWindowTextW(h, buf, length + 1)
                    if "tVisio — Ultra-Low Latency" in buf.value:
                        found_hwnd = int(h)
                        return False
                return True

            user32.EnumWindows(WNDENUMPROC(_enum_cb), 0)
            if found_hwnd:
                self.hwnd = found_hwnd
            return found_hwnd
        except Exception:
            return 0

    def show_or_launch(self) -> None:
        """
        Se a janela já existir (mesmo oculta na bandeja ou minimizada), restaura e traz para frente.
        Caso tenha sido fechada, abre uma nova janela `--app` instantaneamente.
        """
        if os.environ.get("TVISIO_HEADLESS") == "1":
            return

        if os.name == "nt":
            user32 = ctypes.windll.user32
            hwnd = self.hwnd if (self.hwnd and user32.IsWindow(self.hwnd)) else self.find_hwnd()
            if hwnd and user32.IsWindow(hwnd):
                user32.ShowWindow(hwnd, SW_SHOW)
                user32.ShowWindow(hwnd, SW_RESTORE)
                user32.SetForegroundWindow(hwnd)
                self.hidden_to_tray = False
                return

        # Evita múltiplos lançamentos simultâneos em menos de 1.5s
        now = time.monotonic()
        if now - self.last_launch_ts < 1.5:
            return
        self.last_launch_ts = now
        self.hidden_to_tray = False
        self.window_ever_found = False

        candidates = [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            shutil.which("msedge"),
            shutil.which("chrome"),
        ]

        for exe in candidates:
            if exe and Path(exe).exists():
                self.profile_dir.mkdir(parents=True, exist_ok=True)
                self._proc = subprocess.Popen(
                    [
                        str(exe),
                        f"--app={self.desktop_url}",
                        "--window-size=1180,760",
                        f"--user-data-dir={self.profile_dir}",
                        "--no-first-run",
                        "--no-default-browser-check",
                        "--disable-features=Translate",
                    ],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                return

        import webbrowser

        webbrowser.open(self.desktop_url)

    def hide_to_tray(self) -> None:
        """Oculta a janela da Barra de Tarefas, mantendo o tVisio apenas na Bandeja do Sistema."""
        if os.name != "nt":
            return
        user32 = ctypes.windll.user32
        hwnd = self.hwnd if (self.hwnd and user32.IsWindow(self.hwnd)) else self.find_hwnd()
        if hwnd and user32.IsWindow(hwnd):
            user32.ShowWindow(hwnd, SW_HIDE)
            self.hidden_to_tray = True

    def close_window(self) -> None:
        """Envia mensagem de fechamento (`WM_CLOSE`) para a janela gráfica se estiver aberta."""
        if os.name != "nt":
            return
        user32 = ctypes.windll.user32
        hwnd = self.hwnd if (self.hwnd and user32.IsWindow(self.hwnd)) else self.find_hwnd()
        if hwnd and user32.IsWindow(hwnd):
            user32.PostMessageW(hwnd, WM_CLOSE, 0, 0)
            self.hwnd = 0

    async def watch_window_lifecycle(self, server: "TVisioServer") -> None:
        """
        Monitora de forma assíncrona o estado da janela desktop:
        1. Se o usuário minimizar a janela e `minimize_to_tray` estiver ativo -> oculta da barra de tarefas.
        2. Se o usuário fechar a janela (`X`) e `close_to_tray` estiver desativado -> encerra o aplicativo.
        """
        if os.name != "nt" or os.environ.get("TVISIO_HEADLESS") == "1":
            return

        user32 = ctypes.windll.user32
        while not server.shutdown_event.is_set():
            await asyncio.sleep(0.3)
            try:
                hwnd = self.hwnd if (self.hwnd and user32.IsWindow(self.hwnd)) else self.find_hwnd()
                if hwnd and user32.IsWindow(hwnd):
                    self.window_ever_found = True
                    # Verifica se a janela foi minimizada pelo usuário
                    if not self.hidden_to_tray and user32.IsIconic(hwnd):
                        if server.settings.data.get("minimize_to_tray", True):
                            user32.ShowWindow(hwnd, SW_HIDE)
                            self.hidden_to_tray = True
                else:
                    self.hwnd = 0
                    # Se a janela estava aberta e foi fechada pelo botão (X)
                    if self.window_ever_found and not self.hidden_to_tray:
                        self.window_ever_found = False
                        if not server.settings.data.get("close_to_tray", True):
                            logger.info("Janela fechada pelo usuário (close_to_tray=False). Encerrando tVisio...")
                            server.request_shutdown()
                            return
                        else:
                            logger.info("Janela fechada: tVisio continua ativo em segundo plano na Bandeja do Sistema.")
            except Exception:
                pass


class SystemTrayManager:
    """
    Gerencia o ícone do tVisio na Bandeja do Sistema do Windows (System Tray) usando `pystray`.
    Roda em uma thread dedicada sem bloquear o loop de eventos `asyncio`.
    """

    def __init__(
        self,
        server: "TVisioServer",
        window_mgr: DesktopWindowManager,
        icon_path: Path,
        loop: asyncio.AbstractEventLoop,
    ) -> None:
        self.server = server
        self.window_mgr = window_mgr
        self.icon_path = icon_path
        self.loop = loop
        self._tray_icon = None
        self._thread: Optional[threading.Thread] = None

    def _load_image(self) -> Image.Image:
        """Carrega o ícone oficial do tVisio ou gera um ícone de fallback."""
        try:
            if self.icon_path.exists():
                return Image.open(self.icon_path)
        except Exception:
            pass
        return Image.new("RGBA", (64, 64), (0, 240, 255, 255))

    def start(self) -> None:
        """Inicia o ícone na Bandeja do Sistema em thread daemon."""
        if os.environ.get("TVISIO_HEADLESS") == "1":
            return

        try:
            import pystray

            image = self._load_image()

            def on_open(_icon, _item):
                self.loop.call_soon_threadsafe(self.window_mgr.show_or_launch)

            def on_toggle_eco(_icon, _item):
                async def _do():
                    await self.server._send_mobile_command({"command": "toggle_eco_mode"})

                asyncio.run_coroutine_threadsafe(_do(), self.loop)

            def on_toggle_freeze(_icon, _item):
                async def _do():
                    self.server.vcam.toggle_freeze()
                    await self.server.broadcast_telemetry()

                asyncio.run_coroutine_threadsafe(_do(), self.loop)

            def on_toggle_startup(_icon, _item):
                def _do():
                    cur = bool(self.server.settings.data.get("start_with_windows", False))
                    new_state = not cur
                    if set_start_with_windows(new_state):
                        self.server.settings.update(start_with_windows=new_state)
                        asyncio.ensure_future(self.server.broadcast_telemetry())

                self.loop.call_soon_threadsafe(_do)

            def on_toggle_close_tray(_icon, _item):
                def _do():
                    cur = bool(self.server.settings.data.get("close_to_tray", True))
                    self.server.settings.update(close_to_tray=not cur)
                    asyncio.ensure_future(self.server.broadcast_telemetry())

                self.loop.call_soon_threadsafe(_do)

            def on_quit(_icon, _item):
                self.loop.call_soon_threadsafe(self.server.request_shutdown)

            menu = pystray.Menu(
                pystray.MenuItem("Abrir Painel tVisio", on_open, default=True),
                pystray.Menu.SEPARATOR,
                pystray.MenuItem(
                    "Modo Eco OLED no Celular",
                    on_toggle_eco,
                    checked=lambda _item: bool(self.server.mobile_eco_mode),
                ),
                pystray.MenuItem(
                    "Congelar Câmera (Freeze)",
                    on_toggle_freeze,
                    checked=lambda _item: bool(self.server.vcam.frozen),
                ),
                pystray.Menu.SEPARATOR,
                pystray.MenuItem(
                    "Iniciar com o Windows",
                    on_toggle_startup,
                    checked=lambda _item: bool(self.server.settings.data.get("start_with_windows", False)),
                ),
                pystray.MenuItem(
                    "Fechar (X) para a Bandeja",
                    on_toggle_close_tray,
                    checked=lambda _item: bool(self.server.settings.data.get("close_to_tray", True)),
                ),
                pystray.Menu.SEPARATOR,
                pystray.MenuItem("Sair Completamente", on_quit),
            )

            self._tray_icon = pystray.Icon(
                "tVisio",
                image,
                "tVisio — Ultra-Low Latency Virtual Webcam",
                menu,
            )

            self._thread = threading.Thread(target=self._tray_icon.run, name="tVisio-Tray", daemon=True)
            self._thread.start()
            logger.info("Ícone da Bandeja do Sistema (System Tray) inicializado.")
        except Exception as e:
            logger.warning(f"Não foi possível iniciar o ícone da bandeja: {e}")

    def stop(self) -> None:
        """Remove o ícone da bandeja do sistema no encerramento do app."""
        try:
            if self._tray_icon is not None:
                self._tray_icon.stop()
        except Exception:
            pass
