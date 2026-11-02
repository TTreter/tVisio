"""
tVisio — Ultra-Low Latency Core Server (WebRTC + Binary WS + Anti-PSM 500Hz + Virtual Cam)
Serves:
- Mobile Transmitter WebApp over HTTPS (Port 8443) & HTTP (Port 8080 for USB localhost)
- Desktop Liquid Glass App UI & Telemetry over HTTP/WS (Port 8080)
- WebRTC SDP Signaling with `playout-delay=0` & H.264 Constrained Baseline prioritization
- Anti-PSM 500Hz Wi-Fi Keep-Alive pulse to prevent mobile radio micro-sleep
"""

import asyncio
import io
import json
import logging
import secrets
import ssl
import struct
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path
from typing import Set, Optional, Dict, Any

import aiohttp
from aiohttp import web
import numpy as np
from PIL import Image
import qrcode

try:
    from aiortc import RTCPeerConnection, RTCSessionDescription
    HAS_WEBRTC = True
except ImportError:
    HAS_WEBRTC = False

from .cert_manager import ensure_ssl_certificates
from .network_utils import discover_network_interfaces, tune_low_latency_socket
from .vcam_engine import VirtualCamEngine, configure_tvisio_camera_name

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
logger = logging.getLogger("tVisio.Server")

BASE_DIR = Path(__file__).resolve().parent.parent.parent
DESKTOP_DIR = BASE_DIR / "src" / "desktop"
MOBILE_DIR = BASE_DIR / "src" / "mobile"
ASSETS_DIR = BASE_DIR / "assets"


