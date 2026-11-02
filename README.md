<p align="center">
  <img src="assets/icon.svg" width="128" height="128" alt="tVisio Logo" />
</p>

<h1 align="center">tVisio — Ultra-Low Latency Virtual Webcam</h1>

<p align="center">
  <strong>Transforme qualquer smartphone em uma Câmera Virtual DirectShow profissional para Windows com latência ultra-baixa via QR Code.</strong><br/>
  Zero instalação no celular • Interface Translucent Glass UI • 1-Pass Kernel Fusion • Latência &lt; 15ms
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/Licen%C3%A7a-GPLv3-blue.svg" alt="Licença: GPLv3" /></a>
  <img src="https://img.shields.io/badge/Rust-2021%20Edition-orange?logo=rust" alt="Rust 2021" />
  <img src="https://img.shields.io/badge/GUI-Translucent%20Glass%20UI-cyan" alt="Translucent Glass UI" />
  <img src="https://img.shields.io/badge/Lat%C3%AAncia-6--14ms%20G2G-purple" alt="Latência 6-14ms" />
  <img src="https://img.shields.io/badge/Windows-DirectShow-0078D6?logo=windows" alt="DirectShow Virtual Camera" />
  <img src="https://img.shields.io/badge/Mobile-Universal%20PWA-green" alt="Suporte Mobile PWA" />
</p>

---

## 💡 Sobre o tVisio

O **tVisio** é um aplicativo de código aberto de alta performance escrito em **Rust** que transforma qualquer smartphone moderno em uma **webcam virtual profissional de estúdio** para Windows, compatível nativamente com **OBS Studio, Discord, Google Meet, Zoom, Microsoft Teams e qualquer programa DirectShow**.

Principais diferenciais:
1. **Zero instalação no celular**: Não requer baixar aplicativos de lojas. Basta ler o **QR Code** na tela do PC usando a câmera nativa do seu smartphone (Safari, Chrome ou Edge).
2. **Executável Único Ultraleve (~4.5 MB)**: Todos os recursos do transmissor móvel PWA, ícones e DLLs do driver DirectShow são compilados em um único arquivo `tVisio.exe` portátil.
3. **1-Pass Kernel Fusion (L1-LUT)**: O corte de Zoom/Pan digital (ePTZ), espelhamento, rotação (0°/90°/180°/270°), redimensionamento Letterbox e correção de cor via **LUT de 768 bytes no Cache L1** são executados em **uma única passada de memória** diretamente para o driver DirectShow.
4. **Eficiência Térmica & Bateria no Celular**: Inclui **Modo Eco OLED** automático em 8 segundos, mantendo o smartphone frio durante reuniões e transmissões longas.

---

## ⚡ Arquitetura Nativa em Rust

```text
  ┌─────────────────────────────────────────────────────────────┐
  │                 SMARTPHONE (PWA Mobile)                     │
  │  • Sensor da Câmera: Shutter Cap (4ms / 8ms / 16ms)         │
  │  • Lentes Físicas: Principal, Grande Angular, Telefoto      │
  │  • Modo Eco OLED: Apaga os pixels OLED após 8 segundos      │
  └──────────────────────────────┬──────────────────────────────┘
                                 │ Binary Turbo Zero-Queue (WS)
                                 │ ACK Imediato (1-Frame In-Flight)
                                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │                 SERVIDOR CORE RUST (Tokio + Axum)           │
  │  • TLS Multi-SAN Autoassinado em Memória (rcgen + rustls)   │
  │  • Pulso Anti-PSM 500Hz: Impede micro-sleep do rádio Wi-Fi  │
  └──────────────────────────────┬──────────────────────────────┘
                                 │
                                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │          PIPELINE 1-PASS KERNEL FUSION (vcam_engine.rs)     │
  │  • ePTZ Crop + Flip/Rotate + Letterbox + L1-LUT (768B)      │
  │  • Escrita em 1 única passada sem alocações por frame       │
  └──────────────┬───────────────────────────────┬──────────────┘
                 │                               │
                 ▼                               ▼
  ┌─────────────────────────────┐ ┌─────────────────────────────┐
  │  SHARED MEMORY DIRECTSHOW   │ │   JANELA NATIVA OPENGL      │
  │  • UnityCapture_Data0 (SHM) │ │  • Textura RGBA Direta GPU  │
  │  • 1280x720 @ 60 FPS        │ │  • Efeito Ambient Glow      │
  │  • OBS / Discord / Meet     │ │  • Bandeja Nativa Win32     │
  └─────────────────────────────┘ └─────────────────────────────┘
```

---

## 📦 Como Compilar e Executar

### Requisitos para Compilação
- **Sistema Operacional:** Windows 10 ou Windows 11 (64-bit).
- **Rust Toolchain:** Rust 1.80+ instalado via [rustup.rs](https://rustup.rs/).

### Compilando o Executável Otimizado (`tVisio.exe`)
No terminal na raiz do projeto, execute:
```bash
cargo build --release
```
O executável único e 100% portátil será gerado em:
```text
target/release/tVisio.exe
```
*(Você também pode dar um duplo clique diretamente em [`start_tvisio.bat`](start_tvisio.bat)).*

---

## 🗂️ Estrutura do Código-Fonte

- [`src/main.rs`](src/main.rs) — Ponto de entrada, Mutex Win32 de instância única, inicialização do runtime Tokio e janela nativa OpenGL.
- [`src/gui.rs`](src/gui.rs) — Interface Desktop Translucent Glass nativa (`eframe`/`egui`), Miniplayer GPU, gestos ePTZ, Dock de 7 controles e abas segmentadas.
- [`src/vcam_engine.rs`](src/vcam_engine.rs) — Pipeline de vídeo *1-Pass Kernel Fusion* (ePTZ + Flip/Rotate + Letterbox + LUT L1 de 768 bytes).
- [`src/unity_capture.rs`](src/unity_capture.rs) — Cliente nativo de Memória Compartilhada Win32 (`UnityCapture_Data0`) para injeção direta no DirectShow.
- [`src/server.rs`](src/server.rs) — Servidor assíncrono HTTP/HTTPS (`axum` + `rustls`), QR Code, WebSocket Binary Turbo Zero-Queue e pulso Anti-PSM 500Hz.
- [`src/win_integration.rs`](src/win_integration.rs) — Integração nativa com Windows (System Tray `tray-icon`, Registro `HKCU`, UAC `regsvr32`, Firewall e controle de `HWND`).
- [`src/cert_manager.rs`](src/cert_manager.rs) — Gerador instantâneo de certificados TLS X.509 Multi-SAN (`rcgen`).
- [`src/network_utils.rs`](src/network_utils.rs) — Descoberta e classificação de interfaces USB RNDIS/NCM, Hotspot Direto 1-Hop e Wi-Fi LAN.
- [`src/settings.rs`](src/settings.rs) — Persistência em `_local/settings.json` e recursos estáticos embutidos (`include_bytes!`).

---

## 📄 Licença

Este projeto é software livre distribuído sob os termos da licença **[GNU General Public License v3.0 (GPLv3)](LICENSE)**.

```text
Copyright (C) 2026 Tiago Treter (@TTreter)
```
