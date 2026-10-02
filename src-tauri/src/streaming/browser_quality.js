// SPDX-License-Identifier: MIT
// Select on the existing official player: no second stream, URL rewriting,
// permission-dialog clicks, or bypass of its quality/access filter.
(() => {
  "use strict";
  const route = () => window.location.origin === "https://chzzk.naver.com" &&
    /^\/live\/[a-f0-9]{32}\/?$/i.test(window.location.pathname);
  if (!route() || window.top !== window || window.__atsumiQuality) return;
  const PREFERRED = 1080;
  // The official LIVE initializer reads this exact preference before playback.
  try { window.localStorage.setItem("live-player-video-track", JSON.stringify({ label: "1080p", width: 1920, height: 1080 })); } catch { /* Storage denial must not prevent viewing. */ }
  let binding = null, pending = null, alive = true, preparing = null;
  let last = { status: "waiting", height: null }, lastAttempt = 0;
  const failed = new Map();
  const read = (object, key) => { try { return object?.[key]; } catch { return undefined; } };
  const recording = () => {
    try { const state = window.__atsumiEncodedCapture?.getStatus(); return Boolean(state?.active || state?.starting || state?.stopping); }
    catch { return true; }
  };
  const video = () => [...document.querySelectorAll(".chzzk_player.type_live video.webplayer-internal-video")]
    .filter(v => v.isConnected && !v.ended && v.videoWidth > 0)
    .sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0] ?? null;
  const valid = (value, v) => value && value.video === v && value.host.isConnected &&
    value.player.shadowRoot?.contains(v) && value.provider === value.player.srcObject;
  const find = (v) => {
    if (!v) return null;
    if (valid(binding, v)) return binding;
    binding = null; failed.clear(); lastAttempt = 0;
    const host = v.closest(".chzzk_player.type_live");
    if (!host) return null;
    // Bounded traversal of the official React component's player reference.
    // Never scan arbitrary page/window objects or patch global JS prototypes.
    const key = Object.getOwnPropertyNames(host).find(k => k.startsWith("__reactFiber$"));
    const candidates = new Map();
    const inspect = value => {
      try {
        if (!value || typeof value.querySelector !== "function" || !value.shadowRoot?.contains(v)) return;
        const pane = value.querySelector("pzp-setting-quality-pane") ?? value.querySelector("pzp-pc-setting-quality-pane");
        if (typeof pane?.selectVideoTrack !== "function" || typeof pane?.$dispatch !== "function" || !value.videoTracks) return;
        candidates.set(pane, { player: value, pane, video: v, host, provider: value.srcObject });
      } catch { /* Upstream layout changes fail closed, leaving native playback. */ }
    };
    for (let owner = read(host, key), depth = 0; owner && depth < 8; owner = read(owner, "return"), depth++) {
      for (const fiber of [owner, read(owner, "alternate")]) {
        for (let hook = read(fiber, "memoizedState"), count = 0; hook && count < 48; hook = read(hook, "next"), count++) {
          const value = read(hook, "memoizedState"); inspect(value); inspect(read(value, "current"));
        }
      }
    }
    if (candidates.size === 1) binding = [...candidates.values()][0];
    return binding;
  };
  const height = track => {
    const value = Math.min(Number(track?.width), Number(track?.height));
    return Number.isFinite(value) && value >= 144 && value <= 4320 ? value : 0;
  };
  const choices = b => {
    let tracks;
    try { tracks = Array.from(b.player.videoTracks).slice(0, 64); } catch { return []; }
    return tracks.filter(t => {
      if (!height(t) || typeof t.id !== "string" || !t.id || /^(abr|auto)$/i.test(t.label ?? "")) return false;
      if ((failed.get(t.id) ?? 0) > Date.now()) return false;
      try { return typeof b.pane.filter !== "function" || b.pane.filter(t) === true; } catch { return false; }
    }).sort((a, c) => Number(height(c) === PREFERRED) - Number(height(a) === PREFERRED) || height(c) - height(a) ||
      Number(c.selected === true) - Number(a.selected === true) || Number(c.videoBitrate || 0) - Number(a.videoBitrate || 0));
  };
  const settled = (b, track) => b.video.readyState >= 2 && height({ width: b.video.videoWidth, height: b.video.videoHeight }) === height(track);
  const tick = () => {
    if (!alive || !route() || recording()) return;
    const b = find(video());
    if (!b) { last = { status: "native", height: null }; return; }
    if (pending) {
      if (pending.binding !== b) { pending = null; return; }
      if (pending.accepted && settled(b, pending.track)) { last = { status: "ready", height: height(pending.track) }; pending = null; }
      else if (Date.now() >= pending.deadline) { failed.set(pending.track.id, Date.now() + 60_000); pending = null; }
      return;
    }
    const target = choices(b)[0];
    if (!target) { last = { status: "native", height: b.video.videoHeight || null }; return; }
    if (target.selected === true && settled(b, target)) { last = { status: "ready", height: height(target) }; return; }
    if (b.video.paused || b.video.seeking || b.video.readyState < 2 || Date.now() - lastAttempt < 1000) return;
    lastAttempt = Date.now();
    last = { status: "selecting", height: height(target) };
    const attempt = { binding: b, track: target, deadline: Date.now() + 4000, accepted: false };
    pending = attempt;
    try {
      // Same cancellable quality event as the official menu. In particular,
      // do not force a track after the page denies the grid/entitlement check.
      if (b.pane.$dispatch("change", { track: target }) === false) throw new Error("native_denied");
      Promise.resolve(b.pane.selectVideoTrack(target.id)).then(() => {
        if (pending === attempt) attempt.accepted = true;
      }, () => {
        if (pending === attempt) { failed.set(target.id, Date.now() + 60_000); pending = null; }
      });
    } catch {
      failed.set(target.id, Date.now() + 60_000); pending = null;
    }
  };
  const prepare = () => {
    if (preparing) return preparing;
    // Give the preference a bounded chance BEFORE arming the source recorder.
    // Once armed, never cause track_changed by upgrading its source mid-file.
    preparing = (async () => {
      const deadline = Date.now() + 9000;
      do {
        tick();
        if (!alive || !route() || recording() || (!pending && last.status !== "selecting")) break;
        await new Promise(resolve => setTimeout(resolve, 200));
      } while (Date.now() < deadline);
      return alive && route() && !pending && last.status !== "selecting";
    })().finally(() => { preparing = null; });
    return preparing;
  };
  Object.defineProperty(window, "__atsumiQuality", { value: Object.freeze({ prepare,
    canStart: () => !pending && last.status !== "selecting", getStatus: () => ({ ...last }) }) });
  const timer = setInterval(tick, 1000);
  window.addEventListener("pagehide", () => { alive = false; clearInterval(timer); pending = null; binding = null; });
  tick();
})();
