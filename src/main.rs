#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! tVisio — Ponto de Entrada Nativo em Rust (`tVisio.exe`)
//! =======================================================================================
//! Inicializa o Motor de Transmissão de Vídeo de Ultra-Baixa Latência (Zero-Overhead):
//! - Mutex Win32 de Instância Única (`CreateMutexW`) com ativação automática da janela.
//! - Servidores HTTP (`:8080`) e HTTPS (`:8443`) assíncronos em Tokio multithread.
//! - Pipeline de Vídeo 1-Pass Kernel Fusion (ePTZ + Flip/Rotate + L1-LUT + DirectShow SHM).
//! - Interface Desktop Translucent Glass UI acelerada via OpenGL (`eframe`/`egui`).
//! - Bandeja do Sistema Nativa (`tray-icon`) e integração com Registro/Firewall do Windows.
//!
//! Desenvolvido por: Tiago Treter (@TTreter)
//! Licença: GPLv3

mod cert_manager;
mod gui;
mod network_utils;
mod server;
mod settings;
mod unity_capture;
mod vcam_engine;
mod win_integration;

use crate::gui::TVisioGuiApp;
use crate::server::{run_background_servers, SharedState, TVisioAppState};
use crate::settings::{get_data_dir, ICON_PNG};
use crate::win_integration::{check_single_instance, WINDOW_TITLE};
use eframe::egui;
use std::sync::{Arc, Mutex};

fn load_window_icon() -> Arc<egui::IconData> {
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        let (width, height) = rgba.dimensions();
        return Arc::new(egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        });
    }
    Arc::new(egui::IconData {
        rgba: vec![0u8; 32 * 32 * 4],
        width: 32,
        height: 32,
    })
}

fn main() -> eframe::Result<()> {
    let http_port: u16 = std::env::var("TVISIO_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let https_port: u16 = std::env::var("TVISIO_HTTPS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8443);

    // 1. Garante instância única do aplicativo no Windows
    if !check_single_instance(http_port) {
        return Ok(());
    }

    // 2. Detecta flags de linha de comando (--startup / --minimized)
    let args: Vec<String> = std::env::args().map(|a| a.to_lowercase()).collect();
    let start_minimized_flag = args.iter().any(|a| a == "--startup" || a == "--minimized");

    // 3. Inicializa estado global compartilhado e configurações persistentes (_local/settings.json)
    let base_dir = get_data_dir();
    let app_state = TVisioAppState::new(base_dir, http_port, https_port);
    let should_start_minimized = start_minimized_flag || app_state.settings.data.start_minimized;
    let shared_state: SharedState = Arc::new(Mutex::new(app_state));

    // 4. Inicia o runtime Tokio multithread em segundo plano para os servidores HTTP/HTTPS/WS
    let server_state = shared_state.clone();
    std::thread::Builder::new()
        .name("tVisio-Tokio-Server".to_string())
        .spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                rt.block_on(async move {
                    run_background_servers(server_state).await;
                    // Mantém o runtime vivo enquanto a aplicação estiver em execução
                    std::future::pending::<()>().await;
                });
            }
        })
        .expect("Falha ao iniciar thread do servidor Tokio");

    // 5. Configura e abre a Janela Desktop do tVisio
    let viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([1180.0, 740.0])
        .with_min_inner_size([600.0, 440.0])
        .with_icon(load_window_icon());

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(TVisioGuiApp::new(
                cc,
                shared_state,
                should_start_minimized,
            )))
        }),
    )
}
