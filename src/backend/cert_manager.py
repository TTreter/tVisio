"""
tVisio — Automatic Multi-SAN TLS Certificate Generator
Generates X.509 certificates covering all local IPs (LAN, Hotspot, USB) so mobile
browsers (Safari iOS / Chrome Android) allow navigator.mediaDevices.getUserMedia().
All temporary certificate files are stored in `_local/certs/`.
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
    Generate a self-signed TLS certificate with SubjectAlternativeName (SAN)
    for localhost, 127.0.0.1, and all active local IPv4 addresses.
    Stored inside `_local/certs/`.
    """
    cert_dir = base_dir / "_local" / "certs"
    cert_dir.mkdir(parents=True, exist_ok=True)

    cert_path = cert_dir / "tvisio_cert.pem"
    key_path = cert_dir / "tvisio_key.pem"

    # Generate RSA-2048 private key
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

    with open(key_path, "wb") as f:
        f.write(
            private_key.private_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PrivateFormat.TraditionalOpenSSL,
                encryption_algorithm=serialization.NoEncryption(),
            )
        )

    with open(cert_path, "wb") as f:
        f.write(cert.public_bytes(serialization.Encoding.PEM))

    return cert_path, key_path
