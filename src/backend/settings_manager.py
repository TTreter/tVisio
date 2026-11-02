"""
tVisio — Gerenciador de Configurações Persistentes, Resolução de Caminhos (.exe) e Token Fixo
=============================================================================================
Armazena preferências do usuário, configurações de bandeja/inicialização do sistema
e o token de sessão estável em `_local/settings.json`.
Inclui resolução inteligente de diretórios (`get_resource_dir` e `get_data_dir`) para
funcionar perfeitamente tanto em modo de desenvolvimento quanto empacotado em `.exe`
via PyInstaller (`sys._MEIPASS`).
"""

import json
import os
import secrets
import sys
from pathlib import Path
from typing import Dict, Any


def get_resource_dir() -> Path:
    """
    Retorna o diretório base onde estão os recursos estáticos (`assets/`, `src/desktop/`, `src/mobile/`).
    Quando executado como executável PyInstaller (`sys.frozen`), utiliza `sys._MEIPASS`.
    """
    if getattr(sys, "frozen", False) and hasattr(sys, "_MEIPASS"):
        return Path(getattr(sys, "_MEIPASS"))
    return Path(__file__).resolve().parent.parent.parent


def get_data_dir() -> Path:
    """
    Retorna o diretório base gravável onde a pasta `_local/` será mantida
    (certificados TLS, configurações JSON, perfil do app e DLLs DirectShow).
    Prioriza o diretório ao lado do executável (Modo Portátil) e faz fallback
    automático para `%LOCALAPPDATA%\\tVisio` caso não haja permissão de escrita.
    """
    if getattr(sys, "frozen", False):
        exe_dir = Path(sys.executable).resolve().parent
        try:
            test_dir = exe_dir / "_local"
            test_dir.mkdir(parents=True, exist_ok=True)
            probe = test_dir / ".write_test"
            probe.write_text("ok", encoding="utf-8")
            probe.unlink(missing_ok=True)
            return exe_dir
        except Exception:
            appdata = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "tVisio"
            appdata.mkdir(parents=True, exist_ok=True)
            return appdata
    return Path(__file__).resolve().parent.parent.parent


# Valores padrão de inicialização com perfil térmico equilibrado e integração Windows
DEFAULT_SETTINGS: Dict[str, Any] = {
    "session_token": "",
    "mirror_horizontal": False,
    "mirror_vertical": False,
    "rotation": 0,
    "brightness_gain": 1.0,
    "contrast": 1.0,
    "gamma": 1.0,
    "color_temp": 0.0,  # -1.0 (Frio) a +1.0 (Quente)
    "color_preset": "natural",  # natural, warm, cool, cinema, bw
    "zoom_level": 1.0,  # 1.0x até 3.0x
    "pan_x": 0.0,  # -1.0 a +1.0
    "pan_y": 0.0,  # -1.0 a +1.0
    "shutter_cap_ms": 8.0,
    "anti_psm_enabled": False,
    "resolution_profile": "1280x720@30",
    # Configurações de Aplicativo Desktop & Sistema Windows
    "start_with_windows": False,
    "start_minimized": False,
    "minimize_to_tray": True,
    "close_to_tray": True,
    "auto_eco_oled": True,
    "auto_start_vcam": True,
}


class SettingsManager:
    """Carrega, atualiza e persiste o arquivo de configurações JSON."""

    def __init__(self, base_dir: Path | None = None) -> None:
        root = base_dir if base_dir is not None else get_data_dir()
        self.local_dir = root / "_local"
        self.local_dir.mkdir(parents=True, exist_ok=True)
        self.settings_path = self.local_dir / "settings.json"
        self.data: Dict[str, Any] = dict(DEFAULT_SETTINGS)
        self.load()

    def load(self) -> Dict[str, Any]:
        """Carrega as configurações salvas em disco ou gera um novo token seguro caso inexistente."""
        if self.settings_path.exists():
            try:
                with open(self.settings_path, "r", encoding="utf-8") as f:
                    loaded = json.load(f)
                    if isinstance(loaded, dict):
                        for k, v in loaded.items():
                            if k in DEFAULT_SETTINGS:
                                self.data[k] = v
            except Exception:
                pass

        # Cria um token URL-safe de 8 bytes estável caso não exista
        if not self.data.get("session_token"):
            self.data["session_token"] = secrets.token_urlsafe(8)
            self.save()

        return self.data

    def regenerate_token(self) -> str:
        """Gera um novo token criptográfico para o QR Code, invalidando sessões antigas."""
        new_token = secrets.token_urlsafe(8)
        self.data["session_token"] = new_token
        self.save()
        return new_token

    def update(self, **kwargs: Any) -> None:
        """Atualiza valores específicos e persiste imediatamente em disco."""
        changed = False
        for k, v in kwargs.items():
            if k in DEFAULT_SETTINGS and self.data.get(k) != v:
                self.data[k] = v
                changed = True
        if changed:
            self.save()

    def save(self) -> None:
        """Grava atomicamente o estado atual em `_local/settings.json`."""
        try:
            with open(self.settings_path, "w", encoding="utf-8") as f:
                json.dump(self.data, f, indent=2)
        except Exception:
            pass
