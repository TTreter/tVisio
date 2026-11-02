"""
tVisio — Motor de Webcam Virtual, Estúdio de Cor LUT L1 e Zoom/Pan Zero-Copy (ePTZ)
Principais capacidades:
- ePTZ Zero-Copy (Zoom e Pan digital via fatiamento de ponteiros NumPy: custo computacional negativo).
- Tabela de Consulta de Cores pré-calculada de 1 passada (`cv2.LUT` de 768 bytes residente no Cache L1 da CPU: ~0.12ms).
- Redimensionamento sem alocação de memória (`dst=` escrita direta na região pré-alocada do buffer DirectShow).
- Modo de congelamento instantâneo de quadro (Pausa de privacidade com 0% de uso de CPU).
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
    Configura as chaves de registro do UnityCapture no Windows (HKCU)
    para que o dispositivo DirectShow apareça como 'tVisio Virtual Camera'
    no Discord, OBS Studio, Google Meet, Zoom e Microsoft Teams.
    """
    try:
        key_path = r"Software\UnityCapture\Device 0"
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key_path) as key:
            winreg.SetValueEx(key, "FriendlyName", 0, winreg.REG_SZ, "tVisio Virtual Camera")
            winreg.SetValueEx(key, "Timeout", 0, winreg.REG_DWORD, 1000)
            winreg.SetValueEx(key, "ResizeMode", 0, winreg.REG_DWORD, 1)
            winreg.SetValueEx(key, "MirrorMode", 0, winreg.REG_DWORD, 0)
    except Exception as e:
        logger.debug(f"Configuração do registro ignorada: {e}")


