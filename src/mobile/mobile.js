/**
 * tVisio — Transmissor Mobile (PWA de Ultra-Baixa Latência & Baixo Consumo Térmico)
 * =============================================================================
 * Gerencia a captura de vídeo no navegador do celular (Safari, Chrome, Edge),
 * negociação WebRTC P2P acelerada por hardware (H.264), controle do sensor da câmera
 * (obturador manual, foco contínuo/travado, lanterna, seleção de lentes físicas),
 * e proteções térmicas para evitar superaquecimento da bateria.
 *
 * Otimizações de Bateria e Temperatura:
 * 1. Teto Estrito de Framerate: 30 FPS por padrão (evita que o ISP móvel ative modos de 120Hz a voltagens elevadas).
 * 2. Desconexão Total do Elemento <video> no Modo Eco: `videoEl.srcObject = null` enquanto o WebRTC
 *    continua transmitindo diretamente do track de mídia (0 decodificação na tela móvel).
 * 3. Auto-Eco OLED Inteligente: desliga a tela após 8 segundos de inatividade para economizar energia da tela OLED.
 * 4. Bitrate Otimizado: 2.0 Mbps H.264 para reduzir a dissipação de potência da antena transmissora Wi-Fi.
 * =============================================================================
 */

(() => {
  // Elementos do DOM Mobile
  const videoEl = document.getElementById("cameraVideo");
  const capsuleDot = document.getElementById("capsuleDot");
  const capsuleStatus = document.getElementById("capsuleStatus");
  const capsuleRtt = document.getElementById("capsuleRtt");
  const ecoOverlay = document.getElementById("ecoOverlay");
  const mShutterText = document.getElementById("mShutterText");

  // Estado da captura de vídeo
  let currentStream = null;
  let facingMode = "environment"; // Inicia pela câmera traseira (lente principal)
  let selectedDeviceId = "";
  let cameraList = [];
  let torchOn = false;
  let afLocked = false;
  let ecoMode = false;
  let autoEcoEnabled = true;
  let autoEcoTimer = null;
  let shutterCapMs = 8.0;

  // Parâmetros padrão otimizados para evitar superaquecimento (720p @ 30 FPS)
  let targetWidth = 1280;
  let targetHeight = 720;
  let targetFps = 30;

  // Monitoramento de bateria via Battery Status API
  let batteryLevel = null;
  let batteryCharging = false;

  // Estado de rede (WebRTC e WebSocket)
  let ws = null;
  let pc = null;
  let webrtcConnected = false;
  let turboRafId = null;
  let rttMs = 4.5;
  let isConverting = false;
  const tsHeader = new ArrayBuffer(8);
  const tsView = new DataView(tsHeader);

  // Canvas auxiliar para streaming binário de ultra-baixa latência
  const captureCanvas = document.createElement("canvas");
  const captureCtx = captureCanvas.getContext("2d", { alpha: false, desynchronized: true });

  /**
   * Mantém a tela acordada usando Screen Wake Lock API para evitar suspensão pelo SO.
   */
  async function requestWakeLock() {
    if ("wakeLock" in navigator) {
      try {
        await navigator.wakeLock.request("screen");
      } catch (_) {}
    }
  }

  /**
   * Agenda a entrada automática no Modo Eco OLED após 8 segundos de inatividade.
   */
  function scheduleAutoEco() {
    if (autoEcoTimer) clearTimeout(autoEcoTimer);
    if (!ecoMode && autoEcoEnabled) {
      autoEcoTimer = setTimeout(() => {
        const isStreaming = webrtcConnected || (ws && ws.readyState === WebSocket.OPEN);
        if (isStreaming && autoEcoEnabled) {
          toggleEcoMode(true);
        }
      }, 8000);
    }
  }

  /**
   * Inicializa o monitoramento de bateria do dispositivo.
   */
  async function initBatteryMonitor() {
    if ("getBattery" in navigator) {
      try {
        const bat = await navigator.getBattery();
        const updateBat = () => {
          batteryLevel = Math.round(bat.level * 100);
          batteryCharging = !!bat.charging;
        };
        updateBat();
        bat.addEventListener("levelchange", updateBat);
        bat.addEventListener("chargingchange", updateBat);
      } catch (_) {}
    }
  }

  /**
   * Identifica todas as lentes e câmeras físicas disponíveis no smartphone.
   */
  async function discoverPhysicalCameras() {
    if (!navigator.mediaDevices || !navigator.mediaDevices.enumerateDevices) return;
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      const videoInputs = devices.filter((d) => d.kind === "videoinput");
      cameraList = videoInputs.map((d, idx) => ({
        id: d.deviceId,
        label: d.label || `Lente Física #${idx + 1}`,
      }));
      if (currentStream) {
        const track = currentStream.getVideoTracks()[0];
        const settings = track && track.getSettings ? track.getSettings() : {};
        if (settings.deviceId) {
          selectedDeviceId = settings.deviceId;
        }
      }
      sendTelemetry();
    } catch (_) {}
  }

  /**
   * Inicia a captura da câmera utilizando MediaDevices.getUserMedia com restrições térmicas rigorosas.
   */
  async function startCamera() {
    if (currentStream) {
      currentStream.getTracks().forEach((t) => t.stop());
    }

    // Teto rígido em targetFps: impede que o sensor opere a 60/120Hz desnecessariamente
    const videoConstraints = {
      width: { ideal: targetWidth },
      height: { ideal: targetHeight },
      frameRate: { ideal: targetFps, max: targetFps },
    };

    if (selectedDeviceId) {
      videoConstraints.deviceId = { exact: selectedDeviceId };
    } else {
      videoConstraints.facingMode = { ideal: facingMode };
    }

    try {
      currentStream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: videoConstraints,
      });

      // Se não estiver no modo eco, renderiza no elemento de visualização móvel
      if (!ecoMode || !webrtcConnected) {
        videoEl.style.display = "block";
        videoEl.srcObject = currentStream;
        await videoEl.play();
      }

      await discoverPhysicalCameras();
      await applyOpticalSensorTuning();
      capsuleDot.classList.add("live");
      capsuleStatus.textContent = webrtcConnected ? "WebRTC Eco-Cool" : "tVisio Live";

      // Se a conexão WebRTC já estiver ativa, atualiza a faixa de vídeo (replaceTrack)
      if (pc) {
        const videoTrack = currentStream.getVideoTracks()[0];
        const sender = pc.getSenders().find((s) => s.track && s.track.kind === "video");
        if (sender && videoTrack) {
          await sender.replaceTrack(videoTrack);
          await applySenderBitrate(sender);
        }
      } else {
        startWebRTC();
      }

      scheduleAutoEco();
    } catch (err) {
      if (selectedDeviceId) {
        selectedDeviceId = "";
        return startCamera();
      }
      capsuleStatus.textContent = "Permita o acesso à câmera";
      console.error("Erro ao acessar câmera do smartphone:", err);
    }
  }

  /**
   * Configura parâmetros avançados do sensor óptico móvel (obturador, foco).
   */
  async function applyOpticalSensorTuning() {
    if (!currentStream) return;
    const track = currentStream.getVideoTracks()[0];
    if (!track || !track.getCapabilities) return;

    try {
      const caps = track.getCapabilities();
      const adv = {};

      // Limita o tempo de exposição da lente para evitar motion blur excessivo
      if (caps.exposureMode && caps.exposureMode.includes("manual") && shutterCapMs < 30) {
        adv.exposureMode = "manual";
        if (caps.exposureTime) {
          const desired = Math.max(
            caps.exposureTime.min || 1,
            Math.min(caps.exposureTime.max || 100, shutterCapMs * 10)
          );
          adv.exposureTime = desired;
        }
      } else if (caps.exposureMode && caps.exposureMode.includes("continuous")) {
        adv.exposureMode = "continuous";
      }

      // Trava de autofoco
      if (caps.focusMode) {
        if (afLocked && caps.focusMode.includes("manual")) {
          adv.focusMode = "manual";
        } else if (!afLocked && caps.focusMode.includes("continuous")) {
          adv.focusMode = "continuous";
        }
      }

      if (Object.keys(adv).length > 0) {
        await track.applyConstraints({ advanced: [adv] });
      }
    } catch (_) {}
  }

  /**
   * Alterna a trava do autofoco (AF-Lock).
   */
  async function toggleAfLock(forceState) {
    afLocked = typeof forceState === "boolean" ? forceState : !afLocked;
    document.getElementById("mBtnAfLock").classList.toggle("active", afLocked);
    await applyOpticalSensorTuning();
    sendTelemetry();
  }

  /**
   * Alterna a lanterna traseira (Torch/Flash).
   */
  async function toggleTorch(forceState) {
    if (!currentStream) return;
    const track = currentStream.getVideoTracks()[0];
    if (!track || !track.getCapabilities) return;

    try {
      const caps = track.getCapabilities();
      if (caps.torch) {
        torchOn = typeof forceState === "boolean" ? forceState : !torchOn;
        await track.applyConstraints({ advanced: [{ torch: torchOn }] });
        document.getElementById("mBtnTorch").classList.toggle("active", torchOn);
        sendTelemetry();
      }
    } catch (_) {}
  }

  /**
   * Modo Eco OLED com Desconexão de Hardware:
   * Remove o stream do elemento <video> local (`videoEl.srcObject = null`),
   * permitindo que o navegador desligue completamente a GPU móvel enquanto o
   * encoder WebRTC transmite o sinal diretamente para o PC.
   */
  async function toggleEcoMode(forceState) {
    ecoMode = typeof forceState === "boolean" ? forceState : !ecoMode;
    ecoOverlay.classList.toggle("hidden", !ecoMode);
    document.getElementById("mBtnEco").classList.toggle("active", ecoMode);

    if (ecoMode && webrtcConnected) {
      try {
        videoEl.pause();
        videoEl.srcObject = null;
        videoEl.style.display = "none";
      } catch (_) {}
    } else if (!ecoMode && currentStream) {
      try {
        videoEl.style.display = "block";
        if (videoEl.srcObject !== currentStream) {
          videoEl.srcObject = currentStream;
        }
        await videoEl.play();
      } catch (_) {}
      scheduleAutoEco();
    }
    sendTelemetry();
  }

  /**
   * Aplica configurações de taxa de bits e prioridade no sender WebRTC.
   */
  async function applySenderBitrate(sender) {
    if (!sender || !sender.getParameters || !sender.setParameters) return;
    try {
      const params = sender.getParameters();
      params.degradationPreference = "maintain-resolution";
      if (params.encodings && params.encodings.length > 0) {
        params.encodings[0].priority = "high";
        params.encodings[0].networkPriority = "high";
        // Taxa de bits moderada para menor dissipação de calor no transmissor de rádio Wi-Fi
        params.encodings[0].maxBitrate = targetFps <= 30 ? 2_000_000 : 3_200_000;
        params.encodings[0].maxFramerate = targetFps;
        await sender.setParameters(params);
      }
    } catch (_) {}
  }

  /**
   * Inicia negociação WebRTC P2P com o backend local do tVisio via SDP Offer/Answer.
   */
  async function startWebRTC() {
    if (!currentStream || typeof RTCPeerConnection === "undefined") return;

    try {
      pc = new RTCPeerConnection({ iceServers: [] }); // P2P local puro, sem dependência de STUN na nuvem
      const videoTrack = currentStream.getVideoTracks()[0];
      if ("contentHint" in videoTrack) {
        videoTrack.contentHint = "motion";
      }

      const sender = pc.addTrack(videoTrack, currentStream);
      await applySenderBitrate(sender);

      pc.onconnectionstatechange = () => {
        webrtcConnected = pc.connectionState === "connected" || pc.connectionState === "completed";
        if (webrtcConnected) {
          capsuleStatus.textContent = "WebRTC Eco-Cool";
          stopBinaryTurboLoop();
          scheduleAutoEco();
          if (ecoMode) {
            toggleEcoMode(true);
          }
        } else if (pc.connectionState === "failed" || pc.connectionState === "disconnected") {
          try {
            pc.close();
          } catch (_) {}
          pc = null;
          if (ecoMode) toggleEcoMode(false);
          startBinaryTurboLoop();
        }
      };

      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);

      const res = await fetch("/api/webrtc/offer", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sdp: pc.localDescription.sdp,
          type: pc.localDescription.type,
        }),
      });

      if (res.status === 501) {
        // Servidor Rust utiliza WebSocket Binary Turbo otimizado; desativa loop de SDP
        if (pc) {
          try { pc.close(); } catch (_) {}
          pc = null;
        }
        startBinaryTurboLoop();
        return;
      }

      if (res.ok) {
        const answer = await res.json();
        await pc.setRemoteDescription(new RTCSessionDescription(answer));
      }
    } catch (e) {
      console.warn("Fallback WebRTC para Binary Turbo WebSocket:", e);
    }
  }

  /**
   * Conecta o canal de controle WebSocket bidirecional para comandos e telemetria.
   */
  function connectWebSocket() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams(window.location.search);
    const token = params.get("t") || "";
    const tokenQuery = token ? `?t=${encodeURIComponent(token)}` : "";
    ws = new WebSocket(`${proto}//${location.host}/ws/stream${tokenQuery}`);
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      sendTelemetry();
      if (!webrtcConnected) {
        startBinaryTurboLoop();
      }
    };

    ws.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        return;
      }

      try {
        const msg = JSON.parse(event.data);
        if (msg.type === "ack" || msg.type === "pong") {
          const sampleRtt = performance.now() - msg.ts;
          if (sampleRtt > 0 && sampleRtt < 1000) {
            rttMs = rttMs * 0.75 + sampleRtt * 0.25;
            capsuleRtt.textContent = `${(rttMs * 0.5 + shutterCapMs).toFixed(1)} ms`;
          }
        } else if (msg.type === "command") {
          handleRemoteCommand(msg);
        }
      } catch (_) {}
    };

    ws.onclose = () => {
      setTimeout(connectWebSocket, 1500);
    };
  }

  /**
   * Processa comandos remotos enviados pelo painel desktop (Ex: troca de câmera, zoom, etc).
   */
  function handleRemoteCommand(msg) {
    switch (msg.command) {
      case "switch_camera":
        selectedDeviceId = "";
        facingMode = facingMode === "environment" ? "user" : "environment";
        startCamera();
        sendTelemetry();
        break;
      case "select_camera":
      case "select_camera_device":
        selectedDeviceId = msg.device_id || "";
        startCamera();
        sendTelemetry();
        break;
      case "toggle_torch":
        toggleTorch();
        break;
      case "toggle_af_lock":
        toggleAfLock();
        break;
      case "toggle_eco_mode":
        toggleEcoMode();
        break;
      case "set_auto_eco":
        autoEcoEnabled = !!msg.enabled;
        if (autoEcoEnabled) {
          scheduleAutoEco();
        } else {
          if (autoEcoTimer) clearTimeout(autoEcoTimer);
          if (ecoMode) toggleEcoMode(false);
        }
        break;
      case "set_shutter_cap":
        shutterCapMs = parseFloat(msg.shutter_ms || 8.0);
        mShutterText.textContent = shutterCapMs >= 30 ? "Auto" : `${shutterCapMs}ms`;
        applyOpticalSensorTuning();
        sendTelemetry();
        break;
      case "set_resolution": {
        let w = 1280;
        let h = 720;
        let fps = 30;
        if (msg.profile) {
          const parts = msg.profile.split("@");
          const dims = (parts[0] || "1280x720").split("x");
          w = Number(dims[0]) || 1280;
          h = Number(dims[1]) || 720;
          fps = Number(parts[1]) || 30;
        } else if (msg.width && msg.height) {
          w = Number(msg.width) || 1280;
          h = Number(msg.height) || 720;
          fps = Number(msg.fps) || 30;
        }
        if (targetWidth !== w || targetHeight !== h || targetFps !== fps) {
          targetWidth = w;
          targetHeight = h;
          targetFps = fps;
          startCamera();
        }
        break;
      }
    }
  }

  /**
   * Envia pacote com dados de telemetria e estado para o painel de controle desktop.
   */
  function sendTelemetry() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const ua = navigator.userAgent;
    let device = "Smartphone Browser";
    if (/iPhone/i.test(ua)) device = "iPhone (Safari)";
    else if (/iPad/i.test(ua)) device = "iPad (Safari)";
    else if (/Android/i.test(ua)) device = "Android Smartphone";

    ws.send(
      JSON.stringify({
        type: "telemetry",
        rtt_ms: rttMs,
        device,
        eco_mode: ecoMode,
        torch_on: torchOn,
        af_locked: afLocked,
        facing_mode: facingMode,
        shutter_ms: shutterCapMs,
        cameras: cameraList,
        selected_cam_id: selectedDeviceId,
        battery_level: batteryLevel,
        battery_charging: batteryCharging,
      })
    );
  }

  /**
   * Encerra loop de fallback JPEG caso WebRTC esteja ativo.
   */
  function stopBinaryTurboLoop() {
    if (turboRafId !== null) {
      cancelAnimationFrame(turboRafId);
      turboRafId = null;
    }
  }

  /**
   * Loop de streaming binário de ultra-baixa latência sobre WebSocket.
   * Elimina buffers intermediários, utiliza packing zero-copy com Blob e controle anti-bloat via ws.bufferedAmount.
   */
  function startBinaryTurboLoop() {
    stopBinaryTurboLoop();
    let lastFrameTime = 0;
    const tick = (now) => {
      if (webrtcConnected || !ws || ws.readyState !== WebSocket.OPEN) {
        turboRafId = null;
        return;
      }

      // Respeita a taxa de quadros alvo (30 FPS ou 60 FPS)
      const targetInterval = 1000 / targetFps;
      const elapsed = now - lastFrameTime;

      // Controle de fluxo anti-bloat:
      // 1. Aguarda intervalo da taxa de FPS alvo (-3ms de tolerância de jitter RAF)
      // 2. Não sobrepõe conversões ativas de canvas
      // 3. Drop de quadro adaptativo se o buffer de envio TCP/WebSocket estiver acumulando (>64KB)
      if (elapsed >= targetInterval - 3 && !isConverting && videoEl.readyState >= 2) {
        if (ws.bufferedAmount > 65536) {
          // Congestionamento transitório na rede WiFi; descarta quadro na origem para latência Zero
          turboRafId = requestAnimationFrame(tick);
          return;
        }

        lastFrameTime = now;
        isConverting = true;

        const vw = videoEl.videoWidth || targetWidth || 1280;
        const vh = videoEl.videoHeight || targetHeight || 720;
        if (captureCanvas.width !== vw || captureCanvas.height !== vh) {
          captureCanvas.width = vw;
          captureCanvas.height = vh;
        }

        captureCtx.drawImage(videoEl, 0, 0, vw, vh);
        const ts = performance.now();

        captureCanvas.toBlob(
          (blob) => {
            try {
              if (blob && ws && ws.readyState === WebSocket.OPEN && ws.bufferedAmount <= 65536) {
                tsView.setFloat64(0, ts, true);
                ws.send(new Blob([tsHeader, blob]));
              }
            } catch (_) {
            } finally {
              isConverting = false;
            }
          },
          "image/jpeg",
          0.56
        );
      }

      turboRafId = requestAnimationFrame(tick);
    };
    turboRafId = requestAnimationFrame(tick);
  }

  // Intervalo relaxado de ping/telemetria a cada 2.5s (reduz ciclos de rádio móvel)
  setInterval(() => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "ping", ts: performance.now() }));
      sendTelemetry();
    }
  }, 2500);

  // --- Ouvintes de Eventos da Interface Mobile ---
  document.getElementById("mBtnSwitch").addEventListener("click", () => {
    scheduleAutoEco();
    if (cameraList.length > 1) {
      const curIdx = cameraList.findIndex((c) => c.id === selectedDeviceId);
      const nextIdx = (curIdx + 1) % cameraList.length;
      selectedDeviceId = cameraList[nextIdx].id;
    } else {
      facingMode = facingMode === "environment" ? "user" : "environment";
    }
    startCamera();
    sendTelemetry();
  });

  document.getElementById("mBtnTorch").addEventListener("click", () => {
    scheduleAutoEco();
    toggleTorch();
  });

  document.getElementById("mBtnAfLock").addEventListener("click", () => {
    scheduleAutoEco();
    toggleAfLock();
  });

  document.getElementById("mBtnShutter").addEventListener("click", () => {
    scheduleAutoEco();
    const cycle = [4, 8, 16, 33];
    const idx = (cycle.indexOf(shutterCapMs) + 1) % cycle.length;
    shutterCapMs = cycle[idx];
    mShutterText.textContent = shutterCapMs >= 30 ? "Auto" : `${shutterCapMs}ms`;
    applyOpticalSensorTuning();
    sendTelemetry();
  });

  document.getElementById("mBtnEco").addEventListener("click", () => {
    toggleEcoMode(true);
  });

  ecoOverlay.addEventListener("click", () => {
    toggleEcoMode(false);
  });

  // Sequência de inicialização
  requestWakeLock();
  initBatteryMonitor();
  startCamera();
  connectWebSocket();
})();
