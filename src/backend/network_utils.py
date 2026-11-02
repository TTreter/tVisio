"""
tVisio — Network Discovery & Low-Latency Socket Tuning
Detects Wi-Fi LAN, Windows Mobile Hotspot (1-Hop), and USB Tethering (RNDIS/NCM) interfaces,
and applies QoS DSCP Expedited Forwarding (EF = 46) + TCP_NODELAY to sockets.
"""

import socket
import ifaddr
from typing import List, Dict, Any

# DSCP Expedited Forwarding (EF = 46, shifted left by 2 bits for IP_TOS byte = 0xB8 / 184)
IP_TOS_DSCP_EF = 0xB8


def tune_low_latency_socket(sock: socket.socket) -> None:
    """Apply TCP_NODELAY and DSCP EF (WMM Voice/Video queue priority) to a socket."""
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
    Discover all usable IPv4 interfaces on the host PC and classify their topology:
    - 'usb': USB RNDIS / NCM Tethering (< 1ms latency)
    - 'hotspot': Windows Direct Mobile Hotspot 1-Hop (192.168.137.x, ~1-2ms latency)
    - 'lan': Local Wi-Fi / Ethernet Router
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
                if ip.startswith("127.") or ip.startswith("169.254."):
                    continue
                if ip in seen_ips:
                    continue

                # Ignore virtual hypervisors (WSL, Docker, Hyper-V, VMware, VirtualBox)
                if any(v in nice_name for v in ("vethernet", "wsl", "docker", "vmware", "virtualbox", "loopback")):
                    continue

                seen_ips.add(ip)

                # Classify connection type for latency ranking
                if ip.startswith("192.168.137.") or "wi-fi direct" in nice_name or "local area connection*" in nice_name:
                    conn_type = "hotspot"
                    label = "Hotspot Direto 1-Hop (Ultra-Fast)"
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

    # Fallback using standard UDP connect trick if ifaddr found nothing
    if not interfaces:
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(("8.8.8.8", 80))
            primary_ip = s.getsockname()[0]
            s.close()
            interfaces.append({
                "ip": primary_ip,
                "adapter": "Primary Network",
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

    # Sort by priority (USB -> Hotspot 1-Hop -> Primary LAN)
    interfaces.sort(key=lambda x: x["priority"])
    return interfaces
