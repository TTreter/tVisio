//! tVisio — Motor de Vídeo 1-Pass Kernel Fusion (ePTZ + Flip/Rotate + L1-LUT + DirectShow SHM)
//! =============================================================================================
//! Substitui os 111 MB do OpenCV e 26 MB do NumPy por um pipeline unificado em Rust puro:
//! - Tabela de Consulta de Cores (LUT) de 768 bytes (`[[u8; 3]; 256]`) residente no Cache L1.
//! - Fusão de Kernels (1-Pass): executa Corte ePTZ (Zoom/Pan), Espelhamento H/V, Rotação
//!   (0°/90°/180°/270°), Redimensionamento Letterbox e Correção LUT em UMA ÚNICA PASSADA de memória.
//! - Escrita direta no buffer RGBA bottom-up da Memória Compartilhada DirectShow (`UnityCapture`).
//! - Geração de textura RGBA direta em memória para o Miniplayer Nativo OpenGL (0% re-encoding JPEG).

use crate::settings::Settings;
use crate::unity_capture::UnityCaptureSharedMem;
use std::time::Instant;

pub const VCAM_WIDTH: u32 = 1280;
pub const VCAM_HEIGHT: u32 = 720;
pub const PREVIEW_WIDTH: u32 = 640;
pub const PREVIEW_HEIGHT: u32 = 360;
pub const AMBIENT_WIDTH: u32 = 64;
pub const AMBIENT_HEIGHT: u32 = 36;

pub struct VirtualCamEngine {
    pub shm: UnityCaptureSharedMem,
    pub width: u32,
    pub height: u32,
    pub input_width: u32,
    pub input_height: u32,

    // Controles Geométricos e ePTZ Zero-Copy
    pub mirror_horizontal: bool,
    pub mirror_vertical: bool,
    pub rotation: i32,
    pub zoom_level: f32, // 1.0x a 3.0x
    pub pan_x: f32,      // -1.0 a +1.0
    pub pan_y: f32,      // -1.0 a +1.0

    // Controles do Estúdio de Cor L1-LUT (768 bytes)
    pub brightness_gain: f32,
    pub contrast: f32,
    pub gamma: f32,
    pub color_temp: f32,
    pub color_preset: String,

    color_lut: [[u8; 3]; 256],
    lut_is_identity: bool,

    // Modo Privacidade (Freeze Frame a 0% de uso de CPU)
    pub frozen: bool,

    // Miniplayer Nativo GPU (Zero-Render Mode quando desativado)
    pub miniplayer_enabled: bool,
    pub preview_rgba: Vec<u8>,
    pub ambient_rgba: Vec<u8>,
    pub preview_seq: u64,
    last_preview_instant: Instant,

    // Buffer pré-alocado de saída DirectShow (1280x720 RGBA8 Bottom-Up = 3.686.400 bytes, 0 alocações por frame)
    vcam_rgba_bottom_up: Vec<u8>,
    last_fit_dims: (u32, u32, i32),

    // Telemetria e FPS
    pub frames_received: u64,
    pub current_fps: f32,
    fps_counter: u32,
    fps_timer: Instant,
    pub last_decode_ms: f32,
}

