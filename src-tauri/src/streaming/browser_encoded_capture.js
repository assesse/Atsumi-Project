// SPDX-License-Identifier: MIT
// Observe already-appended clear fMP4 in this Window only. Never fetch, clone a
// response, read credentials, create a player/decoder, or wait in appendBuffer.
(() => {
  "use strict";
  const channel = () => window.location.origin === "https://chzzk.naver.com" ?
    /^\/live\/([a-f0-9]{32})\/?$/i.exec(window.location.pathname)?.[1].toLowerCase() ?? null : null;
  if (!channel() || window.top !== window || window.__atsumiEncodedCapture) return;
  const MAX_INIT = 64 * 1024;
  const MAX_APPEND = 16 * 1024 * 1024;
  const MAX_QUEUE = 64 * 1024 * 1024;
  const CHUNK = 128 * 1024;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const recordingID = (id) => typeof id === "string" && (UUID.test(id) || /^[a-f0-9]{32}$/i.test(id));
  const NativeMediaSource = window.MediaSource;
  const NativeSourceBuffer = window.SourceBuffer;
  const NativeURL = window.URL;
  const sources = new WeakMap();
  const buffers = new WeakMap();
  const urls = new Map();
  const observed = new Set();
  const hooks = [];
  let installed = false;
  let active = null;
  let lastDetail = "encoded_unavailable";

  const safeNotify = (current, detail) => {
    lastDetail = detail;
    try { current?.options.onStatus?.(detail); } catch { /* UI callbacks never affect playback. */ }
  };
  const supportedMime = (type) => {
    if (typeof type !== "string" || type.length > 120) return false;
    const normalized = type.toLowerCase().replace(/[\s"]/g, "");
    const match = /^(video|audio)\/mp4;codecs=(.+)$/.exec(normalized);
    if (!match) return false;
    const codecs = match[2].split(",");
    return codecs.length >= 1 && codecs.length <= 2 && new Set(codecs).size === codecs.length &&
      codecs.every((codec) => /^(avc[13]\.[a-f0-9]{6}|mp4a\.40\.2)$/.test(codec)) &&
      (match[1] !== "audio" || (codecs.length === 1 && codecs[0] === "mp4a.40.2"));
  };
  const configurationOK = (state) => {
    try { return state.buffer.mode === "segments" && state.buffer.timestampOffset === 0 &&
      state.buffer.appendWindowStart === 0 && state.buffer.appendWindowEnd === Infinity; }
    catch { return false; }
  };
  const integrity = () => installed && hooks.every(({ target, name, wrapped }) => target[name] === wrapped);
  const sourceForVideo = (video) => {
    try {
      if (!video || video.isConnected === false || video.readyState < 2 || video.ended || video.mediaKeys || video.srcObject) return null;
      const source = urls.get(video.currentSrc || video.src);
      return source && source.channelId === channel() ? source : null;
    } catch { return null; }
  };
  const candidate = (video) => {
    const source = sourceForVideo(video);
    if (!integrity() || !source || source.blocked || video.paused || video.seeking || source.buffers.length < 1 || source.buffers.length > 2) return null;
    try { if (source.mediaSource.sourceBuffers.length !== source.buffers.length) return null; } catch { return null; }
    if (source.buffers.some((state) => state.blocked || !state.init || state.initParts.length || !configurationOK(state))) return null;
    // MIME is only an early eligibility check. Native init/sample validation,
    // including real A/V tracks and encryption rejection, remains authoritative.
    const declared = source.buffers.map((state) => state.mimeType.toLowerCase()).join(",");
    return /avc[13]\./.test(declared) && /mp4a\.40\.2/.test(declared) ? source : null;
  };
  const selectedVideo = () => [...document.querySelectorAll("video")]
    .filter((video) => video.isConnected !== false && video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0 && !video.ended)
    .sort((a, b) => { const x = a.getBoundingClientRect(), y = b.getBoundingClientRect(); return y.width * y.height - x.width * x.height; })[0] ?? null;
  const status = () => {
    const video = active?.video ?? selectedVideo();
    const ready = Boolean(candidate(video));
    return { active: Boolean(active), recording: Boolean(active), starting: Boolean(active && !active.recordingId),
      stopping: Boolean(active?.stopping), recordingId: active?.recordingId ?? null, channelId: active?.channelId ?? channel() ?? null,
      detail: active ? active.stopping ? "encoded_saving" : active.recordingId ? "encoded_recording" : "encoded_starting" : ready ? "encoded_ready" : lastDetail,
      ready, captureChat: active?.captureChat === true, captureMode: "encoded", rateControlAllowed: canChangePlaybackRate(video) };
  };
  const canChangePlaybackRate = (video) => Boolean(active && active.video === video && active.nativeApproved && active.accepting && !active.transportFailed && !active.stopping &&
    active.recordingId && candidate(video) === active.source);
  const getReplayClock = (video) => {
    if (!canChangePlaybackRate(video)) return null;
    const sourceTimeSeconds = video.currentTime;
    if (!Number.isFinite(sourceTimeSeconds) || sourceTimeSeconds < 0 || sourceTimeSeconds > 1e9) return null;
    // segments + untouched timestampOffset/append windows guarantee original
    // presentation time. Native muxing subtracts segment DTS origin and keeps
    // CTS: output PTS = source PTS - origin (not PTS == DTS).
    return { clock: "mse_presentation_v1", sourceId: active.source.sourceId, sourceTimeSeconds };
  };
  const bytesOf = (value) => {
    try {
      if (ArrayBuffer.isView(value)) {
        if (typeof SharedArrayBuffer !== "undefined" && value.buffer instanceof SharedArrayBuffer) return null;
        return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      }
      return value instanceof ArrayBuffer ? new Uint8Array(value) : null;
    } catch { return null; }
  };
  const base64 = (bytes) => {
    let value = "";
    for (let at = 0; at < bytes.length; at += 8192) value += String.fromCharCode(...bytes.subarray(at, at + 8192));
    return btoa(value);
  };
  const failure = (current, reason) => {
    if (!current) return;
    current.interrupted = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    current.accepting = false;
    // A failure raised by a queue operation must not await that same queue.
    void Promise.resolve().then(() => finish(current)).catch(() => undefined);
  };
  const block = (state, reason) => {
    state.blocked = true;
    if (active?.source === state.source) failure(active, reason);
  };
  const makeSource = (mediaSource) => {
    let source = sources.get(mediaSource);
    if (source) return source;
    if (observed.size >= 8) return null;
    source = { mediaSource, sourceId: crypto.randomUUID(), channelId: channel(), buffers: [], blocked: false };
    sources.set(mediaSource, source); observed.add(source);
    mediaSource.addEventListener("sourceclose", () => {
      source.blocked = true; observed.delete(source);
      for (const [url, bound] of urls) if (bound === source) urls.delete(url);
      if (active?.source === source) failure(active, "source_changed");
    });
    return source;
  };
  const makeBuffer = (source, buffer, mimeType) => {
    if (source.buffers.length >= 2 || !supportedMime(mimeType)) { source.blocked = true; return; }
    const state = { source, buffer, trackIndex: source.buffers.length, mimeType, blocked: false, header: new Uint8Array(8), headerBytes: 0,
      kind: null, remaining: 0, initParts: [], initSize: 0, ftyp: null, init: null, forwarding: false };
    source.buffers.push(state); buffers.set(buffer, state);
    for (const event of ["error", "abort"]) buffer.addEventListener(event, () => block(state, "source_buffer_error"));
    if (active?.source === source) failure(active, "track_changed");
  };
  const enqueue = (current, state, parts, byteLength) => {
    if (!current.accepting || byteLength === 0) return;
    if (byteLength > MAX_APPEND || current.queuedBytes + byteLength > MAX_QUEUE) { failure(current, "queue_overflow"); return; }
    current.queuedBytes += byteLength;
    let bytes;
    try { bytes = new Uint8Array(byteLength); let at = 0; for (const part of parts) { bytes.set(part, at); at += part.byteLength; } }
    catch { current.queuedBytes -= byteLength; failure(current, "queue_overflow"); return; }
    const appendIndex = current.appendIndexes[state.trackIndex]++;
    current.queue = current.queue.then(async () => {
      await current.begin;
      if (current.transportFailed) return;
      let chunkIndex = 0;
      for (let at = 0; at < bytes.length; at += CHUNK) {
        const end = Math.min(bytes.length, at + CHUNK);
        await current.options.request("encoded_append", { recordingId: current.recordingId, trackIndex: state.trackIndex,
          appendIndex, chunkIndex: chunkIndex++, finalChunk: end === bytes.length, data: base64(bytes.subarray(at, end)) });
      }
    }).catch(() => { current.transportFailed = true; failure(current, "native_rejected"); })
      .finally(() => { current.queuedBytes -= byteLength; });
  };
  const observeAppend = (state, value) => {
    if (state.blocked) return;
    if (!configurationOK(state)) { block(state, "timeline_changed"); return; }
    const bytes = bytesOf(value);
    if (!bytes || bytes.byteLength > MAX_APPEND) { block(state, "append_unsupported"); return; }
    const current = active?.source === state.source && active.accepting ? active : null;
    const parts = []; let forwarded = 0; let at = 0;
    while (at < bytes.length) {
      if (state.remaining === 0) {
        const count = Math.min(8 - state.headerBytes, bytes.length - at);
        state.header.set(bytes.subarray(at, at + count), state.headerBytes); state.headerBytes += count; at += count;
        if (state.headerBytes < 8) break;
        const size = new DataView(state.header.buffer).getUint32(0);
        const kind = String.fromCharCode(...state.header.subarray(4));
        if (size < 8 || size > MAX_APPEND || !["ftyp", "moov", "moof", "mdat", "styp", "sidx", "emsg", "prft", "free"].includes(kind)) { block(state, "container_unsupported"); return; }
        state.kind = kind; state.remaining = size - 8; state.headerBytes = 0;
        if (kind === "ftyp" || kind === "moov") {
          if (current) { block(state, "init_changed"); return; }
          if (size > MAX_INIT || (kind === "moov" && (!state.ftyp || state.ftyp.length + size > MAX_INIT))) { block(state, "init_unsupported"); return; }
          state.initParts = [state.header.slice()]; state.initSize = 8;
        } else {
          if (kind === "moof") state.forwarding = Boolean(current);
          if (current && state.forwarding) { parts.push(state.header.slice()); forwarded += 8; }
        }
        if (state.remaining === 0) {
          if (kind === "ftyp" || kind === "moov") { block(state, "init_unsupported"); return; }
          continue;
        }
      }
      const count = Math.min(state.remaining, bytes.length - at);
      const part = bytes.subarray(at, at + count);
      if (state.kind === "ftyp" || state.kind === "moov") {
        state.initParts.push(part.slice()); state.initSize += count;
      } else if (current && state.forwarding) { parts.push(part); forwarded += count; }
      at += count; state.remaining -= count;
      if (state.remaining === 0 && (state.kind === "ftyp" || state.kind === "moov")) {
        const complete = new Uint8Array(state.initSize); let offset = 0;
        for (const part of state.initParts) { complete.set(part, offset); offset += part.length; }
        state.initParts = []; state.initSize = 0;
        if (state.kind === "ftyp") { state.ftyp = complete; state.init = null; }
        else { state.init = new Uint8Array(state.ftyp.length + complete.length); state.init.set(state.ftyp); state.init.set(complete, state.ftyp.length); }
      }
    }
    if (current && forwarded) enqueue(current, state, parts, forwarded);
  };
  const hook = (target, name, create) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    if (!descriptor || typeof descriptor.value !== "function") throw new Error("hook_unavailable");
    const wrapped = create(descriptor.value);
    Object.defineProperty(target, name, { ...descriptor, value: wrapped }); hooks.push({ target, name, wrapped, descriptor });
  };
  try {
    if (!NativeMediaSource || !NativeSourceBuffer || !NativeURL) throw new Error("mse_unavailable");
    hook(NativeURL, "createObjectURL", (original) => function createObjectURL(value) {
      const url = Reflect.apply(original, this, arguments);
      try { if (value instanceof NativeMediaSource && urls.size < 16) { const source = makeSource(value); if (source) urls.set(url, source); } } catch { /* Original result is preserved. */ }
      return url;
    });
    hook(NativeMediaSource.prototype, "addSourceBuffer", (original) => function addSourceBuffer(type) {
      const buffer = Reflect.apply(original, this, arguments);
      try { const source = makeSource(this); if (source) makeBuffer(source, buffer, type); } catch { /* Original result is preserved. */ }
      return buffer;
    });
    hook(NativeSourceBuffer.prototype, "appendBuffer", (original) => function appendBuffer(value) {
      const result = Reflect.apply(original, this, arguments);
      // Read/copy after the browser accepted the append, before returning to
      // its caller. Do not detach/reuse/mutate the caller's ArrayBuffer.
      try { const state = buffers.get(this); if (state) observeAppend(state, value); } catch { const state = buffers.get(this); if (state) block(state, "observer_failed"); }
      return result;
    });
    for (const name of ["changeType", "abort"]) if (typeof NativeSourceBuffer.prototype[name] === "function") hook(NativeSourceBuffer.prototype, name, (original) => function () {
      const result = Reflect.apply(original, this, arguments);
      const state = buffers.get(this); if (state) block(state, name === "changeType" ? "codec_changed" : "source_buffer_error"); return result;
    });
    if (typeof NativeMediaSource.prototype.removeSourceBuffer === "function") hook(NativeMediaSource.prototype, "removeSourceBuffer", (original) => function removeSourceBuffer(buffer) {
      const result = Reflect.apply(original, this, arguments); const state = buffers.get(buffer); if (state) block(state, "track_changed"); return result;
    });
    installed = true;
  } catch {
    for (const { target, name, descriptor } of hooks.reverse()) { try { Object.defineProperty(target, name, descriptor); } catch { /* Capability remains disabled. */ } }
  }

  const safetyReason = (current) => {
    if (channel() !== current.channelId) return "channel_changed";
    if (!integrity()) return "observer_changed";
    if (sourceForVideo(current.video) !== current.source || current.source.blocked) return "source_changed";
    if (current.video.seeking) return "seek";
    if (current.video.paused) return "paused";
    if (current.source.buffers.some((state) => state.blocked || !configurationOK(state))) return "timeline_changed";
    return null;
  };
  const cleanUp = (current) => {
    clearInterval(current.timer);
    for (const remove of current.listeners) remove();
    for (const state of current.source.buffers) state.forwarding = false;
    if (active === current) active = null;
  };
  const finish = (current) => {
    if (current.finishPromise) return current.finishPromise;
    current.accepting = false; current.stopping = true; safeNotify(current, "encoded_saving");
    current.finishPromise = (async () => {
      try {
        await current.begin;
        await current.queue;
        await current.options.beforeFinish?.(current.recordingId);
        const response = await current.options.request("encoded_finish", { recordingId: current.recordingId,
          interrupted: current.interrupted || current.transportFailed, reason: current.reason ?? "user_stop" });
        if (!response || response.stopped !== true) throw new Error("native_rejected");
        cleanUp(current);
        safeNotify(current, response.interrupted || current.interrupted || current.transportFailed ? "encoded_interrupted" : "encoded_saved");
        return response;
      } catch (cause) { cleanUp(current); safeNotify(current, "native_rejected"); throw cause; }
    })();
    return current.finishPromise;
  };
  const start = (command, options) => {
    if (active) return Promise.reject(new Error("recording_active"));
    const video = command?.video ?? selectedVideo(); const source = candidate(video);
    if (!source || command?.channelId !== channel() || command.rightsAcknowledged !== true || !UUID.test(command.requestId ?? "") || typeof options?.request !== "function") {
      return Promise.reject(new Error("encoded_unavailable"));
    }
    const current = { source, video, channelId: channel(), options, recordingId: null, captureChat: false, nativeApproved: false,
      accepting: true, starting: true, stopping: false, interrupted: false, transportFailed: false, reason: null,
      appendIndexes: source.buffers.map(() => 0), queuedBytes: 0, queue: Promise.resolve(), begin: null, finishPromise: null, listeners: [], timer: null };
    active = current;
    for (const state of source.buffers) state.forwarding = false;
    const listen = (name, reason) => { const callback = () => failure(current, reason); video.addEventListener(name, callback); current.listeners.push(() => video.removeEventListener(name, callback)); };
    listen("seeking", "seek"); listen("pause", "paused"); listen("emptied", "source_changed"); listen("ended", "source_changed"); listen("encrypted", "encrypted");
    current.timer = setInterval(() => { const reason = safetyReason(current); if (reason && !current.stopping) failure(current, reason); }, 500);
    safeNotify(current, "encoded_starting");
    current.begin = Promise.resolve().then(() => options.request("encoded_begin", { requestId: command.requestId, channelId: current.channelId,
      title: String(document.title ?? "CHZZK").slice(0, 200), sourceId: source.sourceId,
      tracks: source.buffers.map((state) => ({ trackIndex: state.trackIndex, mimeType: state.mimeType, init: base64(state.init) })) }))
      .then((response) => {
        if (!response || !recordingID(response.id) || response.mode !== "encoded" || response.nativeApproved !== true) throw new Error("encoded_not_approved");
        current.recordingId = response.id; current.captureChat = response.captureChat === true; current.nativeApproved = true; current.starting = false;
        const reason = safetyReason(current); if (reason && !current.stopping) failure(current, reason);
        safeNotify(current, current.stopping ? "encoded_saving" : "encoded_recording"); return response;
      }).catch((cause) => {
        current.transportFailed = true; current.accepting = false; cleanUp(current); safeNotify(current, "native_rejected");
        const error = new Error(cause?.message ?? "native_rejected"); error.nativeAttempted = true;
        // This exact code is issued by native init validation BEFORE consuming
        // the arm or opening a recording. No lost ACK/append/finish failure may
        // fall back: those may already own files or an active native session.
        if (cause?.code === "ENCODED_UNSUPPORTED") { source.blocked = true; error.allowLegacyFallback = true; }
        throw error;
      });
    return current.begin;
  };
  window.addEventListener("pagehide", () => { if (active) failure(active, "page_hidden"); });
  Object.defineProperty(window, "__atsumiEncodedCapture", { value: Object.freeze({
    canStart: (video) => Boolean(candidate(video)), supports: (video) => Boolean(candidate(video)),
    canChangePlaybackRate, getReplayClock, getStatus: status, start,
    stop(reason = "user_stop", interrupted = false) {
      if (!active) return Promise.resolve({ stopped: true, interrupted: false });
      active.reason = reason; active.interrupted ||= interrupted; return finish(active);
    },
  }) });
})();
