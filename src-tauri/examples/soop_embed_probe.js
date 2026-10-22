/* Diagnostic only: no media request, account access, playback control or capture. */
(() => {
  const hostOrigin = "http://tauri.localhost";
  const playerOrigin = "https://play.sooplive.com";
  const containers = new Set(["ftyp", "moov", "moof", "mdat", "styp", "sidx", "emsg", "prft", "webm", "unknown"]);
  if (location.origin === hostOrigin && location.pathname === "/soop-probe.html" && window.top === window) {
    window.soopProbe = { frameLoaded: false, samples: 0, frame: null };
    addEventListener("message", (event) => {
      const iframe = document.getElementById("player");
      if (event.origin !== playerOrigin || event.source !== iframe?.contentWindow || event.data?.type !== "atsumi-soop-probe") return;
      // The official child is untrusted. Copy a fixed structural schema rather
      // than logging arbitrary page strings, URLs, account data or chat text.
      const state = event.data.state;
      if (!state || typeof state !== "object") return;
      const count = (value) => Number.isFinite(value) && value >= 0 ? Math.min(value, Number.MAX_SAFE_INTEGER) : 0;
      const rect = (value) => ({ visible: value?.visible === true, width: count(value?.width), height: count(value?.height) });
      const tracks = (value) => Array.isArray(value) ? value.slice(0, 4).map((track) => ({
        mime: mime(track?.mime), appends: count(track?.appends), bytes: count(track?.bytes), changes: count(track?.changes),
        containers: Array.isArray(track?.containers) ? track.containers.filter((kind) => containers.has(kind)).slice(0, 10) : [],
      })) : [];
      window.soopProbe.samples = Math.min(window.soopProbe.samples + 1, 160);
      window.soopProbe.frame = {
        ready: ["loading", "interactive", "complete"].includes(state.ready) ? state.ready : "unknown",
        videos: Array.isArray(state.videos) ? state.videos.slice(0, 8).map((video) => ({
          id: video?.id === "livePlayer" ? "livePlayer" : "other", ad: video?.ad === true,
          width: count(video?.width), height: count(video?.height), time: count(video?.time),
          ready: count(video?.ready), paused: video?.paused === true, ended: video?.ended === true,
          error: [1, 2, 3, 4].includes(video?.error) ? video.error : null,
          paintWidth: count(video?.paintWidth), paintHeight: count(video?.paintHeight), decoded: count(video?.decoded),
          encrypted: video?.encrypted === true, srcObject: video?.srcObject === true,
          sourceBound: video?.sourceBound === true, tracks: tracks(video?.tracks),
        })) : [],
        mse: { available: state.mse?.available === true, hooked: state.mse?.hooked === true,
          sources: count(state.mse?.sources), buffers: count(state.mse?.buffers), overflow: state.mse?.overflow === true },
        chat: { ...rect(state.chat), inputPresent: state.chat?.inputPresent === true, inputEnabled: state.chat?.inputEnabled === true },
        forcedPlay: false, agentAbsenceVerified: false,
      };
    });
    addEventListener("DOMContentLoaded", () => document.getElementById("player")?.addEventListener("load", () => { window.soopProbe.frameLoaded = true; }));
    return;
  }

  if (location.origin !== playerOrigin || !/^\/[a-zA-Z0-9_-]{1,40}\/embed$/.test(location.pathname)
      || window.parent === window || window.parent !== window.top) return;

  const nativeMse = window.MediaSource;
  const nativeBuffer = window.SourceBuffer;
  const nativeUrl = window.URL;
  const sources = new WeakMap();
  const buffers = new WeakMap();
  const urls = new Map();
  const observed = [];
  const hooks = [];
  let bufferCount = 0;
  let overflow = false;
  let hooked = false;
  const track = (source) => {
    if (sources.has(source)) return sources.get(source);
    if (observed.length >= 8) { overflow = true; return null; }
    const result = { tracks: [] };
    sources.set(source, result); observed.push(result);
    return result;
  };
  const hook = (target, name, wrap) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    if (typeof descriptor?.value !== "function") throw new Error("unavailable");
    const value = wrap(descriptor.value);
    Object.defineProperty(target, name, { ...descriptor, value });
    hooks.push({ target, name, descriptor, value });
  };
  const restore = () => {
    for (const { target, name, descriptor, value } of hooks.reverse()) {
      if (Object.getOwnPropertyDescriptor(target, name)?.value === value) Object.defineProperty(target, name, descriptor);
    }
    hooks.length = 0;
  };
  try {
    if (!nativeMse || !nativeBuffer || !nativeUrl) throw new Error("unavailable");
    hook(nativeUrl, "createObjectURL", (original) => function createObjectURL(value) {
      const result = Reflect.apply(original, this, arguments);
      try {
        if (value instanceof nativeMse) {
          const source = track(value);
          if (source && urls.size < 16) urls.set(result, source); else overflow = true;
        }
      } catch { /* Do not change original playback. */ }
      return result;
    });
    hook(nativeMse.prototype, "addSourceBuffer", (original) => function addSourceBuffer(type) {
      const result = Reflect.apply(original, this, arguments);
      try {
        const source = track(this);
        if (source && source.tracks.length < 4 && bufferCount < 16) {
          const entry = { mime: mime(type), appends: 0, bytes: 0, changes: 0, containers: new Set() };
          source.tracks.push(entry); buffers.set(result, entry); bufferCount += 1;
        } else overflow = true;
      } catch { /* Do not change original playback. */ }
      return result;
    });
    if (typeof nativeBuffer.prototype.changeType === "function") hook(nativeBuffer.prototype, "changeType", (original) => function changeType(type) {
      const result = Reflect.apply(original, this, arguments);
      try { const entry = buffers.get(this); if (entry) { entry.mime = mime(type); entry.changes = Math.min(entry.changes + 1, Number.MAX_SAFE_INTEGER); } }
      catch { /* Preserve the original codec change. */ }
      return result;
    });
    hook(nativeBuffer.prototype, "appendBuffer", (original) => function appendBuffer(value) {
      const result = Reflect.apply(original, this, arguments);
      try {
        const entry = buffers.get(this);
        if (entry) {
          entry.appends = Math.min(entry.appends + 1, Number.MAX_SAFE_INTEGER);
          entry.bytes = Math.min(entry.bytes + (value?.byteLength ?? 0), Number.MAX_SAFE_INTEGER);
          const bytes = ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, Math.min(value.byteLength, 8))
            : value instanceof ArrayBuffer ? new Uint8Array(value, 0, Math.min(value.byteLength, 8)) : null;
          if (bytes?.length >= 8) {
            const kind = String.fromCharCode(bytes[4], bytes[5], bytes[6], bytes[7]);
            if (containers.has(kind)) entry.containers.add(kind);
            else if (bytes[0] === 0x1a && bytes[1] === 0x45 && bytes[2] === 0xdf && bytes[3] === 0xa3) entry.containers.add("webm");
            else entry.containers.add("unknown");
          }
        }
      } catch { /* Only bounded counters are diagnostic; never reject an append. */ }
      return result;
    });
    hooked = true;
  } catch { restore(); }

  const visibility = (element) => {
    if (!element) return { visible: false, width: 0, height: 0 };
    const bounds = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return { visible: bounds.width > 0 && bounds.height > 0 && style.display !== "none" && style.visibility !== "hidden",
      width: Math.round(bounds.width), height: Math.round(bounds.height) };
  };
  let samples = 0;
  const timer = setInterval(() => {
    const videos = Array.from(document.querySelectorAll("video")).slice(0, 8).map((video) => {
      const bounds = video.getBoundingClientRect();
      const source = urls.get(video.currentSrc || video.src);
      return { id: video.id === "livePlayer" ? "livePlayer" : "other", width: video.videoWidth, height: video.videoHeight,
        time: video.currentTime, ready: video.readyState, paused: video.paused, ended: video.ended, error: video.error?.code ?? null,
        paintWidth: Math.round(bounds.width), paintHeight: Math.round(bounds.height),
        decoded: video.getVideoPlaybackQuality?.().totalVideoFrames ?? video.webkitDecodedFrameCount ?? 0,
        ad: video.id === "da_video" || !!video.closest("[id*=adPlayer],[id*=AdPlayer],[class*=ad-player],[class*=ad_video]"),
        encrypted: !!video.mediaKeys, srcObject: !!video.srcObject, sourceBound: !!source,
        tracks: source?.tracks.map((entry) => ({ mime: entry.mime, appends: entry.appends, bytes: entry.bytes, changes: entry.changes, containers: [...entry.containers] })) ?? [],
      };
    });
    const chat = document.getElementById("chatting_area");
    const input = chat?.querySelector("textarea, [contenteditable=true], input[type=text]");
    const state = { ready: document.readyState, videos, forcedPlay: false, agentAbsenceVerified: false,
      mse: { available: !!nativeMse && !!nativeBuffer, hooked, sources: observed.length, buffers: bufferCount, overflow },
      chat: { ...visibility(chat), inputPresent: !!input, inputEnabled: !!input && !input.disabled && visibility(input).visible },
    };
    parent.postMessage({ type: "atsumi-soop-probe", state }, hostOrigin);
    if (++samples >= 138) { clearInterval(timer); restore(); }
  }, 650);

  function mime(value) {
    if (typeof value !== "string" || value.length > 120) return "unrecognized";
    return /^(?:video|audio)\/(?:mp4|webm|mp2t)(?:\s*;\s*codecs\s*=\s*"?[a-zA-Z0-9., _-]{1,96}"?)?$/i.test(value) ? value : "unrecognized";
  }
})()