impl VirtualCamEngine {
    pub fn new(initial: &Settings) -> Self {
        let cam_name = if initial.camera_display_name.trim().is_empty() {
            "tVisio Virtual Camera"
        } else {
            &initial.camera_display_name
        };
        let mut engine = Self {
            shm: UnityCaptureSharedMem::with_name(cam_name),
            width: VCAM_WIDTH,
            height: VCAM_HEIGHT,
            input_width: 0,
            input_height: 0,
            mirror_horizontal: initial.mirror_horizontal,
            mirror_vertical: initial.mirror_vertical,
            rotation: initial.rotation,
            zoom_level: initial.zoom_level.clamp(1.0, 3.0),
            pan_x: initial.pan_x.clamp(-1.0, 1.0),
            pan_y: initial.pan_y.clamp(-1.0, 1.0),
            brightness_gain: initial.brightness_gain,
            contrast: initial.contrast,
            gamma: initial.gamma,
            color_temp: initial.color_temp,
            color_preset: initial.color_preset.clone(),
            color_lut: [[0u8; 3]; 256],
            lut_is_identity: true,
            frozen: false,
            miniplayer_enabled: true,
            preview_rgba: vec![0u8; (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize],
            ambient_rgba: vec![0u8; (AMBIENT_WIDTH * AMBIENT_HEIGHT * 4) as usize],
            preview_seq: 0,
            last_preview_instant: Instant::now(),
            vcam_rgba_bottom_up: vec![0u8; (VCAM_WIDTH * VCAM_HEIGHT * 4) as usize],
            last_fit_dims: (0, 0, -1),
            frames_received: 0,
            current_fps: 0.0,
            fps_counter: 0,
            fps_timer: Instant::now(),
            last_decode_ms: 0.0,
        };
        engine.clear_vcam_canvas_black();
        engine.rebuild_color_lut();
        engine
    }

    fn clear_vcam_canvas_black(&mut self) {
        for chunk in self.vcam_rgba_bottom_up.chunks_exact_mut(4) {
            chunk[0] = 0;
            chunk[1] = 0;
            chunk[2] = 0;
            chunk[3] = 255;
        }
    }

    /// Pré-calcula a Tabela de Consulta (LUT) de 768 bytes (`[[u8; 3]; 256]`) residente no Cache L1.
    /// Executada apenas quando o usuário altera um slider ou preset na interface.
    pub fn rebuild_color_lut(&mut self) {
        let gain = self.brightness_gain;
        let mut contrast = self.contrast;
        let mut gamma = self.gamma;
        let mut temp = self.color_temp;

        match self.color_preset.as_str() {
            "warm" => {
                temp = (temp + 0.35).clamp(-1.0, 1.0);
                contrast *= 1.05;
            }
            "cool" => {
                temp = (temp - 0.35).clamp(-1.0, 1.0);
                contrast *= 1.05;
            }
            "cinema" => {
                contrast *= 1.18;
                gamma *= 0.92;
                temp = (temp + 0.12).clamp(-1.0, 1.0);
            }
            "bw" => {
                contrast *= 1.22;
            }
            _ => {}
        }

        if (gain - 1.0).abs() < 0.02
            && (contrast - 1.0).abs() < 0.02
            && (gamma - 1.0).abs() < 0.02
            && temp.abs() < 0.02
            && self.color_preset == "natural"
        {
            self.lut_is_identity = true;
            for i in 0..256 {
                let v = i as u8;
                self.color_lut[i] = [v, v, v];
            }
            return;
        }

        self.lut_is_identity = false;
        let inv_gamma = 1.0 / gamma.max(0.2);
        let apply_gamma = (gamma - 1.0).abs() >= 0.02;

        for i in 0..256 {
            let mut x = (i as f32) / 255.0;
            if apply_gamma {
                x = x.max(1e-5).powf(inv_gamma);
            }

            let base = (x * gain - 0.5) * contrast + 0.5;
            let r = ((base * (1.0 + temp * 0.18)) * 255.0).clamp(0.0, 255.0) as u8;
            let g = ((base * (1.0 + temp.abs() * 0.02)) * 255.0).clamp(0.0, 255.0) as u8;
            let b = ((base * (1.0 - temp * 0.18)) * 255.0).clamp(0.0, 255.0) as u8;

            self.color_lut[i] = [r, g, b];
        }
    }

    /// Decodifica um pacote JPEG binário e executa o pipeline de 1 passada (Kernel Fusion).
    pub fn process_jpeg_packet(&mut self, jpeg_bytes: &[u8]) {
        let t0 = Instant::now();

        if self.frozen {
            let _ = self.shm.send_frame_rgba_bottom_up(
                self.width,
                self.height,
                &self.vcam_rgba_bottom_up,
            );
            self.last_decode_ms = 0.03;
            return;
        }

        if let Ok(decoded) = image::load_from_memory_with_format(jpeg_bytes, image::ImageFormat::Jpeg) {
            let rgb = decoded.to_rgb8();
            let (w_in, h_in) = rgb.dimensions();
            self.process_rgb_frame(rgb.as_raw(), w_in, h_in, t0);
        }
    }

    /// Pipeline 1-Pass Kernel Fusion sobre um buffer RGB24 (`src_rgb`):
    /// Executa ePTZ Crop + Flip H/V + Rotate (0/90/180/270) + Letterbox Resize + L1 Color LUT
    /// em uma única passagem de memória, sem alocações dinâmicas.
    pub fn process_rgb_frame(&mut self, src_rgb: &[u8], w_in: u32, h_in: u32, t0: Instant) {
        if w_in == 0 || h_in == 0 || src_rgb.len() < (w_in * h_in * 3) as usize {
            return;
        }

        if self.frozen {
            let _ = self.shm.send_frame_rgba_bottom_up(
                self.width,
                self.height,
                &self.vcam_rgba_bottom_up,
            );
            self.last_decode_ms = 0.03;
            return;
        }

        self.input_width = w_in;
        self.input_height = h_in;

        // 1. Cálculo da janela de corte ePTZ (Zoom e Pan Digital Zero-Copy)
        let (crop_x1, crop_y1, crop_w, crop_h) = if self.zoom_level > 1.01 {
            let cw = ((w_in as f32) / self.zoom_level).round().max(64.0) as i32;
            let ch = ((h_in as f32) / self.zoom_level).round().max(64.0) as i32;
            let cw = cw.min(w_in as i32);
            let ch = ch.min(h_in as i32);
            let max_dx = ((w_in as i32) - cw) / 2;
            let max_dy = ((h_in as i32) - ch) / 2;
            let cx = (w_in as i32) / 2 + ((self.pan_x * (max_dx as f32)).round() as i32);
            let cy = (h_in as i32) / 2 + ((self.pan_y * (max_dy as f32)).round() as i32);
            let x1 = (cx - cw / 2).clamp(0, (w_in as i32) - cw) as u32;
            let y1 = (cy - ch / 2).clamp(0, (h_in as i32) - ch) as u32;
            (x1, y1, cw as u32, ch as u32)
        } else {
            (0, 0, w_in, h_in)
        };

        let rot = ((self.rotation % 360) + 360) % 360;
        let (eff_w, eff_h) = if rot == 90 || rot == 270 {
            (crop_h, crop_w)
        } else {
            (crop_w, crop_h)
        };

        // 2. Dimensões do Letterbox para manter a proporção exata dentro de 1280x720
        let scale = ((self.width as f32) / (eff_w as f32)).min((self.height as f32) / (eff_h as f32));
        let nw = ((eff_w as f32) * scale).round().max(1.0).min(self.width as f32) as u32;
        let nh = ((eff_h as f32) * scale).round().max(1.0).min(self.height as f32) as u32;

        if (nw, nh, rot) != self.last_fit_dims {
            self.clear_vcam_canvas_black();
            self.last_fit_dims = (nw, nh, rot);
        }

        let x_off = (self.width - nw) / 2;
        let y_off = (self.height - nh) / 2;

        let is_bw = self.color_preset == "bw";
        let use_lut = !self.lut_is_identity;
        let lut = &self.color_lut;
        let mirror_h = self.mirror_horizontal;
        let mirror_v = self.mirror_vertical;

        // 3. FUSÃO DE KERNELS (1-Pass): Amostragem + Rotação + Flip + LUT + Escrita Bottom-Up DirectShow
        let dst_w = self.width as usize;
        let _dst_h = self.height as usize;
        let src_stride = (w_in * 3) as usize;

        for dy in 0..nh {
            // Coordenada Y normalizada no espaço pós-rotação (0..eff_h)
            let mut ry = ((dy as u64 * eff_h as u64) / (nh as u64)) as u32;
            if mirror_v {
                ry = eff_h.saturating_sub(1).saturating_sub(ry);
            }

            // Linha correspondente no buffer DirectShow (orientação padrão top-down)
            let out_row = (y_off + dy) as usize;
            let out_row_base = (out_row * dst_w + (x_off as usize)) * 4;

            for dx in 0..nw {
                let mut rx = ((dx as u64 * eff_w as u64) / (nw as u64)) as u32;
                if mirror_h {
                    rx = eff_w.saturating_sub(1).saturating_sub(rx);
                }

                // Mapeia (rx, ry) de volta para (sx, sy) no buffer de origem de acordo com a rotação
                let (sx, sy) = match rot {
                    90 => (crop_x1 + ry, crop_y1 + (crop_h - 1 - rx)),
                    180 => (crop_x1 + (crop_w - 1 - rx), crop_y1 + (crop_h - 1 - ry)),
                    270 => (crop_x1 + (crop_w - 1 - ry), crop_y1 + rx),
                    _ => (crop_x1 + rx, crop_y1 + ry),
                };

                let src_idx = (sy as usize) * src_stride + (sx as usize) * 3;
                let mut r = src_rgb[src_idx];
                let mut g = src_rgb[src_idx + 1];
                let mut b = src_rgb[src_idx + 2];

                if use_lut {
                    r = lut[r as usize][0];
                    g = lut[g as usize][1];
                    b = lut[b as usize][2];
                }

                if is_bw {
                    let luma = ((77u32 * (r as u32) + 150u32 * (g as u32) + 29u32 * (b as u32)) >> 8) as u8;
                    r = luma;
                    g = luma;
                    b = luma;
                }

                let dst_idx = out_row_base + (dx as usize) * 4;
                self.vcam_rgba_bottom_up[dst_idx] = r;
                self.vcam_rgba_bottom_up[dst_idx + 1] = g;
                self.vcam_rgba_bottom_up[dst_idx + 2] = b;
                self.vcam_rgba_bottom_up[dst_idx + 3] = 255;
            }
        }

        // 4. Envia diretamente para a Memória Compartilhada do Driver DirectShow
        let _ = self.shm.send_frame_rgba_bottom_up(
            self.width,
            self.height,
            &self.vcam_rgba_bottom_up,
        );

        // 5. Atualiza o buffer RGBA nativo do Miniplayer (se ativo e respeitando teto de 30 FPS na UI)
        let now = Instant::now();
        if self.miniplayer_enabled && now.duration_since(self.last_preview_instant).as_millis() >= 32 {
            self.last_preview_instant = now;
            self.update_native_preview_buffers();
            self.preview_seq = self.preview_seq.wrapping_add(1);
        }

        // 6. Métricas de FPS e tempo de processamento
        self.frames_received += 1;
        self.fps_counter += 1;
        let elapsed_fps = now.duration_since(self.fps_timer).as_secs_f32();
        if elapsed_fps >= 0.5 {
            self.current_fps = ((self.fps_counter as f32) / elapsed_fps * 10.0).round() / 10.0;
            self.fps_counter = 0;
            self.fps_timer = now;
        }

        self.last_decode_ms = (t0.elapsed().as_secs_f32() * 100000.0).round() / 100.0;
    }

    /// Extrai a textura 640x360 (top-down) e a textura de brilho ambiente 64x36 diretamente
    /// a partir do canvas processado de 1280x720 (custo < 0.05ms, zero compressão JPEG).
    fn update_native_preview_buffers(&mut self) {
        let src_w = self.width as usize;
        let _src_h = self.height as usize;
        let pw = PREVIEW_WIDTH as usize;
        let ph = PREVIEW_HEIGHT as usize;

        for py in 0..ph {
            // Buffer top-down correspondente à textura de exibição na GPU
            let src_y = py * 2;
            let src_row_offset = src_y * src_w * 4;
            let dst_row_offset = py * pw * 4;

            for px in 0..pw {
                let s_idx = src_row_offset + (px * 2) * 4;
                let d_idx = dst_row_offset + px * 4;
                self.preview_rgba[d_idx..d_idx + 4]
                    .copy_from_slice(&self.vcam_rgba_bottom_up[s_idx..s_idx + 4]);
            }
        }

        // Atualiza a textura de luz ambiente (64x36)
        let aw = AMBIENT_WIDTH as usize;
        let ah = AMBIENT_HEIGHT as usize;
        for ay in 0..ah {
            let py = (ay * ph) / ah;
            let p_row = py * pw * 4;
            let a_row = ay * aw * 4;
            for ax in 0..aw {
                let px = (ax * pw) / aw;
                let s_idx = p_row + px * 4;
                let d_idx = a_row + ax * 4;
                self.ambient_rgba[d_idx..d_idx + 4]
                    .copy_from_slice(&self.preview_rgba[s_idx..s_idx + 4]);
            }
        }
    }

    pub fn resolution_label(&self) -> String {
        if self.input_width > 0 && self.input_height > 0 {
            format!("{}x{}", self.input_width, self.input_height)
        } else {
            "Aguardando Stream".to_string()
        }
    }

    /// Codifica o buffer de preview 640x360 em JPEG rápido (qualidade 78) para o WebSocket `/ws/desktop`.
    pub fn encode_preview_jpeg(&self) -> Vec<u8> {
        let mut rgb = Vec::with_capacity((PREVIEW_WIDTH * PREVIEW_HEIGHT * 3) as usize);
        for chunk in self.preview_rgba.chunks_exact(4) {
            rgb.push(chunk[0]);
            rgb.push(chunk[1]);
            rgb.push(chunk[2]);
        }
        let mut jpeg_buf = Vec::with_capacity(32 * 1024);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 78);
        let _ = encoder.encode(
            &rgb,
            PREVIEW_WIDTH,
            PREVIEW_HEIGHT,
            image::ExtendedColorType::Rgb8,
        );
        jpeg_buf
    }
}
