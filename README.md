<p align="center">
  <img src="assets/icon.svg" width="128" height="128" alt="tVisio Icon" />
</p>

<h1 align="center">tVisio</h1>

<p align="center">
  <strong>Transforme qualquer smartphone ou dispositivo com navegador em uma Webcam Virtual de latência ultra-baixa via QR Code.</strong><br/>
  Zero instalação no celular • <em>Translucent Glass UI</em> (Lucide Icons) • Motor Zero-Buffer (&lt; 15ms)
</p>

---

## ✨ Destaques

- **📱 Zero Instalação no Celular**: Basta ler o QR Code na tela do PC com a câmera nativa do iOS (Safari) ou Android (Chrome).
- **⚡ Engenharia de Latência Extrema (10ms – 17ms Glass-to-Glass)**:
  - **Shutter Speed Capping (`4ms` / `8ms`)**: Elimina o atraso óptico de exposição automática (`33.3ms`) do sensor da câmera.
  - **WebRTC Zero-Buffer (`playout-delay=0`)**: Injeção de extensão RTP no SDP para descarte imediato do Jitter Buffer.
  - **Pulso Anti-PSM (500 Hz)**: Batimento cardíaco UDP/WS de 1 byte a cada `2ms` que impede o chip Wi-Fi do celular de entrar em *Power Save Mode*.
  - **QoS de Hardware WMM (`DSCP EF`)**: Marcação de pacotes com prioridade de voz/vídeo (`IP_TOS = 0xB8`).
  - **DOM Video Detach (Modo Eco-Glass OLED)**: Desacopla a pré-visualização móvel do *VSync* da tela do celular e apaga os pixels OLED para zero aquecimento.
- **💎 Interface Translucent Glass UI & Miniplayer Inteligente**:
  - Enquanto o **Driver de Webcam Virtual** recebe o vídeo em **qualidade máxima (1080p @ 60fps)**, o **Miniplayer** do app renderiza uma sub-amostragem leve (`360p`) com *Ambient Video Glow* (consumindo < 1% de CPU/GPU) e modo *Zero-Render*.
- **🎨 Estúdio Zero-Overhead (ISP & Cache L1 LUT)**:
  - **Color Grading em 1 Passada (`cv2.LUT`)**: Exposição, Contraste, Temperatura de Cor e 5 Presets (*Natural*, *Quente*, *Frio*, *Cinema*, *P&B*) calculados em uma tabela de `768 bytes` no Cache L1 (`~0.12ms`).
  - **Zoom & Pan Digital Zero-Copy (`ePTZ`)**: Use o scroll do mouse sobre o Miniplayer para dar Zoom (`1.0x–3.0x`) e arraste para ajustar o enquadramento via fatiamento de ponteiro NumPy (`0.0ms` de custo).
  - **Controles de Hardware ISP**: Seletor de múltiplas lentes físicas (*Ultra-Wide*, *Principal*, *Telefoto*, *Frontal*), Trava de Foco (`AF-Lock`), Lanterna, Modo *Freeze Frame* (`0% CPU`), Grade de Enquadramento e Monitor de Bateria do celular.
  - **Pareamento Persistente (PWA)**: Token fixo salvo localmente para reconectar em 1 toque sem precisar ler o QR Code novamente.

---

## 🚀 Como Executar

1. Instale as dependências:
   ```bash
   pip install -r requirements.txt
   ```
2. Inicie o **tVisio**:
   ```bash
   python main.py
   ```
   *(Ou dê duplo clique em `start_tvisio.bat` no Windows).*
3. Escaneie o **QR Code** exibido na janela do aplicativo com o seu celular e permita o acesso à câmera!
