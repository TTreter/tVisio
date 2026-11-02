"""
tVisio — Gerador Automático de Certificados TLS Multi-SAN
Gera certificados X.509 locais cobrindo todos os IPs ativos (Wi-Fi, Hotspot, USB e Localhost)
para que navegadores móveis (Safari, Chrome, Edge) autorizem
o acesso à câmera (`navigator.mediaDevices.getUserMedia()`) sob HTTPS.
Todos os arquivos temporários gerados são mantidos isolados em `_local/certs/`.
"""

import datetime
import ipaddress
from pathlib import Path
from typing import List, Tuple
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa


def ensure_ssl_certificates(ip_list: List[str], base_dir: Path) -> Tuple[Path, Path]:
    """
    Gera um certificado TLS com extensões SubjectAlternativeName (SAN)
    para localhost, 127.0.0.1 e todos os endereços IPv4 locais detectados.
    Armazena a chave privada e o certificado na pasta `_local/certs/`.
    """
    cert_dir = base_dir / "_local" / "certs"
    cert_dir.mkdir(parents=True, exist_ok=True)

    cert_path = cert_dir / "tvisio_cert.pem"
    key_path = cert_dir / "tvisio_key.pem"

    # Gera chave privada RSA de 2048 bits
    private_key = rsa.generate_private_key(
        public_exponent=65537,
        key_size=2048,
    )

    subject = issuer = x509.Name([
        x509.NameAttribute(NameOID.COUNTRY_NAME, "BR"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "tVisio Ultra-Low Latency"),
        x509.NameAttribute(NameOID.COMMON_NAME, "tVisio Local Camera Host"),
    ])

    san_entries = [
        x509.DNSName("localhost"),
        x509.DNSName("tvisio.local"),
        x509.IPAddress(ipaddress.IPv4Address("127.0.0.1")),
    ]

    for ip_str in set(ip_list):
        try:
            san_entries.append(x509.IPAddress(ipaddress.ip_address(ip_str)))
        except ValueError:
            pass

    now = datetime.datetime.now(datetime.timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(private_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=365))
        .add_extension(
            x509.SubjectAlternativeName(san_entries),
            critical=False,
        )
        .add_extension(
            x509.BasicConstraints(ca=True, path_length=0),
            critical=True,
        )
        .sign(private_key, hashes.SHA256())
    )

    # Escreve a chave privada sem senha
    with open(key_path, "wb") as f:
        f.write(
            private_key.private_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PrivateFormat.TraditionalOpenSSL,
                encryption_algorithm=serialization.NoEncryption(),
            )
        )

    # Escreve o certificado assinado
    with open(cert_path, "wb") as f:
        f.write(cert.public_bytes(serialization.Encoding.PEM))

    return cert_path, key_path
