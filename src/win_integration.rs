//! tVisio — Integração Nativa com o Sistema Operacional Windows (Win32 API + System Tray)
//! =======================================================================================
//! Gerencia todos os recursos nativos de aplicativo desktop no Windows:
//! 1. Mutex de Instância Única (`CreateMutexW`) com ativação automática da janela principal.
//! 2. Inicialização automática com o Windows via Registro (`HKCU\...\CurrentVersion\Run`).
//! 3. Instalação 100% offline do driver DirectShow (`UnityCaptureFilter64.dll` embutida no `.exe`).
//! 4. Liberação de portas no Firewall do Windows (`netsh advfirewall` via elevação UAC).
//! 5. Ocultação e restauração real da janela nativa (`HWND` via `ShowWindow` / `SW_HIDE`).
//! 6. Bandeja do Sistema (`tray-icon`) com menu de contexto dinâmico em português.

use crate::settings::{DRIVER_DLL_32, DRIVER_DLL_64, ICON_PNG};
use crate::unity_capture::{configure_tvisio_camera_name, is_unity_capture_registered};
use std::fs;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::ptr;
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winreg::enums::*;
use winreg::RegKey;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND},
    System::Threading::CreateMutexW,
    UI::{
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
        },
    },
};

pub const WINDOW_TITLE: &str = "tVisio — Ultra-Low Latency Virtual Webcam";
const RUN_REG_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_APP_NAME: &str = "tVisio";

pub const MENU_ID_OPEN: &str = "open_window";
pub const MENU_ID_ECO: &str = "toggle_eco";
pub const MENU_ID_FREEZE: &str = "toggle_freeze";
pub const MENU_ID_STARTUP: &str = "toggle_startup";
pub const MENU_ID_CLOSE_TRAY: &str = "toggle_close_tray";
pub const MENU_ID_QUIT: &str = "quit_app";

/// Converte uma string Rust (`&str`) para um vetor UTF-16 terminado em nulo (`Vec<u16>`) para a API Win32.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Garante que apenas uma instância do tVisio rode por vez no Windows.
/// Se já houver outra instância ativa (mesmo oculta na bandeja), acorda a janela existente e retorna `false`.
pub fn check_single_instance(http_port: u16) -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        let mutex_name = to_wide("Local\\tVisio_SingleInstance_Mutex_v1");
        let handle = CreateMutexW(ptr::null(), 0, mutex_name.as_ptr());
        if !handle.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            // Acorda a instância existente via loopback TCP instantâneo
            let addr = SocketAddr::from(([127, 0, 0, 1], http_port));
            if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
                let req = format!(
                    "POST /api/window/show HTTP/1.1\r\nHost: 127.0.0.1:{http_port}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                let _ = stream.write_all(req.as_bytes());
            }
            show_native_window();
            return false;
        }
        // Mantém o handle do Mutex vivo durante toda a execução do processo
        let _ = handle;
    }
    true
}

/// Verifica se o tVisio está configurado para iniciar junto com o Windows (`HKCU\...\Run`).
pub fn get_start_with_windows() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN_REG_PATH, KEY_READ) {
        if let Ok(val) = key.get_value::<String, _>(RUN_APP_NAME) {
            return !val.trim().is_empty();
        }
    }
    false
}

/// Ativa ou desativa a inicialização automática com o Windows no registro do usuário atual (`HKCU`).
pub fn set_start_with_windows(enabled: bool) -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok((key, _)) = hkcu.create_subkey(RUN_REG_PATH) else {
        return false;
    };

    if enabled {
        if let Ok(exe_path) = std::env::current_exe() {
            let cmd = format!("\"{}\" --startup", exe_path.display());
            return key.set_value(RUN_APP_NAME, &cmd).is_ok();
        }
        false
    } else {
        let _ = key.delete_value(RUN_APP_NAME);
        true
    }
}

/// Extrai as DLLs DirectShow embutidas no executável para `_local/driver/` e registra via `regsvr32` (UAC).
pub fn install_vcam_driver_offline(base_dir: &Path) -> Result<bool, String> {
    let driver_dir = base_dir.join("_local").join("driver");
    fs::create_dir_all(&driver_dir).map_err(|e| format!("Erro ao criar pasta driver: {e}"))?;

    let dll64_path = driver_dir.join("UnityCaptureFilter64.dll");
    let dll32_path = driver_dir.join("UnityCaptureFilter32.dll");

    if !dll64_path.exists() {
        fs::write(&dll64_path, DRIVER_DLL_64)
            .map_err(|e| format!("Falha ao extrair UnityCaptureFilter64.dll: {e}"))?;
    }
    if !dll32_path.exists() {
        fs::write(&dll32_path, DRIVER_DLL_32)
            .map_err(|e| format!("Falha ao extrair UnityCaptureFilter32.dll: {e}"))?;
    }

    configure_tvisio_camera_name();

    #[cfg(target_os = "windows")]
    unsafe {
        let verb = to_wide("runas");
        let file = to_wide("regsvr32.exe");
        let params = to_wide(&format!(
            "/s \"{}\" \"{}\"",
            dll64_path.display(),
            dll32_path.display()
        ));
        let ret = ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            ptr::null(),
            SW_HIDE,
        );
        if (ret as isize) <= 32 {
            return Err("Elevação UAC cancelada ou falha ao executar regsvr32.exe".to_string());
        }
    }

    // Aguarda brevemente o término do registro pelo regsvr32
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(150));
        if is_unity_capture_registered() {
            configure_tvisio_camera_name();
            return Ok(true);
        }
    }

    configure_tvisio_camera_name();
    Ok(is_unity_capture_registered())
}

