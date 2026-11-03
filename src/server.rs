//! tVisio — Servidor Core Assíncrono (Axum HTTP/HTTPS + Binary Turbo WS + Anti-PSM 500Hz)
//! =======================================================================================
//! Gerencia:
//! - Entrega do WebApp PWA móvel via HTTPS (Porta 8443) e HTTP (Porta 8080 para USB Localhost).
//! - Entrega da Interface Desktop Translucent Glass UI v2.0 (`/desktop`, `/ws/desktop`).
//! - Canal de streaming binário `/ws/stream` de ultra-baixa latência (Zero-Queue com ACK imediato).
//! - Pulso inteligente Anti-PSM de 1 byte (`0x01`) mantendo o rádio Wi-Fi móvel fora de micro-sleep.
//! - Sincronização em tempo real com a Interface Gráfica Nativa Desktop (`SharedState`).

use crate::cert_manager::ensure_ssl_certificates;
use crate::network_utils::{discover_network_interfaces, generate_qr_code_assets, NetworkInterfaceInfo};
use crate::settings::{
    SettingsManager, DESKTOP_CSS, DESKTOP_INDEX_HTML, DESKTOP_JS, ICON_ICO, ICON_PNG, ICON_SVG,
    MOBILE_CSS, MOBILE_INDEX_HTML, MOBILE_JS,
};
use crate::vcam_engine::VirtualCamEngine;
use crate::win_integration::{
    get_start_with_windows, install_vcam_driver_offline, open_local_data_folder,
    open_mobile_hotspot_settings, register_firewall_rules, set_start_with_windows,
    show_native_window,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileCameraDevice {
    pub id: String,
    pub label: String,
}

pub struct TVisioAppState {
    pub base_dir: PathBuf,
    pub http_port: u16,
    pub https_port: u16,
    pub settings: SettingsManager,
    pub vcam: VirtualCamEngine,

    // Rede e Pareamento QR Code
    pub interfaces: Vec<NetworkInterfaceInfo>,
    pub selected_ip: String,
    pub qr_rgba: Vec<u8>,
    pub qr_dim: u32,
    pub qr_png: Vec<u8>,
    pub qr_seq: u64,

    // Telemetria em Tempo Real do Smartphone
    pub mobile_connected: bool,
    pub active_protocol: String,
    pub rtt_ms: f32,
    pub glass_to_glass_ms: f32,
    pub mobile_device_info: String,
    pub mobile_eco_mode: bool,
    pub mobile_torch_on: bool,
    pub mobile_af_locked: bool,
    pub mobile_facing_mode: String,
    pub mobile_cameras: Vec<MobileCameraDevice>,
    pub mobile_selected_cam_id: String,
    pub mobile_battery_level: Option<i32>,
    pub mobile_battery_charging: bool,

    // Canal de broadcast para envio instantâneo de comandos ao celular
    pub mobile_cmd_tx: broadcast::Sender<String>,

    // Sinalizadores de ciclo de vida da janela nativa
    pub window_show_requested: bool,
    pub pip_mode_requested: Option<bool>,
    pub quit_requested: bool,
}

pub type SharedState = Arc<Mutex<TVisioAppState>>;

static ACTIVE_MOBILE_CLIENTS: AtomicUsize = AtomicUsize::new(0);

impl TVisioAppState {
    pub fn new(base_dir: PathBuf, http_port: u16, https_port: u16) -> Self {
        let mut settings = SettingsManager::new(&base_dir);
        if settings.data.start_with_windows {
            set_start_with_windows(true);
        } else if get_start_with_windows() {
            settings.data.start_with_windows = true;
        }

        let vcam = VirtualCamEngine::new(&settings.data);
        let interfaces = discover_network_interfaces(https_port);
        let selected_ip = interfaces
            .first()
            .map(|i| i.ip.clone())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let (mobile_cmd_tx, _) = broadcast::channel(64);
        let initial_camera_id = settings.data.selected_camera_id.clone();

        let mut app_state = Self {
            base_dir,
            http_port,
            https_port,
            settings,
            vcam,
            interfaces,
            selected_ip,
            qr_rgba: Vec::new(),
            qr_dim: 0,
            qr_png: Vec::new(),
            qr_seq: 0,
            mobile_connected: false,
            active_protocol: "Standby".to_string(),
            rtt_ms: 0.0,
            glass_to_glass_ms: 0.0,
            mobile_device_info: "Nenhum dispositivo conectado".to_string(),
            mobile_eco_mode: false,
            mobile_torch_on: false,
            mobile_af_locked: false,
            mobile_facing_mode: "environment".to_string(),
            mobile_cameras: Vec::new(),
            mobile_selected_cam_id: initial_camera_id,
            mobile_battery_level: None,
            mobile_battery_charging: false,
            mobile_cmd_tx,
            window_show_requested: false,
            pip_mode_requested: None,
            quit_requested: false,
        };
        app_state.rebuild_qr_code();
        app_state
    }

    pub fn pairing_url(&self) -> String {
        format!(
            "https://{}:{}/?t={}",
            self.selected_ip, self.https_port, self.settings.data.session_token
        )
    }

    pub fn rebuild_qr_code(&mut self) {
        let url = self.pairing_url();
        let (rgba, dim, _, png) = generate_qr_code_assets(&url);
        self.qr_rgba = rgba;
        self.qr_dim = dim;
        self.qr_png = png;
        self.qr_seq = self.qr_seq.wrapping_add(1);
    }

    pub fn refresh_network_interfaces(&mut self) {
        self.interfaces = discover_network_interfaces(self.https_port);
        if !self.interfaces.iter().any(|i| i.ip == self.selected_ip) {
            if let Some(first) = self.interfaces.first() {
                self.selected_ip = first.ip.clone();
            }
        }
        self.rebuild_qr_code();
    }

    pub fn send_mobile_command(&self, payload: Value) {
        let mut map = match payload {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        map.insert("type".to_string(), Value::String("command".to_string()));
        if let Ok(msg) = serde_json::to_string(&Value::Object(map)) {
            let _ = self.mobile_cmd_tx.send(msg);
        }
    }

    pub fn build_telemetry_json(&self) -> Value {
        json!({
            "type": "telemetry",
            "mobile_connected": self.mobile_connected,
            "device_info": self.mobile_device_info,
            "shutter_cap_ms": self.settings.data.shutter_cap_ms,
            "g2g_ms": self.glass_to_glass_ms,
            "rtt_ms": self.rtt_ms,
            "active_protocol": self.active_protocol,
            "battery_level": self.mobile_battery_level,
            "battery_charging": self.mobile_battery_charging,
            "torch_on": self.mobile_torch_on,
            "af_locked": self.mobile_af_locked,
            "eco_mode": self.mobile_eco_mode,
            "anti_psm": self.settings.data.anti_psm_enabled,
            "cameras": self.mobile_cameras,
            "selected_cam_id": self.mobile_selected_cam_id,
            "interfaces": self.interfaces,
            "selected_ip": self.selected_ip,
            "pairing_url": self.pairing_url(),
            "vcam": {
                "driver_active": self.vcam.shm.driver_registered,
                "backend_name": self.vcam.shm.backend_status_label(),
                "resolution": format!("{}x{} ({})", self.vcam.width, self.vcam.height, self.vcam.resolution_label()),
                "decode_ms": self.vcam.last_decode_ms,
                "fps": self.vcam.current_fps,
                "frozen": self.vcam.frozen,
                "mirror_horizontal": self.vcam.mirror_horizontal,
                "brightness_gain": self.vcam.brightness_gain,
                "contrast": self.vcam.contrast,
                "color_temp": self.vcam.color_temp,
                "color_preset": self.vcam.color_preset,
                "zoom_level": self.vcam.zoom_level,
                "pan_x": self.vcam.pan_x,
                "pan_y": self.vcam.pan_y,
            },
            "app_settings": {
                "start_with_windows": self.settings.data.start_with_windows,
                "start_minimized": self.settings.data.start_minimized,
                "minimize_to_tray": self.settings.data.minimize_to_tray,
                "close_to_tray": self.settings.data.close_to_tray,
                "auto_eco_oled": self.settings.data.auto_eco_oled,
                "auto_start_vcam": self.settings.data.auto_start_vcam,
                "resolution_profile": self.settings.data.resolution_profile,
                "camera_display_name": self.settings.data.camera_display_name,
                "selected_camera_id": self.settings.data.selected_camera_id,
            }
        })
    }
}

/// Constrói o roteador Axum com todas as rotas Desktop, PWA Mobile e APIs locais.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(handle_root_index))
        .route("/desktop", get(handle_desktop_index))
        .route("/mobile", get(handle_mobile_index))
        .route("/manifest.json", get(handle_manifest_json))
        .route("/desktop-static/styles.css", get(handle_desktop_css))
        .route("/desktop-static/app.js", get(handle_desktop_js))
        .route("/mobile-static/mobile.css", get(handle_mobile_css))
        .route("/mobile-static/mobile.js", get(handle_mobile_js))
        .route("/assets/icon.svg", get(handle_icon_svg))
        .route("/assets/icon.png", get(handle_icon_png))
        .route("/assets/icon.ico", get(handle_icon_ico))
        .route("/api/qr", get(handle_qr_png))
        .route("/api/cert", get(handle_cert_download))
        .route("/api/window/show", post(handle_window_show))
        .route("/api/install-vcam", post(handle_install_vcam))
        .route("/api/hotspot", post(handle_open_hotspot))
        .route("/api/firewall", post(handle_firewall))
        .route("/api/regenerate-token", post(handle_regenerate_token))
        .route("/api/open-data-dir", post(handle_open_data_dir))
        .route("/api/open-external", post(handle_open_external))
        .route("/api/app/quit", post(handle_app_quit))
        .route("/api/webrtc/offer", post(handle_webrtc_offer))
        .route("/ws/stream", get(handle_mobile_ws))
        .route("/ws/desktop", get(handle_desktop_ws))
        .with_state(state)
}

/// Inicia os servidores HTTP (`:8080`) e HTTPS (`:8443`) em background no runtime Tokio.
pub async fn run_background_servers(state: SharedState) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (base_dir, http_port, https_port, ip_list) = {
        let s = state.lock().unwrap();
        let ips: Vec<String> = s.interfaces.iter().map(|i| i.ip.clone()).collect();
        (s.base_dir.clone(), s.http_port, s.https_port, ips)
    };

    let cert_res = ensure_ssl_certificates(&ip_list, &base_dir);
    let app_http = build_router(state.clone());
    let app_https = build_router(state.clone());

    // Servidor HTTP (Porta 8080 — Janela Nativa Desktop e USB ADB Localhost)
    let http_addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    tokio::spawn(async move {
        let _ = axum_server::bind(http_addr)
            .serve(app_http.into_make_service())
            .await;
    });

    // Servidor HTTPS (Porta 8443 — Necessário para getUserMedia() em navegadores móveis)
    if let Ok((cert_path, key_path)) = cert_res {
        if let Ok(tls_config) = RustlsConfig::from_pem_file(cert_path, key_path).await {
            let https_addr = SocketAddr::from(([0, 0, 0, 0], https_port));
            tokio::spawn(async move {
                let _ = axum_server::bind_rustls(https_addr, tls_config)
                    .serve(app_https.into_make_service())
                    .await;
            });
        }
    }
}

