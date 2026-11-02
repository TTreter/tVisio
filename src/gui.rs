//! tVisio — Interface Gráfica Desktop (Translucent Glass UI)
//! =======================================================================================
//! Renderiza a interface desktop acelerada por hardware via OpenGL (`eframe`/`egui`):
//! - Miniplayer 16:9 acelerado por GPU com upload direto de textura RGBA.
//! - Efeito Ambient Video Glow, Grade Regra dos Terços, Zoom/Pan Digital ePTZ via Mouse.
//! - Dock Translúcida com 7 controles ópticos e de hardware móvel.
//! - Abas Segmentadas (Conexão QR Code, Estúdio L1-LUT e Ajustes do Sistema Windows).
//! - Integração com a Bandeja do Sistema (`tray-icon`) para Minimizar e Fechar para a Bandeja.

use crate::server::SharedState;
use crate::settings::ICON_PNG;
use crate::vcam_engine::{AMBIENT_HEIGHT, AMBIENT_WIDTH, PREVIEW_HEIGHT, PREVIEW_WIDTH};
use crate::win_integration::{
    create_system_tray, hide_native_window_to_tray, install_vcam_driver_offline,
    is_native_window_minimized, open_local_data_folder, open_mobile_hotspot_settings,
    register_firewall_rules, set_start_with_windows, show_native_window, SystemTrayHandles,
    MENU_ID_CLOSE_TRAY, MENU_ID_ECO, MENU_ID_FREEZE, MENU_ID_OPEN, MENU_ID_QUIT, MENU_ID_STARTUP,
};
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Frame, Id, Key,
    Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use serde_json::json;
use std::time::{Duration, Instant};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Connection,
    Studio,
    Settings,
}

pub struct TVisioGuiApp {
    state: SharedState,
    tray: Option<SystemTrayHandles>,

    // Estado da Interface
    active_tab: SidebarTab,
    show_grid: bool,
    pip_mode: bool,
    hidden_in_tray: bool,
    status_toast: Option<(String, Instant)>,

    // Texturas na GPU
    logo_texture: Option<TextureHandle>,
    qr_texture: Option<TextureHandle>,
    last_qr_seq: u64,
    preview_texture: Option<TextureHandle>,
    ambient_texture: Option<TextureHandle>,
    last_preview_seq: u64,
}

