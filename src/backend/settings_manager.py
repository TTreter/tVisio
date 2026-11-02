"""
tVisio — Persistent Settings & Fixed Pairing Token Manager
Stores user preferences and the fixed session token in `_local/settings.json`
so settings survive restarts and mobile PWA bookmarks reconnect without re-scanning QR.
"""

import json
import secrets
from pathlib import Path
from typing import Dict, Any

DEFAULT_SETTINGS: Dict[str, Any] = {
    "session_token": "",
    "mirror_horizontal": False,
    "mirror_vertical": False,
    "rotation": 0,
    "brightness_gain": 1.0,
    "contrast": 1.0,
    "gamma": 1.0,
    "color_temp": 0.0,  # -1.0 (Cool) to +1.0 (Warm)
    "color_preset": "natural",  # natural, warm, cool, cinema, bw
    "zoom_level": 1.0,  # 1.0x to 3.0x
    "pan_x": 0.0,  # -1.0 to +1.0
    "pan_y": 0.0,  # -1.0 to +1.0
    "shutter_cap_ms": 8.0,
    "anti_psm_enabled": False,
    "resolution_profile": "1280x720@30",
}


class SettingsManager:
    def __init__(self, base_dir: Path) -> None:
        self.settings_path = base_dir / "_local" / "settings.json"
        self.settings_path.parent.mkdir(parents=True, exist_ok=True)
        self.data: Dict[str, Any] = dict(DEFAULT_SETTINGS)
        self.load()

    def load(self) -> Dict[str, Any]:
        if self.settings_path.exists():
            try:
                with open(self.settings_path, "r", encoding="utf-8") as f:
                    loaded = json.load(f)
                    if isinstance(loaded, dict):
                        self.data.update(loaded)
            except Exception:
                pass

        if not self.data.get("session_token"):
            self.data["session_token"] = secrets.token_urlsafe(8)
            self.save()

        return self.data

    def update(self, **kwargs: Any) -> None:
        changed = False
        for k, v in kwargs.items():
            if k in DEFAULT_SETTINGS and self.data.get(k) != v:
                self.data[k] = v
                changed = True
        if changed:
            self.save()

    def save(self) -> None:
        try:
            with open(self.settings_path, "w", encoding="utf-8") as f:
                json.dump(self.data, f, indent=2)
        except Exception:
            pass