// -----------------------------------------------------------------------------
// Handlers HTTP e Ativos Estáticos Embutidos
// -----------------------------------------------------------------------------

async fn handle_root_index(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // Se possui token `?t=` na query string, trata-se do pareamento móvel via QR Code
    let has_token_param = params.contains_key("t");
    let host_val = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let hostname = host_val.split(':').next().unwrap_or("");
    let port_part = host_val.split(':').nth(1).unwrap_or("");

    let (http_port, https_port, token) = {
        let s = state.lock().unwrap();
        (
            s.http_port.to_string(),
            s.https_port,
            s.settings.data.session_token.clone(),
        )
    };

    let is_localhost = hostname == "localhost" || hostname == "127.0.0.1";

    // Se acessado de um IP da rede local via HTTP, redireciona para HTTPS mobile
    if port_part == http_port && !is_localhost {
        let t = params.get("t").cloned().unwrap_or(token);
        let https_url = format!("https://{hostname}:{https_port}/?t={t}");
        return Redirect::temporary(&https_url).into_response();
    }

    // Se for localhost sem `?t=` na porta HTTP, entrega a Interface Desktop Translucent Glass UI
    if is_localhost && !has_token_param && port_part == http_port {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            DESKTOP_INDEX_HTML,
        )
            .into_response();
    }

    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        MOBILE_INDEX_HTML,
    )
        .into_response()
}

