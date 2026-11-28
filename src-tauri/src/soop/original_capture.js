// SPDX-License-Identifier: MIT
// SOOP-local copy of the bounded observer algorithm; no CHZZK runtime state.
// Observe already-appended clear fMP4 in this Window only. Never fetch, clone a
// response, read credentials, create a player/decoder, or wait in appendBuffer.
(() => {
  "use strict";
  const channel = () => window.location.origin === "https://play.sooplive.com" ?
    /^\/([A-Za-z0-9_-]{1,40})\/(?:[0-9]+\/)?embed\/?$/.exec(window.location.pathname)?.[1].toLowerCase() ?? null : null;
  if (!channel() || window.top !== window || window.__atsumiSoopEncodedCapture) return;
  const MAX_INIT = 64 * 1024;
  const MAX_APPEND = 16 * 1024 * 1024;
  const MAX_QUEUE = 64 * 1024 * 1024;
  const MAX_RELAY_QUEUE = 32 * 1024 * 1024;
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
  let relay = null;
  let lastRelayDetail = "relay_idle";
  let lastRelayStop = null;
  let recovering = null;
  let lastDetail = "encoded_unavailable";
  let appendCount = 0;
  let appendBytes = 0;
  let lastAppendAt = null;
  let lastStop = null;
  let lastTransportFault = null;
  let maxAckMs = 0;

  const safeNotify = (current, detail) => {
    if (current?.mode === "relay") {
      lastRelayDetail = detail;
      try { current.options.onStatus?.(detail); } catch { /* No recording UI state. */ }
      return;
    }
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
    try { return state.buffer.mode === "segments" && Number.isFinite(state.buffer.timestampOffset) && Math.abs(state.buffer.timestampOffset) <= 1e9 &&
      state.buffer.appendWindowStart === 0 && state.buffer.appendWindowEnd === Infinity; }
    catch { return false; }
  };
  const integrity = () => installed && hooks.every(({ target, name, wrapped }) => target[name] === wrapped);
  const sourceForVideo = (video) => {
    try {
      // Decoder readiness can drop during catch-up/rebuffering while the same
      // compressed stream is still arriving. It is NOT source identity.
      if (!video || video.isConnected === false || video.mediaKeys || video.srcObject) return null;
      const source = urls.get(video.currentSrc || video.src);
      return source && source.channelId === channel() ? source : null;
    } catch { return null; }
  };
  const candidate = (video) => {
    if (!video || video.readyState < 2 || video.ended) return null;
    const source = sourceForVideo(video);
    if (!integrity() || !source || source.blocked || source.buffers.length < 1 || source.buffers.length > 2) return null;
    try { if (source.mediaSource.sourceBuffers.length !== source.buffers.length) return null; } catch { return null; }
    if (source.buffers.some((state) => state.blocked || !state.init || state.awaitingInit || state.initParts.length || !configurationOK(state))) return null;
    // MIME is only an early eligibility check. Native init/sample validation,
    // including real A/V tracks and encryption rejection, remains authoritative.
    const declared = source.buffers.map((state) => state.mimeType.toLowerCase()).join(",");
    return /avc[13]\./.test(declared) && /mp4a\.40\.2/.test(declared) ? source : null;
  };
  const selectedVideo = () => {
    const video = document.getElementById("livePlayer");
    return video?.tagName === "VIDEO" && video.isConnected !== false && video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0 && !video.ended ? video : null;
  };
  // Only bounded capability facts. Never expose blob/media URLs, credentials,
  // page text or raw media in diagnostics.
  const diagnostics = () => {
    const video = active?.video ?? selectedVideo();
    const source = video ? sourceForVideo(video) : null;
    const sourcesList = [...observed].slice(0, 8);
    const reason = !installed ? "mse_unavailable" : !integrity() ? "observer_changed" : !video ? "waiting_video" :
      video.mediaKeys ? "encrypted" : video.srcObject ? "source_object_unsupported" : !source ? "source_not_observed" :
      video.readyState < 2 ? "buffering" :
      source.blocked ? source.reason ?? "source_blocked" : !source.buffers.length ? "waiting_tracks" :
      source.buffers.some(state => state.blocked) ? source.buffers.find(state => state.blocked)?.reason ?? "buffer_blocked" :
      source.buffers.some(state => !configurationOK(state)) ? "timeline_unsupported" :
      source.buffers.some(state => !state.init || state.awaitingInit || state.initParts.length) ? "waiting_init" : candidate(video) ? "ready" : "tracks_unsupported";
    return { version: 1, reason, installed, sourceCount: sourcesList.length, appendCount, appendBytes, lastStop, lastTransportFault,
      width: video?.videoWidth ?? 0, height: video?.videoHeight ?? 0,
      queuedBytes: active?.queuedBytes ?? 0,
      maxAckMs, relay: { active: Boolean(relay), queuedBytes: relay?.queuedBytes ?? 0, detail: lastRelayDetail, lastStop: lastRelayStop },
      lastAppendAgoMs: lastAppendAt === null ? null : Math.max(0, Date.now() - lastAppendAt),
      sources: sourcesList.map(item => ({ selected: item === source, blocked: item.blocked, reason: item.reason ?? null,
        tracks: item.buffers.slice(0, 2).map(state => ({ mimeType: state.mimeType.slice(0, 120), blocked: state.blocked,
          reason: state.reason ?? null, initBytes: state.init?.byteLength ?? 0,
          mode: state.buffer.mode === "segments" ? "segments" : "sequence", timestampOffset: Number.isFinite(state.buffer.timestampOffset) ? state.buffer.timestampOffset : null })) })) };
  };
  const status = () => {
    const video = active?.video ?? selectedVideo();
    const ready = Boolean(candidate(video));
    return { active: Boolean(active || recovering), recording: Boolean(active || recovering), starting: Boolean(recovering || active && !active.recordingId),
      stopping: Boolean(active?.stopping), recordingId: active?.recordingId ?? null, channelId: active?.channelId ?? channel() ?? null,
      detail: recovering ? "encoded_starting" : active ? active.stopping ? "encoded_saving" : active.recordingId ? active.source.lastAppendAt !== null && Date.now() - active.source.lastAppendAt > 15_000 ? "encoded_waiting" : "encoded_recording" : "encoded_starting" : ready ? "encoded_ready" : lastDetail,
      ready, captureChat: active?.captureChat === true, captureMode: "encoded", rateControlAllowed: canChangePlaybackRate(video) };
  };
  const canChangePlaybackRate = (video) => Boolean(active && active.video === video && active.nativeApproved && active.accepting && !active.transportFailed && !active.stopping &&
    active.recordingId && candidate(video) === active.source);
  const getReplayClock = (video) => {
    if (!canChangePlaybackRate(video) || video.seeking) return null;
    const videoBuffer = active.source.buffers.find(state => /avc[13]\./i.test(state.mimeType));
    if (!videoBuffer || !configurationOK(videoBuffer)) return null;
    const sourceTimeSeconds = video.currentTime - videoBuffer.buffer.timestampOffset;
    if (!Number.isFinite(sourceTimeSeconds) || sourceTimeSeconds < 0 || sourceTimeSeconds > 1e9) return null;
    // MSE adds timestampOffset to coded PTS. Undo that display-only offset;
    // never modify the recorded samples. Native muxing subtracts DTS and keeps
    // CTS: output PTS = source PTS - origin (not PTS == DTS).
    return { clock: "mse_presentation_v1", sourceId: active.source.sourceId, sourceTimeSeconds };
  };
  // Native probes observe this relay without creating a MediaJob or changing
  // playback. A missing observation is never a synthesized zero timestamp.
  const getRelayClock = (runId) => {
    const current = relay;
    if (!current || current.runId !== runId || !current.nativeApproved || current.closed || current.cancelled) return null;
    const result = { sourceId: current.source.sourceId, sample: null };
    const video = current.video, track = current.source.buffers.find(state => /avc[13]\./i.test(state.mimeType));
    if (candidate(video) !== current.source || !track || !configurationOK(track)) return result;
    const currentTimeSeconds = video.currentTime, timestampOffsetSeconds = track.buffer.timestampOffset, playbackRate = video.playbackRate;
    const sourceTimeSeconds = currentTimeSeconds - timestampOffsetSeconds;
    const performanceNowMs = performance.now();
    if (![sourceTimeSeconds, currentTimeSeconds, timestampOffsetSeconds, performanceNowMs, playbackRate].every(Number.isFinite) ||
        sourceTimeSeconds < 0 || sourceTimeSeconds > 1e9 || currentTimeSeconds < 0 || currentTimeSeconds > 1e9 ||
        playbackRate < .0625 || playbackRate > 16) return result;
    result.sample = { sourceTimeSeconds, performanceNowMs, timestampOffsetSeconds,
      currentTimeSeconds, playbackRate, paused: Boolean(video.paused),
      seeking: Boolean(video.seeking), readyState: video.readyState };
    return result;
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
    if (current.mode === "relay") { interruptRelay(current, reason); return; }
    if (!current.interrupted) lastStop = { reason, readyState:current.video.readyState, paused:Boolean(current.video.paused),
      ended:Boolean(current.video.ended), sourceAttached:sourceForVideo(current.video) === current.source, sourceClosed:current.source.blocked };
    current.interrupted = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    current.accepting = false;
    // A failure raised by a queue operation must not await that same queue.
    void Promise.resolve().then(() => finish(current)).catch(() => undefined);
  };
  const consumers = (source) => [active, relay].filter(current => current?.source === source && current.accepting);
  const failSource = (source, reason) => {
    if (active?.source === source) failure(active, reason);
    if (relay?.source === source) failure(relay, reason);
  };
  const block = (state, reason) => {
    state.blocked = true;
    state.reason = reason;
    failSource(state.source, reason);
  };
  const makeSource = (mediaSource) => {
    let source = sources.get(mediaSource);
    if (source) return source;
    if (observed.size >= 8) return null;
    source = { mediaSource, sourceId: crypto.randomUUID(), channelId: channel(), buffers: [], blocked: false, lastAppendAt: null };
    sources.set(mediaSource, source); observed.add(source);
    mediaSource.addEventListener("sourceclose", () => {
      source.blocked = true; observed.delete(source);
      for (const [url, bound] of urls) if (bound === source) urls.delete(url);
      failSource(source, "source_changed");
    });
    return source;
  };
  const makeBuffer = (source, buffer, mimeType) => {
    if (source.buffers.length >= 2 || !supportedMime(mimeType)) { source.blocked = true; source.reason = "codec_unsupported"; return; }
    const state = { source, buffer, trackIndex: source.buffers.length, mimeType, blocked: false, header: new Uint8Array(8), headerBytes: 0,
      kind: null, remaining: 0, initParts: [], initSize: 0, ftyp: null, init: null, awaitingInit: false,
      forwarding: new Set(), headerConsumers: new Set() };
    source.buffers.push(state); buffers.set(buffer, state);
    buffer.addEventListener("error", () => block(state, "source_buffer_error"));
    failSource(source, "track_changed");
  };
  // Native receipts deduplicate the immediately previous, byte-identical
  // chunk. Keep its indexes/payload unchanged and do not advance the serial
  // queue before ACK. Never retry a parser, ownership or disk error.
  const appendWithRetry = async (current, fields) => {
    const retryStartedAt = Date.now();
    for (let attempt = 0; ; attempt++) {
      const requestedAt = Date.now();
      try { const result = await current.options.request("encoded_append", fields); maxAckMs = Math.max(maxAckMs, Date.now() - requestedAt); return result; }
      catch (error) {
        lastTransportFault = { code: error?.code ?? "BRIDGE_POST_FAILED", attempt, at: Date.now(),
          appendIndex: fields.appendIndex, chunkIndex: fields.chunkIndex, queuedBytes: current.queuedBytes };
        if (attempt >= 32 || Date.now() - retryStartedAt >= 60_000 ||
            !["BRIDGE_BUSY", "BROWSER_ACK_TIMEOUT"].includes(error?.code)) throw error;
        await new Promise(resolve => setTimeout(resolve, Math.min(100 * 2 ** Math.min(attempt, 5), 2000)));
      }
    }
  };
  // Parts are immutable private copies shared by independent consumers. Only
  // bounded serialization scratch is copied again; no caller buffer is retained.
  function* chunksOf(parts, byteLength) {
    let remaining = byteLength, chunk = new Uint8Array(Math.min(CHUNK, remaining)), used = 0;
    for (const part of parts) {
      let offset = 0;
      while (offset < part.bytes.length) {
        const count = Math.min(chunk.length - used, part.bytes.length - offset);
        chunk.set(part.bytes.subarray(offset, offset + count), used); used += count; offset += count;
        if (used === chunk.length) {
          yield chunk; remaining -= chunk.length; used = 0;
          if (remaining) chunk = new Uint8Array(Math.min(CHUNK, remaining));
        }
      }
    }
  }
  const enqueue = (current, state, parts, byteLength) => {
    if (!current.accepting || byteLength === 0) return;
    if (current.mode === "relay") { enqueueRelay(current, state, parts, byteLength); return; }
    if (byteLength > MAX_APPEND || current.queuedBytes + byteLength > MAX_QUEUE) { failure(current, "queue_overflow"); return; }
    current.queuedBytes += byteLength;
    const trackIndex = state.trackIndex;
    const appendIndex = current.appendIndexes[trackIndex]++;
    current.queue = current.queue.then(async () => {
      await current.begin;
      if (current.transportFailed) return;
      let chunkIndex = 0;
      let sent = 0;
      for (const bytes of chunksOf(parts, byteLength)) {
        sent += bytes.length;
        await appendWithRetry(current, { recordingId: current.recordingId, trackIndex,
          appendIndex, chunkIndex: chunkIndex++, finalChunk: sent === byteLength, data: base64(bytes) });
      }
    }).catch((error) => { current.transportFailed = true;
      failure(current, ({ BROWSER_ACK_TIMEOUT:"bridge_ack_timeout", BRIDGE_BUSY:"bridge_busy", BRIDGE_POST_FAILED:"bridge_post_failed" })[error?.code] ?? "native_rejected"); })
      .finally(() => { current.queuedBytes -= byteLength; });
  };
  const observeAppend = (state, value) => {
    appendCount += 1;
    appendBytes += value?.byteLength ?? 0;
    lastAppendAt = Date.now();
    state.source.lastAppendAt = lastAppendAt;
    if (state.blocked) return;
    if (!configurationOK(state)) { block(state, "timeline_changed"); return; }
    const bytes = bytesOf(value);
    if (!bytes || bytes.byteLength > MAX_APPEND) { block(state, "append_unsupported"); return; }
    const recipients = consumers(state.source);
    const deliveries = new Map(recipients.map(current => [current, { parts: [], byteLength: 0 }]));
    const collect = (bytes, owned = false) => {
      const part = { bytes, owned };
      for (const [current, delivery] of deliveries) if (current.accepting && state.forwarding.has(current)) {
        delivery.parts.push(part); delivery.byteLength += bytes.length;
      }
    };
    let at = 0;
    while (at < bytes.length) {
      if (state.remaining === 0) {
        if (state.headerBytes === 0) state.headerConsumers = new Set(recipients);
        const count = Math.min(8 - state.headerBytes, bytes.length - at);
        state.header.set(bytes.subarray(at, at + count), state.headerBytes); state.headerBytes += count; at += count;
        if (state.headerBytes < 8) break;
        const size = new DataView(state.header.buffer).getUint32(0);
        const kind = String.fromCharCode(...state.header.subarray(4));
        if (size < 8 || size > MAX_APPEND || !["ftyp", "moov", "moof", "mdat", "styp", "sidx", "emsg", "prft", "free"].includes(kind)) { block(state, "container_unsupported"); return; }
        state.kind = kind; state.remaining = size - 8; state.headerBytes = 0;
        if (kind === "ftyp" || kind === "moov") {
          state.awaitingInit = true;
          if (size > MAX_INIT || (kind === "moov" && (!state.ftyp || state.ftyp.length + size > MAX_INIT))) { block(state, "init_unsupported"); return; }
          state.initParts = [state.header.slice()]; state.initSize = 8;
        } else {
          if (state.awaitingInit) { block(state, "init_unsupported"); return; }
          if (kind === "moof") state.forwarding = new Set(recipients.filter(current => state.headerConsumers.has(current)));
          collect(state.header.slice(), true);
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
      } else collect(part);
      at += count; state.remaining -= count;
      if (state.remaining === 0 && (state.kind === "ftyp" || state.kind === "moov")) {
        const complete = new Uint8Array(state.initSize); let offset = 0;
        for (const part of state.initParts) { complete.set(part, offset); offset += part.length; }
        state.initParts = []; state.initSize = 0;
        if (state.kind === "ftyp") { state.ftyp = complete; }
        else {
          const next = new Uint8Array(state.ftyp.length + complete.length); next.set(state.ftyp); next.set(complete, state.ftyp.length);
          if (state.init && (state.init.length !== next.length || state.init.some((value, index) => value !== next[index]))) {
            failSource(state.source, "init_changed");
            state.forwarding.clear();
          }
          state.init = next;
          state.awaitingInit = false;
        }
      }
    }
    for (const [current, delivery] of deliveries) {
      const limit = current.mode === "relay" ? MAX_RELAY_QUEUE : MAX_QUEUE;
      if (current.accepting && delivery.byteLength && (delivery.byteLength > MAX_APPEND || current.queuedBytes + delivery.byteLength > limit)) failure(current, "queue_overflow");
    }
    for (const [current, delivery] of deliveries) if (current.accepting && delivery.byteLength) {
      for (const part of delivery.parts) if (!part.owned) { part.bytes = part.bytes.slice(); part.owned = true; }
      enqueue(current, state, delivery.parts, delivery.byteLength);
    }
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
    for (const name of ["changeType"]) if (typeof NativeSourceBuffer.prototype[name] === "function") hook(NativeSourceBuffer.prototype, name, (original) => function (mimeType) {
      const result = Reflect.apply(original, this, arguments);
      const state = buffers.get(this);
      if (state && String(mimeType).toLowerCase().replace(/[\s"]/g, "") !== state.mimeType.toLowerCase().replace(/[\s"]/g, "")) {
        if (!supportedMime(mimeType)) block(state, "codec_unsupported");
        else {
          failSource(state.source, "codec_changed");
          state.mimeType = mimeType; state.init = null; state.ftyp = null; state.awaitingInit = true;
          state.initParts = []; state.initSize = 0; state.headerBytes = 0; state.remaining = 0; state.forwarding.clear(); state.headerConsumers.clear();
        }
      }
      return result;
    });
    if (typeof NativeMediaSource.prototype.removeSourceBuffer === "function") hook(NativeMediaSource.prototype, "removeSourceBuffer", (original) => function removeSourceBuffer(buffer) {
      const result = Reflect.apply(original, this, arguments); const state = buffers.get(buffer);
      if (state) {
        failSource(state.source, "track_changed");
        state.blocked = true; buffers.delete(buffer);
        state.source.buffers = state.source.buffers.filter(item => item !== state);
        state.source.buffers.forEach((item, index) => { item.trackIndex = index; });
      }
      return result;
    });
    installed = true;
  } catch {
    for (const { target, name, descriptor } of hooks.reverse()) { try { Object.defineProperty(target, name, descriptor); } catch { /* Capability remains disabled. */ } }
  }

  const safetyReason = (current) => {
    if (channel() !== current.channelId) return "channel_changed";
    if (!integrity()) return "observer_changed";
    if (sourceForVideo(current.video) !== current.source || current.source.blocked) return "source_changed";
    if (current.source.buffers.some((state) => state.blocked || !configurationOK(state))) return "timeline_changed";
    return null;
  };
  const cleanUp = (current) => {
    clearInterval(current.timer);
    for (const remove of current.listeners) remove();
    for (const state of current.source.buffers) { state.forwarding.delete(current); state.headerConsumers.delete(current); }
    if (active === current) active = null;
    if (relay === current) relay = null;
  };
  const continueRecording = (current, command) => {
    if (!command || !UUID.test(command.requestId ?? "") || command.channelId !== current.channelId || current.cancelContinuation) return false;
    const recovery = { current, command, expires: Date.now() + 15_000, timer: null };
    recovering = recovery;
    safeNotify(current, "encoded_starting");
    recovery.timer = setInterval(() => {
      if (recovering !== recovery) { clearInterval(recovery.timer); return; }
      if (Date.now() >= recovery.expires || channel() !== current.channelId || current.video.ended || current.video.mediaKeys || !integrity()) {
        clearInterval(recovery.timer); recovering = null; safeNotify(current, "encoded_interrupted"); return;
      }
      // Stay on the approved document/player. No reconnect, alternate URL or
      // legacy decoder fallback. A fresh native arm validates the new init.
      if (candidate(current.video) !== current.source) return;
      clearInterval(recovery.timer); recovering = null;
      void start({ ...command, video: current.video }, current.options).catch(() => undefined);
    }, 250);
    return true;
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
          interrupted: current.interrupted || current.transportFailed,
          reason: current.transportFailed && ["track_changed", "codec_changed", "init_changed"].includes(current.reason) ? "native_rejected" : current.reason ?? "user_stop" });
        if (!response || response.stopped !== true) throw new Error("native_rejected");
        cleanUp(current);
        if (current.transportFailed || !continueRecording(current, response.continuation)) {
          // An ended video is provisional until native checks the broadcast.
          // Do not turn that pending classification into a persistent IPC error.
          const ending = current.transportFailed ? null : response.endingReason;
          safeNotify(current, ending === "checking" ? "encoded_end_checking" :
            ["broadcast_ended", "broadcast_changed"].includes(ending) ? "encoded_saved" :
            response.interrupted || current.interrupted || current.transportFailed ? "encoded_interrupted" : "encoded_saved");
        }
        return response;
      } catch (cause) { cleanUp(current); safeNotify(current, "native_rejected"); throw cause; }
    })();
    return current.finishPromise;
  };
  const start = (command, options) => {
    if (active || recovering) return Promise.reject(new Error("recording_active"));
    const video = command?.video ?? selectedVideo(); const source = candidate(video);
    if (!source || command?.channelId !== channel() || command.rightsAcknowledged !== true || !UUID.test(command.requestId ?? "") || typeof options?.request !== "function") {
      return Promise.reject(new Error("encoded_unavailable"));
    }
    const current = { source, video, channelId: channel(), options, recordingId: null, captureChat: false, nativeApproved: false,
      accepting: true, starting: true, stopping: false, interrupted: false, transportFailed: false, reason: null,
      appendIndexes: source.buffers.map(() => 0), queuedBytes: 0, queue: Promise.resolve(), begin: null, finishPromise: null, listeners: [], timer: null };
    active = current;
    const listen = (name, reason) => { const callback = () => failure(current, reason); video.addEventListener(name, callback); current.listeners.push(() => video.removeEventListener(name, callback)); };
    // emptied can describe resetting just the decoder. The periodic identity
    // check still stops an actually replaced URL/MediaSource.
    listen("ended", "video_ended"); listen("encrypted", "encrypted");
    current.timer = setInterval(() => { const reason = safetyReason(current); if (reason && !current.stopping) failure(current, reason); }, 500);
    safeNotify(current, "encoded_starting");
    current.begin = Promise.resolve().then(() => options.request("encoded_begin", { requestId: command.requestId, channelId: current.channelId,
      title: String(document.title ?? "SOOP").slice(0, 200), sourceId: source.sourceId,
      tracks: source.buffers.map((state) => ({ trackIndex: state.trackIndex, mimeType: state.mimeType, init: base64(state.init) })) }))
      .then((response) => {
        if (!response || !recordingID(response.id) || response.mode !== "encoded" || response.nativeApproved !== true) throw new Error("encoded_not_approved");
        current.recordingId = response.id; current.captureChat = response.captureChat === true; current.nativeApproved = true; current.starting = false;
        const reason = safetyReason(current); if (reason && !current.stopping) failure(current, reason);
        safeNotify(current, current.stopping ? "encoded_saving" : "encoded_recording");
        if (!current.stopping) options.onStarted?.(response, video);
        return response;
      }).catch((cause) => {
        current.transportFailed = true; current.accepting = false; cleanUp(current); safeNotify(current, "native_rejected");
        const error = new Error(cause?.message ?? "native_rejected"); error.nativeAttempted = true; error.code = cause?.code;
        // This exact code is issued by native init validation BEFORE consuming
        // the arm or opening a recording. No lost ACK/append/finish failure may
        // fall back: those may already own files or an active native session.
        if (cause?.code === "ENCODED_UNSUPPORTED") { source.blocked = true; source.reason = "native_init_unsupported"; error.allowLegacyFallback = true; }
        throw error;
      });
    return current.begin;
  };
  const discardRelayQueue = (current) => {
    for (const task of current.tasks.splice(0)) current.queuedBytes -= task.byteLength;
  };
  const interruptRelay = (current, reason) => {
    if (current.closed) return;
    if (!current.interrupted) lastRelayStop = { reason };
    current.interrupted = true; current.accepting = false; current.cancelled = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    discardRelayQueue(current);
    current.options.cancelData?.();
    void finishRelay(current).catch(() => undefined);
  };
  const drainRelay = (current) => {
    if (current.processing) return;
    current.processing = true;
    current.queue = Promise.resolve().then(async () => {
      await current.begin;
      while (!current.cancelled && current.tasks.length) {
        const task = current.tasks.shift();
        try {
          let sent = 0, chunkIndex = 0;
          for (const bytes of chunksOf(task.parts, task.byteLength)) {
            if (current.cancelled) break;
            sent += bytes.length;
            const reply = await current.options.request("relay_append", { runId: current.runId, sourceId: current.source.sourceId,
              trackIndex: task.trackIndex, appendIndex: task.appendIndex, chunkIndex: chunkIndex++, finalChunk: sent === task.byteLength, data: base64(bytes) });
            if (reply?.acceptedInMemory !== true) throw new Error("native_rejected");
          }
        } finally { current.queuedBytes -= task.byteLength; }
      }
    }).catch(error => {
      current.transportFailed = true;
      interruptRelay(current, ({ BROWSER_ACK_TIMEOUT: "bridge_ack_timeout", BRIDGE_BUSY: "bridge_busy", BRIDGE_POST_FAILED: "bridge_post_failed" })[error?.code] ?? "native_rejected");
    }).finally(() => {
      current.processing = false;
      // An append can arrive between the last ACK and this promise settling.
      // Adopt its drain so a simultaneous normal finish still waits for it.
      if (!current.cancelled && current.tasks.length) return drainRelay(current);
    });
    return current.queue;
  };
  const enqueueRelay = (current, state, parts, byteLength) => {
    if (!current.accepting) return;
    if (byteLength > MAX_APPEND || current.queuedBytes + byteLength > MAX_RELAY_QUEUE || !Number.isSafeInteger(current.appendIndexes[state.trackIndex])) {
      interruptRelay(current, "queue_overflow"); return;
    }
    current.queuedBytes += byteLength;
    current.tasks.push({ parts, byteLength, trackIndex: state.trackIndex, appendIndex: current.appendIndexes[state.trackIndex]++ });
    drainRelay(current);
  };
  const finishRelay = (current) => {
    if (current.finishPromise) return current.finishPromise;
    current.accepting = false; current.stopping = true; safeNotify(current, "relay_stop_requested");
    current.finishPromise = Promise.resolve().then(async () => {
      if (!current.cancelled) {
        try { await current.begin; await current.queue; } catch { current.interrupted = true; current.reason ||= "native_rejected"; }
      }
      const response = await current.options.request("relay_finish", { runId: current.runId, sourceId: current.source.sourceId,
        interrupted: current.interrupted || current.transportFailed, reason: current.reason ?? "user_stop" });
      if (response?.acceptedInMemory !== true || response.stopRequested !== true) throw new Error("native_rejected");
      safeNotify(current, current.interrupted || response.interrupted ? "relay_interrupted" : "relay_stop_requested");
      return response;
    }).catch(error => { safeNotify(current, "relay_interrupted"); throw error; })
      .finally(() => { current.closed = true; discardRelayQueue(current); cleanUp(current); });
    return current.finishPromise;
  };
  const startRelay = (command, options) => {
    if (relay) return Promise.reject(new Error("relay_active"));
    const video = command?.video ?? selectedVideo(), source = candidate(video);
    if (!source || command?.channelId !== channel() || !UUID.test(command.requestId ?? "") || typeof options?.request !== "function") return Promise.reject(new Error("relay_unavailable"));
    const current = { mode: "relay", source, video, channelId: channel(), options, runId: command.requestId,
      accepting: true, stopping: false, interrupted: false, transportFailed: false, cancelled: false, closed: false, nativeApproved: false,
      reason: null, appendIndexes: source.buffers.map(() => 0), queuedBytes: 0, tasks: [], processing: false,
      queue: Promise.resolve(), begin: null, finishPromise: null, listeners: [], timer: null };
    relay = current;
    const listen = (name, reason) => { const callback = () => interruptRelay(current, reason); video.addEventListener(name, callback); current.listeners.push(() => video.removeEventListener(name, callback)); };
    listen("ended", "video_ended"); listen("encrypted", "encrypted");
    current.timer = setInterval(() => { const reason = safetyReason(current); if (reason && !current.stopping) interruptRelay(current, reason); }, 500);
    safeNotify(current, "relay_starting");
    current.begin = Promise.resolve().then(() => {
      if (current.cancelled) throw new Error("relay_cancelled");
      return options.request("relay_begin", { requestId: command.requestId, channelId: current.channelId, sourceId: source.sourceId,
        tracks: source.buffers.map(state => ({ trackIndex: state.trackIndex, mimeType: state.mimeType, init: base64(state.init) })) });
    }).then(response => {
      if (response?.runId !== current.runId || response.mode !== "relay" || response.nativeApproved !== true || response.acceptedInMemory !== true || response.captureChat !== false) throw new Error("relay_not_approved");
      if (current.closed || current.cancelled) return response;
      current.nativeApproved = true;
      const reason = safetyReason(current); if (reason) interruptRelay(current, reason);
      if (!current.stopping) safeNotify(current, "relay_accepting");
      return response;
    }).catch(error => {
      current.transportFailed = true;
      interruptRelay(current, error?.code === "BROWSER_ACK_TIMEOUT" ? "begin_ack_timeout" : "native_rejected");
      throw error;
    });
    return current.begin;
  };
  window.addEventListener("pagehide", () => { if (recovering) { clearInterval(recovering.timer); recovering = null; } if (active) { active.cancelContinuation = true; failure(active, "page_hidden"); } if (relay) interruptRelay(relay, "page_hidden"); });
  Object.defineProperty(window, "__atsumiSoopEncodedCapture", { value: Object.freeze({
    canStart: (video) => Boolean(candidate(video)), supports: (video) => Boolean(candidate(video)), getDiagnostics: diagnostics,
    canChangePlaybackRate, getReplayClock, getRelayClock, getStatus: status, start,
    startRelay,
    getRelayStatus: () => ({ active: Boolean(relay), runId: relay?.runId ?? null, accepting: relay?.accepting === true,
      queuedBytes: relay?.queuedBytes ?? 0, nativeApproved: relay?.nativeApproved === true, detail: lastRelayDetail }),
    stopRelay(runId, reason = "user_stop", interrupted = false) {
      if (!relay || relay.runId !== runId) return Promise.reject(new Error("relay_not_active"));
      if (interrupted) interruptRelay(relay, reason);
      else relay.reason ||= reason;
      return finishRelay(relay);
    },
    stop(reason = "user_stop", interrupted = false) {
      if (recovering) { clearInterval(recovering.timer); recovering = null; }
      if (!active) return Promise.resolve({ stopped: true, interrupted: false });
      active.cancelContinuation = true;
      if (!active.interrupted) active.reason = reason;
      active.interrupted ||= interrupted; return finish(active);
    },
  }) });
})();
