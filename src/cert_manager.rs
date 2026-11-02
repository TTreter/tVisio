//! tVisio — Gerador Automático de Certificados TLS Multi-SAN (Rust Puro / rcgen)
//! =============================================================================
//! Gera certificados X.509 autoassinados com extensões SubjectAlternativeName (SAN)
//! cobrindo todos os IPs ativos (Wi-Fi, Hotspot 1-Hop, USB RNDIS/NCM e Localhost)
//! para que navegadores móveis (Safari, Chrome, Edge) autorizem `getUserMedia()` sob HTTPS.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Gera e persiste o certificado TLS e a chave privada em `_local/certs/`.
pub fn ensure_ssl_certificates(ip_list: &[String], base_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let cert_dir = base_dir.join("_local").join("certs");
    fs::create_dir_all(&cert_dir).map_err(|e| format!("Falha ao criar pasta de certificados: {e}"))?;

    let cert_path = cert_dir.join("tvisio_cert.pem");
    let key_path = cert_dir.join("tvisio_key.pem");

    let mut san_set: HashSet<String> = HashSet::new();
    san_set.insert("localhost".to_string());
    san_set.insert("tvisio.local".to_string());
    san_set.insert("127.0.0.1".to_string());

    for ip in ip_list {
        let trimmed = ip.trim();
        if !trimmed.is_empty() {
            san_set.insert(trimmed.to_string());
        }
    }

    let san_vec: Vec<String> = san_set.into_iter().collect();
    let cert = rcgen::generate_simple_self_signed(san_vec)
        .map_err(|e| format!("Erro ao gerar certificado X.509 Multi-SAN: {e}"))?;

    let cert_pem = cert
        .serialize_pem()
        .map_err(|e| format!("Erro ao serializar certificado PEM: {e}"))?;
    let key_pem = cert.serialize_private_key_pem();

    fs::write(&cert_path, cert_pem).map_err(|e| format!("Erro ao gravar tvisio_cert.pem: {e}"))?;
    fs::write(&key_path, key_pem).map_err(|e| format!("Erro ao gravar tvisio_key.pem: {e}"))?;

    Ok((cert_path, key_path))
}
