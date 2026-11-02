/**
 * tVisio — Desktop Liquid Glass Controller, L1-LUT Color Studio & Zero-Copy ePTZ
 */

(() => {
  const previewCanvas = document.getElementById("previewCanvas");
  const previewCtx = previewCanvas.getContext("2d", { alpha: false });
  const ambientCanvas = document.getElementById("ambientCanvas");
  const ambientCtx = ambientCanvas.getContext("2d", { alpha: false });
  const miniplayerStage = document.getElementById("miniplayerStage");

  const emptyOverlay = document.getElementById("emptyOverlay");
  const pausedOverlay = document.getElementById("pausedOverlay");
  const gridOverlay = document.getElementById("gridOverlay");
  const zoomBadge = document.getElementById("zoomBadge");

  const connectionBadge = document.getElementById("connectionBadge");
  const connectionText = document.getElementById("connectionText");
  const deviceSubtitle = document.getElementById("deviceSubtitle");
  const g2gValue = document.getElementById("g2gValue");
  const rttValue = document.getElementById("rttValue");
  const fpsValue = document.getElementById("fpsValue");
  const batteryValue = document.getElementById("batteryValue");
  const protocolTag = document.getElementById("protocolTag");

  const vcamDot = document.getElementById("vcamDot");
  const vcamName = document.getElementById("vcamName");
  const vcamResolution = document.getElementById("vcamResolution");
  const btnInstallVcam = document.getElementById("btnInstallVcam");

  const qrImage = document.getElementById("qrImage");
  const interfaceSelect = document.getElementById("interfaceSelect");
  const cameraDeviceSelect = document.getElementById("cameraDeviceSelect");
  const pairingUrlText = document.getElementById("pairingUrlText");
  const pairingUrlBox = document.getElementById("pairingUrlBox");

  const sliderGain = document.getElementById("sliderGain");
  const sliderContrast = document.getElementById("sliderContrast");
  const sliderTemp = document.getElementById("sliderTemp");
  const sliderZoom = document.getElementById("sliderZoom");
  const gainValText = document.getElementById("gainValText");
  const contrastValText = document.getElementById("contrastValText");
  const tempValText = document.getElementById("tempValText");
  const zoomValText = document.getElementById("zoomValText");

  let ws = null;
  let previewEnabled = true;
  let ambientFrameSkip = 0;
  let currentInterfaces = [];
  let currentCameras = [];
  let settingsSyncedOnce = false;

  // Zero-Copy ePTZ state
  let currentZoom = 1.0;
  let panX = 0.0;
  let panY = 0.0;
  let isDraggingPan = false;
  let dragStartX = 0;
  let dragStartY = 0;

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
        try {
          const bitmap = await createImageBitmap(event.data);
          if (previewCanvas.width !== bitmap.width || previewCanvas.height !== bitmap.height) {
            previewCanvas.width = bitmap.width;
            previewCanvas.height = bitmap.height;
            ambientCanvas.width = 64;
            ambientCanvas.height = 36;
          }
          previewCtx.drawImage(bitmap, 0, 0);

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

      if (data.battery_level !== null && data.battery_level !== undefined) {
        const bolt = data.battery_charging ? " ⚡" : "";
        batteryValue.textContent = `🔋 ${data.battery_level}%${bolt}`;
        batteryValue.style.color = data.battery_level <= 15 && !data.battery_charging ? "#FF5252" : "";
      } else {
        batteryValue.textContent = "🔋 OK";
      }
    } else {
      deviceSubtitle.textContent = "Aguardando pareamento via QR Code...";
      g2gValue.textContent = "-- ms";
      rttValue.textContent = "-- ms";
      fpsValue.textContent = "0 FPS";
      batteryValue.textContent = "🔋 --%";
      emptyOverlay.classList.remove("hidden");
    }

    // Update Virtual Camera Driver status
    if (data.vcam.driver_active) {
      vcamDot.style.background = "#18F296";
      vcamDot.style.boxShadow = "0 0 12px #18F296";
      vcamName.textContent = data.vcam.backend_name;
      vcamResolution.textContent = `Ativo em ${data.vcam.resolution} • Processamento LUT: ${data.vcam.decode_ms}ms`;
      btnInstallVcam.style.display = "none";
    } else {
      vcamDot.style.background = "#00F0FF";
      vcamName.textContent = "tVisio Virtual Camera (Modo Preview)";
      vcamResolution.textContent = "Clique ao lado para registrar o driver DirectShow no Windows";
      btnInstallVcam.style.display = "inline-flex";
    }

    // Active states on dock buttons
    document.getElementById("btnTorch").classList.toggle("active", !!data.torch_on);
    document.getElementById("btnAfLock").classList.toggle("active", !!data.af_locked);
    document.getElementById("btnFreeze").classList.toggle("active", !!data.vcam.frozen);
    document.getElementById("freezeLabel").textContent = data.vcam.frozen ? "Congelado" : "Congelar";
    document.getElementById("btnMirrorH").classList.toggle("active", !!data.vcam.mirror_horizontal);
    document.getElementById("btnEcoMode").classList.toggle("active", !!data.eco_mode);

    // Sync initial persisted sliders once on load
    if (!settingsSyncedOnce && data.vcam) {
      settingsSyncedOnce = true;
      sliderGain.value = data.vcam.brightness_gain;
      gainValText.textContent = `${Number(data.vcam.brightness_gain).toFixed(2)}x`;
      sliderContrast.value = data.vcam.contrast;
      contrastValText.textContent = `${Number(data.vcam.contrast).toFixed(2)}x`;
      sliderTemp.value = data.vcam.color_temp;
      tempValText.textContent = formatTempLabel(data.vcam.color_temp);
      currentZoom = Number(data.vcam.zoom_level || 1.0);
      panX = Number(data.vcam.pan_x || 0.0);
      panY = Number(data.vcam.pan_y || 0.0);
      updateZoomUI(currentZoom);

      document.querySelectorAll("#presetSegment .seg-btn").forEach((b) => {
        b.classList.toggle("active", b.dataset.preset === data.vcam.color_preset);
      });
    }

    // Populate physical cameras dropdown if changed
    if (data.cameras && JSON.stringify(data.cameras) !== JSON.stringify(currentCameras)) {
      currentCameras = data.cameras;
      cameraDeviceSelect.innerHTML = '<option value="">Lente Padrão (Auto)</option>';
      data.cameras.forEach((cam, i) => {
        const opt = document.createElement("option");
        opt.value = cam.id;
        opt.textContent = cam.label || `Câmera #${i + 1}`;
        if (cam.id === data.selected_cam_id) opt.selected = true;
        cameraDeviceSelect.appendChild(opt);
      });
    }

    // Populate network interfaces dropdown if changed
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

  function formatTempLabel(val) {
    const num = Math.round(val * 100);
    if (num === 0) return "Neutro";
    return num > 0 ? `+${num}Q` : `${num}F`;
  }

  function updateZoomUI(zoom) {
    currentZoom = Math.max(1.0, Math.min(3.0, zoom));
    sliderZoom.value = currentZoom;
    zoomValText.textContent = `${currentZoom.toFixed(1)}x`;
    zoomBadge.textContent = `🔍 ${currentZoom.toFixed(1)}x`;
    zoomBadge.classList.toggle("hidden", currentZoom <= 1.02);
  }

  // Dock & Header Controls
  document.getElementById("btnSwitchCam").addEventListener("click", () => {
    sendCommand({ action: "switch_camera" });
  });

  document.getElementById("btnAfLock").addEventListener("click", () => {
    sendCommand({ action: "toggle_af_lock" });
  });

  document.getElementById("btnTorch").addEventListener("click", () => {
    sendCommand({ action: "toggle_torch" });
  });

  document.getElementById("btnFreeze").addEventListener("click", () => {
    sendCommand({ action: "toggle_freeze" });
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

  document.getElementById("btnToggleGrid").addEventListener("click", (e) => {
    gridOverlay.classList.toggle("hidden");
    e.currentTarget.classList.toggle("active", !gridOverlay.classList.contains("hidden"));
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

  // Physical Lens Selector
  cameraDeviceSelect.addEventListener("change", (e) => {
    sendCommand({ action: "select_camera_device", device_id: e.target.value });
  });

  // Network & QR Controls
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
    pairingUrlText.textContent = "✓ Link fixo copiado!";
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

  // Optical Shutter Speed Capping
  document.querySelectorAll("#shutterSegment .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#shutterSegment .seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      const shutterMs = parseFloat(btn.dataset.shutter);
      sendCommand({ action: "set_shutter_cap", shutter_ms: shutterMs });
    });
  });

  // 1-Pass L1-Cache Color Studio Presets & Sliders
  document.querySelectorAll("#presetSegment .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#presetSegment .seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      sendCommand({ action: "set_color_studio", color_preset: btn.dataset.preset });
    });
  });

  sliderGain.addEventListener("input", () => {
    const val = parseFloat(sliderGain.value);
    gainValText.textContent = `${val.toFixed(2)}x`;
    sendCommand({ action: "set_color_studio", brightness_gain: val });
  });

  sliderContrast.addEventListener("input", () => {
    const val = parseFloat(sliderContrast.value);
    contrastValText.textContent = `${val.toFixed(2)}x`;
    sendCommand({ action: "set_color_studio", contrast: val });
  });

  sliderTemp.addEventListener("input", () => {
    const val = parseFloat(sliderTemp.value);
    tempValText.textContent = formatTempLabel(val);
    sendCommand({ action: "set_color_studio", color_temp: val });
  });

  // Zero-Copy ePTZ Zoom & Mouse Wheel / Drag Pan on Miniplayer
  sliderZoom.addEventListener("input", () => {
    const z = parseFloat(sliderZoom.value);
    updateZoomUI(z);
    sendCommand({ action: "set_eptz", zoom_level: currentZoom });
  });

  zoomBadge.addEventListener("click", () => {
    panX = 0.0;
    panY = 0.0;
    updateZoomUI(1.0);
    sendCommand({ action: "set_eptz", zoom_level: 1.0, pan_x: 0.0, pan_y: 0.0 });
  });

  miniplayerStage.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const delta = e.deltaY < 0 ? 0.12 : -0.12;
      updateZoomUI(currentZoom + delta);
      sendCommand({ action: "set_eptz", zoom_level: currentZoom });
    },
    { passive: false }
  );

  miniplayerStage.addEventListener("mousedown", (e) => {
    if (e.target.closest(".floating-glass-dock") || currentZoom <= 1.02) return;
    isDraggingPan = true;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
  });

  window.addEventListener("mousemove", (e) => {
    if (!isDraggingPan) return;
    const dx = (dragStartX - e.clientX) / 180;
    const dy = (dragStartY - e.clientY) / 180;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    panX = Math.max(-1.0, Math.min(1.0, panX + dx));
    panY = Math.max(-1.0, Math.min(1.0, panY + dy));
    sendCommand({ action: "set_eptz", pan_x: panX, pan_y: panY });
  });

  window.addEventListener("mouseup", () => {
    isDraggingPan = false;
  });

  // Reset Studio Button
  document.getElementById("btnResetStudio").addEventListener("click", () => {
    sliderGain.value = 1.0;
    sliderContrast.value = 1.0;
    sliderTemp.value = 0.0;
    gainValText.textContent = "1.00x";
    contrastValText.textContent = "1.00x";
    tempValText.textContent = "Neutro";
    panX = 0.0;
    panY = 0.0;
    updateZoomUI(1.0);
    document.querySelectorAll("#presetSegment .seg-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.preset === "natural");
    });
    sendCommand({
      action: "set_color_studio",
      brightness_gain: 1.0,
      contrast: 1.0,
      gamma: 1.0,
      color_temp: 0.0,
      color_preset: "natural",
    });
    sendCommand({ action: "set_eptz", zoom_level: 1.0, pan_x: 0.0, pan_y: 0.0 });
  });

  document.getElementById("chkAntiPsm").addEventListener("change", () => {
    sendCommand({ action: "toggle_anti_psm" });
  });

  document.getElementById("resolutionSelect").addEventListener("change", (e) => {
    sendCommand({ action: "set_resolution", profile: e.target.value });
  });

  connectDesktopWS();
})();
