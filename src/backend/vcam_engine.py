"""
tVisio — Virtual Camera Engine & Decoupled Low-Overhead Miniplayer Pipeline
- Uses OpenCV (C++ SIMD) for zero-allocation image transforms, brightness LUTs, and 360p JPEG encoding.
- Keeps a stable Virtual Camera instance (no re-initialization flicker when rotating or adapting resolution).
"""

import time
import winreg
import logging
from typing import Optional, Dict, Any
import cv2
import numpy as np

try:
    import pyvirtualcam
    HAS_PYVIRTUALCAM = True
except ImportError:
    HAS_PYVIRTUALCAM = False

logger = logging.getLogger("tVisio.VCam")


def configure_tvisio_camera_name() -> None:
    """
    Configure UnityCapture registry key under HKCU so the DirectShow device
    appears as 'tVisio Virtual Camera' in Discord, OBS, Zoom, and Meet.
    """
    try:
        key_path = r"Software\UnityCapture\Device 0"
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key_path) as key:
            winreg.SetValueEx(key, "FriendlyName", 0, winreg.REG_SZ, "tVisio Virtual Camera")
            winreg.SetValueEx(key, "Timeout", 0, winreg.REG_DWORD, 1000)
            winreg.SetValueEx(key, "ResizeMode", 0, winreg.REG_DWORD, 1)
            winreg.SetValueEx(key, "MirrorMode", 0, winreg.REG_DWORD, 0)
    except Exception as e:
        logger.debug(f"Registry configuration skipped: {e}")