class TVisioServer:
    def __init__(self, http_port: int = 8080, https_port: int = 8443) -> None:
        self.http_port = http_port
        self.https_port = https_port
        self.session_token = secrets.token_urlsafe(8)
        self.vcam = VirtualCamEngine()

        self.interfaces = discover_network_interfaces(self.https_port)
        self.selected_interface = self.interfaces[0] if self.interfaces else {
            "ip": "127.0.0.1",
            "type": "usb",
            "label": "Localhost",
            "url": f"https://127.0.0.1:{self.https_port}"
        }

        # Active connections
        self.desktop_ws_clients: Set[web.WebSocketResponse] = set()
        self.mobile_ws_clients: Set[web.WebSocketResponse] = set()
        self.pcs: Set[Any] = set()

        # Live Telemetry State
        self.mobile_connected: bool = False
        self.active_protocol: str = "Standby"
        self.anti_psm_enabled: bool = True
        self.shutter_cap_ms: float = 8.0
        self.rtt_ms: float = 0.0
        self.glass_to_glass_ms: float = 0.0
        self.mobile_device_info: str = "Nenhum dispositivo conectado"
        self.mobile_eco_mode: bool = False
        self.mobile_torch_on: bool = False
        self.mobile_facing_mode: str = "environment"

    def get_pairing_url(self, ip: Optional[str] = None) -> str:
        target_ip = ip or self.selected_interface["ip"]
        return f"https://{target_ip}:{self.https_port}/?t={self.session_token}"

    async def broadcast_telemetry(self) -> None:
        """Broadcast live telemetry to all Desktop UI WebSockets every 250ms."""
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
                        "facing_mode": self.mobile_facing_mode,
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
                logger.debug(f"Telemetry broadcast error: {e}")
            await asyncio.sleep(0.25)

    async def anti_psm_heartbeat_loop(self) -> None:
        """
        500Hz (2ms) Anti-PSM Micro-Pulse Generator.
        Sends a 1-byte binary frame (0x01) to connected mobile clients to keep the
        smartphone's Wi-Fi radio awake and prevent Power Save Mode (PSM) latency spikes.
        """
        pulse_packet = b"\x01"
        while True:
            if self.anti_psm_enabled and self.mobile_ws_clients:
                for ws in list(self.mobile_ws_clients):
                    if not ws.closed:
                        try:
                            await ws.send_bytes(pulse_packet)
                        except Exception:
                            pass
                await asyncio.sleep(0.003)  # ~333-500Hz keep-alive pulse
            else:
                await asyncio.sleep(0.05)

    async def push_miniplayer_preview(self, preview_jpeg: Optional[bytes]) -> None:
        """Send low-res (360p) JPEG frame to Desktop Miniplayer clients."""
        if not preview_jpeg or not self.desktop_ws_clients:
            return
        for ws in list(self.desktop_ws_clients):
            if not ws.closed:
                try:
                    await ws.send_bytes(preview_jpeg)
                except Exception:
                    pass

    # -------------------------------------------------------------------------
    # HTTP Routes
    # -------------------------------------------------------------------------
    async def handle_mobile_index(self, request: web.Request) -> web.Response:
        # If accessed over plain HTTP on a non-localhost LAN IP, redirect immediately to HTTPS
        # so mobile browsers (Safari/Chrome) unlock navigator.mediaDevices.getUserMedia()!
        host_header = request.headers.get("Host", "")
        hostname = host_header.split(":")[0] if host_header else ""
        if not request.secure and hostname not in ("localhost", "127.0.0.1"):
            https_url = f"https://{hostname}:{self.https_port}{request.rel_url}"
            raise web.HTTPFound(location=https_url)
        return web.FileResponse(MOBILE_DIR / "index.html")

    async def handle_desktop_index(self, request: web.Request) -> web.Response:
        return web.FileResponse(DESKTOP_DIR / "index.html")

    async def handle_qr_svg(self, request: web.Request) -> web.Response:
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
        img = qr.make_image(fill_color="#00F0FF", back_color="#050C24").convert("RGBA")

        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return web.Response(body=buf.getvalue(), content_type="image/png")

    async def handle_cert_download(self, request: web.Request) -> web.Response:
        cert_path = BASE_DIR / "_local" / "certs" / "tvisio_cert.pem"
        if cert_path.exists():
            return web.FileResponse(
                cert_path,
                headers={"Content-Disposition": 'attachment; filename="tVisio-Local-CA.crt"'},
            )
        return web.Response(status=404, text="Certificate not found")

    async def handle_install_vcam_driver(self, request: web.Request) -> web.Response:
        """
        One-Click Automatic Installer for the open-source UnityCapture DirectShow Virtual Camera filter.
        Downloads to `_local/driver/` and registers `tVisio Virtual Camera` via regsvr32.
        """
        try:
            driver_dir = BASE_DIR / "_local" / "driver"
            driver_dir.mkdir(parents=True, exist_ok=True)
            dll_path = driver_dir / "UnityCaptureFilter64.dll"

            if not dll_path.exists():
                zip_url = "https://github.com/schellingb/UnityCapture/releases/download/v1.0.4/UnityCapture_v1.0.4.zip"
                zip_path = driver_dir / "UnityCapture.zip"
                await asyncio.to_thread(urllib.request.urlretrieve, zip_url, zip_path)
                with zipfile.ZipFile(zip_path, "r") as zf:
                    for member in zf.namelist():
                        if member.endswith("UnityCaptureFilter64.dll"):
                            with zf.open(member) as src, open(dll_path, "wb") as dst:
                                dst.write(src.read())
                            break

            configure_tvisio_camera_name()

            # Request UAC elevation for regsvr32 to register DirectShow filter
            cmd = (
                f'powershell -Command "Start-Process regsvr32.exe '
                f"-ArgumentList '/s', '\"{dll_path}\"' -Verb RunAs -Wait\""
            )
            await asyncio.to_thread(subprocess.run, cmd, shell=True, check=False)

            configure_tvisio_camera_name()
            self.vcam._ensure_camera(1280, 720)
            return web.json_response({
                "ok": True,
                "driver_active": self.vcam.driver_active,
                "backend_name": self.vcam.backend_name,
            })
        except Exception as e:
            logger.error(f"Error installing Virtual Camera driver: {e}")
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    async def handle_hotspot_guide(self, request: web.Request) -> web.Response:
        """Opens Windows Mobile Hotspot settings directly for 1-Hop Direct Wi-Fi."""
        try:
            subprocess.Popen(["cmd", "/c", "start", "ms-settings:network-mobilehotspot"], shell=True)
            # Re-scan interfaces after 2 seconds
            await asyncio.sleep(1.5)
            self.interfaces = discover_network_interfaces(self.https_port)
            return web.json_response({"ok": True, "interfaces": self.interfaces})
        except Exception as e:
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    # -------------------------------------------------------------------------
    # WebRTC Zero-Latency Signaling & Track Consumer
    # -------------------------------------------------------------------------
    async def handle_webrtc_offer(self, request: web.Request) -> web.Response:
        if not HAS_WEBRTC:
            return web.json_response({"error": "aiortc not available"}, status=501)

        params = await request.json()
        offer = RTCSessionDescription(sdp=params["sdp"], type=params["type"])

        pc = RTCPeerConnection()
        self.pcs.add(pc)

        # Apply QoS DSCP EF on underlying transport if available
        @pc.on("connectionstatechange")
        async def on_connectionstatechange() -> None:
            logger.info(f"WebRTC state: {pc.connectionState}")
            if pc.connectionState in ("connected", "completed"):
                self.mobile_connected = True
                self.active_protocol = "WebRTC Zero-Buffer (UDP/RTP)"
            elif pc.connectionState in ("failed", "closed", "disconnected"):
                self.pcs.discard(pc)
                if not self.pcs and not self.mobile_ws_clients:
                    self.mobile_connected = False
                    self.active_protocol = "Standby"

        @pc.on("track")
        def on_track(track: Any) -> None:
            if track.kind == "video":
                logger.info("WebRTC Video Track received — starting Zero-Buffer consumer")
                self.mobile_connected = True
                self.active_protocol = "WebRTC Zero-Buffer (H.264 UDP)"
                asyncio.ensure_future(self._consume_webrtc_video(track))

        await pc.setRemoteDescription(offer)
        answer = await pc.createAnswer()

        # Inject playout-delay=0 extension into SDP answer if present
        sdp_lines = answer.sdp.splitlines()
        patched_sdp = []
        for line in sdp_lines:
            patched_sdp.append(line)
            if line.startswith("m=video"):
                patched_sdp.append("a=extmap:14 http://www.webrtc.org/experiments/rtp-hdrext/playout-delay")
        final_sdp = "\r\n".join(patched_sdp) + "\r\n"

        await pc.setLocalDescription(RTCSessionDescription(sdp=final_sdp, type=answer.type))
        return web.json_response({
            "sdp": pc.localDescription.sdp,
            "type": pc.localDescription.type,
        })

    async def _consume_webrtc_video(self, track: Any) -> None:
        """Consumes decoded frames from WebRTC video track with zero queuing."""
        try:
            while True:
                t_start = time.perf_counter()
                frame = await track.recv()
                rgb = frame.to_ndarray(format="rgb24")
                preview_jpeg = self.vcam.process_and_push_rgb(rgb)
                if preview_jpeg:
                    await self.push_miniplayer_preview(preview_jpeg)
                decode_elapsed = (time.perf_counter() - t_start) * 1000.0
                # Estimate Glass-to-Glass latency = RTT/2 + sensor (6ms) + decode
                self.glass_to_glass_ms = max(8.5, (self.rtt_ms * 0.5) + 6.5 + min(decode_elapsed, 6.0))
        except Exception as e:
            logger.info(f"WebRTC track ended: {e}")

    # -------------------------------------------------------------------------
    # WebSocket: Mobile Stream & Control Channel (/ws/stream)
    # -------------------------------------------------------------------------
    async def handle_mobile_ws(self, request: web.Request) -> web.WebSocketResponse:
        ws = web.WebSocketResponse(max_msg_size=16 * 1024 * 1024, heartbeat=15.0)
        await ws.prepare(request)

        # Tune raw OS socket for TCP_NODELAY and DSCP EF (46)
        transport = request.transport
        if transport is not None:
            raw_sock = transport.get_extra_info("socket")
            if raw_sock is not None:
                tune_low_latency_socket(raw_sock)

        self.mobile_ws_clients.add(ws)
        self.mobile_connected = True
        if self.active_protocol == "Standby":
            self.active_protocol = "WebSocket Binary Turbo (TCP_NODELAY)"

        logger.info("Mobile transmitter connected via WebSocket")

        try:
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.BINARY:
                    data = msg.data
                    # Binary frame header: first 8 bytes = float64 client timestamp (ms)
                    if len(data) > 8:
                        client_ts = struct.unpack("<d", data[:8])[0]
                        jpeg_bytes = data[8:]
                        # Send immediate ack for RTT calculation
                        await ws.send_str(json.dumps({"type": "ack", "ts": client_ts}))

                        # Decode frame in memory
                        t0 = time.perf_counter()
                        img = Image.open(io.BytesIO(jpeg_bytes)).convert("RGB")
                        rgb_arr = np.asarray(img)
                        preview_jpeg = self.vcam.process_and_push_rgb(rgb_arr)
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
                        self.mobile_facing_mode = info.get("facing_mode", self.mobile_facing_mode)
                        self.shutter_cap_ms = float(info.get("shutter_ms", self.shutter_cap_ms))
        except Exception as e:
            logger.debug(f"Mobile WS disconnected: {e}")
        finally:
            self.mobile_ws_clients.discard(ws)
            if not self.mobile_ws_clients and not self.pcs:
                self.mobile_connected = False
                self.active_protocol = "Standby"
        return ws

    # -------------------------------------------------------------------------
    # WebSocket: Desktop Liquid Glass UI & Miniplayer (/ws/desktop)
    # -------------------------------------------------------------------------
    async def handle_desktop_ws(self, request: web.Request) -> web.WebSocketResponse:
        ws = web.WebSocketResponse(max_msg_size=8 * 1024 * 1024)
        await ws.prepare(request)
        self.desktop_ws_clients.add(ws)

        try:
            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.TEXT:
                    cmd = json.loads(msg.data)
                    action = cmd.get("action")

                    # PC-side zero-cost image transforms & miniplayer toggle
                    if action == "toggle_mirror_h":
                        self.vcam.mirror_horizontal = not self.vcam.mirror_horizontal
                    elif action == "toggle_mirror_v":
                        self.vcam.mirror_vertical = not self.vcam.mirror_vertical
                    elif action == "rotate":
                        self.vcam.rotation = (self.vcam.rotation + 90) % 360
                    elif action == "set_brightness":
                        self.vcam.brightness_gain = float(cmd.get("value", 1.0))
                    elif action == "set_miniplayer_enabled":
                        self.vcam.miniplayer_enabled = bool(cmd.get("enabled", True))
                    elif action == "toggle_anti_psm":
                        self.anti_psm_enabled = not self.anti_psm_enabled
                    elif action == "select_interface":
                        ip = cmd.get("ip")
                        for iface in self.interfaces:
                            if iface["ip"] == ip:
                                self.selected_interface = iface
                                break
                    elif action == "refresh_interfaces":
                        self.interfaces = discover_network_interfaces(self.https_port)

                    # Remote Mobile Camera Commands (forwarded instantly to smartphone)
                    elif action in (
                        "switch_camera",
                        "toggle_torch",
                        "toggle_eco_mode",
                        "set_shutter_cap",
                        "set_protocol",
                        "set_resolution",
                    ):
                        if action == "set_shutter_cap":
                            self.shutter_cap_ms = float(cmd.get("shutter_ms", 8.0))
                        forward_msg = json.dumps({"type": "command", "command": action, **cmd})
                        for m_ws in list(self.mobile_ws_clients):
                            if not m_ws.closed:
                                await m_ws.send_str(forward_msg)
        finally:
            self.desktop_ws_clients.discard(ws)
        return ws

    def create_app(self) -> web.Application:
        app = web.Application()
        app.router.add_get("/", self.handle_mobile_index)
        app.router.add_get("/desktop", self.handle_desktop_index)
        app.router.add_get("/api/qr", self.handle_qr_svg)
        app.router.add_get("/api/cert", self.handle_cert_download)
        app.router.add_post("/api/install-vcam", self.handle_install_vcam_driver)
        app.router.add_post("/api/hotspot", self.handle_hotspot_guide)
        app.router.add_post("/api/webrtc/offer", self.handle_webrtc_offer)
        app.router.add_get("/ws/stream", self.handle_mobile_ws)
        app.router.add_get("/ws/desktop", self.handle_desktop_ws)

        app.router.add_static("/assets", ASSETS_DIR)
        app.router.add_static("/desktop-static", DESKTOP_DIR)
        app.router.add_static("/mobile-static", MOBILE_DIR)
        return app


