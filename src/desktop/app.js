/**
 * tVisio — Controlador da Interface Desktop (Translucent Glass UI)
 * =============================================================================
 * Gerencia a comunicação WebSocket com o backend, renderização do miniplayer
 * de baixa latência em canvas acelerado, controles do Estúdio de Cor L1-LUT,
 * sistema de zoom e enquadramento digital zero-copy (ePTZ), e telemetria em tempo real.
 *
 * Tecnologias:
 * - Canvas 2D acelerado por hardware para preview do vídeo sem sobrecarga de DOM.
 * - Suporte a Lucide Icons e estética Translucent Glass UI.
 * - Controles de Estúdio (Ganho, Contraste, Temperatura de Cor, Shutter Cap).
 * - Zoom óptico digital (ePTZ) via rolagem do mouse e arraste para enquadramento.
 * - Reset de controles com duplo clique nos sliders.
 * =============================================================================
 */

(() => {
  // Elementos do Canvas de Visualização e Efeito Ambiente
  const previewCanvas = document.getElementById("previewCanvas");
  const previewCtx = previewCanvas.getContext("2d", { alpha: false });
  const ambientCanvas = document.getElementById("ambientCanvas");
  const ambientCtx = ambientCanvas.getContext("2d", { alpha: false });
  const miniplayerStage = document.getElementById("miniplayerStage");

  // Camadas de sobreposição (Overlays)
  const emptyOverlay = document.getElementById("emptyOverlay");
  const pausedOverlay = document.getElementById("pausedOverlay");
  const gridOverlay = document.getElementById("gridOverlay");
  const zoomBadge = document.getElementById("zoomBadge");
  const zoomBadgeText = document.getElementById("zoomBadgeText");

  // Indicadores de Status e Telemetria
  const connectionBadge = document.getElementById("connectionBadge");
  const connectionText = document.getElementById("connectionText");
  const deviceSubtitle = document.getElementById("deviceSubtitle");
  const g2gValue = document.getElementById("g2gValue");
  const rttValue = document.getElementById("rttValue");
  const fpsValue = document.getElementById("fpsValue");
  const batteryValue = document.getElementById("batteryValue");
  const batteryRow = document.getElementById("batteryRow");
  const protocolTag = document.getElementById("protocolTag");

  // Informações do Driver de Câmera Virtual (DirectShow)
  const vcamDot = document.getElementById("vcamDot");
  const vcamName = document.getElementById("vcamName");
  const vcamResolution = document.getElementById("vcamResolution");
  const btnInstallVcam = document.getElementById("btnInstallVcam");
  const btnInstallVcamText = document.getElementById("btnInstallVcamText");

  // Elementos de Conexão e Rede
  const qrImage = document.getElementById("qrImage");
  const interfaceSelect = document.getElementById("interfaceSelect");
  const cameraDeviceSelect = document.getElementById("cameraDeviceSelect");
  const pairingUrlText = document.getElementById("pairingUrlText");
  const pairingUrlBox = document.getElementById("pairingUrlBox");

  // Controles do Estúdio de Cor e Imagem
  const sliderGain = document.getElementById("sliderGain");
  const sliderContrast = document.getElementById("sliderContrast");
  const sliderTemp = document.getElementById("sliderTemp");
  const sliderZoom = document.getElementById("sliderZoom");
  const gainValText = document.getElementById("gainValText");
  const contrastValText = document.getElementById("contrastValText");
  const tempValText = document.getElementById("tempValText");
  const zoomValText = document.getElementById("zoomValText");

  // Estado interno da aplicação desktop
  let ws = null;
  let previewEnabled = true;
  let ambientFrameSkip = 0;
  let currentInterfaces = [];
  let currentCameras = [];
  let settingsSyncedOnce = false;

  // Estado do ePTZ (Zoom e Pan Zero-Copy)
  let currentZoom = 1.0;
  let panX = 0.0;
  let panY = 0.0;
  let isDraggingPan = false;
  let dragStartX = 0;
  let dragStartY = 0;

  // Alternador de Abas da Barra Lateral (Segmented Tabs Switcher)
  document.querySelectorAll("#sidebarTabs .tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#sidebarTabs .tab-btn").forEach((b) => b.classList.remove("active"));
      document.querySelectorAll(".tab-panel").forEach((p) => p.classList.remove("active"));
      btn.classList.add("active");
      const targetPanel = document.getElementById(btn.dataset.tab);
      if (targetPanel) targetPanel.classList.add("active");
    });
  });

  /**
   * Conecta o WebSocket do painel desktop para receber telemetria e frames de preview.
   * Reconexão automática em caso de queda.
   */
  function connectDesktopWS() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws/desktop`);
    ws.binaryType = "blob";

    ws.onmessage = async (event) => {
      if (typeof event.data === "string") {
        // Mensagem de controle ou telemetria em formato JSON
        const data = JSON.parse(event.data);
        if (data.type === "telemetry") {
          updateTelemetryUI(data);
        }
      } else if (event.data instanceof Blob && previewEnabled) {
        // Frame de vídeo comprimido (JPEG ultraleve) para renderização no miniplayer
        try {
          const bitmap = await createImageBitmap(event.data);
          if (previewCanvas.width !== bitmap.width || previewCanvas.height !== bitmap.height) {
            previewCanvas.width = bitmap.width;
            previewCanvas.height = bitmap.height;
            ambientCanvas.width = 64;
            ambientCanvas.height = 36;
          }
          previewCtx.drawImage(bitmap, 0, 0);

          // Efeito de brilho ambiente dinâmico (executado a cada 4 frames para poupar GPU)
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
      // Tentativa de reconexão automática
      setTimeout(connectDesktopWS, 1200);
    };
  }

  /**
   * Envia comando JSON para o backend via WebSocket.
   */
  function sendCommand(payload) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(payload));
    }
  }

  /**
   * Atualiza os elementos da interface gráfica com base na telemetria mais recente do backend.
   */
  function updateTelemetryUI(data) {
    const connected = data.mobile_connected;
    connectionBadge.classList.toggle("connected", connected);
    connectionText.textContent = connected ? "Transmitindo" : "Standby";

    if (connected) {
      deviceSubtitle.textContent = `Conectado: ${data.device_info} • Obturador ${data.shutter_cap_ms}ms`;
      g2gValue.textContent = `${data.g2g_ms.toFixed(1)} ms`;
      rttValue.textContent = `${data.rtt_ms.toFixed(1)} ms`;
      fpsValue.textContent = `${data.vcam.fps.toFixed(0)} FPS`;
      protocolTag.textContent = data.active_protocol;

      // Nível de bateria do smartphone
      if (data.battery_level !== null && data.battery_level !== undefined) {
        batteryValue.textContent = `${data.battery_level}%`;
        batteryRow.style.color = data.battery_level <= 15 && !data.battery_charging ? "#EF4444" : "";
      } else {
        batteryValue.textContent = "OK";
      }
    } else {
      deviceSubtitle.textContent = "Aguardando pareamento via QR Code...";
      g2gValue.textContent = "-- ms";
      rttValue.textContent = "-- ms";
      fpsValue.textContent = "0 FPS";
      batteryValue.textContent = "--%";
      emptyOverlay.classList.remove("hidden");
    }

    // Status do driver de Câmera Virtual DirectShow no Windows
    if (data.vcam.driver_active) {
      vcamDot.style.background = "#10B981";
      vcamDot.style.boxShadow = "0 0 10px #10B981";
      vcamName.textContent = data.vcam.backend_name;
      vcamResolution.textContent = `Ativo em ${data.vcam.resolution} • Processamento LUT: ${data.vcam.decode_ms}ms`;
      btnInstallVcam.style.display = "none";
    } else {
      vcamDot.style.background = "#00F0FF";
      vcamName.textContent = "tVisio Virtual Camera (Modo Preview)";
      vcamResolution.textContent = "Clique ao lado para registrar o driver DirectShow no Windows";
      btnInstallVcam.style.display = "inline-flex";
    }

    // Atualiza estados ativos dos botões na barra flutuante (Dock)
    document.getElementById("btnTorch").classList.toggle("active", !!data.torch_on);
    document.getElementById("btnAfLock").classList.toggle("active", !!data.af_locked);
    document.getElementById("btnFreeze").classList.toggle("active", !!data.vcam.frozen);
    document.getElementById("freezeLabel").textContent = data.vcam.frozen ? "Pausado" : "Pausar";
    document.getElementById("btnMirrorH").classList.toggle("active", !!data.vcam.mirror_horizontal);
    document.getElementById("btnEcoMode").classList.toggle("active", !!data.eco_mode);

    // Sincroniza valores persistidos na primeira carga
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

      if (data.app_settings) {
        if (data.app_settings.resolution_profile) {
          const resEl = document.getElementById("resolutionSelect");
          if (resEl && !resEl.matches(":focus")) resEl.value = data.app_settings.resolution_profile;
        }
        if (data.app_settings.camera_display_name) {
          const nameEl = document.getElementById("txtCameraDisplayName");
          if (nameEl && !nameEl.matches(":focus")) nameEl.value = data.app_settings.camera_display_name;
        }
      }

      if (data.shutter_cap_ms !== undefined) {
        const shutterVal = Math.round(data.shutter_cap_ms);
        document.querySelectorAll("#shutterSegment .seg-btn").forEach((b) => {
          b.classList.toggle("active", parseFloat(b.dataset.shutter) === shutterVal);
        });
      }
    }

    // Sincroniza configurações de Sistema & Bandeja do Windows (sem sobrescrever campos em foco)
    if (data.app_settings) {
      const s = data.app_settings;
      const elStartWin = document.getElementById("chkStartWindows");
      const elStartMin = document.getElementById("chkStartMinimized");
      const elMinTray = document.getElementById("chkMinToTray");
      const elCloseTray = document.getElementById("chkCloseToTray");
      const elAutoEco = document.getElementById("chkAutoEco");
      const elAutoVcam = document.getElementById("chkAutoStartVcam");
      const activeEl = document.activeElement;

      if (elStartWin && activeEl !== elStartWin) elStartWin.checked = !!s.start_with_windows;
      if (elStartMin && activeEl !== elStartMin) elStartMin.checked = !!s.start_minimized;
      if (elMinTray && activeEl !== elMinTray) elMinTray.checked = !!s.minimize_to_tray;
      if (elCloseTray && activeEl !== elCloseTray) elCloseTray.checked = !!s.close_to_tray;
      if (elAutoEco && activeEl !== elAutoEco) elAutoEco.checked = !!s.auto_eco_oled;
      if (elAutoVcam && activeEl !== elAutoVcam) elAutoVcam.checked = !!s.auto_start_vcam;
      if (s.is_installed !== undefined) isAppInstalled = !!s.is_installed;
    }

    const chkAntiPsmEl = document.getElementById("chkAntiPsm");
    if (chkAntiPsmEl && data.anti_psm !== undefined) {
      chkAntiPsmEl.checked = !!data.anti_psm;
    }

    // Atualiza menu suspenso de lentes físicas se houver alteração
    if (data.cameras && JSON.stringify(data.cameras) !== JSON.stringify(currentCameras)) {
      currentCameras = data.cameras;
      cameraDeviceSelect.innerHTML = '<option value="">Lente Padrão (Auto)</option>';
      data.cameras.forEach((cam, i) => {
        const opt = document.createElement("option");
        opt.value = cam.id;
        opt.textContent = cam.label || `Câmera Física #${i + 1}`;
        if (cam.id === data.selected_cam_id) opt.selected = true;
        cameraDeviceSelect.appendChild(opt);
      });
    }
    if (data.selected_cam_id !== undefined && cameraDeviceSelect.value !== data.selected_cam_id) {
      cameraDeviceSelect.value = data.selected_cam_id;
    }

    // Atualiza lista de interfaces de rede se houver alteração
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

  /**
   * Formata texto amigável de temperatura de cor (-100 a +100).
   */
  function formatTempLabel(val) {
    const num = Math.round(val * 100);
    if (num === 0) return "Neutro";
    return num > 0 ? `+${num} Quente` : `${num} Frio`;
  }

  /**
   * Atualiza a interface gráfica referente ao fator de zoom digital.
   */
  function updateZoomUI(zoom) {
    currentZoom = Math.max(1.0, Math.min(3.0, zoom));
    sliderZoom.value = currentZoom;
    zoomValText.textContent = `${currentZoom.toFixed(1)}x`;
    if (zoomBadgeText) zoomBadgeText.textContent = `${currentZoom.toFixed(1)}x`;
    zoomBadge.classList.toggle("hidden", currentZoom <= 1.02);
  }

  // --- Controles da Barra Flutuante (Dock) e Cabeçalho ---

  // Alternar entre câmera frontal e traseira
  document.getElementById("btnSwitchCam").addEventListener("click", () => {
    sendCommand({ action: "switch_camera" });
  });

  // Trava de Autofoco (AF-Lock) para evitar hunting contínuo
  document.getElementById("btnAfLock").addEventListener("click", () => {
    sendCommand({ action: "toggle_af_lock" });
  });

  // Lanterna / Flash do celular
  document.getElementById("btnTorch").addEventListener("click", () => {
    sendCommand({ action: "toggle_torch" });
  });

  // Congelar imagem na Virtual Cam (Freeze Frame)
  document.getElementById("btnFreeze").addEventListener("click", () => {
    sendCommand({ action: "toggle_freeze" });
  });

  // Espelhamento horizontal da imagem
  document.getElementById("btnMirrorH").addEventListener("click", () => {
    sendCommand({ action: "toggle_mirror_h" });
  });

  // Rotação em passos de 90 graus
  document.getElementById("btnRotate").addEventListener("click", () => {
    sendCommand({ action: "rotate" });
  });

  // Modo Eco / Blackout OLED no celular
  document.getElementById("btnEcoMode").addEventListener("click", () => {
    sendCommand({ action: "toggle_eco_mode" });
  });

  // Grade de Enquadramento (Regra dos Terços)
  document.getElementById("btnToggleGrid").addEventListener("click", (e) => {
    gridOverlay.classList.toggle("hidden");
    e.currentTarget.classList.toggle("active", !gridOverlay.classList.contains("hidden"));
  });

  // Alternar renderização do miniplayer (Modo Zero-Render para economia máxima de CPU desktop)
  document.getElementById("btnTogglePreview").addEventListener("click", () => {
    previewEnabled = !previewEnabled;
    sendCommand({ action: "set_miniplayer_enabled", enabled: previewEnabled });
    document.getElementById("eyeIconOpen").classList.toggle("hidden", !previewEnabled);
    document.getElementById("eyeIconOff").classList.toggle("hidden", previewEnabled);
    document.getElementById("previewToggleText").textContent = previewEnabled ? "Preview Ativo" : "Zero-Render";
    pausedOverlay.classList.toggle("hidden", previewEnabled);
  });

  // Modo Picture-in-Picture (PiP)
  document.getElementById("btnPipMode").addEventListener("click", () => {
    const isPip = document.getElementById("appShell").classList.toggle("pip-mode");
    sendCommand({ action: "set_pip_mode", enabled: isPip });
  });

  // Seletor de Lente Física
  cameraDeviceSelect.addEventListener("change", (e) => {
    sendCommand({ action: "select_camera_device", device_id: e.target.value });
  });

  // --- Controles de Rede e QR Code ---
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
    pairingUrlText.textContent = "Link copiado para a área de transferência!";
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

  // Registro do Driver Virtual Cam DirectShow
  btnInstallVcam.addEventListener("click", async () => {
    btnInstallVcam.disabled = true;
    btnInstallVcamText.textContent = "Registrando Driver DirectShow...";
    try {
      const res = await fetch("/api/install-vcam", { method: "POST" });
      const result = await res.json();
      if (result.ok && result.driver_active) {
        btnInstallVcamText.textContent = "Driver Ativo!";
      } else {
        btnInstallVcamText.textContent = "Tentar Novamente (Admin)";
        btnInstallVcam.disabled = false;
      }
    } catch (_) {
      btnInstallVcam.disabled = false;
      btnInstallVcamText.textContent = "Ativar Driver Virtual Cam";
    }
  });

  // Limite de Velocidade do Obturador Óptico (Shutter Cap)
  document.querySelectorAll("#shutterSegment .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#shutterSegment .seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      const shutterMs = parseFloat(btn.dataset.shutter);
      sendCommand({ action: "set_shutter_cap", shutter_ms: shutterMs });
    });
  });

  // Presets de Cor do Estúdio L1-Cache
  document.querySelectorAll("#presetSegment .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("#presetSegment .seg-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      sendCommand({ action: "set_color_studio", color_preset: btn.dataset.preset });
    });
  });

  // Controles Deslizantes com Reset por Duplo Clique
  sliderGain.addEventListener("input", () => {
    const val = parseFloat(sliderGain.value);
    gainValText.textContent = `${val.toFixed(2)}x`;
    sendCommand({ action: "set_color_studio", brightness_gain: val });
  });
  sliderGain.addEventListener("dblclick", () => {
    sliderGain.value = 1.0;
    gainValText.textContent = "1.00x";
    sendCommand({ action: "set_color_studio", brightness_gain: 1.0 });
  });

  sliderContrast.addEventListener("input", () => {
    const val = parseFloat(sliderContrast.value);
    contrastValText.textContent = `${val.toFixed(2)}x`;
    sendCommand({ action: "set_color_studio", contrast: val });
  });
  sliderContrast.addEventListener("dblclick", () => {
    sliderContrast.value = 1.0;
    contrastValText.textContent = "1.00x";
    sendCommand({ action: "set_color_studio", contrast: 1.0 });
  });

  sliderTemp.addEventListener("input", () => {
    const val = parseFloat(sliderTemp.value);
    tempValText.textContent = formatTempLabel(val);
    sendCommand({ action: "set_color_studio", color_temp: val });
  });
  sliderTemp.addEventListener("dblclick", () => {
    sliderTemp.value = 0.0;
    tempValText.textContent = "Neutro";
    sendCommand({ action: "set_color_studio", color_temp: 0.0 });
  });

  // Controles de Zoom Digital (ePTZ) e Arraste Panorâmico no Miniplayer
  sliderZoom.addEventListener("input", () => {
    const z = parseFloat(sliderZoom.value);
    updateZoomUI(z);
    sendCommand({ action: "set_eptz", zoom_level: currentZoom });
  });
  sliderZoom.addEventListener("dblclick", () => {
    panX = 0.0;
    panY = 0.0;
    updateZoomUI(1.0);
    sendCommand({ action: "set_eptz", zoom_level: 1.0, pan_x: 0.0, pan_y: 0.0 });
  });

  zoomBadge.addEventListener("click", () => {
    panX = 0.0;
    panY = 0.0;
    updateZoomUI(1.0);
    sendCommand({ action: "set_eptz", zoom_level: 1.0, pan_x: 0.0, pan_y: 0.0 });
  });

  // Rolagem da roda do mouse sobre o miniplayer ajusta o zoom
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

  // Arraste com o botão esquerdo para enquadrar imagem com zoom
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

  // Botão de Redefinição Geral do Estúdio
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

  // Alternar pulso Anti-PSM para Wi-Fi de baixa latência
  document.getElementById("chkAntiPsm").addEventListener("change", () => {
    sendCommand({ action: "toggle_anti_psm" });
  });

  // Seleção de Perfil de Resolução e Taxa de Quadros
  document.getElementById("resolutionSelect").addEventListener("change", (e) => {
    sendCommand({ action: "set_resolution", profile: e.target.value });
  });

  // --- Controles da Aba de Ajustes (Sistema & Bandeja do Windows) ---
  document.getElementById("chkStartWindows").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", start_with_windows: e.target.checked });
  });

  document.getElementById("chkStartMinimized").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", start_minimized: e.target.checked });
  });

  document.getElementById("chkMinToTray").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", minimize_to_tray: e.target.checked });
  });

  document.getElementById("chkCloseToTray").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", close_to_tray: e.target.checked });
  });

  document.getElementById("chkAutoEco").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", auto_eco_oled: e.target.checked });
  });

  document.getElementById("chkAutoStartVcam").addEventListener("change", (e) => {
    sendCommand({ action: "set_app_settings", auto_start_vcam: e.target.checked });
  });

  const btnSaveCameraName = document.getElementById("btnSaveCameraName");
  const txtCameraDisplayName = document.getElementById("txtCameraDisplayName");
  if (btnSaveCameraName && txtCameraDisplayName) {
    btnSaveCameraName.addEventListener("click", () => {
      const name = txtCameraDisplayName.value.trim() || "tVisio Virtual Camera";
      sendCommand({ action: "set_camera_name", name });
      const prev = btnSaveCameraName.textContent;
      btnSaveCameraName.textContent = "Salvo!";
      setTimeout(() => {
        btnSaveCameraName.textContent = prev;
      }, 1800);
    });
    txtCameraDisplayName.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        btnSaveCameraName.click();
      }
    });
  }

  // Ações de Segurança e Manutenção do Sistema
  document.getElementById("btnFirewall").addEventListener("click", async () => {
    const label = document.getElementById("btnFirewallText");
    const prev = label.textContent;
    label.textContent = "Configurando UAC...";
    try {
      const res = await fetch("/api/firewall", { method: "POST" });
      const data = await res.json();
      label.textContent = data.ok ? "Firewall Liberado!" : "Cancelado";
    } catch (_) {
      label.textContent = "Erro ao liberar";
    }
    setTimeout(() => {
      label.textContent = prev;
    }, 2200);
  });

  document.getElementById("btnRegenToken").addEventListener("click", async () => {
    const label = document.getElementById("btnRegenTokenText");
    const prev = label.textContent;
    try {
      const res = await fetch("/api/regenerate-token", { method: "POST" });
      const data = await res.json();
      if (data.ok) {
        qrImage.src = `/api/qr?t=${Date.now()}`;
        if (data.pairing_url) pairingUrlText.textContent = data.pairing_url;
        label.textContent = "Token Renovado!";
      }
    } catch (_) {}
    setTimeout(() => {
      label.textContent = prev;
    }, 2000);
  });

  document.getElementById("btnOpenDataDir").addEventListener("click", async () => {
    try {
      await fetch("/api/open-data-dir", { method: "POST" });
    } catch (_) {}
  });

  document.getElementById("btnQuitApp").addEventListener("click", async () => {
    try {
      await fetch("/api/app/quit", { method: "POST" });
    } catch (_) {}
    setTimeout(() => {
      window.close();
    }, 150);
  });

  // --- Verificador de Atualizações do GitHub e Download Automático ---
  const CURRENT_APP_VERSION = "v2.2.2";
  let latestGitHubRelease = null;
  let isAppInstalled = false;
  let isDownloadingUpdate = false;
  let downloadedUpdateFilePath = null;

  function compareSemanticVersions(v1, v2) {
    const clean1 = (v1 || "").replace(/^v/, "").split(".").map((x) => parseInt(x, 10) || 0);
    const clean2 = (v2 || "").replace(/^v/, "").split(".").map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < Math.max(clean1.length, clean2.length); i++) {
      const n1 = clean1[i] || 0;
      const n2 = clean2[i] || 0;
      if (n1 > n2) return 1;
      if (n1 < n2) return -1;
    }
    return 0;
  }

  function resolveTargetAssets(release) {
    if (!release || !Array.isArray(release.assets) || release.assets.length === 0) {
      return { primary: null, secondary: null, primaryLabel: "Baixar Atualização", secondaryLabel: "" };
    }

    const setupAsset = release.assets.find(
      (a) => a.name.toLowerCase().includes("setup") && a.name.toLowerCase().endsWith(".exe")
    );
    const zipAsset = release.assets.find(
      (a) => a.name.toLowerCase().includes("windows") && a.name.toLowerCase().endsWith(".zip")
    );
    const exeAsset = release.assets.find(
      (a) => a.name.toLowerCase() === "tvisio.exe"
    );

    if (isAppInstalled) {
      return {
        primary: setupAsset || exeAsset || zipAsset || release.assets[0],
        secondary: zipAsset || null,
        primaryLabel: "Baixar Instalador Oficial (.exe)",
        secondaryLabel: zipAsset ? "Ou baixar Pacote Portátil (.zip)" : "",
      };
    } else {
      return {
        primary: zipAsset || setupAsset || exeAsset || release.assets[0],
        secondary: setupAsset || null,
        primaryLabel: zipAsset ? "Baixar Atualização Portátil (.zip)" : "Baixar Instalador Oficial (.exe)",
        secondaryLabel: setupAsset ? "Ou baixar Instalador Oficial (.exe)" : "",
      };
    }
  }

  async function startAssetDownload(asset) {
    if (isDownloadingUpdate) return;
    if (!asset || !asset.browser_download_url) {
      const fallbackUrl = (latestGitHubRelease && latestGitHubRelease.html_url) || "https://github.com/TTreter/tVisio/releases/latest";
      window.open(fallbackUrl, "_blank");
      return;
    }

    const btnDownloadUpdate = document.getElementById("btnDownloadUpdate");
    const btnDownloadUpdateText = document.getElementById("btnDownloadUpdateText");
    const linkOpenDownloadsDir = document.getElementById("linkOpenDownloadsDir");
    const linkDownloadAlt = document.getElementById("linkDownloadAlt");

    isDownloadingUpdate = true;
    if (btnDownloadUpdateText) {
      btnDownloadUpdateText.textContent = `Baixando ${asset.name}...`;
    }
    if (btnDownloadUpdate) {
      btnDownloadUpdate.disabled = true;
      btnDownloadUpdate.style.opacity = "0.8";
    }

    try {
      // 1. Tenta download direto pelo backend (curl para a pasta Downloads do Windows)
      const res = await fetch("/api/app/update/download", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          url: asset.browser_download_url,
          filename: asset.name,
        }),
      });

      const data = await res.json();
      if (res.ok && data.ok && data.file_path) {
        downloadedUpdateFilePath = data.file_path;
        isDownloadingUpdate = false;
        if (btnDownloadUpdate) {
          btnDownloadUpdate.disabled = false;
          btnDownloadUpdate.style.opacity = "1";
          btnDownloadUpdate.style.background = "linear-gradient(135deg, #10B981 0%, #059669 100%)";
        }

        if (data.is_installer) {
          if (btnDownloadUpdateText) btnDownloadUpdateText.textContent = "Instalar Atualização (1 Clique)";
        } else {
          if (btnDownloadUpdateText) btnDownloadUpdateText.textContent = "Abrir Atualização Baixada";
        }

        if (linkOpenDownloadsDir) {
          linkOpenDownloadsDir.textContent = "Abrir pasta do instalador";
          linkOpenDownloadsDir.style.display = "inline";
          linkOpenDownloadsDir.onclick = (e) => {
            e.preventDefault();
            fetch("/api/app/update/launch", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ file_path: downloadedUpdateFilePath, open_folder: true }),
            });
          };
        }

        if (linkDownloadAlt) {
          linkDownloadAlt.style.display = "none";
        }
        return;
      }
      throw new Error(data.error || "Falha no download local");
    } catch (err) {
      console.warn("Falha no download nativo via backend, abrindo download direto no navegador:", err);
      // 2. Fallback: abre o link direto do arquivo (.exe / .zip) no navegador padrão (Chrome/Edge baixa na hora)
      try {
        await fetch("/api/open-external", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ url: asset.browser_download_url }),
        });
      } catch (_) {
        window.open(asset.browser_download_url, "_blank");
      }

      if (btnDownloadUpdateText) {
        btnDownloadUpdateText.textContent = "Download iniciado no navegador!";
      }
      setTimeout(() => {
        if (btnDownloadUpdateText) {
          btnDownloadUpdateText.textContent = "Baixar Novamente";
        }
        if (btnDownloadUpdate) {
          btnDownloadUpdate.disabled = false;
          btnDownloadUpdate.style.opacity = "1";
        }
      }, 3500);
    } finally {
      isDownloadingUpdate = false;
    }
  }

  async function checkAppUpdates(isManual = false) {
    const btnCheckUpdate = document.getElementById("btnCheckUpdate");
    const btnCheckUpdateText = document.getElementById("btnCheckUpdateText");
    const updateStatusText = document.getElementById("updateStatusText");
    const updateActionRow = document.getElementById("updateActionRow");
    const updateIcon = document.getElementById("updateIcon");
    const versionBadge = document.getElementById("versionBadge");

    if (isManual && btnCheckUpdateText) {
      btnCheckUpdateText.textContent = "Buscando...";
      if (updateIcon) updateIcon.style.animation = "spin 0.9s linear infinite";
    }

    try {
      const res = await fetch("https://api.github.com/repos/TTreter/tVisio/releases/latest", {
        headers: { Accept: "application/vnd.github.v3+json" },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const release = await res.json();
      latestGitHubRelease = release;

      const latestTag = release.tag_name || "";
      const isNewer = compareSemanticVersions(latestTag, CURRENT_APP_VERSION) > 0;

      if (isNewer) {
        if (updateStatusText) {
          updateStatusText.textContent = `Nova versão ${latestTag} disponível!`;
          updateStatusText.style.color = "#00F0FF";
        }
        if (updateActionRow) updateActionRow.style.display = "flex";

        const { primary, secondary, primaryLabel, secondaryLabel } = resolveTargetAssets(release);
        const btnDownloadUpdateText = document.getElementById("btnDownloadUpdateText");
        if (btnDownloadUpdateText && !downloadedUpdateFilePath) {
          btnDownloadUpdateText.textContent = primaryLabel;
        }

        const linkDownloadAlt = document.getElementById("linkDownloadAlt");
        if (linkDownloadAlt) {
          if (secondary && secondaryLabel && !downloadedUpdateFilePath) {
            linkDownloadAlt.textContent = secondaryLabel;
            linkDownloadAlt.style.display = "inline";
            linkDownloadAlt.onclick = (e) => {
              e.preventDefault();
              startAssetDownload(secondary);
            };
          } else {
            linkDownloadAlt.style.display = "none";
          }
        }

        if (versionBadge) {
          versionBadge.textContent = `UPDATE: ${latestTag}`;
          versionBadge.style.cursor = "pointer";
          versionBadge.title = `Nova versão ${latestTag} disponível! Clique para ver.`;
          versionBadge.onclick = () => {
            document.querySelectorAll("#sidebarTabs .tab-btn").forEach((b) => {
              b.classList.toggle("active", b.dataset.tab === "tabSettings");
            });
            document.querySelectorAll(".tab-panel").forEach((p) => {
              p.classList.toggle("active", p.id === "tabSettings");
            });
          };
        }
        if (isManual && btnCheckUpdateText) btnCheckUpdateText.textContent = "Disponível!";
      } else {
        if (updateStatusText) {
          updateStatusText.textContent = `Você está na versão mais recente (${CURRENT_APP_VERSION}).`;
          updateStatusText.style.color = "";
        }
        if (updateActionRow) updateActionRow.style.display = "none";
        if (isManual && btnCheckUpdateText) btnCheckUpdateText.textContent = "Atualizado";
      }
    } catch (_) {
      if (updateStatusText) {
        updateStatusText.textContent = "Não foi possível verificar no GitHub.";
      }
      if (isManual && btnCheckUpdateText) btnCheckUpdateText.textContent = "Tentar";
    } finally {
      if (updateIcon) updateIcon.style.animation = "";
    }
  }

  const btnCheckUpdate = document.getElementById("btnCheckUpdate");
  if (btnCheckUpdate) {
    btnCheckUpdate.addEventListener("click", () => checkAppUpdates(true));
  }

  const btnDownloadUpdate = document.getElementById("btnDownloadUpdate");
  if (btnDownloadUpdate) {
    btnDownloadUpdate.addEventListener("click", async () => {
      // Se já foi baixado, executa o instalador ou abre no Explorer
      if (downloadedUpdateFilePath) {
        try {
          const btnDownloadUpdateText = document.getElementById("btnDownloadUpdateText");
          if (btnDownloadUpdateText) btnDownloadUpdateText.textContent = "Iniciando instalador...";
          await fetch("/api/app/update/launch", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ file_path: downloadedUpdateFilePath, open_folder: false }),
          });
        } catch (_) {}
        return;
      }

      const { primary } = resolveTargetAssets(latestGitHubRelease);
      startAssetDownload(primary);
    });
  }

  // Checagem automática não-bloqueante 2s após a inicialização
  setTimeout(() => checkAppUpdates(false), 2200);

  // Inicialização do WebSocket
  connectDesktopWS();
})();
