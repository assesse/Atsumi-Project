// Disposable feasibility observer. No media copies, URL logs, credentials,
// messages, auth actions, extra requests or changes to player presentation.
(() => {
  "use strict";
  if (window.top !== window || location.origin !== "https://www.youtube.com" ||
      !["/watch", "/live_chat"].includes(location.pathname) || window.__atsumiYouTubeOriginalProbe) return;
  const started = performance.now();
  const tracks = [];
  const tracked = new WeakMap();
  let observerError = false;
  const safeMime = value => {
    if (typeof value !== "string" || value.length > 160) return "other";
    const match = /^(video|audio)\/(mp4|webm)(?:\s*;\s*codecs="?([a-zA-Z0-9., _-]+)"?)?$/i.exec(value);
    if (!match) return "other";
    const codecs = match[3]?.split(",").map(part => part.trim()) ?? [];
    if (codecs.length > 2 || codecs.some(codec => !/^(?:avc[13]\.[0-9a-f]{6}|mp4a\.\d{1,2}\.\d{1,2}|vp0[89](?:\.\d{1,2}){3,8}|vp[89]|av01\.[0-9a-z.]{5,40}|opus|vorbis|(?:hev1|hvc1)\.[0-9a-z.]{3,40})$/i.test(codec))) return "other";
    return value;
  };
  const bounds = node => {
    if (!node) return null;
    const r = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height),
      displayed: r.width > 0 && r.height > 0 && style.display !== "none" && style.visibility !== "hidden", opacity: Number(style.opacity) };
  };
  try {
    if (window.MediaSource?.prototype?.addSourceBuffer && window.SourceBuffer?.prototype?.appendBuffer) {
      const add = MediaSource.prototype.addSourceBuffer;
      const append = SourceBuffer.prototype.appendBuffer;
      MediaSource.prototype.addSourceBuffer = function (...args) {
        const buffer = Reflect.apply(add, this, args);
        try {
          if (tracks.length < 16) {
            const track = { mime: safeMime(args[0]), appends: 0, bytes: 0, lastAppendMs: null };
            tracks.push(track); tracked.set(buffer, track);
          }
        } catch { observerError = true; }
        return buffer;
      };
      SourceBuffer.prototype.appendBuffer = function (...args) {
        const result = Reflect.apply(append, this, args);
        try {
          const track = tracked.get(this);
          if (track) {
            const size = args[0]?.byteLength;
            track.appends = Math.min(Number.MAX_SAFE_INTEGER, track.appends + 1);
            if (Number.isSafeInteger(size) && size >= 0) track.bytes = Math.min(Number.MAX_SAFE_INTEGER, track.bytes + size);
            track.lastAppendMs = Math.round(performance.now() - started);
          }
        } catch { observerError = true; }
        return result;
      };
    }
  } catch { observerError = true; }
  const snapshot = () => {
    const video = document.querySelector("#movie_player video") ?? document.querySelector("video");
    const query = selector => bounds(document.querySelector(selector));
    let errorCode;
    try { errorCode = document.querySelector("#movie_player")?.getPlayerResponse?.()?.playabilityStatus?.status; } catch { /* A site error is not a probe failure. */ }
    let decodedFrames = null;
    try { const count = video?.getVideoPlaybackQuality?.()?.totalVideoFrames; if (Number.isSafeInteger(count) && count >= 0) decodedFrames = count; } catch { /* Optional browser metric. */ }
    return {
      version: 1, document: location.pathname === "/live_chat" ? "chat" : "watch", elapsedMs: Math.round(performance.now() - started),
      readyState: document.readyState, observerError, mse: Boolean(window.MediaSource),
      advertisement: Boolean(document.querySelector("#movie_player.ad-showing, #movie_player.ad-interrupting")),
      tracks: tracks.map(track => ({ ...track })),
      video: video ? { currentTime: Number.isFinite(video.currentTime) ? video.currentTime : null,
        duration: Number.isFinite(video.duration) ? video.duration : null, paused: Boolean(video.paused), ended: Boolean(video.ended),
        readyState: video.readyState, width: video.videoWidth, height: video.videoHeight, protected: Boolean(video.mediaKeys),
        sourceObject: Boolean(video.srcObject), decodedFrames, bounds: bounds(video) } : null,
      player: query("#movie_player"), controls: query("#movie_player .ytp-chrome-bottom"),
      settings: query("#movie_player .ytp-settings-button"), fullscreen: query("#movie_player .ytp-fullscreen-button"),
      chat: query("yt-live-chat-app"), composer: query("yt-live-chat-text-input-field-renderer"),
      signInEntry: Boolean(document.querySelector('a[href*="accounts.google.com/"],a[href*="accounts.youtube.com/"],a[href*="ServiceLogin"],ytd-button-renderer[is-paper-button] a[href*="signin"]')),
      playability: ["OK", "ERROR", "UNPLAYABLE", "LOGIN_REQUIRED", "LIVE_STREAM_OFFLINE", "CONTENT_CHECK_REQUIRED", "AGE_CHECK_REQUIRED"].includes(errorCode) ? errorCode : null,
      errorVisible: Boolean(document.querySelector("#movie_player .ytp-error")),
    };
  };
  Object.defineProperty(window, "__atsumiYouTubeOriginalProbe", { value: { snapshot }, configurable: false });
})();
