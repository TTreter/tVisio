<p align="center">
  <img src="assets/icon.svg" width="128" height="128" alt="tVisio Logo" />
</p>

<h1 align="center">tVisio — Ultra-Low Latency Virtual Webcam</h1>

<p align="center">
  <strong>Transforme qualquer smartphone em uma Câmera Virtual DirectShow profissional para Windows com latência ultra-baixa via QR Code.</strong><br/>
  Zero instalação no celular • PWA WebRTC H.264 • Interface Translucent Glass UI • Lucide Icons • Latência &lt; 15ms
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/Licen%C3%A7a-MIT-blue.svg" alt="Licença: MIT" /></a>
  <img src="https://img.shields.io/badge/Python-3.10%2B-blue?logo=python" alt="Python 3.10+" />
  <img src="https://img.shields.io/badge/WebRTC-Zero--Buffer-emerald" alt="WebRTC Zero-Buffer" />
  <img src="https://img.shields.io/badge/Lat%C3%AAncia-10--17ms%20G2G-purple" alt="Latência 10-17ms" />
  <img src="https://img.shields.io/badge/Windows-DirectShow-0078D6?logo=windows" alt="DirectShow Virtual Camera" />
  <img src="https://img.shields.io/badge/Mobile-iOS%20%7C%20Android-green" alt="Suporte iOS e Android" />
</p>

---

## 💡 Sobre o tVisio

O **tVisio** é um aplicativo de código aberto de alta performance que transforma qualquer smartphone (iPhone ou Android) em uma **webcam virtual profissional de estúdio** para o seu computador com Windows, compatível nativamente com **OBS Studio, Discord, Google Meet, Zoom, Microsoft Teams e qualquer programa DirectShow**.

Diferente de soluções convencionais pesadas e proprietárias (como DroidCam, EpocCam ou Camo):
1. **Zero instalação no celular**: Não requer baixar aplicativos de lojas. Basta ler o **QR Code** na tela do PC usando a câmera nativa do iOS (Safari) ou Android (Chrome).
2. **Engenharia de Latência Extrema (10ms a 17ms Glass-to-Glass)**: O sinal de vídeo chega ao PC quase instantaneamente, dispensando sincronizadores manuais de áudio no OBS.
3. **Eficiência Térmica & Bateria**: Inclui **Modo Eco OLED**, auto-blackout em 8 segundos e desconexão de hardware do elemento `<video>` móvel, mantendo o celular frio e poupando bateria durante transmissões longas.
4. **Estúdio de Imagem no Cache L1 (Zero-Overhead)**: Ajuste de ganho, contraste, temperatura e zoom digital ePTZ com custo de CPU praticamente nulo via tabelas LUT de 768 bytes processadas em SIMD C++ (OpenCV).

---

## ⚡ Arquitetura e Engenharia de Baixa Latência

```
  ┌─────────────────────────────────────────────────────────────┐
  │                 SMARTPHONE (iOS / Android)                  │
  │  • Sensor da Câmera: Shutter Cap (4ms / 8ms / 16ms)         │
  │  • Lentes Físicas: Principal, Grande Angular, Telefoto      │
  │  • Modo Eco OLED: Desconexão de Hardware (0 decodificação)  │
  └──────────────┬───────────────────────────────┬──────────────┘
                 │ WebRTC H.264 (P2P Local)      │ Fallback Turbo WS
                 │ playout-delay=0 (Sem Jitter)  │ JPEG Binário
                 ▼                               ▼
  ┌─────────────────────────────────────────────────────────────┐
  │                   REDE LOCAL WI-FI / CABO                   │
  │  • Pulso Anti-PSM 500Hz: Impede o rádio Wi-Fi de dormir     │
  │  • QoS WMM Prioritário: Marcação DSCP EF (IP_TOS = 0xB8)    │
  └──────────────┬───────────────────────────────┬──────────────┘
                 │
                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │                   MOTOR BACKEND (tVisio PC)                 │
  │  • aiohttp + aiortc: Servidor assíncrono multithread        │
  │  • Estúdio L1-LUT: cv2.LUT (Tabela 768B em Cache L1, 0.1ms) │
  │  • ePTZ Zero-Copy: Fatiamento NumPy de alta performance     │
  └──────────────┬───────────────────────────────┬──────────────┘
                 │
                 ├───────────────────────────────┐
                 ▼                               ▼
  ┌─────────────────────────────┐ ┌─────────────────────────────┐
  │    DRIVER VIRTUAL CAMERA    │ │  TRANSLUCENT GLASS DESKTOP  │
  │  • DirectShow UnityCapture  │ │  • Miniplayer Leve (360p)   │
  │  • Resolução Nativa até     │ │  • Efeito Glow Ambiente     │
  │    1080p @ 60 FPS           │ │  • Controles de Estúdio     │
  │  • OBS / Discord / Meet     │ │  • Zero-Render Mode         │
  └─────────────────────────────┘ └─────────────────────────────┘
```

