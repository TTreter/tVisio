//! tVisio — Janela Nativa Desktop Win32 (`tao` + `wry`) — Translucent Glass UI v2.0
//! =======================================================================================
//! Renderiza 100% da interface original Translucent Glass UI v2.0 (`src/desktop/index.html`,
//! `styles.css`, `app.js`) embutida dentro do binário `tVisio.exe` em uma janela Win32 própria,
//! integrada nativamente à Bandeja do Sistema (`tray-icon`), Modo PiP Compacto Always-on-Top
//! e suspensão Zero-Render quando oculta em segundo plano.

use crate::server::SharedState;
use crate::settings::{ICON_PNG, WEBVIEW2_LOADER_DLL};
use crate::win_integration::{
    create_system_tray, set_start_with_windows, show_native_window, MENU_ID_CLOSE_TRAY,
    MENU_ID_ECO, MENU_ID_FREEZE, MENU_ID_OPEN, MENU_ID_QUIT, MENU_ID_STARTUP, WINDOW_TITLE,
};
use serde_json::json;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Icon, Theme, WindowBuilder},
};
use tray_icon::{
    menu::MenuEvent,
    MouseButton, MouseButtonState, TrayIconEvent,
};
use wry::{WebContext, WebViewBuilder};

fn load_window_icon() -> Option<Icon> {
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        let (w, h) = rgba.dimensions();
        return Icon::from_rgba(rgba.into_raw(), w, h).ok();
    }
    None
}

/// Aguarda até 1,5s para garantir que o servidor HTTP local (`127.0.0.1:8080`) já esteja escutando
/// antes de abrir a janela nativa, evitando qualquer tela de carregamento inicial.
fn wait_for_local_server(http_port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], http_port));
    for _ in 0..30 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Inicia o loop de eventos da janela nativa Win32 do tVisio e gerencia o ciclo de vida da Bandeja do Sistema.