/// Solicita elevação UAC para liberar as portas TCP (8080/8443) e UDP no Firewall do Windows.
pub fn register_firewall_rules(http_port: u16, https_port: u16) -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        let commands = format!(
            "/c netsh advfirewall firewall delete rule name=\"tVisio Virtual Webcam\" >nul 2>&1 & \
             netsh advfirewall firewall add rule name=\"tVisio Virtual Webcam\" dir=in action=allow protocol=TCP localport={http_port},{https_port} profile=any & \
             netsh advfirewall firewall add rule name=\"tVisio Virtual Webcam\" dir=in action=allow protocol=UDP localport=1024-65535 profile=private,domain"
        );
        let verb = to_wide("runas");
        let file = to_wide("cmd.exe");
        let params = to_wide(&commands);
        let ret = ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            ptr::null(),
            SW_HIDE,
        );
        return (ret as isize) > 32;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (http_port, https_port);
        false
    }
}

/// Abre a tela de configurações nativas do Mobile Hotspot 5GHz do Windows 10/11.
pub fn open_mobile_hotspot_settings() {
    let _ = Command::new("cmd")
        .args(["/C", "start", "ms-settings:network-mobilehotspot"])
        .spawn();
}

/// Abre a pasta de dados locais (`_local`) diretamente no Windows Explorer.
pub fn open_local_data_folder(folder_path: &Path) {
    let _ = fs::create_dir_all(folder_path);
    let _ = Command::new("explorer").arg(folder_path).spawn();
}

/// Localiza o `HWND` da janela nativa do tVisio.
#[cfg(target_os = "windows")]
pub fn find_tvisio_hwnd() -> HWND {
    unsafe {
        let title = to_wide(WINDOW_TITLE);
        FindWindowW(ptr::null(), title.as_ptr())
    }
}

/// Verifica se a janela nativa do tVisio está minimizada (`IsIconic`).
#[allow(dead_code)]
pub fn is_native_window_minimized() -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        let hwnd = find_tvisio_hwnd();
        if !hwnd.is_null() {
            return IsIconic(hwnd) != 0;
        }
    }
    false
}

/// Oculta a janela nativa da Barra de Tarefas do Windows (`SW_HIDE`), mantendo o app na Bandeja do Sistema.
#[allow(dead_code)]
pub fn hide_native_window_to_tray() {
    #[cfg(target_os = "windows")]
    unsafe {
        let hwnd = find_tvisio_hwnd();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Restaura a janela nativa da Bandeja do Sistema e traz para o primeiro plano (`SW_RESTORE` + `SetForegroundWindow`).
pub fn show_native_window() {
    #[cfg(target_os = "windows")]
    unsafe {
        let hwnd = find_tvisio_hwnd();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_SHOW);
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

/// Estrutura detentora do ícone e itens de menu da Bandeja do Sistema (System Tray).
pub struct SystemTrayHandles {
    pub _tray_icon: TrayIcon,
    pub item_eco: CheckMenuItem,
    pub item_freeze: CheckMenuItem,
    pub item_startup: CheckMenuItem,
    pub item_close_tray: CheckMenuItem,
}

/// Cria e inicializa o ícone oficial do tVisio na Bandeja do Sistema do Windows.
pub fn create_system_tray(
    start_with_windows: bool,
    close_to_tray: bool,
) -> Option<SystemTrayHandles> {
    let icon = load_tray_icon()?;
    let menu = Menu::new();

    let item_open = MenuItem::with_id(MENU_ID_OPEN, "Abrir Painel tVisio", true, None);
    let item_eco = CheckMenuItem::with_id(
        MENU_ID_ECO,
        "Modo Eco OLED no Celular",
        true,
        false,
        None,
    );
    let item_freeze = CheckMenuItem::with_id(
        MENU_ID_FREEZE,
        "Congelar Câmera (Freeze)",
        true,
        false,
        None,
    );
    let item_startup = CheckMenuItem::with_id(
        MENU_ID_STARTUP,
        "Iniciar com o Windows",
        true,
        start_with_windows,
        None,
    );
    let item_close_tray = CheckMenuItem::with_id(
        MENU_ID_CLOSE_TRAY,
        "Fechar (X) para a Bandeja",
        true,
        close_to_tray,
        None,
    );
    let item_quit = MenuItem::with_id(MENU_ID_QUIT, "Sair Completamente", true, None);

    let _ = menu.append(&item_open);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item_eco);
    let _ = menu.append(&item_freeze);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item_startup);
    let _ = menu.append(&item_close_tray);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item_quit);

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(WINDOW_TITLE)
        .with_icon(icon)
        .build()
        .ok()?;

    Some(SystemTrayHandles {
        _tray_icon: tray_icon,
        item_eco,
        item_freeze,
        item_startup,
        item_close_tray,
    })
}

fn load_tray_icon() -> Option<Icon> {
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        return Icon::from_rgba(rgba.into_raw(), 64, 64).ok();
    }
    let fallback = vec![0u8; 32 * 32 * 4];
    Icon::from_rgba(fallback, 32, 32).ok()
}
