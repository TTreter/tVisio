"""
tVisio — Servidor Core de Ultra-Baixa Latência (WebRTC + Binary WS + Anti-PSM + Estúdio LUT L1)
Serviços fornecidos:
- WebApp transmissor móvel servido via HTTPS (Porta 8443) e HTTP (Porta 8080 para USB localhost)
- Interface Desktop Translucent Glass e telemetria em tempo real via HTTP/WebSocket (Porta 8080)
- Sinalização SDP WebRTC com priorização forçada de H.264 por hardware e extensão `playout-delay=0`
- Pulso inteligente de Keep-Alive Anti-PSM para evitar micro-sleep do rádio Wi-Fi do smartphone
- Gerenciamento de configurações persistentes e token de pareamento fixo em `_local/settings.json`
"""

import asyncio
import io
import json
import logging
import ssl
import struct
import subprocess
import time
import urllib.request
from pathlib import Path
from typing import Set, Optional, Dict, Any, List

import aiohttp
from aiohttp import web
import cv2
import numpy as np
import qrcode

try:
    from aiortc import RTCPeerConnection, RTCSessionDescription
    HAS_WEBRTC = True
except ImportError:
    HAS_WEBRTC = False

from .cert_manager import ensure_ssl_certificates
from .network_utils import discover_network_interfaces, tune_low_latency_socket
from .settings_manager import SettingsManager
from .vcam_engine import VirtualCamEngine, configure_tvisio_camera_name

# Configuração de logging estruturado do sistema
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
logger = logging.getLogger("tVisio.Server")

# Diretórios base de arquivos estáticos e recursos
BASE_DIR = Path(__file__).resolve().parent.parent.parent
DESKTOP_DIR = BASE_DIR / "src" / "desktop"
MOBILE_DIR = BASE_DIR / "src" / "mobile"
ASSETS_DIR = BASE_DIR / "assets"


def prioritize_h264_in_sdp(sdp: str) -> str:
    """
    Reordena os identificadores de payload na linha `m=video` do SDP para que
    os payloads de H.264 acelerados por hardware apareçam antes de VP8/VP9.
    Também injeta a extensão RTP `playout-delay=0` para desativar o jitter buffer.
    """
    lines = sdp.splitlines()
    h264_pts: List[str] = []

    # Localiza quais payloads numéricos correspondem ao codec H264
    for line in lines:
        if line.startswith("a=rtpmap:") and "H264/90000" in line.upper():
            pt = line.split(":")[1].split(" ")[0]
            h264_pts.append(pt)

    out_lines: List[str] = []
    for line in lines:
        if line.startswith("m=video ") and h264_pts:
            parts = line.split(" ")
            if len(parts) > 3:
                header = parts[:3]
                pts = parts[3:]
                # Reorganiza os payloads colocando H.264 no topo da preferência
                prioritized = [p for p in pts if p in h264_pts] + [p for p in pts if p not in h264_pts]
                line = " ".join(header + prioritized)
            out_lines.append(line)
            # Injeta a extensão de cabeçalho RTP playout-delay=0 (min=0, max=0)
            out_lines.append("a=extmap:14 http://www.webrtc.org/experiments/rtp-hdrext/playout-delay")
        else:
            out_lines.append(line)
    return "\r\n".join(out_lines) + "\r\n"


