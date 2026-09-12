// SPDX-License-Identifier: MIT
// Atsumi's own browser capture bridge. No site credentials or media URLs are read.
(() => {
  "use strict";

  const channel = () => {
    if (window.location.origin !== "https://chzzk.naver.com") return null;
    return /^\/live\/([a-f0-9]{32})\/?$/i.exec(window.location.pathname)?.[1].toLowerCase() ?? null;
  };
  if (!channel() || window.top !== window || !window.chrome?.webview?.postMessage) return;
  if (window.__atsumiBrowserCaptureInstalled) return;
  Object.defineProperty(window, "__atsumiBrowserCaptureInstalled", { value: true });

  const CHUNK_BYTES = 128 * 1024;
  const QUEUE_BYTES = 16 * 1024 * 1024;
  const SEGMENT_MS = 15_000;
  const ACK_MS = 15_000;
  // Wry's default WebMessage handler expects a string before custom handlers
  // run. The native host routes this reserved prefix outside Tauri IPC.
  const MESSAGE_PREFIX = "ATSUMI_BROWSER_CAPTURE:";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const recordingIdIsValid = (id) => typeof id === "string" &&
    (UUID.test(id) || /^[0-9a-f]{32}$/i.test(id));
  const pending = new Map();
  let session = null;
  let generation = 0;
  let lastDetail = "ready";
  let toolbar = null;
  let encodedStarting = false;
  let encodedChat = null;
  const encodedState = () => window.__atsumiEncodedCapture?.getStatus() ?? null;
  const encodedDetail = (detail) => ({ encoded_ready: "ready", encoded_starting: "starting",
    encoded_recording: "recording", encoded_saving: "saving", encoded_saved: "saved",
    encoded_interrupted: "native_rejected", encoded_unavailable: "unavailable" })[detail] ?? detail;

  // The native host must also validate the WebView label, current URL and its
  // one-use start requestId. Page script/events are never an authorization boundary.
  const post = (kind, fields) => {
    const id = crypto.randomUUID();
    window.chrome.webview.postMessage(MESSAGE_PREFIX + JSON.stringify({ atsumiBrowserCapture: 1, id, kind, ...fields }));
    return id;
  };
  const request = (kind, fields) => new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error("native_rejected"));
    }, ACK_MS);
    pending.set(id, { resolve, reject, timer });
    try {
      window.chrome.webview.postMessage(MESSAGE_PREFIX + JSON.stringify({ atsumiBrowserCapture: 1, id, kind, ...fields }));
    } catch {
      clearTimeout(timer);
      pending.delete(id);
      reject(new Error("native_rejected"));
    }
  });
  window.addEventListener("atsumi-browser-reply", (event) => {
    const reply = event.detail;
    if (!reply || typeof reply.id !== "string") return;
    const waiter = pending.get(reply.id);
    if (!waiter) return;
    pending.delete(reply.id);
    clearTimeout(waiter.timer);
    if (reply.ok === true) waiter.resolve(reply.data);
    else {
      const error = new Error("native_rejected");
      // A definitive pre-arm format rejection is different from a lost ACK.
      if (typeof reply.error?.code === "string") error.code = reply.error.code;
      waiter.reject(error);
    }
  });

  const videoSource = () => [...document.querySelectorAll("video")]
    .filter((video) => video.isConnected !== false && video.readyState >= 2 &&
      video.videoWidth > 0 && video.videoHeight > 0 && !video.ended)
    .sort((a, b) => {
      const left = a.getBoundingClientRect();
      const right = b.getBoundingClientRect();
      return right.width * right.height - left.width * left.height;
    })[0] ?? null;
  const chooseMime = () => {
    if (typeof MediaRecorder === "undefined") return null;
    return [
      "video/webm;codecs=vp8,opus",
      "video/mp4;codecs=avc1.42E01E,mp4a.40.2",
      "video/mp4;codecs=avc1,mp4a.40.2",
      "video/mp4",
    ].find((mime) => MediaRecorder.isTypeSupported(mime)) ?? null;
  };
  const bufferSeconds = (video) => {
    try {
      const ranges = video?.buffered;
      if (!ranges?.length) return undefined;
      const seconds = ranges.end(ranges.length - 1) - video.currentTime;
      return Number.isFinite(seconds) ? Math.max(0, seconds) : undefined;
    } catch { return undefined; }
  };
  const detailText = {
    ready: "녹화를 시작할 수 있습니다",
    unavailable: "영상 또는 녹화 기능을 기다리는 중",
    starting: "녹화 준비 중",
    recording: "녹화 중",
    saving: "남은 영상 저장 중",
    saved: "녹화 저장 완료",
    rights_required: "Atsumi에서 녹화 권한을 확인하세요",
    no_audio: "오디오 트랙 없음 · 재생 상태를 확인하세요",
    seek: "탐색으로 녹화 중단",
    rate_change: "배속 변경으로 녹화 중단",
    video_changed: "영상 변경으로 녹화 중단",
    channel_changed: "채널 이동으로 녹화 중단",
    page_hidden: "페이지 종료로 녹화 중단",
    recorder_error: "인코더 오류로 녹화 중단",
    queue_overflow: "저장 지연으로 녹화 중단",
    native_rejected: "저장 오류 · 일부 녹화가 보존되지 않았습니다",
    empty_segment: "빈 녹화 조각 · 정상 저장되지 않았습니다",
  };
  const renderStatus = (detail) => {
    if (!toolbar && document.body) {
      toolbar = document.createElement("div");
      toolbar.id = "atsumi-browser-capture-status";
      toolbar.setAttribute("role", "status");
      toolbar.tabIndex = 0;
      toolbar.style.cssText = "position:fixed;right:12px;top:46px;z-index:2147483646;max-width:min(240px,calc(100vw - 24px));padding:4px 8px;border:1px solid #53606e;border-radius:6px;background:#151c24d9;color:#eef3f8;font:11px/1.5 system-ui,sans-serif;white-space:nowrap;overflow:hidden;text-overflow:ellipsis";
      document.body.appendChild(toolbar);
    }
    const compact = { ready: "녹화 준비", unavailable: "영상 대기", starting: "녹화 준비 중",
      recording: "녹화 중", saving: "저장 중", saved: "저장 완료" };
    const text = compact[detail] ?? detailText[detail] ?? compact.unavailable;
    const description = `${detailText[detail] ?? detailText.unavailable}${detail === "recording" ? " · 완료된 영상은 15초마다 저장합니다" : ""}`;
    if (toolbar) {
      toolbar.setAttribute("aria-label", description);
      toolbar.setAttribute("title", description);
    }
    if (toolbar && toolbar.textContent !== text) toolbar.textContent = text;
  };
  const status = () => {
    const video = videoSource();
    const encoded = encodedState();
    const encodedActive = Boolean(encodedStarting || encoded?.active);
    const ready = Boolean(channel() && video && !video.paused && !video.seeking &&
      (window.__atsumiEncodedCapture?.canStart(video) || (video.playbackRate === 1 && video.captureStream && chooseMime())));
    const detail = encodedActive ? (encodedStarting && !encoded?.active ? "starting" : encodedDetail(encoded?.detail ?? "starting")) : session ? (session.stopping ? "saving" : session.recordingId ? "recording" : "starting") :
      lastDetail === "ready" && !ready ? "unavailable" : lastDetail;
    renderStatus(detail);
    const fields = { channelId: session?.channelId ?? channel(), ready,
      recording: Boolean(session || encodedActive), detail, captureMode: encodedActive ? "encoded" : "reencoded" };
    if (video) {
      if (Number.isInteger(video.videoWidth) && video.videoWidth >= 0 && video.videoWidth <= 16384) fields.videoWidth = video.videoWidth;
      if (Number.isInteger(video.videoHeight) && video.videoHeight >= 0 && video.videoHeight <= 16384) fields.videoHeight = video.videoHeight;
      fields.paused = Boolean(video.paused);
    }
    const buffered = bufferSeconds(video);
    if (buffered !== undefined) fields.bufferSeconds = buffered;
    if (session?.recordingId) fields.recordingId = session.recordingId;
    if (encoded?.recordingId) fields.recordingId = encoded.recordingId;
    window.__atsumiPlayerUI?.update(fields);
    try { post("status", fields); } catch { /* Native teardown is handled by the host. */ }
  };
  const stopTracks = (stream) => {
    for (const track of stream.getTracks()) {
      try { track.stop(); } catch { /* A source track may already have ended. */ }
    }
  };
  const listen = (current, target, name, callback) => {
    target.addEventListener(name, callback);
    current.listeners.push(() => target.removeEventListener(name, callback));
  };

  const fail = (current, reason) => {
    current.interrupted = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    stop(current, current.reason, true);
  };
  // All writes, including segment boundaries, share one ACK-ordered queue.
  // Reservations include Blobs waiting for arrayBuffer(), not just encoded messages.
  const enqueue = (current, bytes, operation) => {
    current.queuedBytes += bytes;
    current.queue = current.queue.then(async () => {
      try {
        if (!current.transportFailed) await operation();
      } catch {
        current.transportFailed = true;
        fail(current, "native_rejected");
      } finally {
        current.queuedBytes -= bytes;
      }
    });
  };
  const acceptBlob = (current, segment, blob) => {
    if (!blob || blob.size === 0) return;
    if (segment.closed || current.transportFailed || segment.invalid) return;
    if (!Number.isFinite(blob.size) || blob.size < 0 ||
        current.queuedBytes + blob.size > QUEUE_BYTES) {
      segment.invalid = true;
      fail(current, "queue_overflow");
      return;
    }
    segment.bytes += blob.size;
    enqueue(current, blob.size, async () => {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      for (let offset = 0; offset < bytes.length; offset += CHUNK_BYTES) {
        const part = bytes.subarray(offset, offset + CHUNK_BYTES);
        let binary = "";
        for (let index = 0; index < part.length; index += 8192) {
          binary += String.fromCharCode(...part.subarray(index, index + 8192));
        }
        await request("chunk", { recordingId: current.recordingId,
          segmentIndex: segment.index, chunkIndex: segment.nextChunkIndex, data: btoa(binary) });
        segment.nextChunkIndex += 1;
      }
    });
  };
  const drainChat = async (recordingId) => {
    let deadline;
    try {
      // Leave time within native exit's eight-second drain for the video tail.
      // A stuck chat ACK must not prevent either recording path from finishing.
      await Promise.race([
        Promise.resolve().then(() => window.__atsumiPageChat?.stop(recordingId)),
        new Promise((_, reject) => { deadline = setTimeout(() => reject(new Error("chat_drain_timeout")), 2000); }),
      ]);
    } catch {
      try { post("chat_status", { recordingId, detail: "storage_failed", droppedMessages: 0 }); } catch { /* Native recovery owns final state. */ }
    } finally { clearTimeout(deadline); }
  };
  const finish = (current) => {
    if (current.finishing || !current.recordingId || current.activeSegment) return;
    if (!current.nextSegmentIndex && !current.interrupted) {
      current.interrupted = true;
      current.reason = "empty_segment";
    }
    current.finishing = true;
    for (const remove of current.listeners) remove();
    stopTracks(current.stream);
    void (async () => {
      await current.queue;
      if (current.chatDrain) await current.chatDrain;
      let detail = current.interrupted ? current.reason : "saved";
      try {
        const result = await request("finish", { recordingId: current.recordingId,
          interrupted: current.interrupted,
          ...(current.interrupted ? { reason: current.reason } : {}) });
        if (result?.interrupted === true || result?.stopped === false ||
            String(result?.status ?? "").toLowerCase() === "interrupted") {
          detail = current.interrupted ? current.reason : "native_rejected";
        }
      } catch { detail = "native_rejected"; }
      // A stale completion must never change a replacement generation's UI.
      if (session === current && generation === current.generation) {
        session = null;
        lastDetail = detail;
        status();
      }
    })();
  };
  const safetyReason = (current) => {
    if (channel() !== current.channelId) return "channel_changed";
    if (videoSource() !== current.video) return "video_changed";
    if (current.video.seeking) return "seek";
    if (current.video.playbackRate !== 1) return "rate_change";
    if (!current.stream.getAudioTracks().some((track) => track.readyState !== "ended")) return "no_audio";
    return null;
  };
  const startSegment = (current) => {
    const segment = { index: current.nextSegmentIndex++, nextChunkIndex: 0,
      bytes: 0, invalid: false, closed: false, timer: null, startedAt: performance.now(), recorder: null };
    try {
      const recorder = new MediaRecorder(current.stream, {
        mimeType: current.mimeType, videoBitsPerSecond: 8_000_000, audioBitsPerSecond: 128_000,
      });
      segment.recorder = recorder;
      current.activeSegment = segment;
      recorder.ondataavailable = (event) => acceptBlob(current, segment, event.data);
      recorder.onerror = () => { segment.invalid = true; fail(current, "recorder_error"); };
      recorder.onstop = () => {
        if (segment.closed) return;
        segment.closed = true;
        clearTimeout(segment.timer);
        if (current.activeSegment === segment) current.activeSegment = null;
        const seconds = ((segment.stoppedAt ?? performance.now()) - segment.startedAt) / 1000;
        const empty = !segment.bytes || !Number.isFinite(seconds) || seconds <= 0;
        if (empty) {
          segment.invalid = true;
        }
        enqueue(current, 0, async () => {
          if (!segment.invalid) await request("segment", {
            recordingId: current.recordingId, segmentIndex: segment.index, durationSeconds: seconds,
          });
        });
        if (empty) fail(current, "empty_segment");
        if (!current.stopping) {
          const reason = safetyReason(current);
          if (reason) fail(current, reason);
          else startSegment(current);
        }
        if (current.stopping) finish(current);
      };
      recorder.start(1000);
      segment.timer = setTimeout(() => {
        if (recorder.state !== "inactive") {
          try { segment.stoppedAt = performance.now(); recorder.stop(); }
          catch { segment.invalid = true; fail(current, "recorder_error"); }
        }
      }, SEGMENT_MS);
    } catch {
      segment.invalid = true;
      current.activeSegment = null;
      fail(current, "recorder_error");
    }
  };
  const stop = (current, reason, interrupted) => {
    if (session !== current) return;
    current.stopping = true;
    if (current.chatStarted && !current.chatDrain) {
      current.chatDrain = drainChat(current.recordingId);
    }
    if (interrupted) current.interrupted = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    const segment = current.activeSegment;
    if (segment && !segment.closed) {
      clearTimeout(segment.timer);
      if (segment.recorder.state !== "inactive") {
        try { segment.stoppedAt = performance.now(); segment.recorder.stop(); }
        catch {
          segment.invalid = true;
          segment.closed = true;
          current.activeSegment = null;
          current.interrupted = true;
          current.reason = "recorder_error";
          finish(current);
        }
      }
      // state becomes inactive before the final dataavailable/onstop events.
      // Wait for those events instead of closing the native file here.
    } else finish(current);
    status();
  };
  const start = async (command) => {
    if (session || encodedStarting || encodedState()?.active) return;
    if (command.rightsAcknowledged !== true || !UUID.test(command.requestId ?? "")) {
      lastDetail = "rights_required";
      status();
      return;
    }
    const channelId = channel();
    const video = videoSource();
    if (channelId && channelId === command.channelId && video && !video.paused &&
        window.__atsumiEncodedCapture?.canStart(video)) {
      encodedStarting = true; lastDetail = "starting"; status();
      let safeFallback = false;
      try {
        const response = await window.__atsumiEncodedCapture.start(command, {
          request, post,
          onStatus: (detail) => { lastDetail = encodedDetail(detail); status(); },
          beforeFinish: async (recordingId) => {
            const chat = encodedChat;
            if (!chat || chat.recordingId !== recordingId) return;
            if (!chat.drain) chat.drain = drainChat(recordingId);
            await chat.drain;
            if (encodedChat === chat) encodedChat = null;
          },
        });
        if (!response || !recordingIdIsValid(response.id) || response.mode !== "encoded" || response.nativeApproved !== true) throw new Error("native_rejected");
        encodedStarting = false;
        if (response.captureChat === true && encodedState()?.active && !encodedState()?.stopping) {
          const reportChat = (detail, droppedMessages) => {
            try { post("chat_status", { recordingId: response.id, detail, droppedMessages }); } catch { /* Separate from video persistence. */ }
          };
          try {
            const started = window.__atsumiPageChat?.start({ recordingId: response.id, channelId,
              getVideo: () => video,
              sendBatch: (events) => request("chat_batch", { recordingId: response.id, events }), onStatus: reportChat }) === true;
            if (started) encodedChat = { recordingId: response.id, drain: null };
            else reportChat("observer_unavailable", 0);
          } catch { reportChat("observer_unavailable", 0); }
        }
      } catch (error) {
        // An attempted encoded begin must never silently arm a second legacy
        // recorder after a lost ACK. Native ownership/recovery resolves it.
        encodedStarting = false; lastDetail = "native_rejected";
        safeFallback = error?.allowLegacyFallback === true;
      }
      if (!safeFallback) { status(); return; }
      // Only the native parser's explicit BEFORE-arm rejection permits this.
      // The encoded observer disables that source, so subsequent retries use 1x.
      lastDetail = "ready";
    }
    window.__atsumiPlayerUI?.stopCatchup();
    const mimeType = chooseMime();
    if (!channelId || channelId !== command.channelId || !video || video.paused || !video.captureStream || !mimeType) {
      lastDetail = "unavailable";
      status();
      return;
    }
    if (video.playbackRate !== 1 || video.seeking) {
      lastDetail = video.seeking ? "seek" : "rate_change";
      status();
      return;
    }
    let stream;
    try { stream = video.captureStream(); }
    catch { lastDetail = "unavailable"; status(); return; }
    if (!stream.getAudioTracks().some((track) => track.readyState !== "ended") || !stream.getVideoTracks().length) {
      stopTracks(stream);
      lastDetail = "no_audio";
      status();
      return;
    }
    const current = { generation: ++generation, channelId, video, stream, mimeType,
      recordingId: null, nextSegmentIndex: 0, activeSegment: null,
      queue: Promise.resolve(), queuedBytes: 0, transportFailed: false,
      chatStarted: false, chatDrain: null,
      stopping: false, interrupted: false, finishing: false, reason: null, listeners: [] };
    session = current;
    listen(current, video, "seeking", () => fail(current, "seek"));
    listen(current, video, "ratechange", () => {
      if (video.playbackRate !== 1) fail(current, "rate_change");
    });
    listen(current, video, "ended", () => fail(current, "video_changed"));
    listen(current, video, "emptied", () => fail(current, "video_changed"));
    for (const track of stream.getTracks()) {
      listen(current, track, "ended", () => fail(current, track.kind === "audio" ? "no_audio" : "video_changed"));
    }
    status();
    try {
      const response = await request("begin", { channelId,
        title: String(document.title ?? "CHZZK").slice(0, 200), mimeType, requestId: command.requestId });
      if (!response || !recordingIdIsValid(response.id)) throw new Error("native_rejected");
      current.recordingId = response.id;
      if (response.captureChat === true && !current.stopping) {
        const reportChat = (detail, droppedMessages) => {
          try { post("chat_status", { recordingId: current.recordingId, detail, droppedMessages }); } catch { /* Recording and chat errors are separate. */ }
        };
        try {
          current.chatStarted = window.__atsumiPageChat?.start({ recordingId: current.recordingId,
            channelId: current.channelId,
            getVideo: () => current.video,
            sendBatch: (events) => request("chat_batch", { recordingId: current.recordingId, events }),
            onStatus: reportChat,
          }) === true;
          if (!current.chatStarted) reportChat("observer_unavailable", 0);
        } catch { reportChat("observer_unavailable", 0); }
      }
      if (!current.stopping) {
        const reason = safetyReason(current);
        if (reason) fail(current, reason);
        else startSegment(current);
      }
      if (current.stopping) finish(current);
      status();
    } catch {
      for (const remove of current.listeners) remove();
      stopTracks(stream);
      if (session === current) {
        session = null;
        lastDetail = "native_rejected";
        status();
      }
    }
  };
  window.addEventListener("atsumi-browser-command", (event) => {
    const command = event.detail;
    if (!command || typeof command !== "object") return;
    if (command.kind === "start") void start(command);
    else if (command.kind === "stop" && encodedState()?.active && command.channelId === encodedState()?.channelId) {
      void window.__atsumiEncodedCapture.stop("user_stop", false).catch(() => { lastDetail = "native_rejected"; status(); });
    }
    else if (command.kind === "stop" && session && command.channelId === session.channelId) {
      stop(session, "user_stop", false);
    }
  });
  window.addEventListener("pagehide", () => {
    // Best effort only: the native host owns crash/window-destruction recovery.
    // visibilitychange is deliberately NOT a stop signal.
    if (session) stop(session, "page_hidden", true);
  });
  setInterval(() => {
    if (session && !session.stopping) {
      const reason = safetyReason(session);
      if (reason) fail(session, reason);
    }
    status();
  }, 1000);
  status();
})();