class VirtualCamEngine:
    def __init__(self) -> None:
        configure_tvisio_camera_name()
        self.cam: Optional[Any] = None
        # Fixed stable canvas for the Virtual Camera driver so OBS/Discord never flicker
        self.width: int = 1280
        self.height: int = 720
        self.input_width: int = 0
        self.input_height: int = 0
        self.fps: int = 60
        self.backend_name: str = "Standby (Preview Ativo)"
        self.driver_active: bool = False
        self.driver_error: str = ""

        # Image processing controls (Zero cost on mobile, applied via C++ SIMD on PC)
        self.mirror_horizontal: bool = False
        self.mirror_vertical: bool = False
        self.rotation: int = 0  # 0, 90, 180, 270
        self.brightness_gain: float = 1.0

        # Miniplayer state (Decoupled low-res 360p stream)
        self.miniplayer_enabled: bool = True
        self.miniplayer_max_fps: float = 28.0
        self._last_miniplayer_time: float = 0.0
        self.latest_miniplayer_jpeg: Optional[bytes] = None

        # Pre-allocated black canvas for letterboxing when rotating portrait <-> landscape
        self._vcam_canvas: np.ndarray = np.zeros((self.height, self.width, 3), dtype=np.uint8)

        # Telemetry metrics
        self.frames_received: int = 0
        self.current_fps: float = 0.0
        self._fps_counter: int = 0
        self._fps_timer: float = time.perf_counter()
        self.last_decode_ms: float = 0.0

    def _ensure_camera(self, target_w: int = 1280, target_h: int = 720) -> None:
        if not HAS_PYVIRTUALCAM:
            self.driver_active = False
            self.driver_error = "pyvirtualcam não instalado"
            return

        if self.cam is not None and self.driver_active:
            return

        self.width = target_w
        self.height = target_h
        self._vcam_canvas = np.zeros((self.height, self.width, 3), dtype=np.uint8)

        for backend in ("unitycapture", "obs"):
            try:
                self.cam = pyvirtualcam.Camera(
                    width=self.width,
                    height=self.height,
                    fps=self.fps,
                    fmt=pyvirtualcam.PixelFormat.BGR,
                    backend=backend,
                )
                self.driver_active = True
                self.backend_name = f"{self.cam.device} ({backend})"
                self.driver_error = ""
                logger.info(
                    f"Virtual Camera initialized once: {self.backend_name} ({self.width}x{self.height}@{self.fps}fps)"
                )
                return
            except Exception as err:
                self.driver_error = str(err)

        self.driver_active = False
        self.backend_name = "Driver Virtual Pendente (1-Click Install)"

    def process_and_push_bgr(self, bgr_array: np.ndarray) -> Optional[bytes]:
        """
        Receives a decoded BGR numpy array (H, W, 3) directly from PyAV/OpenCV:
        1. Applies ultra-fast C++ OpenCV SIMD transforms (Mirror, Rotate, Brightness).
        2. Sends frame to Virtual Camera Driver without re-initializing shared memory.
        3. Encodes a lightweight 360p JPEG via OpenCV C++ for the Desktop Miniplayer.
        """
        t0 = time.perf_counter()
        frame = bgr_array

        # 1. C++ SIMD Flip / Rotate / Brightness (Zero float32 allocations!)
        if self.mirror_horizontal and self.mirror_vertical:
            frame = cv2.flip(frame, -1)
        elif self.mirror_horizontal:
            frame = cv2.flip(frame, 1)
        elif self.mirror_vertical:
            frame = cv2.flip(frame, 0)

        if self.rotation == 90:
            frame = cv2.rotate(frame, cv2.ROTATE_90_CLOCKWISE)
        elif self.rotation == 180:
            frame = cv2.rotate(frame, cv2.ROTATE_180)
        elif self.rotation == 270:
            frame = cv2.rotate(frame, cv2.ROTATE_90_COUNTERCLOCKWISE)

        if abs(self.brightness_gain - 1.0) > 0.03:
            frame = cv2.convertScaleAbs(frame, alpha=self.brightness_gain, beta=0)

        h, w = frame.shape[:2]
        self.input_width = w
        self.input_height = h

        # 2. Push to Virtual Webcam Driver (Keeping fixed driver dimensions to avoid memory leaks)
        if not self.driver_active and self.frames_received % 180 == 0:
            self._ensure_camera(1280, 720)

        if self.driver_active and self.cam is not None:
            try:
                if w == self.width and h == self.height:
                    self.cam.send(frame)
                else:
                    # Aspect-preserving fast fit into the Virtual Camera canvas
                    scale = min(self.width / w, self.height / h)
                    nw, nh = int(w * scale), int(h * scale)
                    resized = cv2.resize(frame, (nw, nh), interpolation=cv2.INTER_LINEAR)
                    if nw == self.width and nh == self.height:
                        self.cam.send(resized)
                    else:
                        self._vcam_canvas.fill(0)
                        y_off = (self.height - nh) // 2
                        x_off = (self.width - nw) // 2
                        self._vcam_canvas[y_off : y_off + nh, x_off : x_off + nw] = resized
                        self.cam.send(self._vcam_canvas)
            except Exception as e:
                logger.warning(f"VirtualCam send error: {e}")

        # 3. Update Telemetry FPS
        self.frames_received += 1
        self._fps_counter += 1
        now = time.perf_counter()
        elapsed = now - self._fps_timer
        if elapsed >= 0.5:
            self.current_fps = round(self._fps_counter / elapsed, 1)
            self._fps_counter = 0
            self._fps_timer = now

        # 4. Generate Low-Overhead Miniplayer Frame (360p Downscale via OpenCV C++ JPEG encoder)
        preview_jpeg: Optional[bytes] = None
        if self.miniplayer_enabled and (now - self._last_miniplayer_time) >= (1.0 / self.miniplayer_max_fps):
            self._last_miniplayer_time = now
            step = max(1, h // 320)
            small_bgr = frame[::step, ::step]
            ok, enc = cv2.imencode(".jpg", small_bgr, [int(cv2.IMWRITE_JPEG_QUALITY), 60])
            if ok:
                preview_jpeg = enc.tobytes()
                self.latest_miniplayer_jpeg = preview_jpeg

        self.last_decode_ms = round((time.perf_counter() - t0) * 1000.0, 2)
        return preview_jpeg

    def get_status(self) -> Dict[str, Any]:
        res_str = f"{self.input_width}x{self.input_height}" if self.input_width else "Aguardando Stream"
        return {
            "driver_active": self.driver_active,
            "backend_name": self.backend_name,
            "resolution": res_str,
            "fps": self.current_fps,
            "frames_received": self.frames_received,
            "decode_ms": self.last_decode_ms,
            "mirror_horizontal": self.mirror_horizontal,
            "mirror_vertical": self.mirror_vertical,
            "rotation": self.rotation,
            "brightness_gain": self.brightness_gain,
            "miniplayer_enabled": self.miniplayer_enabled,
        }

    def close(self) -> None:
        if self.cam is not None:
            try:
                self.cam.close()
            except Exception:
                pass
            self.cam = None
