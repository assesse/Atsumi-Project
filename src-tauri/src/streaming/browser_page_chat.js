// SPDX-License-Identifier: MIT
// Observe the official page's receive-only chat delivery. Never open a socket,
// inspect send/authentication payloads, read cookies or export raw profiles.
(() => {
  "use strict";
  const channel = () => window.location.origin === "https://chzzk.naver.com" ?
    /^\/live\/([a-f0-9]{32})\/?$/i.exec(window.location.pathname)?.[1].toLowerCase() ?? null : null;
  if (!channel() || window.top !== window || window.__atsumiPageChat) return;
  const MAX_FRAME = 256 * 1024;
  const MAX_EVENT = 32 * 1024;
  const MAX_BATCH = 128 * 1024;
  const MAX_QUEUE = 2 * 1024 * 1024;
  const MAX_BATCH_EVENTS = 32;
  const VIEWER_INTERVAL = 10000;
  const VIEWER_TIMEOUT = 8000;
  const VIEWER_RESPONSE_LIMIT = 32 * 1024;
  const VIEWER_SOURCE = "chzzk_live_status_api_v1";
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const sockets = new Set();
  const socketChannels = new WeakMap();
  let active = null;
  let generation = 0;
  let installed = false;
  let observerOverflow = false;
  const bytes = (value) => encoder.encode(JSON.stringify(value)).byteLength;
  const text = (value, limit) => typeof value === "string" ? Array.from(value).slice(0, limit).join("") : undefined;
  const object = (value) => {
    try {
      if (typeof value === "string") {
        if (encoder.encode(value).byteLength > 32 * 1024) return null;
        value = JSON.parse(value);
      }
      return value && typeof value === "object" && !Array.isArray(value) ? value : null;
    } catch { return null; }
  };
  const image = (value) => {
    if (typeof value !== "string" || value.length > 2048) return undefined;
    try {
      const url = new URL(value);
      if (url.protocol !== "https:" || url.username || url.password || url.port ||
          !(url.hostname === "pstatic.net" || url.hostname.endsWith(".pstatic.net"))) return undefined;
      // Display assets do not need query/fragment values (which could contain
      // credentials in an unexpected upstream field).
      return url.origin + url.pathname;
    } catch { return undefined; }
  };
  const badge = (source) => {
    const imageUrl = image(source?.imageUrl);
    if (!imageUrl) return undefined;
    return { imageUrl, title: text(source.title ?? source.name, 128), badgeId: text(source.badgeId ?? source.badge_id, 128) };
  };
  const color = (value) => typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value) ? value : undefined;
  const senderKey = async (current, message) => {
    // Only an opaque ID actually delivered with this ordinary message qualifies.
    // The random salt lives for this recording (including socket reconnects).
    const id = object(message.profile)?.userIdHash;
    if (!current.salt || typeof id !== "string" || !/^[a-z0-9_-]{1,128}$/i.test(id)) return undefined;
    try {
      const input = encoder.encode(id), bytes = new Uint8Array(current.salt.length + input.length);
      bytes.set(current.salt); bytes.set(input, current.salt.length);
      let deadline;
      const result = await Promise.race([crypto.subtle.digest("SHA-256", bytes), new Promise(resolve => { deadline = setTimeout(() => resolve(null), 1500); })]).finally(() => clearTimeout(deadline));
      // Disable the optional digest for this recording after one failure, rather
      // than waiting another 1.5s for EVERY subsequent sender in a busy chat.
      if (!result) { current.salt = null; return undefined; }
      const digest = new Uint8Array(result);
      return "sha256:" + Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
    } catch { current.salt = null; return undefined; }
  };
  const observeClock = (current) => {
    try {
      const receivedAtMs = Date.now(), observedMonotonicMs = performance.now();
      if (!Number.isSafeInteger(receivedAtMs) || receivedAtMs < 0 || !Number.isFinite(observedMonotonicMs) || observedMonotonicMs < 0) return undefined;
      const video = current.getVideo?.() ?? null;
      // Source URLs are compared in memory only; no URL is exported or fetched.
      const identity = video?.currentSrc || video?.src || "";
      if (current.clockVideo !== video || current.sourceIdentity !== identity) {
        for (const remove of current.clockListeners) remove();
        current.clockListeners = []; current.clockVideo = video; current.sourceIdentity = identity;
        current.sourceGeneration++;
        if (video?.addEventListener) for (const name of ["seeking", "emptied", "loadedmetadata"]) {
          const invalidate = () => { current.sourceGeneration++; };
          video.addEventListener(name, invalidate);
          current.clockListeners.push(() => video.removeEventListener(name, invalidate));
        }
      }
      const clock = { version: 1, receivedAtMs, observedMonotonicMs, sourceGeneration: current.sourceGeneration,
        clock: "player_observation" };
      if (video && video.isConnected !== false && !video.seeking && !video.ended && video.readyState >= 2 &&
          Number.isFinite(video.currentTime) && video.currentTime >= 0 && video.currentTime <= 1e9) {
        clock.mediaTimeSeconds = video.currentTime;
        if (Number.isFinite(video.playbackRate) && video.playbackRate >= .25 && video.playbackRate <= 4) clock.playbackRate = video.playbackRate;
        const source = window.__atsumiEncodedCapture?.getReplayClock?.(video);
        if (source?.clock === "mse_presentation_v1" && Number.isFinite(source.sourceTimeSeconds) && source.sourceTimeSeconds >= 0 && source.sourceTimeSeconds <= 1e9 &&
            /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(source.sourceId ?? "")) {
          // Use the approved observer's one presentation sample for both fields;
          // multiple currentTime reads need not return bit-identical fractions.
          clock.clock = source.clock; clock.sourceId = source.sourceId;
          clock.mediaTimeSeconds = clock.sourceTimeSeconds = source.sourceTimeSeconds;
        }
      }
      return clock;
    } catch { return undefined; }
  };
  const normalize = (message) => {
    if (!message || typeof message !== "object" || Array.isArray(message) ||
        (message.msgStatusType ?? message.messageStatusType) === "HIDDEN" ||
        (message.msgTypeCode ?? message.messageTypeCode) !== 1) return null;
    const rawText = message.msg ?? message.content;
    if (typeof rawText !== "string") return null;
    const msg = text(rawText, 4096);
    const supplied = object(message.profile) ?? {};
    const property = object(supplied.streamingProperty) ?? {};
    const profile = { nickname: text(supplied.nickname, 128) ?? "알 수 없음" };
    // Official public profile-card and ranking links use /${userIdHash}. Store
    // only this canonical public destination, never the raw profile/token.
    if (typeof supplied.userIdHash === "string" && /^[a-f0-9]{32}$/i.test(supplied.userIdHash))
      profile.publicProfileUrl = `https://chzzk.naver.com/${supplied.userIdHash.toLowerCase()}`;
    // Official defaults add UTF-16 character codes of userIdHash and the real
    // chatChannelId, modulo the 40-color palette. Export only the 0..39 color
    // seed; native combines it with already-fetched, memory-only metadata.
    if (typeof supplied.userIdHash === "string" && /^[a-z0-9_-]{1,128}$/i.test(supplied.userIdHash))
      profile.nicknameColorSeed = Array.from(supplied.userIdHash).reduce((sum, character) => sum + character.charCodeAt(0), 0) % 40;
    const title = { name: text(supplied.title?.name, 128), color: color(supplied.title?.color) };
    if (title.name || title.color) profile.title = title;
    const profileBadge = badge(supplied.badge);
    if (profileBadge) profile.badge = profileBadge;
    const display = {};
    const palette = "#EEA05D.#EAA35F.#E98158.#E97F58.#E76D53.#E66D5F.#E16490.#E481AE.#E481AE.#D25FAC.#D263AE.#D66CB4.#D071B6.#AF71B5.#A96BB2.#905FAA.#B38BC2.#9D78B8.#8D7AB8.#7F68AE.#9F99C8.#717DC6.#7E8BC2.#5A90C0.#628DCC.#81A1CA.#ADD2DE.#83C5D6.#8BC8CB.#91CBC6.#83C3BB.#7DBFB2.#AAD6C2.#84C194.#92C896.#94C994.#9FCE8E.#A6D293.#ABD373.#BFDE73".split(".");
    const code = property.nicknameColor?.colorCode;
    // Public CD001..CD040 palette, dark replay presentation. Unknown symbolic
    // colors are not guessed; native resolves defaults only with a chat ID.
    const paletteIndex = typeof code === "string" && /^CD0[0-4][0-9]$/.test(code) ? Number(code.slice(2)) - 1 : -1;
    const nicknameColor = color(code) ?? (paletteIndex >= 0 && paletteIndex < palette.length ? palette[paletteIndex] : undefined);
    if (nicknameColor) display.nicknameColor = { colorCode: nicknameColor };
    const subscription = badge(property.subscription?.badge);
    if (subscription) {
      display.subscription = { badge: subscription };
      const months = property.subscription.accumulativeMonth;
      if (Number.isSafeInteger(months) && months >= 0 && months <= 1200) display.subscription.accumulativeMonth = months;
    }
    const donation = badge(property.realTimeDonationRanking?.badge);
    if (donation) display.realTimeDonationRanking = { badge: donation };
    if (Object.keys(display).length) profile.streamingProperty = display;
    const activities = Array.isArray(supplied.activityBadges) ? supplied.activityBadges : [];
    profile.activityBadges = activities.slice(0, 32).filter((item) => item?.activated === true)
      .map((item) => { const value = badge(item); return value ? { ...value, activated: true } : null; }).filter(Boolean);
    const viewers = Array.isArray(supplied.viewerBadges) ? supplied.viewerBadges : [];
    profile.viewerBadges = viewers.slice(0, 16).map((item) => {
      const value = badge(item?.badge);
      return value ? { type: text(item.type, 128), badge: value } : null;
    }).filter(Boolean);
    const emojis = {};
    const suppliedEmojis = object(object(message.extras)?.emojis);
    if (suppliedEmojis) {
      for (const [id, source] of Object.entries(suppliedEmojis)) {
        if (Object.keys(emojis).length >= 32) break;
        if (!/^[a-z0-9_-]{1,64}$/i.test(id) || !msg.includes(`{:${id}:}`)) continue;
        const url = image(source);
        if (url) emojis[id] = url;
      }
    }
    const normalized = { msgTypeCode: 1, msg, profile, extras: { emojis } };
    const timestamp = message.msgTime ?? message.messageTime;
    const time = typeof timestamp === "string" && /^\d{1,16}$/.test(timestamp) ? Number(timestamp) : timestamp;
    if (Number.isSafeInteger(time) && time >= 0) normalized.msgTime = time;
    return { event: normalized, truncated: msg !== rawText };
  };
  const notify = (current, detail, force = false) => {
    current.detail = detail;
    const key = `${detail}:${current.dropped}`;
    if (current.noticeKey === key && !force) return;
    current.noticeKey = key;
    try { current.onStatus(detail, current.dropped); } catch { /* Status cannot affect the official socket. */ }
  };
  const fail = (current, detail, dropped = 0) => {
    if (current.failed) return;
    current.failed = true;
    current.accepting = false;
    stopViewers(current);
    current.dropped += dropped + current.queue.length;
    current.queuedBytes -= current.queue.reduce((sum, entry) => sum + entry.bytes, 0);
    current.queue = [];
    notify(current, detail);
  };
  const pump = (current) => {
    if (current.inFlight || current.failed || !current.queue.length) return;
    const batch = [];
    let batchBytes = 2;
    while (current.queue.length && batch.length < MAX_BATCH_EVENTS) {
      const item = current.queue[0];
      if (batch.length && batchBytes + item.bytes + 1 > MAX_BATCH) break;
      current.queue.shift();
      batch.push(item);
      batchBytes += item.bytes + (batch.length > 1 ? 1 : 0);
    }
    const reserved = batch.reduce((sum, item) => sum + item.bytes, 0);
    const batchId = ++current.batchId;
    current.inFlight = (async () => {
      try {
        const events = batch.map((item) => item.event);
        // Final UTF-8 check, including serialized decoration fields and commas.
        if (bytes(events) > MAX_BATCH) throw new Error("batch_limit");
        // Native deduplicates the same batch after a lost acknowledgement.
        const retryStartedAt = Date.now();
        for (let attempt = 0; ; attempt++) {
          try { await current.sendBatch(events, batchId); break; }
          catch (error) {
            if (attempt >= 32 || Date.now() - retryStartedAt >= 60_000 ||
                !["BRIDGE_BUSY", "BROWSER_ACK_TIMEOUT"].includes(error?.code)) throw error;
            await new Promise(resolve => setTimeout(resolve, Math.min(100 * 2 ** Math.min(attempt, 5), 2000)));
          }
        }
        current.lastSavedAt = Date.now();
      } catch {
        current.dropped += batch.length;
        fail(current, "storage_failed");
      } finally { current.queuedBytes -= reserved; }
    })().finally(() => {
      current.inFlight = null;
      if (!current.accepting || current.queue.length >= MAX_BATCH_EVENTS) pump(current);
    });
  };
  const receive = (socket, event) => {
    const current = active;
    if (!current?.accepting || current.channelId !== channel() || socketChannels.get(socket) !== current.channelId) return;
    // Capture synchronously at socket delivery, before Blob decode, JS queue,
    // digest, batching or IPC can change the observed frame/time.
    const replayClock = observeClock(current);
    const data = event.data;
    const length = typeof data === "string" ? encoder.encode(data).byteLength :
      data instanceof ArrayBuffer ? data.byteLength : data instanceof Blob ? data.size : -1;
    if (length < 0) { current.gap = true; notify(current, "unsupported_frame"); return; }
    if (length > MAX_FRAME || current.rawBytes + current.queuedBytes + length > MAX_QUEUE) {
      fail(current, length > MAX_FRAME ? "frame_too_large" : "queue_overflow", 1);
      return;
    }
    current.rawBytes += length;
    current.decodeQueue = current.decodeQueue.then(async () => {
      try {
        if (current.failed) return;
        let deadline;
        const raw = typeof data === "string" ? data : data instanceof Blob ? await Promise.race([data.text(), new Promise((_, reject) => { deadline = setTimeout(() => reject(new Error("decode_timeout")), 2000); })]).finally(() => clearTimeout(deadline)) : decoder.decode(data);
        if (encoder.encode(raw).byteLength > MAX_FRAME) { fail(current, "frame_too_large", 1); return; }
        let document;
        try { document = JSON.parse(raw); } catch { current.gap = true; notify(current, "invalid_frame"); return; }
        // Authentication, heartbeats, donations and history envelopes are never
        // exported. This observes only ordinary live messages already received.
        if (document?.cmd !== 93101) return;
        const body = document.bdy;
        const messages = Array.isArray(body) ? body : body?.messageList;
        if (!Array.isArray(messages)) { current.gap = true; notify(current, "invalid_frame"); return; }
        for (const message of messages) {
          const normalized = normalize(message);
          if (!normalized) continue;
          if (replayClock) normalized.event.replayClock = replayClock;
          const key = await senderKey(current, message);
          if (current.failed) break;
          if (key) normalized.event.senderKey = key;
          const size = bytes(normalized.event);
          if (size > MAX_EVENT || current.queuedBytes + current.rawBytes + size > MAX_QUEUE) {
            fail(current, size > MAX_EVENT ? "message_too_large" : "queue_overflow", 1);
            break;
          }
          if (normalized.truncated) { current.gap = true; notify(current, "message_truncated"); }
          current.queue.push({ event: normalized.event, bytes: size });
          current.queuedBytes += size;
        }
        if (!current.failed && !current.gap && current.detail !== "receiving") notify(current, "receiving");
        if (current.queue.length >= MAX_BATCH_EVENTS) pump(current);
      } catch { current.gap = true; current.dropped++; notify(current, "decode_failed"); }
      finally { current.rawBytes -= length; }
    });
  };
  const stopViewers = (current) => {
    clearInterval(current.viewerTimer);
    current.viewerRequest?.abort();
  };
  const queueViewer = (current, viewerCount, replayClock, broadcastStartedAt = null) => {
    const event = { atsumiViewerSample: 1, viewerSource: VIEWER_SOURCE,
      viewerChannelId: current.channelId, viewerBroadcastStartedAt: broadcastStartedAt,
      viewerCount, replayClock };
    // Timestamp on response/timeout, not request start. This preserves receipt
    // order with chat while the optional HTTP request is in flight.
    current.decodeQueue = current.decodeQueue.then(() => {
      if (current.failed) return;
      const size = bytes(event);
      if (current.queuedBytes + current.rawBytes + size > MAX_QUEUE) return;
      current.queue.push({ event, bytes: size }); current.queuedBytes += size;
    });
  };
  const viewerDocument = async (response) => {
    if (!response.ok || response.status !== 200 || response.redirected ||
        !/^application\/json(?:;|$)/i.test(response.headers.get("content-type") ?? "") ||
        Number(response.headers.get("content-length") ?? 0) > VIEWER_RESPONSE_LIMIT || !response.body?.getReader)
      throw new Error("viewer_response");
    const reader = response.body.getReader();
    let size = 0; const chunks = [];
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > VIEWER_RESPONSE_LIMIT) throw new Error("viewer_response_limit");
        chunks.push(value);
      }
      const data = new Uint8Array(size); let at = 0;
      for (const chunk of chunks) { data.set(chunk, at); at += chunk.byteLength; }
      return JSON.parse(decoder.decode(data));
    } finally {
      // Cancel also bounds an oversized/chunked body; nothing from the full
      // response (including routing identifiers or tokens) is logged/exported.
      try { await reader.cancel(); } catch { /* Already aborted. */ }
      reader.releaseLock();
    }
  };
  const sampleViewers = (current) => {
    if (!current.accepting || current.failed || !current.viewerFetch) return;
    if (current.channelId !== channel()) { stopViewers(current); return; }
    if (current.viewerRequest) return;
    const before = observeClock(current);
    if (!before) return;
    const controller = new AbortController();
    current.viewerRequest = controller;
    let timeout;
    // Official np() / jo: /polling/v3.1/.../live-status. The page currently
    // refreshes at 30s; re-reading DOM every 2s repeats that old snapshot.
    // Request only this public metadata at 10s while recording, never media,
    // recommendation/ad APIs, cookies, auth headers or another chat socket.
    const task = Promise.resolve().then(() => {
      if (controller.signal.aborted || !current.accepting || active !== current || current.channelId !== channel()) throw new Error("viewer_stopped");
      return current.viewerFetch(`https://api.chzzk.naver.com/polling/v3.1/channels/${current.channelId}/live-status`, {
        method: "GET", credentials: "omit", cache: "no-store", redirect: "error",
        referrerPolicy: "no-referrer", signal: controller.signal,
      });
    }).then(viewerDocument);
    const deadline = new Promise((_, reject) => {
      timeout = setTimeout(() => { controller.abort(); reject(new Error("viewer_timeout")); }, VIEWER_TIMEOUT);
      controller.signal.addEventListener("abort", () => reject(new Error("viewer_aborted")), { once: true });
    });
    Promise.race([task, deadline]).then((document) => {
      if (!current.accepting || current.failed || active !== current || current.channelId !== channel()) return;
      const clock = observeClock(current);
      if (!clock) return;
      const content = document?.code === 200 ? object(document.content) : null;
      const date = content?.openDate;
      const opened = typeof date === "string" && /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(date) ?
        Date.parse(date.replace(" ", "T") + "+09:00") : NaN;
      let valid = clock.sourceGeneration === before.sourceGeneration &&
        content?.channelId === current.channelId && content.status === "OPEN" &&
        Number.isSafeInteger(opened) && opened > 0 && opened <= clock.receivedAtMs;
      if (valid && current.viewerBroadcast === null) current.viewerBroadcast = opened;
      if (valid && current.viewerBroadcast !== opened) {
        valid = false;
        // A new live on the same channel belongs to a new recording session.
        stopViewers(current);
      }
      const count = content?.concurrentUserCount;
      const fresh = valid && content.cvExposure === true && Number.isSafeInteger(count) && count >= 0 && count <= 100000000;
      queueViewer(current, fresh ? count : null, clock, valid ? opened : null);
    }).catch(() => {
      if (!current.accepting || current.failed || active !== current || current.channelId !== channel()) return;
      const clock = observeClock(current);
      if (clock) queueViewer(current, null, clock);
    }).finally(() => {
      clearTimeout(timeout);
      if (current.viewerRequest === controller) current.viewerRequest = null;
    });
  };
  try {
    const NativeSocket = window.WebSocket;
    if (typeof NativeSocket === "function") {
      const WrappedSocket = new Proxy(NativeSocket, {
        construct(target, args, newTarget) {
          const socket = Reflect.construct(target, args, newTarget);
          try {
            const url = new URL(socket.url);
            if (url.protocol !== "wss:" || !/^kr-ss[1-9]\.chat\.naver\.com$/.test(url.hostname) ||
                url.port || url.username || url.password || url.pathname !== "/chat") return socket;
            if (sockets.size >= 8) {
              observerOverflow = true;
              if (active) { active.gap = true; notify(active, "observer_overflow"); }
              return socket;
            }
            sockets.add(socket);
            socketChannels.set(socket, channel());
            socket.addEventListener("message", (event) => receive(socket, event));
            socket.addEventListener("close", () => {
              sockets.delete(socket);
              if (active?.accepting && socketChannels.get(socket) === active.channelId) {
                active.gap = true; notify(active, "connection_gap");
              }
            });
          } catch { /* Leave the page's constructor and socket behavior untouched. */ }
          return socket;
        },
      });
      window.WebSocket = WrappedSocket;
      installed = window.WebSocket === WrappedSocket;
    }
  } catch { /* Unsupported environments retain the native page unchanged. */ }

  Object.defineProperty(window, "__atsumiPageChat", { value: Object.freeze({
    start(options) {
      if (active || !options || options.channelId !== channel() ||
          !/^(?:[a-f0-9]{32}|[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12})$/i.test(options.recordingId ?? "") ||
          typeof options.sendBatch !== "function" || typeof options.onStatus !== "function") return false;
      if (!installed) { options.onStatus("observer_unavailable", 0); return false; }
      let salt = null;
      try { if (crypto?.subtle && typeof crypto.getRandomValues === "function") salt = crypto.getRandomValues(new Uint8Array(32)); } catch { /* Stable-user metadata remains unknown. */ }
      const current = { generation: ++generation, channelId: options.channelId, recordingId: options.recordingId,
        salt, getVideo: typeof options.getVideo === "function" ? options.getVideo : null,
        sourceGeneration: 1, clockVideo: null, sourceIdentity: "", clockListeners: [],
        viewerFetch: typeof window.fetch === "function" && typeof AbortController === "function" ? window.fetch.bind(window) : null,
        viewerRequest: null, viewerBroadcast: null, viewerTimer: null,
        sendBatch: options.sendBatch, onStatus: options.onStatus, accepting: true, failed: false,
        batchId: 0, lastSavedAt: Date.now(), lastPulseAt: Date.now(), lastViewerAt: Date.now(),
        gap: observerOverflow, dropped: 0, detail: "waiting_socket", queue: [], queuedBytes: 0,
        noticeKey: null,
        rawBytes: 0, decodeQueue: Promise.resolve(), inFlight: null, stopping: null, timer: null };
      active = current;
      current.timer = setInterval(() => pump(current), 250);
      if (current.viewerFetch) current.viewerTimer = setInterval(() => sampleViewers(current), VIEWER_INTERVAL);
      sampleViewers(current);
      const connected = [...sockets].some((socket) => socketChannels.get(socket) === current.channelId);
      notify(current, observerOverflow ? "observer_overflow" : connected ? "observing" : "waiting_socket");
      return true;
    },
    pulse() {
      const current = active;
      if (!current?.accepting) return;
      pump(current);
      // Also driven by received video/status events, so presentation/background
      // timer throttling cannot silently stop the statistics pump.
      const now = Date.now();
      if (now - current.lastViewerAt >= VIEWER_INTERVAL) { current.lastViewerAt = now; sampleViewers(current); }
      if (now - current.lastPulseAt >= 15000) {
        current.lastPulseAt = now;
        if (now - current.lastSavedAt > 45000 && current.queuedBytes > 0) { current.gap = true; notify(current, "partial", true); }
        else notify(current, current.detail, true);
      }
    },
    summary() { return active ? { recordingId: active.recordingId, detail: active.failed || active.gap ? "partial" : "stopped" } : null; },
    stop(recordingId) {
      const current = active;
      if (!current || current.recordingId !== recordingId) return Promise.resolve();
      if (current.stopping) return current.stopping;
      current.accepting = false;
      clearInterval(current.timer);
      stopViewers(current);
      current.stopping = (async () => {
        await current.decodeQueue;
        while (current.inFlight || (!current.failed && current.queue.length)) {
          pump(current);
          if (current.inFlight) await current.inFlight;
        }
        if (!current.failed) notify(current, current.gap ? "partial" : "stopped");
        for (const remove of current.clockListeners) remove();
        current.salt?.fill(0); current.salt = null;
        if (active === current && generation === current.generation) active = null;
      })();
      return current.stopping;
    },
  }) });
})();