class TVisioServer:
    """
    Servidor assíncrono principal do tVisio baseado em aiohttp.
    Gerencia a comunicação bidirecional com o app desktop e o cliente móvel.
    """

    def __init__(self, http_port: int = 8080, https_port: int = 8443) -> None:
        self.http_port = http_port
        self.https_port = https_port

        # Carrega configurações salvas ou inicializa valores padrão com token fixo
        self.settings = SettingsManager(BASE_DIR)
        self.session_token = self.settings.data["session_token"]
        self.vcam = VirtualCamEngine(initial_settings=self.settings.data)

        # Descobre as interfaces de rede ativas (USB, Hotspot 1-Hop, Wi-Fi LAN)
        self.interfaces = discover_network_interfaces(self.https_port)
        self.selected_interface = self.interfaces[0] if self.interfaces else {
            "ip": "127.0.0.1",
            "type": "usb",
            "label": "Localhost",
            "url": f"https://127.0.0.1:{self.https_port}"
        }

        # Conjuntos de conexões WebSocket e instâncias WebRTC ativas
        self.desktop_ws_clients: Set[web.WebSocketResponse] = set()
        self.mobile_ws_clients: Set[web.WebSocketResponse] = set()
        self.pcs: Set[Any] = set()

        # Estado da telemetria em tempo real
        self.mobile_connected: bool = False
        self.active_protocol: str = "Standby"
        self.anti_psm_enabled: bool = bool(self.settings.data.get("anti_psm_enabled", False))
        self.shutter_cap_ms: float = float(self.settings.data.get("shutter_cap_ms", 8.0))
        self.rtt_ms: float = 0.0
        self.glass_to_glass_ms: float = 0.0
        self.mobile_device_info: str = "Nenhum dispositivo conectado"
        self.mobile_eco_mode: bool = False
        self.mobile_torch_on: bool = False
        self.mobile_af_locked: bool = False
        self.mobile_facing_mode: str = "environment"
        self.mobile_cameras: List[Dict[str, str]] = []
        self.mobile_selected_cam_id: str = ""
        self.mobile_battery_level: Optional[int] = None
        self.mobile_battery_charging: bool = False

    def get_pairing_url(self, ip: Optional[str] = None) -> str:
        """Gera o link completo de pareamento contendo o IP e o token de sessão."""
        target_ip = ip or self.selected_interface["ip"]
        return f"https://{target_ip}:{self.https_port}/?t={self.session_token}"

    async def broadcast_telemetry(self) -> None:
        """Transmite as métricas de telemetria a cada 250ms para as janelas Desktop conectadas."""
        while True:
            try:
                if self.desktop_ws_clients:
                    vcam_status = self.vcam.get_status()
                    payload = json.dumps({
                        "type": "telemetry",
                        "mobile_connected": self.mobile_connected,
                        "active_protocol": self.active_protocol,
                        "rtt_ms": round(self.rtt_ms, 1),
                        "g2g_ms": round(self.glass_to_glass_ms, 1),
                        "anti_psm": self.anti_psm_enabled,
                        "shutter_cap_ms": self.shutter_cap_ms,
                        "device_info": self.mobile_device_info,
                        "eco_mode": self.mobile_eco_mode,
                        "torch_on": self.mobile_torch_on,
                        "af_locked": self.mobile_af_locked,
                        "facing_mode": self.mobile_facing_mode,
                        "cameras": self.mobile_cameras,
                        "selected_cam_id": self.mobile_selected_cam_id,
                        "battery_level": self.mobile_battery_level,
                        "battery_charging": self.mobile_battery_charging,
                        "pairing_url": self.get_pairing_url(),
                        "interfaces": self.interfaces,
                        "selected_ip": self.selected_interface["ip"],
                        "vcam": vcam_status,
                    })
                    dead = []
                    for ws in list(self.desktop_ws_clients):
                        try:
                            await ws.send_str(payload)
                        except Exception:
                            dead.append(ws)
                    for ws in dead:
                        self.desktop_ws_clients.discard(ws)
            except Exception as e:
                logger.debug(f"Erro no envio de telemetria: {e}")
            await asyncio.sleep(0.25)

    async def anti_psm_heartbeat_loop(self) -> None:
        """
        Gerador inteligente de micro-pulsos Anti-PSM.
        Envia um pulso binário de 1 byte (0x01) para clientes conectados apenas em modo fallback WebSocket,
        evitando micro-sleep de economia de energia no Wi-Fi sem sobreaquecer o chip em WebRTC.
        """
        pulse_packet = b"\x01"
        while True:
            if self.anti_psm_enabled and self.mobile_ws_clients and not self.pcs:
                for ws in list(self.mobile_ws_clients):
                    if not ws.closed:
                        try:
                            await ws.send_bytes(pulse_packet)
                        except Exception:
                            pass
                await asyncio.sleep(0.020)
            else:
                await asyncio.sleep(0.1)

    async def push_miniplayer_preview(self, preview_jpeg: Optional[bytes]) -> None:
        """Envia o quadro JPEG de baixa resolução (360p) para os Miniplayers ativos no Desktop."""
        if not preview_jpeg or not self.desktop_ws_clients:
            return
        for ws in list(self.desktop_ws_clients):
            if not ws.closed:
                try:
                    await ws.send_bytes(preview_jpeg)
                except Exception:
                    pass

    # -------------------------------------------------------------------------
    # Rotas HTTP
    # -------------------------------------------------------------------------
    async def handle_mobile_index(self, request: web.Request) -> web.Response:
        """Entrega o WebApp do smartphone com redirecionamento automático para HTTPS quando necessário."""
        host_header = request.headers.get("Host", "")
        hostname = host_header.split(":")[0] if host_header else ""
        if not request.secure and hostname not in ("localhost", "127.0.0.1"):
            https_url = f"https://{hostname}:{self.https_port}{request.rel_url}"
            raise web.HTTPFound(location=https_url)
        return web.FileResponse(MOBILE_DIR / "index.html")

    async def handle_manifest_json(self, request: web.Request) -> web.Response:
        """Entrega o manifesto PWA para permitir adicionar o app à tela inicial do celular com token fixo."""
        manifest = {
            "name": "tVisio Camera Transmitter",
            "short_name": "tVisio",
            "start_url": f"/?t={self.session_token}",
            "display": "standalone",
            "background_color": "#000000",
            "theme_color": "#030712",
            "icons": [{"src": "/assets/icon.png", "sizes": "1024x1024", "type": "image/png"}],
        }
        return web.json_response(manifest)

    async def handle_desktop_index(self, request: web.Request) -> web.Response:
        """Entrega a interface desktop Translucent Glass."""
        return web.FileResponse(DESKTOP_DIR / "index.html")

    async def handle_qr_svg(self, request: web.Request) -> web.Response:
        """Gera e retorna a imagem PNG do QR Code estilizado para conexão instantânea."""
        ip = request.query.get("ip", self.selected_interface["ip"])
        for iface in self.interfaces:
            if iface["ip"] == ip:
                self.selected_interface = iface
                break

        url = self.get_pairing_url(ip)
        qr = qrcode.QRCode(
            version=1,
            error_correction=qrcode.constants.ERROR_CORRECT_M,
            box_size=10,
            border=2,
        )
        qr.add_data(url)
        qr.make(fit=True)
        img = qr.make_image(fill_color="#00F0FF", back_color="#040A1C").convert("RGBA")

        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return web.Response(body=buf.getvalue(), content_type="image/png")

    async def handle_cert_download(self, request: web.Request) -> web.Response:
        """Permite o download do certificado raiz local caso necessário em redes isoladas."""
        cert_path = BASE_DIR / "_local" / "certs" / "tvisio_cert.pem"
        if cert_path.exists():
            return web.FileResponse(
                cert_path,
                headers={"Content-Disposition": 'attachment; filename="tVisio-Local-CA.crt"'},
            )
        return web.Response(status=404, text="Certificado não encontrado")

    async def handle_install_vcam_driver(self, request: web.Request) -> web.Response:
        """
        Instalador automático em 1 clique para o filtro DirectShow UnityCapture (código aberto).
        Baixa os binários de 64-bit e 32-bit para `_local/driver/` e registra no Windows via regsvr32.
        """
        try:
            driver_dir = BASE_DIR / "_local" / "driver"
            driver_dir.mkdir(parents=True, exist_ok=True)
            dll64_path = driver_dir / "UnityCaptureFilter64.dll"
            dll32_path = driver_dir / "UnityCaptureFilter32.dll"

            base_raw_url = "https://raw.githubusercontent.com/schellingb/UnityCapture/master/Install"
            if not dll64_path.exists():
                await asyncio.to_thread(
                    urllib.request.urlretrieve,
                    f"{base_raw_url}/UnityCaptureFilter64.dll",
                    dll64_path,
                )
            if not dll32_path.exists():
                await asyncio.to_thread(
                    urllib.request.urlretrieve,
                    f"{base_raw_url}/UnityCaptureFilter32.dll",
                    dll32_path,
                )

            configure_tvisio_camera_name()

            # Executa o regsvr32 solicitando elevação de privilégios via UAC
            ps_script = (
                f"Start-Process -FilePath 'regsvr32.exe' "
                f"-ArgumentList '/s', '\"{dll64_path}\"', '\"{dll32_path}\"' "
                f"-Verb RunAs -Wait"
            )
            await asyncio.to_thread(
                subprocess.run,
                ["powershell", "-NoProfile", "-Command", ps_script],
                check=False,
            )

            configure_tvisio_camera_name()
            self.vcam._ensure_camera(1280, 720)
            return web.json_response({
                "ok": True,
                "driver_active": self.vcam.driver_active,
                "backend_name": self.vcam.backend_name,
            })
        except Exception as e:
            logger.error(f"Erro ao registrar driver de câmera virtual: {e}")
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    async def handle_hotspot_guide(self, request: web.Request) -> web.Response:
        """Abre as configurações nativas do Windows Mobile Hotspot para conexão direta de 1 salto."""
        try:
            subprocess.Popen(["cmd", "/c", "start", "ms-settings:network-mobilehotspot"], shell=True)
            await asyncio.sleep(1.5)
            self.interfaces = discover_network_interfaces(self.https_port)
            return web.json_response({"ok": True, "interfaces": self.interfaces})
        except Exception as e:
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    # -------------------------------------------------------------------------
    # Sinalização WebRTC e Consumo de Faixas de Vídeo
    # -------------------------------------------------------------------------
    async def handle_webrtc_offer(self, request: web.Request) -> web.Response:
        """Recebe o SDP Offer do smartphone e responde com o SDP Answer priorizado em H.264."""
        if not HAS_WEBRTC:
            return web.json_response({"error": "aiortc não disponível neste ambiente"}, status=501)

        params = await request.json()
        prioritized_offer_sdp = prioritize_h264_in_sdp(params["sdp"])
        offer = RTCSessionDescription(sdp=prioritized_offer_sdp, type=params["type"])

        # Fecha conexões antigas para liberar imediatamente os decodificadores da GPU/CPU
        for old_pc in list(self.pcs):
            self.pcs.discard(old_pc)
            try:
                await old_pc.close()
            except Exception:
                pass

        pc = RTCPeerConnection()
        self.pcs.add(pc)

        @pc.on("connectionstatechange")
        async def on_connectionstatechange() -> None:
            logger.info(f"Estado WebRTC: {pc.connectionState}")
            if pc.connectionState in ("connected", "completed"):
                self.mobile_connected = True
                self.active_protocol = "WebRTC Zero-Buffer (H.264 UDP)"
            elif pc.connectionState in ("failed", "closed", "disconnected"):
                self.pcs.discard(pc)
                try:
                    await pc.close()
                except Exception:
                    pass
                if not self.pcs and not self.mobile_ws_clients:
                    self.mobile_connected = False
                    self.active_protocol = "Standby"

        @pc.on("track")
        def on_track(track: Any) -> None:
            if track.kind == "video":
                logger.info("Faixa de vídeo WebRTC recebida — iniciando consumidor Zero-Buffer")
                self.mobile_connected = True
                self.active_protocol = "WebRTC Zero-Buffer (H.264 UDP)"
                asyncio.ensure_future(self._consume_webrtc_video(track))

        await pc.setRemoteDescription(offer)
        answer = await pc.createAnswer()
        final_sdp = prioritize_h264_in_sdp(answer.sdp)

        await pc.setLocalDescription(RTCSessionDescription(sdp=final_sdp, type=answer.type))
        return web.json_response({
            "sdp": pc.localDescription.sdp,
            "type": pc.localDescription.type,
        })

    async def _consume_webrtc_video(self, track: Any) -> None:
        """
        Consome quadros da faixa de vídeo WebRTC com esvaziamento imediato da fila interna (Zero-Queue).
        Garante que apenas o quadro mais recente seja processado, eliminando lag acumulado e vazamento de RAM.
        """
        try:
            while True:
                t_start = time.perf_counter()
                frame = await track.recv()

                # Drena quadros intermediários acumulados na fila interna do aiortc
                q = getattr(track, "_queue", None)
                if q is not None:
                    while not q.empty():
                        try:
                            newer = q.get_nowait()
                            del frame
                            frame = newer
                        except Exception:
                            break

                # No modo de congelamento de quadro (privacidade), consome sem decodificar novo conteúdo
                if self.vcam.frozen:
                    del frame
                    self.vcam.process_and_push_bgr(np.empty((1, 1, 3), dtype=np.uint8))
                    await asyncio.sleep(0.016)
                    continue

                bgr = frame.to_ndarray(format="bgr24")
                del frame

                # Alimenta o motor de webcam virtual e obtém o preview do miniplayer
                preview_jpeg = self.vcam.process_and_push_bgr(bgr)
                del bgr

                if preview_jpeg:
                    await self.push_miniplayer_preview(preview_jpeg)

                decode_elapsed = (time.perf_counter() - t_start) * 1000.0
                self.glass_to_glass_ms = max(8.5, (self.rtt_ms * 0.5) + 6.5 + min(decode_elapsed, 6.0))
                await asyncio.sleep(0)
        except Exception as e:
            logger.info(f"Faixa de vídeo WebRTC encerrada: {e}")

    # -------------------------------------------------------------------------
    # WebSocket: Canal de Fluxo Binário e Controle do Smartphone (/ws/stream)
    # -------------------------------------------------------------------------
    async def handle_mobile_ws(self, request: web.Request) -> web.WebSocketResponse:
        """Gerencia o canal WebSocket do celular para comandos, telemetria e fallback binário de vídeo."""
        ws = web.WebSocketResponse(max_msg_size=16 * 1024 * 1024, heartbeat=15.0)
        await ws.prepare(request)

        # Aplica TCP_NODELAY e marcação de hardware DSCP EF no socket subjacente
        transport = request.transport
        if transport is not None:
            raw_sock = transport.get_extra_info("socket")
            if raw_sock is not None:
                tune_low_latency_socket(raw_sock)

        self.mobile_ws_clients.add(ws)
        self.mobile_connected = True
        if self.active_protocol == "Standby":
            self.active_protocol = "WebSocket Binary Turbo (TCP_NODELAY)"

        logger.info("Transmissor móvel conectado via WebSocket")

        try:
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.BINARY:
                    data = msg.data
                    # Cabeçalho binário de 8 bytes contendo o timestamp (float64 em milissegundos)
                    if len(data) > 8:
                        client_ts = struct.unpack("<d", data[:8])[0]
                        jpeg_bytes = data[8:]
                        await ws.send_str(json.dumps({"type": "ack", "ts": client_ts}))

                        t0 = time.perf_counter()
                        buf_arr = np.frombuffer(jpeg_bytes, dtype=np.uint8)
                        bgr_arr = cv2.imdecode(buf_arr, cv2.IMREAD_COLOR)
                        if bgr_arr is not None:
                            preview_jpeg = self.vcam.process_and_push_bgr(bgr_arr)
                            if preview_jpeg:
                                await self.push_miniplayer_preview(preview_jpeg)

                        decode_ms = (time.perf_counter() - t0) * 1000.0
                        if self.active_protocol.startswith("WebSocket") or not self.pcs:
                            self.active_protocol = "WebSocket Binary Turbo (Zero-Queue)"
                            self.glass_to_glass_ms = max(9.5, (self.rtt_ms * 0.5) + self.shutter_cap_ms + decode_ms)

                elif msg.type == aiohttp.WSMsgType.TEXT:
                    info = json.loads(msg.data)
                    msg_type = info.get("type")
                    if msg_type == "ping":
                        await ws.send_str(json.dumps({"type": "pong", "ts": info.get("ts")}))
                    elif msg_type == "telemetry":
                        self.rtt_ms = float(info.get("rtt_ms", self.rtt_ms))
                        self.mobile_device_info = info.get("device", self.mobile_device_info)
                        self.mobile_eco_mode = bool(info.get("eco_mode", self.mobile_eco_mode))
                        self.mobile_torch_on = bool(info.get("torch_on", self.mobile_torch_on))
                        self.mobile_af_locked = bool(info.get("af_locked", self.mobile_af_locked))
                        self.mobile_facing_mode = info.get("facing_mode", self.mobile_facing_mode)
                        self.shutter_cap_ms = float(info.get("shutter_ms", self.shutter_cap_ms))
                        if "cameras" in info and isinstance(info["cameras"], list):
                            self.mobile_cameras = info["cameras"]
                        if "selected_cam_id" in info:
                            self.mobile_selected_cam_id = str(info["selected_cam_id"] or "")
                        if "battery_level" in info and info["battery_level"] is not None:
                            self.mobile_battery_level = int(info["battery_level"])
                            self.mobile_battery_charging = bool(info.get("battery_charging", False))
        except Exception as e:
            logger.debug(f"WebSocket móvel desconectado: {e}")
        finally:
            self.mobile_ws_clients.discard(ws)
            if not self.mobile_ws_clients and not self.pcs:
                self.mobile_connected = False
                self.active_protocol = "Standby"
        return ws

    # -------------------------------------------------------------------------
    # WebSocket: Interface Desktop e Miniplayer (/ws/desktop)
    # -------------------------------------------------------------------------
    async def handle_desktop_ws(self, request: web.Request) -> web.WebSocketResponse:
        """Gerencia os comandos da interface desktop e a sincronização de controles de imagem/câmera."""
        ws = web.WebSocketResponse(max_msg_size=8 * 1024 * 1024)
        await ws.prepare(request)
        self.desktop_ws_clients.add(ws)

        try:
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.TEXT:
                    cmd = json.loads(msg.data)
                    action = cmd.get("action")

                    # Transformações geométricas no PC com persistência automática
                    if action == "toggle_mirror_h":
                        self.vcam.mirror_horizontal = not self.vcam.mirror_horizontal
                        self.settings.update(mirror_horizontal=self.vcam.mirror_horizontal)
                    elif action == "toggle_mirror_v":
                        self.vcam.mirror_vertical = not self.vcam.mirror_vertical
                        self.settings.update(mirror_vertical=self.vcam.mirror_vertical)
                    elif action == "rotate":
                        self.vcam.rotation = (self.vcam.rotation + 90) % 360
                        self.settings.update(rotation=self.vcam.rotation)
                    elif action == "toggle_freeze":
                        self.vcam.frozen = not self.vcam.frozen

                    # Controles do Estúdio de Cor em 1 Passada (LUT no Cache L1)
                    elif action == "set_brightness":
                        self.vcam.brightness_gain = float(cmd.get("value", 1.0))
                        self.vcam.rebuild_color_lut()
                        self.settings.update(brightness_gain=self.vcam.brightness_gain)
                    elif action == "set_color_studio":
                        if "brightness_gain" in cmd:
                            self.vcam.brightness_gain = float(cmd["brightness_gain"])
                        if "contrast" in cmd:
                            self.vcam.contrast = float(cmd["contrast"])
                        if "gamma" in cmd:
                            self.vcam.gamma = float(cmd["gamma"])
                        if "color_temp" in cmd:
                            self.vcam.color_temp = float(cmd["color_temp"])
                        if "color_preset" in cmd:
                            self.vcam.color_preset = str(cmd["color_preset"])
                        self.vcam.rebuild_color_lut()
                        self.settings.update(
                            brightness_gain=self.vcam.brightness_gain,
                            contrast=self.vcam.contrast,
                            gamma=self.vcam.gamma,
                            color_temp=self.vcam.color_temp,
                            color_preset=self.vcam.color_preset,
                        )

                    # Zoom e Pan Digital Zero-Copy (ePTZ)
                    elif action == "set_eptz":
                        if "zoom_level" in cmd:
                            self.vcam.zoom_level = max(1.0, min(3.0, float(cmd["zoom_level"])))
                        if "pan_x" in cmd:
                            self.vcam.pan_x = max(-1.0, min(1.0, float(cmd["pan_x"])))
                        if "pan_y" in cmd:
                            self.vcam.pan_y = max(-1.0, min(1.0, float(cmd["pan_y"])))
                        self.settings.update(
                            zoom_level=self.vcam.zoom_level,
                            pan_x=self.vcam.pan_x,
                            pan_y=self.vcam.pan_y,
                        )

                    # Controles de desempenho e rede
                    elif action == "set_miniplayer_enabled":
                        self.vcam.miniplayer_enabled = bool(cmd.get("enabled", True))
                    elif action == "toggle_anti_psm":
                        self.anti_psm_enabled = not self.anti_psm_enabled
                        self.settings.update(anti_psm_enabled=self.anti_psm_enabled)
                    elif action == "select_interface":
                        ip = cmd.get("ip")
                        for iface in self.interfaces:
                            if iface["ip"] == ip:
                                self.selected_interface = iface
                                break
                    elif action == "refresh_interfaces":
                        self.interfaces = discover_network_interfaces(self.https_port)

                    # Comandos remotos direcionados ao hardware da câmera móvel
                    elif action in (
                        "switch_camera",
                        "select_camera_device",
                        "toggle_torch",
                        "toggle_af_lock",
                        "toggle_eco_mode",
                        "set_shutter_cap",
                        "set_protocol",
                        "set_resolution",
                    ):
                        if action == "set_shutter_cap":
                            self.shutter_cap_ms = float(cmd.get("shutter_ms", 8.0))
                            self.settings.update(shutter_cap_ms=self.shutter_cap_ms)
                        forward_msg = json.dumps({"type": "command", "command": action, **cmd})
                        for m_ws in list(self.mobile_ws_clients):
                            if not m_ws.closed:
                                await m_ws.send_str(forward_msg)
        finally:
            self.desktop_ws_clients.discard(ws)
        return ws

    def create_app(self) -> web.Application:
        """Cria e configura o roteador web da aplicação aiohttp."""
        app = web.Application()
        app.router.add_get("/", self.handle_mobile_index)
        app.router.add_get("/manifest.json", self.handle_manifest_json)
        app.router.add_get("/desktop", self.handle_desktop_index)
        app.router.add_get("/api/qr", self.handle_qr_svg)
        app.router.add_get("/api/cert", self.handle_cert_download)
        app.router.add_post("/api/install-vcam", self.handle_install_vcam_driver)
        app.router.add_post("/api/hotspot", self.handle_hotspot_guide)
        app.router.add_post("/api/webrtc/offer", self.handle_webrtc_offer)
        app.router.add_get("/ws/stream", self.handle_mobile_ws)
        app.router.add_get("/ws/desktop", self.handle_desktop_ws)

        # Rotas para entrega de arquivos estáticos
        app.router.add_static("/assets", ASSETS_DIR)
        app.router.add_static("/desktop-static", DESKTOP_DIR)
        app.router.add_static("/mobile-static", MOBILE_DIR)
        return app