---

## ✨ Principais Recursos

| Recurso | Descrição |
| :--- | :--- |
| **📱 Conexão Instantânea via QR Code** | Escaneie o QR Code na tela do computador para iniciar a transmissão imediatamente. |
| **⚡ Motor Zero-Buffer** | SDP modificado com `playout-delay=0` descartando buffers de atraso do WebRTC. |
| **📶 Pulso Anti-PSM (500 Hz)** | Batimento de 1 byte mantendo o chip de rádio Wi-Fi 802.11 acordado em potência máxima. |
| **🎯 Shutter Speed Capping (4ms / 8ms)** | Limita a exposição máxima do sensor do smartphone para eliminar atrasos de exposição (`33ms`). |
| **❄️ Modo Eco OLED Inteligente** | Desconecta a camada de renderização móvel e apaga a tela OLED após 8s para evitar aquecimento. |
| **🎨 Estúdio de Cor L1-LUT** | Ajuste de exposição, contraste, temperatura e presets (*Natural, Quente, Frio, Cinema, P&B*) em 0.1ms. |
| **🔍 Zoom e Enquadramento ePTZ** | Zoom digital de 1.0x a 3.0x via scroll do mouse e arraste para enquadramento (*pan*) sem cópia de memória. |
| **📷 Controle de Lentes Físicas** | Selecione diretamente no PC qual lente do smartphone utilizar (Ultra-Wide, Macro, Telefoto ou Frontal). |
| **🔒 Trava de Foco (AF-Lock)** | Trava o autofoco da câmera para evitar respiração de foco (*focus hunting*) durante reuniões. |
| **🧊 Freeze Frame (Congelar Imagem)** | Congela a imagem na câmera virtual instantaneamente para privacidade sem interromper a chamada. |
| **💎 Interface Translucent Glass UI** | Interface translúcida com efeitos de vidro fosco, Lucide Icons e layout 100vh sem rolagem. |
| **🔋 Monitor de Bateria Integrado** | Acompanha a porcentagem de bateria e status de carregamento do smartphone no painel do PC. |

---

## 📦 Instalação e Requisitos