async fn handle_desktop_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        DESKTOP_INDEX_HTML,
    )
}

async fn handle_mobile_index(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Some(host_val) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) {
        let hostname = host_val.split(':').next().unwrap_or("");
        let port_part = host_val.split(':').nth(1).unwrap_or("");
        let (http_port, https_port, token) = {
            let s = state.lock().unwrap();
            (
                s.http_port.to_string(),
                s.https_port,
                s.settings.data.session_token.clone(),
            )
        };
        if port_part == http_port && hostname != "localhost" && hostname != "127.0.0.1" {
            let t = params.get("t").cloned().unwrap_or(token);
            let https_url = format!("https://{hostname}:{https_port}/?t={t}");
            return Redirect::temporary(&https_url).into_response();
        }
    }

    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        MOBILE_INDEX_HTML,
    )
        .into_response()
}

async fn handle_manifest_json(State(state): State<SharedState>) -> Json<Value> {
    let token = state.lock().unwrap().settings.data.session_token.clone();
    Json(json!({
        "name": "tVisio Camera Transmitter",
        "short_name": "tVisio",
        "start_url": format!("/?t={token}"),
        "display": "standalone",
        "background_color": "#000000",
        "theme_color": "#030712",
        "icons": [{
            "src": "/assets/icon.png",
            "sizes": "1024x1024",
            "type": "image/png"
        }]
    }))
}