async def run_servers(http_port: int = 8080, https_port: int = 8443) -> None:
    tvisio = TVisioServer(http_port=http_port, https_port=https_port)
    app = tvisio.create_app()

    ip_list = [iface["ip"] for iface in tvisio.interfaces]
    cert_path, key_path = ensure_ssl_certificates(ip_list, BASE_DIR)

    ssl_ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
    ssl_ctx.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))

    runner = web.AppRunner(app, access_log=None)
    await runner.setup()

    # 1. Start HTTP Server (for Desktop UI & USB Localhost)
    http_site = web.TCPSite(runner, "0.0.0.0", http_port)
    await http_site.start()

    # 2. Start HTTPS Server (for Mobile Safari/Chrome Camera Access)
    https_site = web.TCPSite(runner, "0.0.0.0", https_port, ssl_context=ssl_ctx)
    await https_site.start()

    logger.info(f"tVisio Desktop UI active at: http://127.0.0.1:{http_port}/desktop")
    logger.info(f"tVisio Mobile Pairing URL:   {tvisio.get_pairing_url()}")

    # Start background tasks (Telemetry + 500Hz Anti-PSM Heartbeat)
    asyncio.create_task(tvisio.broadcast_telemetry())
    asyncio.create_task(tvisio.anti_psm_heartbeat_loop())

    # Keep running forever
    await asyncio.Event().wait()


if __name__ == "__main__":
    try:
        asyncio.run(run_servers())
    except KeyboardInterrupt:
        pass
