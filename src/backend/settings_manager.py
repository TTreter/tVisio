"""
tVisio — Gerenciador de Configurações Persistentes e Token Fixo de Pareamento
Armazena preferências do usuário e o token de sessão estável em `_local/settings.json`.
Permite que o usuário adicione o WebApp à tela inicial do celular (PWA)
e reconecte instantaneamente sem precisar escanear o QR Code a cada reinicialização.
"""

import json
import secrets
from pathlib import Path
from typing import Dict, Any

# Valores padrão de inicialização com perfil térmico equilibrado
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
}


class SettingsManager:
    """Carrega, atualiza e persiste o arquivo de configurações JSON."""

    def __init__(self, base_dir: Path) -> None:
        self.settings_path = base_dir / "_local" / "settings.json"
        self.settings_path.parent.mkdir(parents=True, exist_ok=True)
        self.data: Dict[str, Any] = dict(DEFAULT_SETTINGS)
        self.load()

    def load(self) -> Dict[str, Any]:
        """Carrega as configurações salvas em disco ou gera um novo token seguro caso inexistente."""
        if self.settings_path.exists():
            try:
                with open(self.settings_path, "r", encoding="utf-8") as f:
                    loaded = json.load(f)
                    if isinstance(loaded, dict):
                        self.data.update(loaded)
            except Exception:
                pass

        # Cria um token URL-safe de 8 bytes estável caso não exista
        if not self.data.get("session_token"):
            self.data["session_token"] = secrets.token_urlsafe(8)
            self.save()

        return self.data

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