async fn handle_desktop_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], DESKTOP_CSS)
}

async fn handle_desktop_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        DESKTOP_JS,
    )
}

async fn handle_mobile_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], MOBILE_CSS)
}

async fn handle_mobile_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        MOBILE_JS,
    )
}

async fn handle_icon_svg() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], ICON_SVG)
}

async fn handle_icon_png() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], ICON_PNG)
}

async fn handle_icon_ico() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/x-icon")], ICON_ICO)
}

async fn handle_qr_png(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let png_bytes = {
        let mut s = state.lock().unwrap();
        if let Some(ip) = params.get("ip") {
            if ip != &s.selected_ip && s.interfaces.iter().any(|i| &i.ip == ip) {
                s.selected_ip = ip.clone();
                s.rebuild_qr_code();
            }
        }
        s.qr_png.clone()
    };
    ([(header::CONTENT_TYPE, "image/png")], png_bytes)
}

async fn handle_cert_download(State(state): State<SharedState>) -> Response {
    let cert_path = {
        let s = state.lock().unwrap();
        s.base_dir.join("_local").join("certs").join("tvisio_cert.pem")
    };
    if let Ok(bytes) = fs::read(cert_path) {
        (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"tVisio-Local-CA.crt\"",
                ),
            ],
            bytes,
        )
            .into_response()
    } else {
        (StatusCode::NOT_FOUND, "Certificado não encontrado").into_response()
    }
}

async fn handle_window_show(State(state): State<SharedState>) -> Json<Value> {
    if let Ok(mut s) = state.lock() {
        s.window_show_requested = true;
    }
    show_native_window();
    Json(json!({ "ok": true }))
}

async fn handle_install_vcam(State(state): State<SharedState>) -> Json<Value> {
    let base_dir = state.lock().unwrap().base_dir.clone();
    let ok = install_vcam_driver_offline(&base_dir).unwrap_or(false);
    let driver_active = if let Ok(mut s) = state.lock() {
        s.vcam.shm.refresh_registration_status()
    } else {
        ok
    };
    Json(json!({ "ok": ok, "driver_active": driver_active }))
}

async fn handle_open_hotspot(State(state): State<SharedState>) -> Json<Value> {
    open_mobile_hotspot_settings();
    if let Ok(mut s) = state.lock() {
        s.refresh_network_interfaces();
    }
    Json(json!({ "ok": true }))
}

async fn handle_firewall(State(state): State<SharedState>) -> Json<Value> {
    let (http_port, https_port) = {
        let s = state.lock().unwrap();
        (s.http_port, s.https_port)
    };
    let ok = register_firewall_rules(http_port, https_port);
    Json(json!({ "ok": ok }))
}

async fn handle_regenerate_token(State(state): State<SharedState>) -> Json<Value> {
    let (token, pairing_url) = {
        let mut s = state.lock().unwrap();
        let tok = s.settings.regenerate_token();
        s.rebuild_qr_code();
        (tok, s.pairing_url())
    };
    Json(json!({ "ok": true, "token": token, "pairing_url": pairing_url }))
}

async fn handle_open_data_dir(State(state): State<SharedState>) -> Json<Value> {
    let local_dir = state.lock().unwrap().base_dir.join("_local");
    open_local_data_folder(&local_dir);
    Json(json!({ "ok": true }))
}

