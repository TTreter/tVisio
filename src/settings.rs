//! tVisio — Gerenciador de Configurações Persistentes, Recursos Embutidos e Token Fixo
//! ====================================================================================
//! Armazena as preferências do usuário, configurações de bandeja/inicialização do sistema
//! e o token criptográfico estável em `_local/settings.json`.
//! Todos os ativos estáticos (PWA Mobile, Ícones e DLLs DirectShow) são compilados
//! diretamente na seção `.rdata` do binário único `tVisio.exe` via `include_bytes!`.

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// Ativos estáticos embutidos diretamente no executável (.exe 100% autônomo e portátil)
pub static MOBILE_INDEX_HTML: &[u8] = include_bytes!("mobile/index.html");
pub static MOBILE_CSS: &[u8] = include_bytes!("mobile/mobile.css");
pub static MOBILE_JS: &[u8] = include_bytes!("mobile/mobile.js");

pub static ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");
pub static ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
pub static ICON_ICO: &[u8] = include_bytes!("../assets/icon.ico");

pub static DRIVER_DLL_64: &[u8] = include_bytes!("../assets/driver/UnityCaptureFilter64.dll");
pub static DRIVER_DLL_32: &[u8] = include_bytes!("../assets/driver/UnityCaptureFilter32.dll");

/// Retorna o diretório base gravável onde a pasta `_local/` é mantida.
/// Prioriza o diretório atual/ao lado do executável (Modo Portátil) e faz fallback
/// automático para `%LOCALAPPDATA%\tVisio` caso não haja permissão de escrita.
pub fn get_data_dir() -> PathBuf {
    let candidates = [
        std::env::current_dir().ok(),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf())),
    ];

    for candidate in candidates.into_iter().flatten() {
        let test_local = candidate.join("_local");
        if fs::create_dir_all(&test_local).is_ok() {
            let probe = test_local.join(".write_test");
            if fs::write(&probe, b"ok").is_ok() {
                let _ = fs::remove_file(&probe);
                return candidate;
            }
        }
    }

    if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        let fallback = PathBuf::from(local_appdata).join("tVisio");
        let _ = fs::create_dir_all(fallback.join("_local"));
        return fallback;
    }

    PathBuf::from(".")
}

/// Gera um token URL-safe criptográfico de 11 caracteres para o QR Code.
pub fn generate_session_token() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::thread_rng();
    (0..11)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Estrutura de configurações persistentes compatível 1:1 com `_local/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub session_token: String,
    pub mirror_horizontal: bool,
    pub mirror_vertical: bool,
    pub rotation: i32,
    pub brightness_gain: f32,
    pub contrast: f32,
    pub gamma: f32,
    pub color_temp: f32,      // -1.0 (Frio) a +1.0 (Quente)
    pub color_preset: String, // natural, warm, cool, cinema, bw
    pub zoom_level: f32,      // 1.0x até 3.0x
    pub pan_x: f32,           // -1.0 a +1.0
    pub pan_y: f32,           // -1.0 a +1.0
    pub shutter_cap_ms: f32,
    pub anti_psm_enabled: bool,
    pub resolution_profile: String,
    // Integração Nativa com Windows e Comportamento da Janela
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub auto_eco_oled: bool,
    pub auto_start_vcam: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            session_token: generate_session_token(),
            mirror_horizontal: false,
            mirror_vertical: false,
            rotation: 0,
            brightness_gain: 1.0,
            contrast: 1.0,
            gamma: 1.0,
            color_temp: 0.0,
            color_preset: "natural".to_string(),
            zoom_level: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            shutter_cap_ms: 8.0,
            anti_psm_enabled: false,
            resolution_profile: "1280x720@30".to_string(),
            start_with_windows: false,
            start_minimized: false,
            minimize_to_tray: true,
            close_to_tray: true,
            auto_eco_oled: true,
            auto_start_vcam: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsManager {
    pub settings_path: PathBuf,
    pub data: Settings,
}

impl SettingsManager {
    pub fn new(base_dir: &Path) -> Self {
        let local_dir = base_dir.join("_local");
        let _ = fs::create_dir_all(&local_dir);
        let settings_path = local_dir.join("settings.json");

        let mut mgr = Self {
            settings_path,
            data: Settings::default(),
        };
        mgr.load();
        mgr
    }

    pub fn load(&mut self) {
        if self.settings_path.exists() {
            if let Ok(raw) = fs::read_to_string(&self.settings_path) {
                if let Ok(parsed) = serde_json::from_str::<Settings>(&raw) {
                    self.data = parsed;
                }
            }
        }
        if self.data.session_token.trim().is_empty() {
            self.data.session_token = generate_session_token();
            self.save();
        }
    }

    pub fn save(&self) {
        if let Ok(json_str) = serde_json::to_string_pretty(&self.data) {
            let _ = fs::write(&self.settings_path, json_str);
        }
    }

    pub fn regenerate_token(&mut self) -> String {
        let new_token = generate_session_token();
        self.data.session_token = new_token.clone();
        self.save();
        new_token
    }
}