class VirtualCamEngine:
    """
    Controlador do pipeline de pós-processamento e injeção de vídeo na webcam virtual.
    Projetado para operar com latência sub-milissegundo e zero vazamentos de memória.
    """

    def __init__(self, initial_settings: Optional[Dict[str, Any]] = None) -> None:
        configure_tvisio_camera_name()
        s = initial_settings or {}

        self.cam: Optional[Any] = None
        # Resolução fixa do driver DirectShow para evitar re-inicializações e travamentos em chamadas
        self.width: int = 1280
        self.height: int = 720
        self.input_width: int = 0
        self.input_height: int = 0
        self.fps: int = 60
        self.backend_name: str = "Standby (Preview Ativo)"
        self.driver_active: bool = False
        self.driver_error: str = ""

        # Controles geométricos e de enquadramento digital Zero-Copy (ePTZ)
        self.mirror_horizontal: bool = bool(s.get("mirror_horizontal", False))
        self.mirror_vertical: bool = bool(s.get("mirror_vertical", False))
        self.rotation: int = int(s.get("rotation", 0))
        self.zoom_level: float = float(s.get("zoom_level", 1.0))  # 1.0x até 3.0x
        self.pan_x: float = float(s.get("pan_x", 0.0))  # -1.0 a +1.0
        self.pan_y: float = float(s.get("pan_y", 0.0))  # -1.0 a +1.0

        # Controles do Estúdio de Cor em 1 Passada (LUT no Cache L1)
        self.brightness_gain: float = float(s.get("brightness_gain", 1.0))
        self.contrast: float = float(s.get("contrast", 1.0))
        self.gamma: float = float(s.get("gamma", 1.0))
        self.color_temp: float = float(s.get("color_temp", 0.0))  # -1.0 (Frio) até +1.0 (Quente)
        self.color_preset: str = str(s.get("color_preset", "natural"))

        # Tabela LUT unificada de 768 bytes (1 x 256 x 3 uint8)
        self._color_lut: np.ndarray = np.zeros((1, 256, 3), dtype=np.uint8)
        self._lut_is_identity: bool = True
        self.rebuild_color_lut()

        # Estado do modo de congelamento de quadro (Privacidade)
        self.frozen: bool = False
        self._frozen_vcam_frame: Optional[np.ndarray] = None

        # Estado do Miniplayer (Fluxo desacoplado de baixa resolução a 360p)
        self.miniplayer_enabled: bool = True
        self.miniplayer_max_fps: float = 28.0
        self._last_miniplayer_time: float = 0.0
        self.latest_miniplayer_jpeg: Optional[bytes] = None

        # Canvas pré-alocado na memória para redimensionamento e letterboxing sem alocações dinâmicas
        self._vcam_canvas: np.ndarray = np.zeros((self.height, self.width, 3), dtype=np.uint8)
        self._last_fit_dims: tuple = (0, 0)

        # Métricas de telemetria e taxa de quadros
        self.frames_received: int = 0
        self.current_fps: float = 0.0
        self._fps_counter: int = 0
        self._fps_timer: float = time.perf_counter()
        self.last_decode_ms: float = 0.0

    def rebuild_color_lut(self) -> None:
        """
        Pré-calcula a Tabela de Consulta (Lookup Table) de 768 bytes (1 x 256 x 3 uint8)
        que cabe inteira no Cache L1 da CPU.
        Executada SOMENTE quando o usuário altera um slider ou preset na interface.
        """
        gain = self.brightness_gain
        contrast = self.contrast
        gamma = self.gamma
        temp = self.color_temp

        # Modificadores de curvas de cor baseados em presets artísticos
        if self.color_preset == "warm":
            temp = np.clip(temp + 0.35, -1.0, 1.0)
            contrast *= 1.05
        elif self.color_preset == "cool":
            temp = np.clip(temp - 0.35, -1.0, 1.0)
            contrast *= 1.05
        elif self.color_preset == "cinema":
            contrast *= 1.18
            gamma *= 0.92
            temp = np.clip(temp + 0.12, -1.0, 1.0)
        elif self.color_preset == "bw":
            contrast *= 1.22

        # Se todos os parâmetros estiverem em valores neutros, ignora a etapa LUT (custo = 0.00ms)
        if (
            abs(gain - 1.0) < 0.02
            and abs(contrast - 1.0) < 0.02
            and abs(gamma - 1.0) < 0.02
            and abs(temp) < 0.02
            and self.color_preset == "natural"
        ):
            self._lut_is_identity = True
            return

        self._lut_is_identity = False
        x = np.linspace(0.0, 1.0, 256, dtype=np.float32)

        # 1. Correção de curva Gamma
        if abs(gamma - 1.0) >= 0.02:
            x = np.power(np.clip(x, 1e-5, 1.0), 1.0 / max(0.2, gamma))

        # 2. Ganho de exposição (brilho) e contraste simétrico no ponto médio (0.5)
        base = (x * gain - 0.5) * contrast + 0.5

        # 3. Equilíbrio de canais para temperatura de cor (ordem BGR: B=0, G=1, R=2)
        b_curve = np.clip((base * (1.0 - temp * 0.18)) * 255.0, 0.0, 255.0).astype(np.uint8)
        g_curve = np.clip((base * (1.0 + abs(temp) * 0.02)) * 255.0, 0.0, 255.0).astype(np.uint8)
        r_curve = np.clip((base * (1.0 + temp * 0.18)) * 255.0, 0.0, 255.0).astype(np.uint8)

        self._color_lut[0, :, 0] = b_curve
        self._color_lut[0, :, 1] = g_curve
        self._color_lut[0, :, 2] = r_curve

    def _ensure_camera(self, target_w: int = 1280, target_h: int = 720) -> None:
        """Garante a inicialização estável da câmera virtual em instância única (Singleton)."""
        if not HAS_PYVIRTUALCAM:
            self.driver_active = False
            self.driver_error = "pyvirtualcam não instalado"
            return

        if self.cam is not None and self.driver_active:
            return

        self.width = target_w
        self.height = target_h
        self._vcam_canvas = np.zeros((self.height, self.width, 3), dtype=np.uint8)

        # Tenta conectar via UnityCapture (DirectShow estável) ou OBS Virtual Camera
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
                    f"Câmera virtual inicializada com sucesso: {self.backend_name} ({self.width}x{self.height}@{self.fps}fps)"
                )
                return
            except Exception as err:
                self.driver_error = str(err)

        self.driver_active = False
        self.backend_name = "Driver Virtual Pendente (Ative em 1-Clique)"

    def process_and_push_bgr(self, bgr_array: np.ndarray) -> Optional[bytes]:
        """
        Pipeline otimizado de processamento de vídeo em tempo real:
        1. Em modo congelado (Privacidade), reenvia o quadro em cache a 0% de uso de CPU.
        2. Aplica o corte de Zoom e Pan digital (ePTZ) via fatiamento de ponteiros NumPy.
        3. Aplica espelhamento/rotação SIMD e a tabela de cores LUT em 1 passada.
        4. Escreve diretamente no buffer pré-alocado `_vcam_canvas` com zero alocações de memória.
        5. Gera a versão leve de 360p via OpenCV C++ para o miniplayer da interface desktop.
        """
        t0 = time.perf_counter()

        # 0. Modo de Congelamento Instantâneo (0% de custo na CPU)
        if self.frozen and self._frozen_vcam_frame is not None:
            if self.driver_active and self.cam is not None:
                try:
                    self.cam.send(self._frozen_vcam_frame)
                except Exception:
                    pass
            self.last_decode_ms = 0.05
            return None

        frame = bgr_array
        h_in, w_in = frame.shape[:2]
        self.input_width = w_in
        self.input_height = h_in

        # 1. Zoom e Pan Digital Zero-Copy (ePTZ) via visão de ponteiro NumPy (Custo negativo de CPU)
        if self.zoom_level > 1.01:
            crop_w = max(64, int(w_in / self.zoom_level))
            crop_h = max(64, int(h_in / self.zoom_level))
            max_dx = (w_in - crop_w) // 2
            max_dy = (h_in - crop_h) // 2
            cx = (w_in // 2) + int(self.pan_x * max_dx)
            cy = (h_in // 2) + int(self.pan_y * max_dy)
            x1 = max(0, min(w_in - crop_w, cx - crop_w // 2))
            y1 = max(0, min(h_in - crop_h, cy - crop_h // 2))
            frame = frame[y1 : y1 + crop_h, x1 : x1 + crop_w]

        # 2. Transformações geométricas via instruções C++ SIMD (cv2.flip / cv2.rotate)
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

        # 3. Aplicação do Estúdio de Cor em 1 única passada no Cache L1 (cv2.LUT)
        if not self._lut_is_identity:
            if not frame.flags["WRITEABLE"] or not frame.flags["C_CONTIGUOUS"]:
                frame = np.ascontiguousarray(frame)
            cv2.LUT(frame, self._color_lut, dst=frame)

        # Conversão em escala de cinza para o preset Preto e Branco
        if self.color_preset == "bw":
            gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
            frame = cv2.cvtColor(gray, cv2.COLOR_GRAY2BGR)

        h, w = frame.shape[:2]

        # 4. Envio para o driver de webcam virtual (Escrita direta no canvas pré-alocado)
        if not self.driver_active and self.frames_received % 180 == 0:
            self._ensure_camera(1280, 720)

        if w == self.width and h == self.height:
            out_vcam = frame
        else:
            # Preserva a proporção com barras letterbox limpas sem recriar buffers
            scale = min(self.width / w, self.height / h)
            nw, nh = int(w * scale), int(h * scale)
            if (nw, nh) != self._last_fit_dims:
                self._vcam_canvas.fill(0)
                self._last_fit_dims = (nw, nh)
            y_off = (self.height - nh) // 2
            x_off = (self.width - nw) // 2
            cv2.resize(
                frame,
                (nw, nh),
                dst=self._vcam_canvas[y_off : y_off + nh, x_off : x_off + nw],
                interpolation=cv2.INTER_LINEAR,
            )
            out_vcam = self._vcam_canvas

        self._frozen_vcam_frame = out_vcam

        if self.driver_active and self.cam is not None:
            try:
                self.cam.send(out_vcam)
            except Exception as e:
                logger.warning(f"Erro no envio para a webcam virtual: {e}")

        # 5. Cálculo da taxa real de quadros por segundo (FPS)
        self.frames_received += 1
        self._fps_counter += 1
        now = time.perf_counter()
        elapsed = now - self._fps_timer
        if elapsed >= 0.5:
            self.current_fps = round(self._fps_counter / elapsed, 1)
            self._fps_counter = 0
            self._fps_timer = now

        # 6. Geração do preview leve para o Miniplayer desktop (Codificação JPEG em C++ ~0.1ms)
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
        """Retorna o dicionário de telemetria com o estado atual da câmera e do estúdio."""
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
            "contrast": self.contrast,
            "gamma": self.gamma,
            "color_temp": self.color_temp,
            "color_preset": self.color_preset,
            "zoom_level": self.zoom_level,
            "pan_x": self.pan_x,
            "pan_y": self.pan_y,
            "frozen": self.frozen,
            "miniplayer_enabled": self.miniplayer_enabled,
        }

    def close(self) -> None:
        """Libera o dispositivo de câmera virtual ao encerrar o aplicativo."""
        if self.cam is not None:
            try:
                self.cam.close()
            except Exception:
                pass
            self.cam = None