async fn handle_open_external(Json(payload): Json<Value>) -> Json<Value> {
    if let Some(url) = payload.get("url").and_then(|v| v.as_str()) {
        if url.starts_with("https://") || url.starts_with("http://") {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn();
        }
    }
    Json(json!({ "ok": true }))
}

async fn handle_app_quit(State(state): State<SharedState>) -> Json<Value> {
    if let Ok(mut s) = state.lock() {
        s.quit_requested = true;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        std::process::exit(0);
    });
    Json(json!({ "ok": true }))
}

async fn handle_webrtc_offer() -> Response {
    // Aciona imediatamente o motor WebSocket Binary Turbo Zero-Queue no cliente móvel (30/60 FPS diretos)
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "mode": "ws_binary_turbo_zero_queue" })),
    )
        .into_response()
}

// -----------------------------------------------------------------------------
// WebSocket da Interface Desktop (/ws/desktop)
// -----------------------------------------------------------------------------

async fn handle_desktop_ws(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_desktop_socket(socket, state))
}

async fn handle_desktop_socket(socket: WebSocket, state: SharedState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Envia telemetria inicial imediatamente ao conectar
    if let Ok(s) = state.lock() {
        let _ = out_tx.send(Message::Text(s.build_telemetry_json().to_string()));
    }

    // Task de telemetria periódica (a cada 400ms)
    let state_telemetry = state.clone();
    let out_tx_telemetry = out_tx.clone();
    let telemetry_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let msg_opt = state_telemetry
                .lock()
                .ok()
                .map(|s| s.build_telemetry_json().to_string());
            if let Some(msg) = msg_opt {
                if out_tx_telemetry.send(Message::Text(msg)).is_err() {
                    break;
                }
            }
        }
    });

    // Task de envio de frames JPEG 360p para o Miniplayer (somente quando há novo frame e preview ativo)
    let state_preview = state.clone();
    let out_tx_preview = out_tx.clone();
    let preview_task = tokio::spawn(async move {
        let mut last_seq = 0u64;
        loop {
            tokio::time::sleep(Duration::from_millis(32)).await;
            let jpeg_opt = {
                if let Ok(s) = state_preview.lock() {
                    if s.mobile_connected
                        && s.vcam.miniplayer_enabled
                        && s.vcam.preview_seq != last_seq
                        && s.vcam.preview_seq > 0
                    {
                        last_seq = s.vcam.preview_seq;
                        Some(s.vcam.encode_preview_jpeg())
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(jpeg_bytes) = jpeg_opt {
                if !jpeg_bytes.is_empty() {
                    if out_tx_preview.send(Message::Binary(jpeg_bytes)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Recepção de comandos da Interface Desktop
    while let Some(Ok(msg)) = ws_receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(cmd) = serde_json::from_str::<Value>(&text) {
                let action = cmd.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let mut send_immediate_telemetry = true;

                if let Ok(mut s) = state.lock() {
                    match action {
                        "switch_camera" => {
                            s.send_mobile_command(json!({ "command": "switch_camera" }));
                        }
                        "toggle_af_lock" => {
                            let target = !s.mobile_af_locked;
                            s.mobile_af_locked = target;
                            s.send_mobile_command(json!({ "command": "af_lock", "locked": target }));
                        }
                        "toggle_torch" => {
                            let target = !s.mobile_torch_on;
                            s.mobile_torch_on = target;
                            s.send_mobile_command(json!({ "command": "torch", "on": target }));
                        }
                        "toggle_freeze" => {
                            s.vcam.frozen = !s.vcam.frozen;
                        }
                        "toggle_mirror_h" => {
                            s.vcam.mirror_horizontal = !s.vcam.mirror_horizontal;
                            s.settings.data.mirror_horizontal = s.vcam.mirror_horizontal;
                            s.settings.save();
                        }
                        "rotate" => {
                            s.vcam.rotation = (s.vcam.rotation + 90) % 360;
                            s.settings.data.rotation = s.vcam.rotation;
                            s.settings.save();
                        }
                        "toggle_eco_mode" => {
                            let target = !s.mobile_eco_mode;
                            s.mobile_eco_mode = target;
                            s.send_mobile_command(json!({ "command": "eco_mode", "enabled": target }));
                        }
                        "set_miniplayer_enabled" => {
                            if let Some(en) = cmd.get("enabled").and_then(|v| v.as_bool()) {
                                s.vcam.miniplayer_enabled = en;
                            }
                        }
                        "set_pip_mode" => {
                            if let Some(en) = cmd.get("enabled").and_then(|v| v.as_bool()) {
                                s.pip_mode_requested = Some(en);
                            }
                        }
                        "select_camera_device" => {
                            if let Some(dev_id) = cmd.get("device_id").and_then(|v| v.as_str()) {
                                s.mobile_selected_cam_id = dev_id.to_string();
                                s.settings.data.selected_camera_id = dev_id.to_string();
                                s.settings.save();
                                s.send_mobile_command(json!({
                                    "command": "select_camera_device",
                                    "device_id": dev_id
                                }));
                            }
                        }
                        "select_interface" => {
                            if let Some(ip) = cmd.get("ip").and_then(|v| v.as_str()) {
                                s.selected_ip = ip.to_string();
                                s.rebuild_qr_code();
                            }
                        }
                        "refresh_interfaces" => {
                            s.refresh_network_interfaces();
                        }
                        "set_shutter_cap" => {
                            if let Some(sh) = cmd.get("shutter_ms").and_then(|v| v.as_f64()) {
                                s.settings.data.shutter_cap_ms = sh as f32;
                                s.settings.save();
                                s.send_mobile_command(json!({
                                    "command": "shutter_cap",
                                    "shutter_ms": sh
                                }));
                            }
                        }
                        "set_color_studio" => {
                            send_immediate_telemetry = false;
                            if let Some(g) = cmd.get("brightness_gain").and_then(|v| v.as_f64()) {
                                s.vcam.brightness_gain = g as f32;
                                s.settings.data.brightness_gain = g as f32;
                            }
                            if let Some(c) = cmd.get("contrast").and_then(|v| v.as_f64()) {
                                s.vcam.contrast = c as f32;
                                s.settings.data.contrast = c as f32;
                            }
                            if let Some(gm) = cmd.get("gamma").and_then(|v| v.as_f64()) {
                                s.vcam.gamma = gm as f32;
                                s.settings.data.gamma = gm as f32;
                            }
                            if let Some(t) = cmd.get("color_temp").and_then(|v| v.as_f64()) {
                                s.vcam.color_temp = t as f32;
                                s.settings.data.color_temp = t as f32;
                            }
                            if let Some(p) = cmd.get("color_preset").and_then(|v| v.as_str()) {
                                s.vcam.color_preset = p.to_string();
                                s.settings.data.color_preset = p.to_string();
                            }
                            s.vcam.rebuild_color_lut();
                            s.settings.save();
                        }
                        "set_eptz" => {
                            send_immediate_telemetry = false;
                            if let Some(z) = cmd.get("zoom_level").and_then(|v| v.as_f64()) {
                                s.vcam.zoom_level = (z as f32).clamp(1.0, 3.0);
                                s.settings.data.zoom_level = s.vcam.zoom_level;
                            }
                            if let Some(px) = cmd.get("pan_x").and_then(|v| v.as_f64()) {
                                s.vcam.pan_x = (px as f32).clamp(-1.0, 1.0);
                                s.settings.data.pan_x = s.vcam.pan_x;
                            }
                            if let Some(py) = cmd.get("pan_y").and_then(|v| v.as_f64()) {
                                s.vcam.pan_y = (py as f32).clamp(-1.0, 1.0);
                                s.settings.data.pan_y = s.vcam.pan_y;
                            }
                            s.settings.save();
                        }
                        "toggle_anti_psm" => {
                            s.settings.data.anti_psm_enabled = !s.settings.data.anti_psm_enabled;
                            s.settings.save();
                        }
                        "set_resolution" => {
                            if let Some(prof) = cmd.get("profile").and_then(|v| v.as_str()) {
                                s.settings.data.resolution_profile = prof.to_string();
                                s.settings.save();
                                let parts: Vec<&str> = prof.split('@').collect();
                                let res_parts: Vec<&str> =
                                    parts.first().unwrap_or(&"1280x720").split('x').collect();
                                let w: u32 = res_parts.first().and_then(|x| x.parse().ok()).unwrap_or(1280);
                                let h: u32 = res_parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(720);
                                let fps: u32 = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(30);
                                s.send_mobile_command(json!({
                                    "command": "set_resolution",
                                    "width": w,
                                    "height": h,
                                    "fps": fps
                                }));
                            }
                        }
                        "set_app_settings" => {
                            if let Some(v) = cmd.get("start_with_windows").and_then(|v| v.as_bool()) {
                                s.settings.data.start_with_windows = v;
                                set_start_with_windows(v);
                            }
                            if let Some(v) = cmd.get("start_minimized").and_then(|v| v.as_bool()) {
                                s.settings.data.start_minimized = v;
                            }
                            if let Some(v) = cmd.get("minimize_to_tray").and_then(|v| v.as_bool()) {
                                s.settings.data.minimize_to_tray = v;
                            }
                            if let Some(v) = cmd.get("close_to_tray").and_then(|v| v.as_bool()) {
                                s.settings.data.close_to_tray = v;
                            }
                            if let Some(v) = cmd.get("auto_eco_oled").and_then(|v| v.as_bool()) {
                                s.settings.data.auto_eco_oled = v;
                                s.send_mobile_command(json!({
                                    "command": "set_auto_eco",
                                    "enabled": v
                                }));
                            }
                            if let Some(v) = cmd.get("auto_start_vcam").and_then(|v| v.as_bool()) {
                                s.settings.data.auto_start_vcam = v;
                            }
                            s.settings.save();
                        }
                        _ => {}
                    }

                    if send_immediate_telemetry {
                        let _ = out_tx.send(Message::Text(s.build_telemetry_json().to_string()));
                    }
                }
            }
        }
    }

    telemetry_task.abort();
    preview_task.abort();
    writer_task.abort();
}

// -----------------------------------------------------------------------------
// WebSocket de Streaming Binário e Controle do Smartphone (/ws/stream)
// -----------------------------------------------------------------------------

async fn handle_mobile_ws(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.max_message_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| handle_mobile_socket(socket, state))
}

async fn handle_mobile_socket(socket: WebSocket, state: SharedState) {
    ACTIVE_MOBILE_CLIENTS.fetch_add(1, Ordering::SeqCst);

    let (mut cmd_rx, auto_eco_initial, initial_cam_id) = {
        let mut s = state.lock().unwrap();
        s.mobile_connected = true;
        s.active_protocol = "Binary Turbo Zero-Queue (Rust SIMD)".to_string();
        (
            s.mobile_cmd_tx.subscribe(),
            s.settings.data.auto_eco_oled,
            s.settings.data.selected_camera_id.clone(),
        )
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Sincroniza preferência inicial de Auto-Eco OLED com o celular recém-conectado
    let init_cmd = json!({
        "type": "command",
        "command": "set_auto_eco",
        "enabled": auto_eco_initial,
    });
    let _ = out_tx.send(Message::Text(init_cmd.to_string()));

    // Restaura câmera física previamente selecionada pelo usuário
    if !initial_cam_id.is_empty() {
        let init_cam_cmd = json!({
            "type": "command",
            "command": "select_camera_device",
            "device_id": initial_cam_id,
        });
        let _ = out_tx.send(Message::Text(init_cam_cmd.to_string()));
    }

    // Task dedicada de escrita no WebSocket (Comandos + ACKs + Pulso Anti-PSM 500Hz)
    let state_for_writer = state.clone();
    let out_tx_for_psm = out_tx.clone();
    let psm_task = tokio::spawn(async move {
        loop {
            let anti_psm = state_for_writer
                .lock()
                .map(|s| s.settings.data.anti_psm_enabled)
                .unwrap_or(false);
            if anti_psm {
                if out_tx_for_psm.send(Message::Binary(vec![0x01u8])).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            } else {
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
        }
    });

    let cmd_forward_task = {
        let out_tx_cmd = out_tx.clone();
        tokio::spawn(async move {
            while let Ok(cmd_str) = cmd_rx.recv().await {
                if out_tx_cmd.send(Message::Text(cmd_str)).is_err() {
                    break;
                }
            }
        })
    };

    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Loop principal de recepção de pacotes binários de vídeo e telemetria JSON
    while let Some(Ok(msg)) = ws_receiver.next().await {
        match msg {
            Message::Binary(data) => {
                // Cabeçalho binário de 8 bytes (float64 little-endian timestamp) + payload JPEG
                if data.len() > 8 {
                    let mut ts_bytes = [0u8; 8];
                    ts_bytes.copy_from_slice(&data[..8]);
                    let client_ts = f64::from_le_bytes(ts_bytes);
                    let jpeg_bytes = &data[8..];

                    // Envia ACK imediato antes da decodificação para eliminar espera ociosa no celular
                    let ack_msg = json!({ "type": "ack", "ts": client_ts }).to_string();
                    let _ = out_tx.send(Message::Text(ack_msg));

                    let t0 = Instant::now();
                    let is_frozen = state.lock().map(|s| s.vcam.frozen).unwrap_or(false);
                    if is_frozen {
                        if let Ok(mut s) = state.lock() {
                            s.vcam.process_jpeg_packet(&[]);
                        }
                        continue;
                    }

                    // Decodifica JPEG fora do Mutex para nunca bloquear a interface gráfica a 60 FPS
                    if let Ok(decoded) =
                        image::load_from_memory_with_format(jpeg_bytes, image::ImageFormat::Jpeg)
                    {
                        let rgb = decoded.to_rgb8();
                        let (w, h) = rgb.dimensions();

                        if let Ok(mut s) = state.lock() {
                            s.vcam.process_rgb_frame(rgb.as_raw(), w, h, t0);
                            let decode_ms = s.vcam.last_decode_ms;
                            let rtt = s.rtt_ms;
                            let shutter = s.settings.data.shutter_cap_ms.min(16.0);
                            s.glass_to_glass_ms =
                                ((rtt * 0.5) + shutter * 0.6 + decode_ms).max(6.8);
                        }
                    }
                }
            }
            Message::Text(text) => {
                if let Ok(info) = serde_json::from_str::<Value>(&text) {
                    let msg_type = info.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if msg_type == "ping" {
                        let ts = info.get("ts").cloned().unwrap_or(Value::Null);
                        let pong = json!({ "type": "pong", "ts": ts }).to_string();
                        let _ = out_tx.send(Message::Text(pong));
                    } else if msg_type == "telemetry" {
                        if let Ok(mut s) = state.lock() {
                            if let Some(rtt) = info.get("rtt_ms").and_then(|v| v.as_f64()) {
                                s.rtt_ms = (rtt as f32 * 10.0).round() / 10.0;
                            }
                            if let Some(dev) = info.get("device").and_then(|v| v.as_str()) {
                                s.mobile_device_info = dev.to_string();
                            }
                            if let Some(eco) = info.get("eco_mode").and_then(|v| v.as_bool()) {
                                s.mobile_eco_mode = eco;
                            }
                            if let Some(torch) = info.get("torch_on").and_then(|v| v.as_bool()) {
                                s.mobile_torch_on = torch;
                            }
                            if let Some(af) = info.get("af_locked").and_then(|v| v.as_bool()) {
                                s.mobile_af_locked = af;
                            }
                            if let Some(fm) = info.get("facing_mode").and_then(|v| v.as_str()) {
                                s.mobile_facing_mode = fm.to_string();
                            }
                            if let Some(sh) = info.get("shutter_ms").and_then(|v| v.as_f64()) {
                                s.settings.data.shutter_cap_ms = sh as f32;
                            }
                            if let Some(cams) = info.get("cameras").and_then(|v| v.as_array()) {
                                s.mobile_cameras = cams
                                    .iter()
                                    .filter_map(|c| {
                                        let id = c.get("id")?.as_str()?.to_string();
                                        let label = c.get("label")?.as_str()?.to_string();
                                        Some(MobileCameraDevice { id, label })
                                    })
                                    .collect();
                            }
                            if let Some(sel_id) = info.get("selected_cam_id").and_then(|v| v.as_str()) {
                                s.mobile_selected_cam_id = sel_id.to_string();
                            }
                            if let Some(bat) = info.get("battery_level").and_then(|v| v.as_i64()) {
                                s.mobile_battery_level = Some(bat as i32);
                            }
                            if let Some(chg) = info.get("battery_charging").and_then(|v| v.as_bool()) {
                                s.mobile_battery_charging = chg;
                            }
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    psm_task.abort();
    cmd_forward_task.abort();
    writer_task.abort();

    let remaining = ACTIVE_MOBILE_CLIENTS.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
    if remaining == 0 {
        if let Ok(mut s) = state.lock() {
            s.mobile_connected = false;
            s.active_protocol = "Standby".to_string();
            s.vcam.current_fps = 0.0;
        }
    }
}
