/**
 * tVisio Mobile Transmitter — Ultra-Low Latency Camera Engine
 * Supports:
 * 1. WebRTC Zero-Buffer H.264 (UDP/RTP + DSCP High Priority + playout-delay=0)
 * 2. WebSocket Binary Turbo Stream (Zero-Queue Instant Start & Fallback)
 * 3. ISP Shutter Speed Capping (4ms / 8ms / 16ms) + Torch + DOM Video Detach (Eco-OLED)
 */

(() => {
  const videoEl = document.getElementById("cameraVideo");
  const capsuleDot = document.getElementById("capsuleDot");
  const capsuleStatus = document.getElementById("capsuleStatus");
  const capsuleRtt = document.getElementById("capsuleRtt");
  const ecoOverlay = document.getElementById("ecoOverlay");
  const mShutterText = document.getElementById("mShutterText");

  let currentStream = null;
  let facingMode = "environment";
  let torchOn = false;
  let ecoMode = false;
  let shutterCapMs = 8.0;
  let targetWidth = 1280;
  let targetHeight = 720;
  let targetFps = 60;

  let ws = null;
  let pc = null;
  let webrtcConnected = false;
  let rttMs = 4.5;
  let frameInFlight = false;

  // Offscreen canvas for instant Zero-Queue Binary Turbo start before/alongside WebRTC
  const captureCanvas = document.createElement("canvas");
  const captureCtx = captureCanvas.getContext("2d", { alpha: false, desynchronized: true });

  async function requestWakeLock() {
    if ("wakeLock" in navigator) {
      try {
        await navigator.wakeLock.request("screen");
      } catch (_) {}
    }
  }

  async function startCamera() {
    if (currentStream) {
      currentStream.getTracks().forEach((t) => t.stop());
    }

    const constraints = {
      audio: false,
      video: {
        facingMode: { ideal: facingMode },
        width: { ideal: targetWidth },
        height: { ideal: targetHeight },
        frameRate: { ideal: targetFps, max: 120 },
      },
    };

    try {
      currentStream = await navigator.mediaDevices.getUserMedia(constraints);
      videoEl.srcObject = currentStream;
      await videoEl.play();

      await applyOpticalSensorTuning();
      capsuleDot.classList.add("live");
      capsuleStatus.textContent = "tVisio Live";

      if (pc) {
        const videoTrack = currentStream.getVideoTracks()[0];
        const sender = pc.getSenders().find((s) => s.track && s.track.kind === "video");
        if (sender && videoTrack) {
          await sender.replaceTrack(videoTrack);
        }
      } else {
        startWebRTC();
      }
    } catch (err) {
      capsuleStatus.textContent = "Permita o acesso à câmera";
      console.error("Camera access error:", err);
    }
  }

  /**
   * Stage 1 Optimization: Optical Sensor Shutter Speed Capping (ISP Tuning)
   * Prevents mobile auto-exposure from dropping to 1/30s (33.3ms sensor delay).
   */
  async function applyOpticalSensorTuning() {
    if (!currentStream) return;
    const track = currentStream.getVideoTracks()[0];
    if (!track || !track.getCapabilities) return;

    try {
      const caps = track.getCapabilities();
      const adv = {};

      if (caps.exposureMode && caps.exposureMode.includes("manual") && shutterCapMs < 30) {
        adv.exposureMode = "manual";
        if (caps.exposureTime) {
          // exposureTime in MediaStreamTrack capabilities is typically in 100-microsecond units or ms
          const desired = Math.max(caps.exposureTime.min || 1, Math.min(caps.exposureTime.max || 100, shutterCapMs * 10));
          adv.exposureTime = desired;
        }
      } else if (caps.exposureMode && caps.exposureMode.includes("continuous")) {
        adv.exposureMode = "continuous";
      }

      if (caps.focusMode && caps.focusMode.includes("continuous")) {
        adv.focusMode = "continuous";
      }

      if (Object.keys(adv).length > 0) {
        await track.applyConstraints({ advanced: [adv] });
      }
    } catch (_) {}
  }

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
   * Stage 2 Optimization: DOM Video Detach (Eco-OLED Mode)
   * Detaches local <video> rendering from the smartphone's display VSync compositor.
   */
  function toggleEcoMode(forceState) {
    ecoMode = typeof forceState === "boolean" ? forceState : !ecoMode;
    ecoOverlay.classList.toggle("hidden", !ecoMode);
    // Hide video element visually so mobile SurfaceFlinger/CoreAnimation skips compositing
    videoEl.style.opacity = ecoMode ? "0" : "1";
    document.getElementById("mBtnEco").classList.toggle("active", ecoMode);
    sendTelemetry();
  }

  /**
   * Stage 3 Optimization: WebRTC Zero-Buffer H.264 Stream with DSCP QoS
   */
  async function startWebRTC() {
    if (!currentStream || typeof RTCPeerConnection === "undefined") return;

    try {
      pc = new RTCPeerConnection({ iceServers: [] });
      const videoTrack = currentStream.getVideoTracks()[0];
      // Hint browser encoder to optimize for motion / zero-latency fluid video
      if ("contentHint" in videoTrack) {
        videoTrack.contentHint = "motion";
      }

      const sender = pc.addTrack(videoTrack, currentStream);

      // Set DSCP High Priority (WMM Voice/Video queue) & tuned low-latency bitrate
      if (sender.getParameters && sender.setParameters) {
        try {
          const params = sender.getParameters();
          params.degradationPreference = "maintain-resolution";
          if (params.encodings && params.encodings.length > 0) {
            params.encodings[0].priority = "high";
            params.encodings[0].networkPriority = "high";
            params.encodings[0].maxBitrate = 4_500_000;
            params.encodings[0].maxFramerate = 60;
            await sender.setParameters(params);
          }
        } catch (_) {}
      }

      pc.onconnectionstatechange = () => {
        webrtcConnected = pc.connectionState === "connected" || pc.connectionState === "completed";
        if (webrtcConnected) {
          capsuleStatus.textContent = "WebRTC Zero-Buffer";
        } else if (pc.connectionState === "failed" || pc.connectionState === "disconnected") {
          try {
            pc.close();
          } catch (_) {}
          pc = null;
          setTimeout(startWebRTC, 1200);
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

      if (res.ok) {
        const answer = await res.json();
        await pc.setRemoteDescription(new RTCSessionDescription(answer));
      }
    } catch (e) {
      console.warn("WebRTC fallback to Binary Turbo WS:", e);
    }
  }

  /**
   * WebSocket Control Channel + Anti-PSM Receiver + Zero-Queue Binary Turbo Stream
   */
  function connectWebSocket() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws/stream`);
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      sendTelemetry();
      startBinaryTurboLoop();
    };

    ws.onmessage = (event) => {
      // 1-byte binary packet (0x01) = 500Hz Anti-PSM keep-alive pulse from PC!
      if (event.data instanceof ArrayBuffer) {
        return;
      }

      try {
        const msg = JSON.parse(event.data);
        if (msg.type === "ack" || msg.type === "pong") {
          const sampleRtt = performance.now() - msg.ts;
          if (sampleRtt > 0 && sampleRtt < 1000) {
            rttMs = rttMs * 0.75 + sampleRtt * 0.25;
            capsuleRtt.textContent = `⚡ ${(rttMs * 0.5 + shutterCapMs).toFixed(1)} ms`;
          }
          frameInFlight = false;
        } else if (msg.type === "command") {
          handleRemoteCommand(msg);
        }
      } catch (_) {}
    };

    ws.onclose = () => {
      webrtcConnected = false;
      setTimeout(connectWebSocket, 1000);
    };
  }

  function handleRemoteCommand(msg) {
    switch (msg.command) {
      case "switch_camera":
        facingMode = facingMode === "environment" ? "user" : "environment";
        startCamera();
        sendTelemetry();
        break;
      case "toggle_torch":
        toggleTorch();
        break;
      case "toggle_eco_mode":
        toggleEcoMode();
        break;
      case "set_shutter_cap":
        shutterCapMs = parseFloat(msg.shutter_ms || 8.0);
        mShutterText.textContent = shutterCapMs >= 30 ? "Auto" : `${shutterCapMs}ms`;
        applyOpticalSensorTuning();
        sendTelemetry();
        break;
      case "set_resolution":
        if (msg.profile) {
          const [dims, fps] = msg.profile.split("@");
          const [w, h] = dims.split("x").map(Number);
          targetWidth = w || 1280;
          targetHeight = h || 720;
          targetFps = Number(fps) || 60;
          startCamera();
        }
        break;
    }
  }

  function sendTelemetry() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const ua = navigator.userAgent;
    let device = "Smartphone Browser";
    if (/iPhone/i.test(ua)) device = "iPhone (Safari iOS)";
    else if (/iPad/i.test(ua)) device = "iPad (Safari iPadOS)";
    else if (/Android/i.test(ua)) device = "Android Smartphone";

    ws.send(
      JSON.stringify({
        type: "telemetry",
        rtt_ms: rttMs,
        device,
        eco_mode: ecoMode,
        torch_on: torchOn,
        facing_mode: facingMode,
        shutter_ms: shutterCapMs,
      })
    );
  }

  /**
   * Zero-Queue Binary Turbo Stream (Active immediately & whenever WebRTC is not primary)
   */
  function startBinaryTurboLoop() {
    const tick = () => {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;

      if (!webrtcConnected && !frameInFlight && videoEl.readyState >= 2) {
        const vw = videoEl.videoWidth || 1280;
        const vh = videoEl.videoHeight || 720;
        if (captureCanvas.width !== vw || captureCanvas.height !== vh) {
          captureCanvas.width = vw;
          captureCanvas.height = vh;
        }

        captureCtx.drawImage(videoEl, 0, 0, vw, vh);
        frameInFlight = true;
        const ts = performance.now();

        captureCanvas.toBlob(
          async (blob) => {
            if (!blob || !ws || ws.readyState !== WebSocket.OPEN) {
              frameInFlight = false;
              return;
            }
            const imgBuf = await blob.arrayBuffer();
            const packet = new Uint8Array(8 + imgBuf.byteLength);
            new DataView(packet.buffer).setFloat64(0, ts, true);
            packet.set(new Uint8Array(imgBuf), 8);
            ws.send(packet.buffer);
          },
          "image/jpeg",
          0.78
        );
      }

      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);

    // Periodic ping for sub-ms RTT tracking even when WebRTC carries the video
    setInterval(() => {
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "ping", ts: performance.now() }));
        sendTelemetry();
      }
    }, 600);
  }

  // Mobile UI Button Listeners
  document.getElementById("mBtnSwitch").addEventListener("click", () => {
    facingMode = facingMode === "environment" ? "user" : "environment";
    startCamera();
    sendTelemetry();
  });

  document.getElementById("mBtnTorch").addEventListener("click", () => {
    toggleTorch();
  });

  document.getElementById("mBtnShutter").addEventListener("click", () => {
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

  // Boot sequence
  requestWakeLock();
  startCamera();
  connectWebSocket();
})();