### Requisitos do Sistema
- **Sistema Operacional:** Windows 10 ou Windows 11 (64-bit).
- **Python:** Versão 3.10 ou superior instalada ([python.org](https://www.python.org/downloads/)).
- **Navegador no Celular:** Safari (iOS 15+) ou Google Chrome / Edge (Android 9+).
- **Rede:** Computador e celular conectados à mesma rede Wi-Fi local (redes de 5 GHz recomendadas).

### Passo a Passo de Instalação

1. **Clone o repositório:**
   ```bash
   git clone https://github.com/TTreter/tVisio.git
   cd tVisio
   ```

2. **Crie e ative um ambiente virtual (recomendado):**
   ```bash
   python -m venv venv
   .\venv\Scripts\activate
   ```

3. **Instale as dependências:**
   ```bash
   pip install -r requirements.txt
   ```

---

## 🚀 Como Iniciar

Execute o aplicativo tVisio com o comando:
```bash
python main.py
```
*(Ou simplesmente dê um duplo clique no arquivo [`start_tvisio.bat`](file:///j:/Projetos/tVisio/start_tvisio.bat) no Windows).*

A janela nativa desktop do **tVisio** abrirá automaticamente.

### Conectando seu Celular:
1. Aponte a câmera do celular para o **QR Code** exibido no painel de conexão.
2. Abra o link gerado (`https://192.168.x.x:8443/...`).
3. Como o tVisio gera certificados SSL locais com criptografia ECDSA para permitir o acesso da câmera pelo navegador, toque em **"Avançado" -> "Continuar"** no navegador do celular.
4. Conceda a permissão de acesso à câmera e pronto: seu celular já estará transmitindo em tempo real!

### Ativando a Câmera Virtual no Windows:
- Na primeira execução, clique no botão **"Ativar Driver Virtual Cam"** no painel do tVisio. O aplicativo registrará o driver DirectShow automaticamente.
- Abra o **OBS Studio, Discord, Zoom ou Google Meet**, selecione a câmera **"tVisio Virtual Camera"** (ou *"Unity Video Capture"*) nas configurações de vídeo.

---

## ⌨️ Atalhos e Gestos no Miniplayer

- **Zoom Digital:** Gire a roda do mouse (*scroll*) sobre a imagem de vídeo para aplicar zoom de `1.0x` até `3.0x`.
- **Enquadramento Panorâmico:** Com o zoom ativo, clique e arraste com o botão esquerdo do mouse para centralizar o enquadramento.
- **Redefinir Ajustes:** Dê um **duplo clique** em qualquer controle deslizante (Ganho, Contraste, Temperatura ou Zoom) para restaurar instantaneamente para o padrão.
- **Modo Zero-Render:** Clique no botão de olho (*Preview Ativo*) no miniplayer para desativar a renderização interna de tela do PC caso queira economizar 100% da GPU durante gravações ou jogos pesados.
- **Grade de Enquadramento:** Pressione o botão `# Grade` para exibir a grade clássica da regra dos terços.

---

## 🛠️ Solução de Problemas

<details>
<summary><strong>1. O QR Code não carrega no celular ou a página fica em carregamento infinito</strong></summary>

- Certifique-se de que o computador e o celular estão conectados na **mesma rede Wi-Fi**.
- Verifique se a rede do Windows está definida como **Rede Privada** (não pública).
- No firewall do Windows, certifique-se de autorizar o Python nas portas `8080` e `8443`.
- Se seu roteador tiver isolamento de clientes AP ativado, você pode usar o botão **"Abrir Hotspot Direto 1-Hop (5GHz)"** no tVisio para conectar o celular diretamente ao hotspot Wi-Fi do computador.
</details>

<details>
<summary><strong>2. Alerta de certificado no navegador do smartphone</strong></summary>

- O tVisio gera um certificado HTTPS auto-assinado localmente na inicialização para viabilizar as APIs modernas do WebRTC (`getUserMedia`).
- No **Chrome Android**: Toque em *Avançado* e depois em *Ir para [IP] (não seguro)*.
- No **Safari iOS**: Toque em *Mostrar detalhes* e depois em *Visitar este site não seguro*.
</details>

<details>
<summary><strong>3. O celular esquenta após muito tempo de transmissão</strong></summary>

- O tVisio já vem configurado de fábrica em **720p @ 30 FPS**, que consome até 60% menos energia do sensor do que modos de 60/120 FPS.
- Ative o **Modo Eco** (ou deixe o auto-eco acionar em 8 segundos). O visor do celular apagará os pixels OLED completamente, reduzindo a dissipação de calor a quase zero.
</details>

---

## 📄 Licença

Este projeto é software livre licenciado sob a licença **[MIT](LICENSE)**.

```
Copyright (c) 2026 Tiago Treter
```
Consulte o arquivo [`LICENSE`](LICENSE) para mais informações.
