//! tVisio — Descoberta de Interfaces de Rede, QR Code Nativo e Ajuste de Sockets QoS WMM
//! =====================================================================================
//! - Identifica interfaces Cabo USB (RNDIS/NCM), Hotspot Direto 1-Hop (5GHz) e Rede LAN.
//! - Gera QR Codes estilizados (#00F0FF sobre #040A1C) diretamente em memória para GPU e HTTP.

use image::{ImageEncoder, RgbaImage};
use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, UdpSocket};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkInterfaceInfo {
    pub ip: String,
    pub adapter: String,
    #[serde(rename = "type")]
    pub conn_type: String,
    pub label: String,
    pub priority: u8,
    pub url: String,
}

/// Descobre todas as interfaces IPv4 utilizáveis no PC e classifica a topologia por prioridade de latência:
/// - `0` (`usb`): Conexão via Cabo USB RNDIS / NCM (< 1ms de latência, jitter 0)
/// - `1` (`hotspot`): Ponto de Acesso Móvel Direto do Windows 1-Hop (~1ms a 2ms, sem roteador intermediário)
/// - `2` (`lan`): Rede Wi-Fi / Ethernet convencional
pub fn discover_network_interfaces(https_port: u16) -> Vec<NetworkInterfaceInfo> {
    let mut interfaces: Vec<NetworkInterfaceInfo> = Vec::new();
    let mut seen_ips: HashSet<String> = HashSet::new();

    if let Ok(netifas) = local_ip_address::list_afinet_netifas() {
        for (name, ip_addr) in netifas {
            let IpAddr::V4(ipv4) = ip_addr else {
                continue;
            };
            let ip_str = ipv4.to_string();

            // Ignora loopback e endereços APIPA link-local
            if ip_str.starts_with("127.") || ip_str.starts_with("169.254.") {
                continue;
            }
            if seen_ips.contains(&ip_str) {
                continue;
            }

            let lower_name = name.to_lowercase();
            // Ignora adaptadores virtuais (WSL, Docker, Hyper-V, VMware, VirtualBox)
            if ["vethernet", "wsl", "docker", "vmware", "virtualbox", "loopback"]
                .iter()
                .any(|v| lower_name.contains(v))
            {
                continue;
            }

            seen_ips.insert(ip_str.clone());

            let (conn_type, label, priority) = if ip_str.starts_with("192.168.137.")
                || lower_name.contains("wi-fi direct")
                || lower_name.contains("local area connection*")
                || lower_name.contains("conexão local*")
            {
                (
                    "hotspot".to_string(),
                    "Hotspot Direto 1-Hop (Ultra-Rápido)".to_string(),
                    1,
                )
            } else if ["rndis", "ncm", "usb", "apple mobile", "android"]
                .iter()
                .any(|k| lower_name.contains(k))
            {
                (
                    "usb".to_string(),
                    "Cabo USB Direto (Sub-1ms)".to_string(),
                    0,
                )
            } else {
                (
                    "lan".to_string(),
                    format!("Rede Local ({name})"),
                    2,
                )
            };

            interfaces.push(NetworkInterfaceInfo {
                url: format!("https://{ip_str}:{https_port}"),
                ip: ip_str,
                adapter: name,
                conn_type,
                label,
                priority,
            });
        }
    }

    // Fallback via UDP probe caso nenhuma interface tenha sido retornada
    if interfaces.is_empty() {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
            if sock.connect("8.8.8.8:80").is_ok() {
                if let Ok(local_addr) = sock.local_addr() {
                    let ip_str = local_addr.ip().to_string();
                    interfaces.push(NetworkInterfaceInfo {
                        url: format!("https://{ip_str}:{https_port}"),
                        label: format!("Rede Local ({ip_str})"),
                        ip: ip_str,
                        adapter: "Rede Principal".to_string(),
                        conn_type: "lan".to_string(),
                        priority: 2,
                    });
                }
            }
        }
    }

    if interfaces.is_empty() {
        interfaces.push(NetworkInterfaceInfo {
            ip: "127.0.0.1".to_string(),
            adapter: "Localhost (USB ADB)".to_string(),
            conn_type: "usb".to_string(),
            label: "Localhost / USB".to_string(),
            priority: 0,
            url: format!("https://127.0.0.1:{https_port}"),
        });
    }

    interfaces.sort_by_key(|i| i.priority);
    interfaces
}

/// Gera a imagem do QR Code estilizado (#00F0FF sobre #040A1C) retornando:
/// `(rgba_pixels, width, height, png_bytes)`
pub fn generate_qr_code_assets(url: &str) -> (Vec<u8>, u32, u32, Vec<u8>) {
    let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)
        .unwrap_or_else(|_| QrCode::new(b"https://127.0.0.1:8443").unwrap());

    let qr_width = code.width() as u32;
    let border: u32 = 2;
    let box_size: u32 = 8;
    let total_modules = qr_width + border * 2;
    let img_dim = total_modules * box_size;

    let fg = [0u8, 240u8, 255u8, 255u8]; // #00F0FF Cyan Neon
    let bg = [4u8, 10u8, 28u8, 255u8];   // #040A1C Dark Obsidian

    let mut rgba = vec![0u8; (img_dim * img_dim * 4) as usize];

    for py in 0..img_dim {
        let my = (py / box_size) as i32 - (border as i32);
        for px in 0..img_dim {
            let mx = (px / box_size) as i32 - (border as i32);
            let is_dark = if mx >= 0 && my >= 0 && (mx as u32) < qr_width && (my as u32) < qr_width {
                code[(mx as usize, my as usize)] == qrcode::Color::Dark
            } else {
                false
            };
            let color = if is_dark { fg } else { bg };
            let idx = ((py * img_dim + px) * 4) as usize;
            rgba[idx..idx + 4].copy_from_slice(&color);
        }
    }

    let mut png_bytes: Vec<u8> = Vec::with_capacity(4096);
    if let Some(img_buf) = RgbaImage::from_raw(img_dim, img_dim, rgba.clone()) {
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        let _ = encoder.write_image(
            img_buf.as_raw(),
            img_dim,
            img_dim,
            image::ExtendedColorType::Rgba8,
        );
    }

    (rgba, img_dim, img_dim, png_bytes)
}
