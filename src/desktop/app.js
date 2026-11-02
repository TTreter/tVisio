/**
 * tVisio — Desktop Liquid Glass Controller & Low-Overhead Miniplayer Engine
 */

(() => {
  const previewCanvas = document.getElementById("previewCanvas");
  const previewCtx = previewCanvas.getContext("2d", { alpha: false });
  const ambientCanvas = document.getElementById("ambientCanvas");
  const ambientCtx = ambientCanvas.getContext("2d", { alpha: false });

  const emptyOverlay = document.getElementById("emptyOverlay");
  const pausedOverlay = document.getElementById("pausedOverlay");
  const connectionBadge = document.getElementById("connectionBadge");
  const connectionText = document.getElementById("connectionText");
  const deviceSubtitle = document.getElementById("deviceSubtitle");
  const g2gValue = document.getElementById("g2gValue");
  const rttValue = document.getElementById("rttValue");
  const fpsValue = document.getElementById("fpsValue");
  const protocolTag = document.getElementById("protocolTag");

  const vcamDot = document.getElementById("vcamDot");
  const vcamName = document.getElementById("vcamName");
  const vcamResolution = document.getElementById("vcamResolution");
  const btnInstallVcam = document.getElementById("btnInstallVcam");

  const qrImage = document.getElementById("qrImage");
  const interfaceSelect = document.getElementById("interfaceSelect");
  const pairingUrlText = document.getElementById("pairingUrlText");
  const pairingUrlBox = document.getElementById("pairingUrlBox");

  let ws = null;
  let previewEnabled = true;
  let ambientFrameSkip = 0;
  let currentInterfaces = [];

  function connectDesktopWS() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws/desktop`);
    ws.binaryType = "blob";

    ws.onmessage = async (event) => {
      if (typeof event.data === "string") {
        const data = JSON.parse(event.data);
        if (data.type === "telemetry") {
          updateTelemetryUI(data);
        }
      } else if (event.data instanceof Blob && previewEnabled) {
        // Hardware-accelerated non-blocking bitmap decode for 360p Miniplayer
        try {
          const bitmap = await createImageBitmap(event.data);
          if (previewCanvas.width !== bitmap.width || previewCanvas.height !== bitmap.height) {
            previewCanvas.width = bitmap.width;
            previewCanvas.height = bitmap.height;
            ambientCanvas.width = 64;
            ambientCanvas.height = 36;
          }
          previewCtx.drawImage(bitmap, 0, 0);

          // Update Ambient Video Glow every 4th frame (~7fps) for near-zero GPU cost
          ambientFrameSkip = (ambientFrameSkip + 1) % 4;
          if (ambientFrameSkip === 0) {
            ambientCtx.drawImage(bitmap, 0, 0, 64, 36);
          }
          bitmap.close();
          emptyOverlay.classList.add("hidden");
        } catch (_) {}
      }
    };

    ws.onclose = () => {
      setTimeout(connectDesktopWS, 1200);
    };
  }

  function sendCommand(payload) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(payload));
    }
  }

  function updateTelemetryUI(data) {
    const connected = data.mobile_connected;
    connectionBadge.classList.toggle("connected", connected);
    connectionText.textContent = connected ? "Transmitindo" : "Standby";

    if (connected) {
      deviceSubtitle.textContent = `Conectado: ${data.device_info} • Obturador ${data.shutter_cap_ms}ms`;
      g2gValue.textContent = `⚡ ${data.g2g_ms.toFixed(1)} ms`;
      rttValue.textContent = `${data.rtt_ms.toFixed(1)} ms`;
      fpsValue.textContent = `${data.vcam.fps.toFixed(0)} FPS`;
      protocolTag.textContent = data.active_protocol;
    } else {
      deviceSubtitle.textContent = "Aguardando pareamento via QR Code...";
      g2gValue.textContent = "-- ms";
      rttValue.textContent = "-- ms";
      fpsValue.textContent = "0 FPS";
      emptyOverlay.classList.remove("hidden");
    }

    // Update Virtual Camera Driver status
    if (data.vcam.driver_active) {
      vcamDot.style.background = "#18F296";
      vcamDot.style.boxShadow = "0 0 12px #18F296";
      vcamName.textContent = data.vcam.backend_name;
      vcamResolution.textContent = `Ativo em ${data.vcam.resolution} • Decode: ${data.vcam.decode_ms}ms`;
      btnInstallVcam.style.display = "none";
    } else {
      vcamDot.style.background = "#00F0FF";
      vcamName.textContent = "tVisio Virtual Camera (Modo Preview)";
      vcamResolution.textContent = "Clique ao lado para registrar o driver DirectShow no Windows";
      btnInstallVcam.style.display = "inline-flex";
    }

    // Active states on dock buttons
    document.getElementById("btnTorch").classList.toggle("active", !!data.torch_on);
    document.getElementById("btnMirrorH").classList.toggle("active", !!data.vcam.mirror_horizontal);
    document.getElementById("btnEcoMode").classList.toggle("active", !!data.eco_mode);

    // Populate interfaces dropdown if changed
    if (data.interfaces && JSON.stringify(data.interfaces) !== JSON.stringify(currentInterfaces)) {
      currentInterfaces = data.interfaces;
      interfaceSelect.innerHTML = "";
      data.interfaces.forEach((iface) => {
        const opt = document.createElement("option");
        opt.value = iface.ip;
        opt.textContent = `${iface.label} (${iface.ip})`;
        if (iface.ip === data.selected_ip) opt.selected = true;
        interfaceSelect.appendChild(opt);
      });
    }

    if (data.pairing_url && pairingUrlText.textContent !== data.pairing_url) {
      pairingUrlText.textContent = data.pairing_url;
    }
  }

  // Controls Event Listeners
  document.getElementById("btnSwitchCam").addEventListener("click", () => {
    sendCommand({ action: "switch_camera" });
  });

  document.getElementById("btnTorch").addEventListener("click", () => {
    sendCommand({ action: "toggle_torch" });
  });

  document.getElementById("btnMirrorH").addEventListener("click", () => {
    sendCommand({ action: "toggle_mirror_h" });
  });

  document.getElementById("btnRotate").addEventListener("click", () => {
    sendCommand({ action: "rotate" });
  });

  document.getElementById("btnEcoMode").addEventListener("click", () => {
    sendCommand({ action: "toggle_eco_mode" });
  });

  document.getElementById("btnTogglePreview").addEventListener("click", () => {
    previewEnabled = !previewEnabled;
    sendCommand({ action: "set_miniplayer_enabled", enabled: previewEnabled });
    document.getElementById("previewToggleIcon").textContent = previewEnabled ? "👁️" : "⚡";
    document.getElementById("previewToggleText").textContent = previewEnabled ? "Preview Ativo" : "Zero-Render";
    pausedOverlay.classList.toggle("hidden", previewEnabled);
  });

  document.getElementById("btnPipMode").addEventListener("click", () => {
    document.getElementById("appShell").classList.toggle("pip-mode");
  });

  interfaceSelect.addEventListener("change", (e) => {
    const ip = e.target.value;
    sendCommand({ action: "select_interface", ip });
    qrImage.src = `/api/qr?ip=${encodeURIComponent(ip)}&t=${Date.now()}`;
  });

  document.getElementById("btnRefreshNet").addEventListener("click", () => {
    sendCommand({ action: "refresh_interfaces" });
    qrImage.src = `/api/qr?t=${Date.now()}`;
  });

  pairingUrlBox.addEventListener("click", () => {
    navigator.clipboard.writeText(pairingUrlText.textContent);
    const prev = pairingUrlText.textContent;
    pairingUrlText.textContent = "✓ Link copiado para a área de transferência!";
    setTimeout(() => {
      pairingUrlText.textContent = prev;
    }, 1500);
  });

  document.getElementById("btnHotspot").addEventListener("click", async () => {
    await fetch("/api/hotspot", { method: "POST" });
    setTimeout(() => {
      qrImage.src = `/api/qr?t=${Date.now()}`;
    }, 1800);
  });

  btnInstallVcam.addEventListener("click", async () => {
    btnInstallVcam.disabled = true;
    btnInstallVcam.innerHTML = "<span>⏳ Registrando Driver DirectShow...</span>";
    try {
      const res = await fetch("/api/install-vcam", { method: "POST" });
      const result = await res.json();
      if (result.ok && result.driver_active) {
        btnInstallVcam.innerHTML = "<span>✓ Driver Ativo!</span>";
      } else {
        btnInstallVcam.innerHTML = "<span>⚡ Tentar Novamente (Admin)</span>";
        btnInstallVcam.disabled = false;
      }
    } catch (_) {
      btnInstallVcam.disabled = false;
      btnInstallVcam.innerHTML = "<span>⚡ Ativar Driver Virtual Cam</span>";
    }
  });

  // Segmented Shutter Speed Buttons
  document.querySelectorAll("#shutterSegment .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#shutterSegment .seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      const shutterMs = parseFloat(btn.dataset.shutter);
      sendCommand({ action: "set_shutter_cap", shutter_ms: shutterMs });
    });
  });

  document.getElementById("chkAntiPsm").addEventListener("change", () => {
    sendCommand({ action: "toggle_anti_psm" });
  });

  const sliderGain = document.getElementById("sliderGain");
  const gainValText = document.getElementById("gainValText");
  sliderGain.addEventListener("input", () => {
    const val = parseFloat(sliderGain.value);
    gainValText.textContent = `${val.toFixed(2)}x`;
    sendCommand({ action: "set_brightness", value: val });
  });

  document.getElementById("resolutionSelect").addEventListener("change", (e) => {
    sendCommand({ action: "set_resolution", profile: e.target.value });
  });

  connectDesktopWS();
})();