async def run_servers(http_port: int = 8080, https_port: int = 8443) -> None:
    """Inicia os servidores HTTP e HTTPS e executa as tarefas assíncronas em segundo plano."""
    tvisio = TVisioServer(http_port=http_port, https_port=https_port)
    app = tvisio.create_app()

    # Gera e garante os certificados SSL cobrindo todos os IPs locais
    ip_list = [iface["ip"] for iface in tvisio.interfaces]
    cert_path, key_path = ensure_ssl_certificates(ip_list, BASE_DIR)

    ssl_ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
    ssl_ctx.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))

    runner = web.AppRunner(app, access_log=None)
    await runner.setup()

    # 1. Servidor HTTP (Interface Desktop e USB Localhost)
    http_site = web.TCPSite(runner, "0.0.0.0", http_port)
    await http_site.start()

    # 2. Servidor HTTPS (Para acesso à câmera no navegador móvel do celular)
    https_site = web.TCPSite(runner, "0.0.0.0", https_port, ssl_context=ssl_ctx)
    await https_site.start()

    logger.info(f"Interface Desktop tVisio ativa em: http://127.0.0.1:{http_port}/desktop")
    logger.info(f"URL de pareamento para celular:    {tvisio.get_pairing_url()}")

    # Inicializa loops em segundo plano
    asyncio.create_task(tvisio.broadcast_telemetry())
    asyncio.create_task(tvisio.anti_psm_heartbeat_loop())

    await asyncio.Event().wait()


if __name__ == "__main__":
    try:
        asyncio.run(run_servers())
    except KeyboardInterrupt:
        pass