pub fn run_native_desktop_window(state: SharedState, should_start_minimized: bool) {
    let (http_port, base_dir, start_with_win, close_to_tray_init) = {
        let s = state.lock().unwrap();
        (
            s.http_port,
            s.base_dir.clone(),
            s.settings.data.start_with_windows,
            s.settings.data.close_to_tray,
        )
    };

    wait_for_local_server(http_port);

    // Extrai o WebView2Loader embutido no .exe para `_local/driver/WebView2Loader.dll`
    let driver_dir = base_dir.join("_local").join("driver");
    let _ = std::fs::create_dir_all(&driver_dir);
    let wv2_dll_path = driver_dir.join("WebView2Loader.dll");
    if !wv2_dll_path.exists() {
        let _ = std::fs::write(&wv2_dll_path, WEBVIEW2_LOADER_DLL);
    }
    std::env::set_var("TVISIO_WV2_DLL_PATH", &wv2_dll_path);

    let event_loop = EventLoop::new();

    // Inicializa o ícone e menu de contexto na Bandeja do Sistema do Windows
    let tray_handles = create_system_tray(start_with_win, close_to_tray_init);

    let window = WindowBuilder::new()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(1180.0, 740.0))
        .with_min_inner_size(LogicalSize::new(420.0, 380.0))
        .with_theme(Some(Theme::Dark))
        .with_window_icon(load_window_icon())
        .with_visible(!should_start_minimized)
        .build(&event_loop)
        .expect("Falha ao criar a janela nativa Win32 do tVisio");

    if should_start_minimized {
        if let Ok(mut s) = state.lock() {
            s.vcam.miniplayer_enabled = false;
        }
    }

    // Armazena o cache de composição na pasta gravável `_local/webview_data`
    let webview_data_dir = base_dir.join("_local").join("webview_data");
    let _ = std::fs::create_dir_all(&webview_data_dir);
    let mut web_context = WebContext::new(Some(webview_data_dir));

    let desktop_url = format!("http://127.0.0.1:{http_port}/desktop");
    let _webview = WebViewBuilder::with_web_context(&mut web_context)
        .with_url(&desktop_url)
        .with_background_color((3, 7, 18, 255))
        .build(&window)
        .expect("Falha ao inicializar camada gráfica na janela nativa");

    let mut is_hidden_to_tray = should_start_minimized;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(80));

        // 1. Processa cliques no menu de contexto da Bandeja do Sistema
        while let Ok(menu_ev) = MenuEvent::receiver().try_recv() {
            match menu_ev.id.0.as_str() {
                MENU_ID_OPEN => {
                    is_hidden_to_tray = false;
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
                    show_native_window();
                    if let Ok(mut s) = state.lock() {
                        s.vcam.miniplayer_enabled = true;
                    }
                }
                MENU_ID_ECO => {
                    if let Ok(mut s) = state.lock() {
                        let target = !s.mobile_eco_mode;
                        s.mobile_eco_mode = target;
                        s.send_mobile_command(json!({ "command": "eco_mode", "enabled": target }));
                        if let Some(ref th) = tray_handles {
                            th.item_eco.set_checked(target);
                        }
                    }
                }
                MENU_ID_FREEZE => {
                    if let Ok(mut s) = state.lock() {
                        s.vcam.frozen = !s.vcam.frozen;
                        if let Some(ref th) = tray_handles {
                            th.item_freeze.set_checked(s.vcam.frozen);
                        }
                    }
                }
                MENU_ID_STARTUP => {
                    if let Ok(mut s) = state.lock() {
                        let target = !s.settings.data.start_with_windows;
                        s.settings.data.start_with_windows = target;
                        s.settings.save();
                        set_start_with_windows(target);
                        if let Some(ref th) = tray_handles {
                            th.item_startup.set_checked(target);
                        }
                    }
                }
                MENU_ID_CLOSE_TRAY => {
                    if let Ok(mut s) = state.lock() {
                        let target = !s.settings.data.close_to_tray;
                        s.settings.data.close_to_tray = target;
                        s.settings.save();
                        if let Some(ref th) = tray_handles {
                            th.item_close_tray.set_checked(target);
                        }
                    }
                }
                MENU_ID_QUIT => {
                    *control_flow = ControlFlow::Exit;
                    std::process::exit(0);
                }
                _ => {}
            }
        }

        // 2. Clique esquerdo no ícone da Bandeja restaura a janela principal
        while let Ok(tray_ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = tray_ev
            {
                is_hidden_to_tray = false;
                window.set_visible(true);
                window.set_minimized(false);
                window.set_focus();
                show_native_window();
                if let Ok(mut s) = state.lock() {
                    s.vcam.miniplayer_enabled = true;
                }
            }
        }

        // 3. Sincroniza solicitações vindas do backend (ex: segunda instância, PiP Mode, Quit)
        let (show_req, pip_req, quit_req, eco_state, freeze_state, startup_state, close_tray_state) = {
            if let Ok(mut s) = state.lock() {
                let sr = s.window_show_requested;
                s.window_show_requested = false;
                let pr = s.pip_mode_requested.take();
                let qr = s.quit_requested;
                (
                    sr,
                    pr,
                    qr,
                    s.mobile_eco_mode,
                    s.vcam.frozen,
                    s.settings.data.start_with_windows,
                    s.settings.data.close_to_tray,
                )
            } else {
                (false, None, false, false, false, false, true)
            }
        };

        if let Some(ref th) = tray_handles {
            if th.item_eco.is_checked() != eco_state {
                th.item_eco.set_checked(eco_state);
            }
            if th.item_freeze.is_checked() != freeze_state {
                th.item_freeze.set_checked(freeze_state);
            }
            if th.item_startup.is_checked() != startup_state {
                th.item_startup.set_checked(startup_state);
            }
            if th.item_close_tray.is_checked() != close_tray_state {
                th.item_close_tray.set_checked(close_tray_state);
            }
        }

        if quit_req {
            *control_flow = ControlFlow::Exit;
            std::process::exit(0);
        }

        if show_req {
            is_hidden_to_tray = false;
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
            show_native_window();
            if let Ok(mut s) = state.lock() {
                s.vcam.miniplayer_enabled = true;
            }
        }

        if let Some(enable_pip) = pip_req {
            if enable_pip {
                window.set_always_on_top(true);
                window.set_inner_size(LogicalSize::new(500.0, 430.0));
            } else {
                window.set_always_on_top(false);
                window.set_inner_size(LogicalSize::new(1180.0, 740.0));
            }
        }

        // 4. Eventos de Janela (Fechar X para Bandeja e Minimizar para Bandeja)
        if let Event::WindowEvent { event: win_ev, .. } = event {
            match win_ev {
                WindowEvent::CloseRequested => {
                    let close_to_tray = state
                        .lock()
                        .map(|s| s.settings.data.close_to_tray)
                        .unwrap_or(true);
                    if close_to_tray {
                        is_hidden_to_tray = true;
                        window.set_visible(false);
                        if let Ok(mut s) = state.lock() {
                            s.vcam.miniplayer_enabled = false;
                        }
                    } else {
                        *control_flow = ControlFlow::Exit;
                        std::process::exit(0);
                    }
                }
                WindowEvent::Resized(_) => {
                    if !is_hidden_to_tray && window.is_minimized() {
                        let min_to_tray = state
                            .lock()
                            .map(|s| s.settings.data.minimize_to_tray)
                            .unwrap_or(true);
                        if min_to_tray {
                            is_hidden_to_tray = true;
                            window.set_visible(false);
                            if let Ok(mut s) = state.lock() {
                                s.vcam.miniplayer_enabled = false;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    });
}
