"""
tVisio — Descoberta de Interfaces de Rede e Ajuste de Sockets de Baixa Latência
Capacidades:
- Identifica interfaces Wi-Fi LAN, Hotspot Direto 1-Hop do Windows e Cabo USB (RNDIS/NCM).
- Aplica QoS de hardware WMM (DSCP Expedited Forwarding = 46) e TCP_NODELAY aos sockets.
"""

import socket
import ifaddr
from typing import List, Dict, Any

# Valor DSCP Expedited Forwarding (EF = 46, deslocado 2 bits à esquerda para o byte IP_TOS = 0xB8 / 184)
# Prioriza fisicamente os pacotes na fila de voz e vídeo (AC_VO / AC_VI) da antena Wi-Fi
IP_TOS_DSCP_EF = 0xB8


def tune_low_latency_socket(sock: socket.socket) -> None:
    """
    Aplica TCP_NODELAY (desativa o algoritmo de Nagle) e marcação DSCP EF
    ao socket do sistema operacional para eliminar buffers e atrasos na transmissão.
    """
    try:
        if sock.type == socket.SOCK_STREAM:
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    except Exception:
        pass

    try:
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_TOS, IP_TOS_DSCP_EF)
    except Exception:
        pass


def discover_network_interfaces(port: int) -> List[Dict[str, Any]]:
    """
    Descobre todas as interfaces IPv4 utilizáveis no PC e classifica a topologia por prioridade de latência:
    - 'usb': Conexão via Cabo USB RNDIS / NCM (< 1ms de latência, jitter 0)
    - 'hotspot': Ponto de Acesso Móvel Direto do Windows 1-Hop (~1ms a 2ms, sem roteador intermediário)
    - 'lan': Rede Wi-Fi / Ethernet convencional
    """
    interfaces: List[Dict[str, Any]] = []
    seen_ips = set()

    try:
        adapters = ifaddr.get_adapters()
        for adapter in adapters:
            nice_name = (adapter.nice_name or "").lower()
            for ip_info in adapter.ips:
                ip = ip_info.ip
                if not isinstance(ip, str):
                    continue
                # Ignora loopback e endereços de link-local APIPA
                if ip.startswith("127.") or ip.startswith("169.254."):
                    continue
                if ip in seen_ips:
                    continue

                # Ignora adaptadores de máquinas virtuais e containers (WSL, Docker, Hyper-V, VMware)
                if any(v in nice_name for v in ("vethernet", "wsl", "docker", "vmware", "virtualbox", "loopback")):
                    continue

                seen_ips.add(ip)

                # Classificação automática por topologia
                if ip.startswith("192.168.137.") or "wi-fi direct" in nice_name or "local area connection*" in nice_name:
                    conn_type = "hotspot"
                    label = "Hotspot Direto 1-Hop (Ultra-Rápido)"
                    priority = 1
                elif any(k in nice_name for k in ("rndis", "ncm", "usb", "apple mobile", "android")):
                    conn_type = "usb"
                    label = "Cabo USB Direto (Sub-1ms)"
                    priority = 0
                else:
                    conn_type = "lan"
                    label = f"Rede Local ({adapter.nice_name})"
                    priority = 2

                interfaces.append({
                    "ip": ip,
                    "adapter": adapter.nice_name,
                    "type": conn_type,
                    "label": label,
                    "priority": priority,
                    "url": f"https://{ip}:{port}"
                })
    except Exception:
        pass

    # Fallback usando conexão UDP de teste caso nenhuma interface tenha sido listada
    if not interfaces:
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(("8.8.8.8", 80))
            primary_ip = s.getsockname()[0]
            s.close()
            interfaces.append({
                "ip": primary_ip,
                "adapter": "Rede Principal",
                "type": "lan",
                "label": f"Rede Local ({primary_ip})",
                "priority": 2,
                "url": f"https://{primary_ip}:{port}"
            })
        except Exception:
            interfaces.append({
                "ip": "127.0.0.1",
                "adapter": "Localhost (USB ADB)",
                "type": "usb",
                "label": "Localhost / USB",
                "priority": 0,
                "url": f"https://127.0.0.1:{port}"
            })

    # Ordena as interfaces por menor latência teórica (USB -> Hotspot 1-Hop -> Wi-Fi LAN)
    interfaces.sort(key=lambda x: x["priority"])
    return interfaces