impl TVisioGuiApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        state: SharedState,
        start_minimized: bool,
    ) -> Self {
        Self::configure_translucent_glass_theme(&cc.egui_ctx);

        let (start_with_win, close_to_tray) = {
            let s = state.lock().unwrap();
            (s.settings.data.start_with_windows, s.settings.data.close_to_tray)
        };

        let tray = create_system_tray(start_with_win, close_to_tray);

        let logo_texture = image::load_from_memory(ICON_PNG).ok().map(|img| {
            let resized = img.resize_exact(96, 96, image::imageops::FilterType::Lanczos3);
            let rgba = resized.to_rgba8();
            let color_img = ColorImage::from_rgba_unmultiplied([96, 96], rgba.as_raw());
            cc.egui_ctx
                .load_texture("tvisio_logo", color_img, TextureOptions::LINEAR)
        });

        let mut app = Self {
            state,
            tray,
            active_tab: SidebarTab::Connection,
            show_grid: false,
            pip_mode: false,
            hidden_in_tray: false,
            status_toast: None,
            logo_texture,
            qr_texture: None,
            last_qr_seq: 0,
            preview_texture: None,
            ambient_texture: None,
            last_preview_seq: 0,
        };

        if start_minimized {
            app.hidden_in_tray = true;
            hide_native_window_to_tray();
        }

        app
    }

    fn configure_translucent_glass_theme(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(3, 7, 18); // #030712 Obsidian Deep
        visuals.window_fill = Color32::from_rgb(9, 14, 28);
        visuals.faint_bg_color = Color32::from_rgb(13, 21, 38);
        visuals.extreme_bg_color = Color32::from_rgb(5, 10, 22);

        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(11, 18, 34);
        visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 59));
        visuals.widgets.noninteractive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240));
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        visuals.widgets.inactive.bg_fill = Color32::from_rgb(15, 24, 44);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(38, 54, 82));
        visuals.widgets.inactive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240));
        visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

        visuals.widgets.hovered.bg_fill = Color32::from_rgb(22, 36, 64);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0, 240, 255));
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

        visuals.widgets.active.bg_fill = Color32::from_rgb(0, 175, 205);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0, 240, 255));
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(3, 7, 18));
        visuals.widgets.active.corner_radius = CornerRadius::same(8);

        visuals.selection.bg_fill = Color32::from_rgb(0, 185, 215);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(3, 7, 18));

        ctx.set_visuals(visuals);
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), Instant::now()));
    }

    /// Botão estilizado em vidro translúcido com estado ativo/inativo sempre legível.
    fn glass_button(
        ui: &mut egui::Ui,
        label: &str,
        active: bool,
        size: Vec2,
    ) -> egui::Response {
        let actual_size = Vec2::new(
            if size.x <= 0.0 {
                ui.available_width()
            } else {
                size.x
            },
            size.y,
        );
        let (bg, fg, border) = if active {
            (
                Color32::from_rgba_unmultiplied(0, 220, 250, 42),
                Color32::from_rgb(0, 240, 255),
                Color32::from_rgb(0, 240, 255),
            )
        } else {
            (
                Color32::from_rgb(15, 24, 44),
                Color32::from_rgb(226, 232, 240),
                Color32::from_rgb(36, 52, 80),
            )
        };

        let btn = egui::Button::new(RichText::new(label).size(12.0).strong().color(fg))
            .fill(bg)
            .stroke(Stroke::new(1.0_f32, border))
            .corner_radius(CornerRadius::same(8))
            .min_size(actual_size);

        ui.add_sized(actual_size, btn)
    }

    fn poll_tray_and_window_events(&mut self, ctx: &egui::Context) {
        // 1. Clique direto no ícone da Bandeja do Sistema -> Restaura a janela
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                self.restore_window(ctx);
            }
        }

        // 2. Itens do menu de contexto da Bandeja do Sistema
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.0.as_str() {
                MENU_ID_OPEN => {
                    self.restore_window(ctx);
                }
                MENU_ID_ECO => {
                    let s = self.state.lock().unwrap();
                    s.send_mobile_command(json!({ "command": "toggle_eco_mode" }));
                }
                MENU_ID_FREEZE => {
                    let mut s = self.state.lock().unwrap();
                    s.vcam.frozen = !s.vcam.frozen;
                }
                MENU_ID_STARTUP => {
                    let mut s = self.state.lock().unwrap();
                    let next = !s.settings.data.start_with_windows;
                    if set_start_with_windows(next) {
                        s.settings.data.start_with_windows = next;
                        s.settings.save();
                    }
                }
                MENU_ID_CLOSE_TRAY => {
                    let mut s = self.state.lock().unwrap();
                    s.settings.data.close_to_tray = !s.settings.data.close_to_tray;
                    s.settings.save();
                }
                MENU_ID_QUIT => {
                    {
                        let mut s = self.state.lock().unwrap();
                        s.quit_requested = true;
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                _ => {}
            }
        }

        // 3. Sincroniza checkboxes do menu da bandeja e verifica pedidos externos de restauração
        let (want_show, want_quit, eco_on, frozen, startup, close_tray, min_to_tray) = {
            let mut s = self.state.lock().unwrap();
            let ws = s.window_show_requested;
            s.window_show_requested = false;
            (
                ws,
                s.quit_requested,
                s.mobile_eco_mode,
                s.vcam.frozen,
                s.settings.data.start_with_windows,
                s.settings.data.close_to_tray,
                s.settings.data.minimize_to_tray,
            )
        };

        if let Some(tray) = &self.tray {
            tray.item_eco.set_checked(eco_on);
            tray.item_freeze.set_checked(frozen);
            tray.item_startup.set_checked(startup);
            tray.item_close_tray.set_checked(close_tray);
        }

        if want_show {
            self.restore_window(ctx);
        }

        // 4. Se o usuário minimizou a janela e `minimize_to_tray` está ativo -> oculta da barra de tarefas
        if !self.hidden_in_tray && min_to_tray && is_native_window_minimized() {
            self.hidden_in_tray = true;
            hide_native_window_to_tray();
        }

        // 5. Intercepta o clique no botão (X) para fechar para a bandeja sem encerrar o motor
        if ctx.input(|i| i.viewport().close_requested()) && !want_quit {
            if close_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hidden_in_tray = true;
                hide_native_window_to_tray();
            }
        }
    }

    fn restore_window(&mut self, ctx: &egui::Context) {
        self.hidden_in_tray = false;
        show_native_window();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn sync_gpu_textures(&mut self, ctx: &egui::Context) {
        let mut s = self.state.lock().unwrap();

        // Atualiza textura do QR Code somente quando o IP ou Token mudar
        if self.qr_texture.is_none() || self.last_qr_seq != s.qr_seq {
            let dim = s.qr_dim as usize;
            if dim > 0 && s.qr_rgba.len() == dim * dim * 4 {
                let img = ColorImage::from_rgba_unmultiplied([dim, dim], &s.qr_rgba);
                self.qr_texture =
                    Some(ctx.load_texture("tvisio_qr", img, TextureOptions::NEAREST));
                self.last_qr_seq = s.qr_seq;
            }
        }

        // Atualiza texturas do Miniplayer somente quando um novo quadro chegar
        if s.vcam.miniplayer_enabled && self.last_preview_seq != s.vcam.preview_seq {
            let pw = PREVIEW_WIDTH as usize;
            let ph = PREVIEW_HEIGHT as usize;
            if s.vcam.preview_rgba.len() == pw * ph * 4 {
                let p_img = ColorImage::from_rgba_unmultiplied([pw, ph], &s.vcam.preview_rgba);
                if let Some(tex) = &mut self.preview_texture {
                    tex.set(p_img, TextureOptions::LINEAR);
                } else {
                    self.preview_texture =
                        Some(ctx.load_texture("tvisio_preview", p_img, TextureOptions::LINEAR));
                }
            }

            let aw = AMBIENT_WIDTH as usize;
            let ah = AMBIENT_HEIGHT as usize;
            if s.vcam.ambient_rgba.len() == aw * ah * 4 {
                let a_img = ColorImage::from_rgba_unmultiplied([aw, ah], &s.vcam.ambient_rgba);
                if let Some(tex) = &mut self.ambient_texture {
                    tex.set(a_img, TextureOptions::LINEAR);
                } else {
                    self.ambient_texture =
                        Some(ctx.load_texture("tvisio_ambient", a_img, TextureOptions::LINEAR));
                }
            }

            self.last_preview_seq = s.vcam.preview_seq;
        }

        // Verifica periodicamente o estado do driver DirectShow
        if s.vcam.frames_received % 120 == 0 {
            s.vcam.shm.refresh_registration_status();
        }
    }

    /// Desenha um card de métrica compacto com tamanho fixo via Painter (imune a distorções de layout).
    fn draw_metric_box(
        ui: &mut egui::Ui,
        label: &str,
        value: &str,
        val_color: Color32,
        width: f32,
    ) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(9), Color32::from_rgb(13, 22, 42));
        painter.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0_f32, Color32::from_rgb(30, 45, 72)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.top() + 11.0),
            Align2::LEFT_CENTER,
            label,
            FontId::new(9.0, FontFamily::Proportional),
            Color32::from_rgb(115, 132, 158),
        );
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.bottom() - 11.0),
            Align2::LEFT_CENTER,
            value,
            FontId::new(12.5, FontFamily::Proportional),
            val_color,
        );
    }

    /// Desenha a pílula de status de conexão com indicador circular vetorial.
    fn draw_connection_badge(ui: &mut egui::Ui, connected: bool) {
        let (bg, border, fg, text) = if connected {
            (
                Color32::from_rgba_unmultiplied(16, 185, 129, 32),
                Color32::from_rgb(16, 185, 129),
                Color32::from_rgb(52, 211, 153),
                "Ao Vivo",
            )
        } else {
            (
                Color32::from_rgb(13, 22, 42),
                Color32::from_rgb(45, 62, 92),
                Color32::from_rgb(148, 163, 184),
                "Standby",
            )
        };

        let width = if connected { 92.0 } else { 88.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(17), bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(17),
            Stroke::new(1.0_f32, border),
            egui::StrokeKind::Inside,
        );

        let dot_center = Pos2::new(rect.left() + 15.0, rect.center().y);
        painter.circle_filled(dot_center, 4.0, fg);

        painter.text(
            Pos2::new(rect.left() + 26.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::new(12.0, FontFamily::Proportional),
            fg,
        );
    }

    fn render_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (connected, device_info, g2g, rtt, fps, bat_lvl, bat_chg) = {
            let s = self.state.lock().unwrap();
            (
                s.mobile_connected,
                s.mobile_device_info.clone(),
                s.glass_to_glass_ms,
                s.rtt_ms,
                s.vcam.current_fps,
                s.mobile_battery_level,
                s.mobile_battery_charging,
            )
        };

        Frame::new()
            .fill(Color32::from_rgb(9, 15, 30))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 42, 68)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(16, 10))
            .show(ui, |ui| {
                ui.set_height(42.0);
                ui.horizontal_centered(|ui| {
                    // Lado Esquerdo: Ícone + Marca + Subtítulo
                    if let Some(logo) = &self.logo_texture {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                        ui.painter().image(
                            logo.id(),
                            rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        ui.add_space(6.0);
                    }

                    ui.allocate_ui_with_layout(
                        Vec2::new(320.0, 40.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("tVisio")
                                        .size(19.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                Frame::new()
                                    .fill(Color32::from_rgba_unmultiplied(0, 240, 255, 28))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0, 210, 235)))
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(Margin::symmetric(8, 2))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ZERO-OVERHEAD ENGINE")
                                                .size(9.5)
                                                .strong()
                                                .color(Color32::from_rgb(0, 240, 255)),
                                        );
                                    });
                            });

                            let sub_text = if connected {
                                format!("Conectado: {device_info}")
                            } else {
                                "Aguardando pareamento via QR Code...".to_string()
                            };
                            ui.label(
                                RichText::new(sub_text)
                                    .size(11.5)
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        },
                    );

                    // Lado Direito: Chips de Telemetria com tamanho fixo + Botões
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let pip_label = if self.pip_mode { "Expandir" } else { "PiP" };
                        if Self::glass_button(ui, pip_label, self.pip_mode, Vec2::new(62.0, 34.0))
                            .on_hover_text("Alternar Modo Miniplayer Compacto (Picture-in-Picture)")
                            .clicked()
                        {
                            self.pip_mode = !self.pip_mode;
                            let target_size = if self.pip_mode {
                                Vec2::new(640.0, 480.0)
                            } else {
                                Vec2::new(1180.0, 740.0)
                            };
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target_size));
                        }

                        if Self::glass_button(
                            ui,
                            "⚙",
                            self.active_tab == SidebarTab::Settings,
                            Vec2::new(36.0, 34.0),
                        )
                        .on_hover_text("Abrir Aba de Ajustes do Sistema")
                        .clicked()
                        {
                            self.pip_mode = false;
                            self.active_tab = SidebarTab::Settings;
                        }

                        if !self.pip_mode {
                            let bat_str = match bat_lvl {
                                Some(b) => {
                                    if bat_chg {
                                        format!("⚡ {b}%")
                                    } else {
                                        format!("{b}%")
                                    }
                                }
                                None => "--%".to_string(),
                            };
                            Self::draw_metric_box(
                                ui,
                                "BATERIA",
                                &bat_str,
                                Color32::from_rgb(226, 232, 240),
                                78.0,
                            );

                            let fps_str = if connected {
                                format!("{fps:.0} FPS")
                            } else {
                                "0 FPS".to_string()
                            };
                            Self::draw_metric_box(
                                ui,
                                "TAXA",
                                &fps_str,
                                Color32::from_rgb(226, 232, 240),
                                74.0,
                            );

                            let rtt_str = if connected {
                                format!("{rtt:.1} ms")
                            } else {
                                "-- ms".to_string()
                            };
                            Self::draw_metric_box(
                                ui,
                                "PING RÁDIO",
                                &rtt_str,
                                Color32::from_rgb(226, 232, 240),
                                86.0,
                            );

                            let g2g_str = if connected {
                                format!("⚡ {g2g:.1} ms")
                            } else {
                                "-- ms".to_string()
                            };
                            Self::draw_metric_box(
                                ui,
                                "G2G DELAY",
                                &g2g_str,
                                Color32::from_rgb(0, 240, 255),
                                92.0,
                            );
                        }

                        Self::draw_connection_badge(ui, connected);
                    });
                });
            });
    }

    fn render_miniplayer_card(&mut self, ui: &mut egui::Ui) {
        let (
            connected,
            protocol,
            zoom_level,
            preview_enabled,
            frozen,
            af_locked,
            torch_on,
            mirror_h,
            eco_mode,
            driver_registered,
            backend_label,
            res_label,
            decode_ms,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.mobile_connected,
                s.active_protocol.clone(),
                s.vcam.zoom_level,
                s.vcam.miniplayer_enabled,
                s.vcam.frozen,
                s.mobile_af_locked,
                s.mobile_torch_on,
                s.vcam.mirror_horizontal,
                s.mobile_eco_mode,
                s.vcam.shm.driver_registered,
                s.vcam.shm.backend_status_label(),
                s.vcam.resolution_label(),
                s.vcam.last_decode_ms,
            )
        };

        Frame::new()
            .fill(Color32::from_rgb(9, 15, 30))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 42, 68)))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                // 1. Cabeçalho do Miniplayer
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(Color32::from_rgb(17, 28, 52))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(32, 54, 90)))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("MINIPLAYER LEVE (360p)")
                                    .size(10.5)
                                    .strong()
                                    .color(Color32::from_rgb(0, 240, 255)),
                            );
                        });

                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(&protocol)
                            .size(11.5)
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    if zoom_level > 1.01 {
                        if Self::glass_button(
                            ui,
                            &format!("🔍 {zoom_level:.1}x"),
                            true,
                            Vec2::new(72.0, 26.0),
                        )
                        .on_hover_text("Clique para voltar ao Zoom 1.0x")
                        .clicked()
                        {
                            let mut s = self.state.lock().unwrap();
                            s.vcam.zoom_level = 1.0;
                            s.vcam.pan_x = 0.0;
                            s.vcam.pan_y = 0.0;
                            s.settings.data.zoom_level = 1.0;
                            s.settings.data.pan_x = 0.0;
                            s.settings.data.pan_y = 0.0;
                            s.settings.save();
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let prev_txt = if preview_enabled {
                            "👁 Preview Ativo"
                        } else {
                            "⚡ Zero-Render"
                        };
                        if Self::glass_button(ui, prev_txt, !preview_enabled, Vec2::new(116.0, 28.0))
                            .on_hover_text(
                                "Desativa a renderização na janela para 0% de uso de GPU na interface",
                            )
                            .clicked()
                        {
                            let mut s = self.state.lock().unwrap();
                            s.vcam.miniplayer_enabled = !s.vcam.miniplayer_enabled;
                        }

                        if Self::glass_button(ui, "# Grade", self.show_grid, Vec2::new(74.0, 28.0))
                            .on_hover_text("Exibir grade da Regra dos Terços")
                            .clicked()
                        {
                            self.show_grid = !self.show_grid;
                        }
                    });
                });

                ui.add_space(10.0);

                // 2. Palco de Vídeo 16:9 Interativo (Scroll = Zoom ePTZ, Arraste = Pan ePTZ)
                let avail_w = ui.available_width();
                let avail_h = (ui.available_height() - 104.0).max(190.0);
                let (stage_rect, stage_resp) =
                    ui.allocate_exact_size(Vec2::new(avail_w, avail_h), Sense::click_and_drag());

                let painter = ui.painter_at(stage_rect);
                painter.rect_filled(
                    stage_rect,
                    CornerRadius::same(12),
                    Color32::from_rgb(4, 8, 18),
                );

                // Interação de Roda do Mouse (Zoom Digital ePTZ 1.0x a 3.0x)
                if stage_resp.hovered() {
                    let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll_y.abs() > 0.1 {
                        let mut s = self.state.lock().unwrap();
                        let delta = if scroll_y > 0.0 { 0.12 } else { -0.12 };
                        s.vcam.zoom_level = (s.vcam.zoom_level + delta).clamp(1.0, 3.0);
                        if s.vcam.zoom_level <= 1.01 {
                            s.vcam.pan_x = 0.0;
                            s.vcam.pan_y = 0.0;
                        }
                        s.settings.data.zoom_level = s.vcam.zoom_level;
                        s.settings.data.pan_x = s.vcam.pan_x;
                        s.settings.data.pan_y = s.vcam.pan_y;
                        s.settings.save();
                    }
                }

                // Interação de Arraste com Mouse (Pan ePTZ)
                if stage_resp.dragged() && zoom_level > 1.01 {
                    let delta = stage_resp.drag_delta();
                    let mut s = self.state.lock().unwrap();
                    let sensitivity = 2.0 / stage_rect.width().max(200.0);
                    s.vcam.pan_x = (s.vcam.pan_x - delta.x * sensitivity).clamp(-1.0, 1.0);
                    s.vcam.pan_y = (s.vcam.pan_y - delta.y * sensitivity).clamp(-1.0, 1.0);
                    s.settings.data.pan_x = s.vcam.pan_x;
                    s.settings.data.pan_y = s.vcam.pan_y;
                }

                // Duplo clique no vídeo reseta o Zoom ePTZ
                if stage_resp.double_clicked() {
                    let mut s = self.state.lock().unwrap();
                    s.vcam.zoom_level = 1.0;
                    s.vcam.pan_x = 0.0;
                    s.vcam.pan_y = 0.0;
                    s.settings.data.zoom_level = 1.0;
                    s.settings.data.pan_x = 0.0;
                    s.settings.data.pan_y = 0.0;
                    s.settings.save();
                }

                // Renderização da Textura de Vídeo ou Overlays de Estado
                if !preview_enabled {
                    painter.text(
                        stage_rect.center() - Vec2::new(0.0, 12.0),
                        Align2::CENTER_CENTER,
                        "⚡ Modo Zero-Render Ativo",
                        FontId::new(18.0, FontFamily::Proportional),
                        Color32::from_rgb(0, 240, 255),
                    );
                    painter.text(
                        stage_rect.center() + Vec2::new(0.0, 16.0),
                        Align2::CENTER_CENTER,
                        "Miniplayer pausado (0% GPU na janela). Sua Webcam Virtual segue transmitindo em qualidade máxima.",
                        FontId::new(12.5, FontFamily::Proportional),
                        Color32::from_rgb(148, 163, 184),
                    );
                } else if connected {
                    if let Some(amb_tex) = &self.ambient_texture {
                        painter.image(
                            amb_tex.id(),
                            stage_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::from_rgba_unmultiplied(255, 255, 255, 48),
                        );
                    }

                    if let Some(prev_tex) = &self.preview_texture {
                        let target_aspect = 16.0 / 9.0;
                        let stage_aspect = stage_rect.width() / stage_rect.height().max(1.0);
                        let (vw, vh) = if stage_aspect > target_aspect {
                            let h = stage_rect.height();
                            (h * target_aspect, h)
                        } else {
                            let w = stage_rect.width();
                            (w, w / target_aspect)
                        };
                        let video_rect =
                            Rect::from_center_size(stage_rect.center(), Vec2::new(vw, vh));
                        painter.image(
                            prev_tex.id(),
                            video_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                } else {
                    if let Some(logo) = &self.logo_texture {
                        let logo_rect = Rect::from_center_size(
                            stage_rect.center() - Vec2::new(0.0, 34.0),
                            Vec2::splat(64.0),
                        );
                        painter.image(
                            logo.id(),
                            logo_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    painter.text(
                        stage_rect.center() + Vec2::new(0.0, 16.0),
                        Align2::CENTER_CENTER,
                        "Pronto para Conectar",
                        FontId::new(18.0, FontFamily::Proportional),
                        Color32::WHITE,
                    );
                    painter.text(
                        stage_rect.center() + Vec2::new(0.0, 40.0),
                        Align2::CENTER_CENTER,
                        "Aponte a câmera do seu celular para o QR Code ao lado para iniciar a transmissão.",
                        FontId::new(12.5, FontFamily::Proportional),
                        Color32::from_rgb(148, 163, 184),
                    );
                }

                // Grade da Regra dos Terços (# Grade)
                if self.show_grid {
                    let grid_stroke =
                        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 240, 255, 90));
                    let w3 = stage_rect.width() / 3.0;
                    let h3 = stage_rect.height() / 3.0;
                    for i in 1..=2 {
                        let x = stage_rect.left() + w3 * (i as f32);
                        painter.line_segment(
                            [
                                Pos2::new(x, stage_rect.top()),
                                Pos2::new(x, stage_rect.bottom()),
                            ],
                            grid_stroke,
                        );
                        let y = stage_rect.top() + h3 * (i as f32);
                        painter.line_segment(
                            [
                                Pos2::new(stage_rect.left(), y),
                                Pos2::new(stage_rect.right(), y),
                            ],
                            grid_stroke,
                        );
                    }
                }

                // Badge de Quadro Congelado (Privacidade)
                if frozen {
                    let badge_rect = Rect::from_center_size(
                        Pos2::new(stage_rect.center().x, stage_rect.top() + 26.0),
                        Vec2::new(240.0, 30.0),
                    );
                    painter.rect_filled(
                        badge_rect,
                        CornerRadius::same(15),
                        Color32::from_rgba_unmultiplied(139, 92, 246, 220),
                    );
                    painter.text(
                        badge_rect.center(),
                        Align2::CENTER_CENTER,
                        "⏸ IMAGEM CONGELADA (0% CPU)",
                        FontId::new(11.5, FontFamily::Proportional),
                        Color32::WHITE,
                    );
                }

                painter.rect_stroke(
                    stage_rect,
                    CornerRadius::same(12),
                    Stroke::new(1.0_f32, Color32::from_rgb(30, 45, 72)),
                    egui::StrokeKind::Inside,
                );

                ui.add_space(10.0);

                // 3. Dock Translúcida de Controle Rápido (Todos os 7 botões com visual uniforme)
                ui.columns(7, |cols| {
                    let full_h = Vec2::new(0.0, 32.0);
                    if Self::glass_button(&mut cols[0], "🔄 Lente", false, full_h)
                        .on_hover_text("Alternar entre as câmeras físicas do smartphone")
                        .clicked()
                    {
                        self.state
                            .lock()
                            .unwrap()
                            .send_mobile_command(json!({ "command": "switch_camera" }));
                    }

                    if Self::glass_button(&mut cols[1], "🎯 Foco", af_locked, full_h)
                        .on_hover_text("Travar Foco Óptico (AF-Lock)")
                        .clicked()
                    {
                        self.state
                            .lock()
                            .unwrap()
                            .send_mobile_command(json!({ "command": "toggle_af_lock" }));
                    }

                    if Self::glass_button(&mut cols[2], "🔦 Luz", torch_on, full_h)
                        .on_hover_text("Ligar / Desligar Lanterna Traseira")
                        .clicked()
                    {
                        self.state
                            .lock()
                            .unwrap()
                            .send_mobile_command(json!({ "command": "toggle_torch" }));
                    }

                    let freeze_txt = if frozen { "▶ Retomar" } else { "⏸ Pausar" };
                    if Self::glass_button(&mut cols[3], freeze_txt, frozen, full_h)
                        .on_hover_text("Congelar quadro instantaneamente (Privacidade)")
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.vcam.frozen = !s.vcam.frozen;
                    }

                    if Self::glass_button(&mut cols[4], "↔ Espelhar", mirror_h, full_h)
                        .on_hover_text("Espelhar imagem horizontalmente")
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.vcam.mirror_horizontal = !s.vcam.mirror_horizontal;
                        s.settings.data.mirror_horizontal = s.vcam.mirror_horizontal;
                        s.settings.save();
                    }

                    let rot_active = {
                        let s = self.state.lock().unwrap();
                        s.vcam.rotation != 0
                    };
                    if Self::glass_button(&mut cols[5], "⟳ Girar", rot_active, full_h)
                        .on_hover_text("Girar imagem em 90°")
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.vcam.rotation = (s.vcam.rotation + 90) % 360;
                        s.settings.data.rotation = s.vcam.rotation;
                        s.settings.save();
                    }

                    if Self::glass_button(&mut cols[6], "🌙 Eco", eco_mode, full_h)
                        .on_hover_text("Modo Eco OLED (Apagar tela do celular mantendo transmissão)")
                        .clicked()
                    {
                        self.state
                            .lock()
                            .unwrap()
                            .send_mobile_command(json!({ "command": "toggle_eco_mode" }));
                    }
                });

                ui.add_space(8.0);

                // 4. Barra de Status do Driver DirectShow Virtual Camera
                Frame::new()
                    .fill(Color32::from_rgb(13, 21, 40))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 45, 72)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            // Ponto indicador vetorial (Verde quando registrado, Âmbar quando pendente)
                            let dot_color = if driver_registered {
                                Color32::from_rgb(16, 185, 129)
                            } else {
                                Color32::from_rgb(245, 158, 11)
                            };
                            let (dot_rect, _) =
                                ui.allocate_exact_size(Vec2::new(16.0, 28.0), Sense::hover());
                            ui.painter()
                                .circle_filled(dot_rect.center(), 5.0, dot_color);

                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&backend_label)
                                        .size(12.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "Entrada: {res_label} • Saída DirectShow: 1280x720 • Pipeline: {decode_ms:.2}ms"
                                    ))
                                    .size(11.0)
                                    .color(Color32::from_rgb(148, 163, 184)),
                                );
                            });

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let btn_label = if driver_registered {
                                    "✓ Driver Virtual Cam Ativo"
                                } else {
                                    "⚡ Ativar Driver Virtual Cam (1-Clique)"
                                };
                                if Self::glass_button(
                                    ui,
                                    btn_label,
                                    !driver_registered,
                                    Vec2::new(235.0, 32.0),
                                )
                                .clicked()
                                {
                                    let base_dir = self.state.lock().unwrap().base_dir.clone();
                                    match install_vcam_driver_offline(&base_dir) {
                                        Ok(_) => {
                                            self.state
                                                .lock()
                                                .unwrap()
                                                .vcam
                                                .shm
                                                .refresh_registration_status();
                                            self.show_toast(
                                                "Driver DirectShow registrado com sucesso!",
                                            );
                                        }
                                        Err(e) => {
                                            self.show_toast(e);
                                        }
                                    }
                                }
                            });
                        });
                    });
            });
    }

    fn render_sidebar_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Frame::new()
            .fill(Color32::from_rgb(9, 15, 30))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 42, 68)))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                // Seletor de Abas Segmentadas (100% visível tanto ativo quanto inativo)
                Frame::new()
                    .fill(Color32::from_rgb(5, 10, 22))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(24, 36, 60)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(4))
                    .show(ui, |ui| {
                        ui.columns(3, |cols| {
                            let tab_sz = Vec2::new(0.0, 30.0);
                            if Self::glass_button(
                                &mut cols[0],
                                "Conexão",
                                self.active_tab == SidebarTab::Connection,
                                tab_sz,
                            )
                            .clicked()
                            {
                                self.active_tab = SidebarTab::Connection;
                            }
                            if Self::glass_button(
                                &mut cols[1],
                                "Estúdio",
                                self.active_tab == SidebarTab::Studio,
                                tab_sz,
                            )
                            .clicked()
                            {
                                self.active_tab = SidebarTab::Studio;
                            }
                            if Self::glass_button(
                                &mut cols[2],
                                "⚙ Ajustes",
                                self.active_tab == SidebarTab::Settings,
                                tab_sz,
                            )
                            .clicked()
                            {
                                self.active_tab = SidebarTab::Settings;
                            }
                        });
                    });

                ui.add_space(10.0);

                match self.active_tab {
                    SidebarTab::Connection => self.render_tab_connection(ui, ctx),
                    SidebarTab::Studio => self.render_tab_studio(ui),
                    SidebarTab::Settings => self.render_tab_settings(ui, ctx),
                }

                if let Some((msg, ts)) = &self.status_toast {
                    if ts.elapsed() < Duration::from_secs(4) {
                        ui.add_space(6.0);
                        Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(0, 240, 255, 25))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0, 240, 255)))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(10, 5))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(msg)
                                        .size(11.0)
                                        .color(Color32::from_rgb(0, 240, 255)),
                                );
                            });
                    }
                }
            });
    }

    fn render_tab_connection(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (interfaces, selected_ip, pairing_url, http_port, https_port) = {
            let s = self.state.lock().unwrap();
            (
                s.interfaces.clone(),
                s.selected_ip.clone(),
                s.pairing_url(),
                s.http_port,
                s.https_port,
            )
        };

        // 1. Vitrine do QR Code dimensionada para caber sem barra de rolagem
        ui.vertical_centered(|ui| {
            Frame::new()
                .fill(Color32::from_rgb(4, 10, 28))
                .stroke(Stroke::new(1.5_f32, Color32::from_rgb(0, 210, 240)))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    if let Some(qr_tex) = &self.qr_texture {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(162.0), Sense::hover());
                        ui.painter().image(
                            qr_tex.id(),
                            rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Escaneie com a câmera do celular para iniciar")
                    .size(11.5)
                    .color(Color32::from_rgb(148, 163, 184)),
            );
        });

        ui.add_space(10.0);

        // 2. Seletor de Interface de Rede Local
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Topologia de Rede Local")
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(203, 213, 225)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button(RichText::new("↻ Atualizar IPs").size(11.0))
                    .clicked()
                {
                    self.state.lock().unwrap().refresh_network_interfaces();
                    self.show_toast("Adaptadores de rede atualizados.");
                }
            });
        });

        ui.add_space(4.0);

        let current_label = interfaces
            .iter()
            .find(|i| i.ip == selected_ip)
            .map(|i| format!("{} — {}", i.label, i.ip))
            .unwrap_or_else(|| selected_ip.clone());

        egui::ComboBox::from_id_salt(Id::new("net_iface_combo"))
            .width(ui.available_width())
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for iface in &interfaces {
                    let item_text = format!("{} — {}", iface.label, iface.ip);
                    if ui
                        .selectable_label(iface.ip == selected_ip, item_text)
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.selected_ip = iface.ip.clone();
                        s.rebuild_qr_code();
                    }
                }
            });

        ui.add_space(8.0);

        // 3. Link Direto de Pareamento (Clique para Copiar)
        ui.label(
            RichText::new("Link Direto de Pareamento (Token Fixo)")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(203, 213, 225)),
        );
        ui.add_space(4.0);

        if Self::glass_button(
            ui,
            &format!("🔗 {pairing_url}"),
            true,
            Vec2::new(ui.available_width(), 32.0),
        )
        .on_hover_text("Clique para copiar o link de pareamento")
        .clicked()
        {
            ctx.copy_text(pairing_url);
            self.show_toast("Link copiado para a área de transferência!");
        }

        ui.add_space(10.0);

        // 4. Botões de Ferramentas de Rede e Segurança
        if Self::glass_button(
            ui,
            "🔑 Regerar Link / Token do QR Code",
            false,
            Vec2::new(ui.available_width(), 32.0),
        )
        .clicked()
        {
            {
                let mut s = self.state.lock().unwrap();
                s.settings.regenerate_token();
                s.rebuild_qr_code();
            }
            self.show_toast("Novo token de segurança gerado para o QR Code!");
        }

        ui.add_space(6.0);

        if Self::glass_button(
            ui,
            "⚡ Abrir Hotspot Direto 1-Hop (5GHz)",
            false,
            Vec2::new(ui.available_width(), 32.0),
        )
        .on_hover_text("Abre o Mobile Hotspot do Windows para conexão direta celular-PC sem roteador")
        .clicked()
        {
            open_mobile_hotspot_settings();
            self.show_toast(
                "Configurações do Hotspot abertas. Clique em 'Atualizar IPs' após ativar.",
            );
        }

        ui.add_space(6.0);

        if Self::glass_button(
            ui,
            "⚙ Liberar Portas no Firewall (UAC)",
            false,
            Vec2::new(ui.available_width(), 32.0),
        )
        .on_hover_text("Cria regra automática no Firewall do Windows para as portas 8080 e 8443")
        .clicked()
        {
            if register_firewall_rules(http_port, https_port) {
                self.show_toast("Regras de Firewall registradas com sucesso!");
            } else {
                self.show_toast("Elevação UAC cancelada.");
            }
        }
    }

    fn render_tab_studio(&mut self, ui: &mut egui::Ui) {
        let (
            cameras,
            selected_cam_id,
            res_profile,
            preset,
            mut gain,
            mut contrast,
            mut temp,
            mut zoom,
            shutter_ms,
            mut anti_psm,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.mobile_cameras.clone(),
                s.mobile_selected_cam_id.clone(),
                s.settings.data.resolution_profile.clone(),
                s.vcam.color_preset.clone(),
                s.vcam.brightness_gain,
                s.vcam.contrast,
                s.vcam.color_temp,
                s.vcam.zoom_level,
                s.settings.data.shutter_cap_ms,
                s.settings.data.anti_psm_enabled,
            )
        };

        // 1. Lente Física do Smartphone e Perfil de Resolução
        ui.label(
            RichText::new("Lente Física do Smartphone")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(203, 213, 225)),
        );
        let cam_label = cameras
            .iter()
            .find(|c| c.id == selected_cam_id)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| "Automática (Lente Principal)".to_string());

        egui::ComboBox::from_id_salt(Id::new("mobile_lens_combo"))
            .width(ui.available_width())
            .selected_text(cam_label)
            .show_ui(ui, |ui| {
                if cameras.is_empty() {
                    ui.label("Conecte o celular para listar as lentes físicas");
                }
                for cam in &cameras {
                    if ui
                        .selectable_label(cam.id == selected_cam_id, &cam.label)
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.mobile_selected_cam_id = cam.id.clone();
                        s.send_mobile_command(json!({
                            "command": "select_camera_device",
                            "device_id": cam.id,
                        }));
                    }
                }
            });

        ui.add_space(8.0);

        ui.label(
            RichText::new("Perfil de Transmissão (Resolução & Taxa)")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(203, 213, 225)),
        );
        let profiles = [
            ("1280x720@30", "HD 720p @ 30 FPS (Eco-Cool Recomendado)"),
            ("1280x720@60", "HD 720p @ 60 FPS (Ultra Fluido)"),
            ("1920x1080@30", "Full HD 1080p @ 30 FPS (Estúdio)"),
            ("1920x1080@60", "Full HD 1080p @ 60 FPS (Máxima Performance)"),
        ];
        let cur_prof_label = profiles
            .iter()
            .find(|(k, _)| *k == res_profile)
            .map(|(_, v)| *v)
            .unwrap_or("HD 720p @ 30 FPS");

        egui::ComboBox::from_id_salt(Id::new("res_profile_combo"))
            .width(ui.available_width())
            .selected_text(cur_prof_label)
            .show_ui(ui, |ui| {
                for (code, label) in profiles {
                    if ui.selectable_label(res_profile == code, label).clicked() {
                        let mut s = self.state.lock().unwrap();
                        s.settings.data.resolution_profile = code.to_string();
                        s.settings.save();
                        s.send_mobile_command(json!({
                            "command": "set_resolution",
                            "profile": code,
                        }));
                    }
                }
            });

        ui.add_space(10.0);

        // 2. Presets do Estúdio de Cor L1-LUT
        ui.label(
            RichText::new("Presets de Cor (Cache L1 768B)")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(203, 213, 225)),
        );
        ui.add_space(3.0);
        ui.columns(5, |cols| {
            let presets = [
                ("natural", "Natural"),
                ("warm", "Quente"),
                ("cool", "Frio"),
                ("cinema", "Cinema"),
                ("bw", "P&B"),
            ];
            for (idx, (id, name)) in presets.into_iter().enumerate() {
                if Self::glass_button(&mut cols[idx], name, preset == id, Vec2::new(0.0, 28.0))
                    .clicked()
                {
                    let mut s = self.state.lock().unwrap();
                    s.vcam.color_preset = id.to_string();
                    s.vcam.rebuild_color_lut();
                    s.settings.data.color_preset = id.to_string();
                    s.settings.save();
                }
            }
        });

        ui.add_space(8.0);

        // 3. Sliders de Imagem (Duplo clique restaura o valor padrão)
        let mut lut_changed = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Ganho de Exposição").size(11.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{gain:.2}x"))
                        .size(11.5)
                        .color(Color32::from_rgb(0, 240, 255)),
                );
            });
        });
        let r_gain = ui.add(egui::Slider::new(&mut gain, 0.4..=2.2).show_value(false));
        if r_gain.double_clicked() {
            gain = 1.0;
            lut_changed = true;
        } else if r_gain.changed() {
            lut_changed = true;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Contraste").size(11.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{contrast:.2}x"))
                        .size(11.5)
                        .color(Color32::from_rgb(0, 240, 255)),
                );
            });
        });
        let r_cont = ui.add(egui::Slider::new(&mut contrast, 0.5..=1.8).show_value(false));
        if r_cont.double_clicked() {
            contrast = 1.0;
            lut_changed = true;
        } else if r_cont.changed() {
            lut_changed = true;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Temperatura de Cor").size(11.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{temp:+.2}"))
                        .size(11.5)
                        .color(Color32::from_rgb(0, 240, 255)),
                );
            });
        });
        let r_temp = ui.add(egui::Slider::new(&mut temp, -1.0..=1.0).show_value(false));
        if r_temp.double_clicked() {
            temp = 0.0;
            lut_changed = true;
        } else if r_temp.changed() {
            lut_changed = true;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Zoom Digital (ePTZ)").size(11.5));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{zoom:.2}x"))
                        .size(11.5)
                        .color(Color32::from_rgb(0, 240, 255)),
                );
            });
        });
        let r_zoom = ui.add(egui::Slider::new(&mut zoom, 1.0..=3.0).show_value(false));
        if r_zoom.double_clicked() {
            let mut s = self.state.lock().unwrap();
            s.vcam.zoom_level = 1.0;
            s.vcam.pan_x = 0.0;
            s.vcam.pan_y = 0.0;
            s.settings.data.zoom_level = 1.0;
            s.settings.data.pan_x = 0.0;
            s.settings.data.pan_y = 0.0;
            s.settings.save();
        } else if r_zoom.changed() {
            let mut s = self.state.lock().unwrap();
            s.vcam.zoom_level = zoom;
            if zoom <= 1.01 {
                s.vcam.pan_x = 0.0;
                s.vcam.pan_y = 0.0;
            }
            s.settings.data.zoom_level = zoom;
            s.settings.save();
        }

        if lut_changed {
            let mut s = self.state.lock().unwrap();
            s.vcam.brightness_gain = gain;
            s.vcam.contrast = contrast;
            s.vcam.color_temp = temp;
            s.vcam.rebuild_color_lut();
            s.settings.data.brightness_gain = gain;
            s.settings.data.contrast = contrast;
            s.settings.data.color_temp = temp;
            s.settings.save();
        }

        ui.add_space(6.0);

        // 4. Obturador (Shutter Speed Cap) & Anti-PSM
        ui.label(
            RichText::new("Teto de Obturador Óptico (Shutter Cap)")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(203, 213, 225)),
        );
        ui.add_space(3.0);
        ui.columns(4, |cols| {
            let caps = [(4.0f32, "4ms"), (8.0, "8ms"), (16.0, "16ms"), (33.0, "Auto")];
            for (idx, (val, label)) in caps.into_iter().enumerate() {
                if Self::glass_button(
                    &mut cols[idx],
                    label,
                    (shutter_ms - val).abs() < 0.5,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
                {
                    let mut s = self.state.lock().unwrap();
                    s.settings.data.shutter_cap_ms = val;
                    s.settings.save();
                    s.send_mobile_command(json!({
                        "command": "set_shutter_cap",
                        "shutter_ms": val,
                    }));
                }
            }
        });

        ui.add_space(8.0);

        if ui
            .checkbox(
                &mut anti_psm,
                RichText::new("Pulso Anti-PSM Wi-Fi (500Hz Keep-Alive)").size(11.5),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.anti_psm_enabled = anti_psm;
            s.settings.save();
        }
    }

    fn render_tab_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (
            mut start_win,
            mut start_min,
            mut min_tray,
            mut close_tray,
            mut auto_eco,
            mut auto_vcam,
            base_dir,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.settings.data.start_with_windows,
                s.settings.data.start_minimized,
                s.settings.data.minimize_to_tray,
                s.settings.data.close_to_tray,
                s.settings.data.auto_eco_oled,
                s.settings.data.auto_start_vcam,
                s.base_dir.clone(),
            )
        };

        ui.label(
            RichText::new("Sistema Windows & Bandeja")
                .size(12.5)
                .strong()
                .color(Color32::from_rgb(0, 240, 255)),
        );
        ui.add_space(6.0);

        if ui
            .checkbox(
                &mut start_win,
                RichText::new("Iniciar automaticamente com o Windows").size(12.0),
            )
            .changed()
        {
            if set_start_with_windows(start_win) {
                let mut s = self.state.lock().unwrap();
                s.settings.data.start_with_windows = start_win;
                s.settings.save();
            }
        }

        if ui
            .checkbox(
                &mut start_min,
                RichText::new("Iniciar minimizado na Bandeja do Sistema").size(12.0),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.start_minimized = start_min;
            s.settings.save();
        }

        if ui
            .checkbox(
                &mut min_tray,
                RichText::new("Minimizar ocultando da Barra de Tarefas").size(12.0),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.minimize_to_tray = min_tray;
            s.settings.save();
        }

        if ui
            .checkbox(
                &mut close_tray,
                RichText::new("Fechar (X) mantém o tVisio na Bandeja").size(12.0),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.close_to_tray = close_tray;
            s.settings.save();
        }

        ui.add_space(10.0);
        ui.label(
            RichText::new("Automação de Energia & Driver")
                .size(12.5)
                .strong()
                .color(Color32::from_rgb(0, 240, 255)),
        );
        ui.add_space(6.0);

        if ui
            .checkbox(
                &mut auto_eco,
                RichText::new("Modo Eco OLED Automático no Celular (8s)").size(12.0),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.auto_eco_oled = auto_eco;
            s.settings.save();
            s.send_mobile_command(json!({
                "command": "set_auto_eco",
                "enabled": auto_eco,
            }));
        }

        if ui
            .checkbox(
                &mut auto_vcam,
                RichText::new("Inicializar Webcam Virtual Automaticamente").size(12.0),
            )
            .changed()
        {
            let mut s = self.state.lock().unwrap();
            s.settings.data.auto_start_vcam = auto_vcam;
            s.settings.save();
        }

        ui.add_space(14.0);

        if Self::glass_button(
            ui,
            "⚙ Abrir Pasta de Dados Locais (_local)",
            false,
            Vec2::new(ui.available_width(), 32.0),
        )
        .clicked()
        {
            open_local_data_folder(&base_dir.join("_local"));
        }

        ui.add_space(8.0);

        if ui
            .add_sized(
                [ui.available_width(), 32.0],
                egui::Button::new(
                    RichText::new("⏻ Encerrar tVisio Completamente")
                        .size(12.0)
                        .strong()
                        .color(Color32::from_rgb(248, 113, 113)),
                )
                .fill(Color32::from_rgba_unmultiplied(239, 68, 68, 25))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68))),
            )
            .clicked()
        {
            {
                let mut s = self.state.lock().unwrap();
                s.quit_requested = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for TVisioGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray_and_window_events(ctx);

        // Enquanto oculto na bandeja, reduz a frequência de atualização da UI para economizar 100% da GPU
        if self.hidden_in_tray {
            ctx.request_repaint_after(Duration::from_millis(250));
            return;
        }

        self.sync_gpu_textures(ctx);

        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.pip_mode {
            self.pip_mode = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1180.0, 740.0)));
        }

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(3, 7, 18))
                    .inner_margin(Margin::same(14)),
            )
            .show(ctx, |ui| {
                self.render_header(ui, ctx);
                ui.add_space(12.0);

                if self.pip_mode {
                    self.render_miniplayer_card(ui);
                } else {
                    let total_w = ui.available_width();
                    let sidebar_w = 365.0f32.min(total_w * 0.36);
                    let miniplayer_w = (total_w - sidebar_w - 12.0).max(320.0);

                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(miniplayer_w, ui.available_height()),
                            Layout::top_down(Align::Min),
                            |ui| {
                                self.render_miniplayer_card(ui);
                            },
                        );

                        ui.add_space(12.0);

                        ui.allocate_ui_with_layout(
                            Vec2::new(sidebar_w, ui.available_height()),
                            Layout::top_down(Align::Min),
                            |ui| {
                                self.render_sidebar_card(ui, ctx);
                            },
                        );
                    });
                }
            });

        // Solicita repaint suave a ~30 FPS quando conectado, ou 4 Hz em standby para consumo nulo de GPU
        let is_connected = self.state.lock().map(|s| s.mobile_connected).unwrap_or(false);
        if is_connected {
            ctx.request_repaint_after(Duration::from_millis(30));
        } else {
            ctx.request_repaint_after(Duration::from_millis(220));
        }
    }
}
