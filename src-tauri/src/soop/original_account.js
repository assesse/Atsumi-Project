// Observe official account results only; never inspect callback arguments.
(() => {
  "use strict";
  const loginOrigin = "https://login.sooplive.com";
  const homeOrigin = "https://www.sooplive.com";
  const atHome = () => location.origin === homeOrigin && location.pathname === "/" && !location.search && !location.hash;
  if (window.top !== window || (location.origin !== loginOrigin && !atHome()) || window.__atsumiSoopAccount) return;
  const nonce = __ATSUMI_SOOP_ACCOUNT_NONCE__;
  const homeDocument = atHome();
  let epoch = 0, last = null, observed = null;
  let serial = 0, active = null, timer = null, startedAt = 0, attempts = 0, closed = false;
  const flush = () => {
    if (!epoch || typeof observed !== "boolean" || observed === last) return;
    try {
      window.chrome.webview.postMessage("ATSUMI_SOOP_ACCOUNT_V1:" + JSON.stringify({ nonce, epoch, authenticated: observed }));
      last = observed;
    } catch {}
  };
  const report = () => {
    const value = window.isLogin;
    if (typeof value !== "boolean") return;
    observed = value;
    flush();
  };
  const install = () => {
    if (homeDocument) return;
    for (const name of ["onAfterLogin", "onAfterSecondLogin", "onAfterLogOut"]) {
      const original = window[name];
      if (typeof original !== "function" || original.__atsumiAccountObserved) continue;
      const wrapped = function () { try { return Reflect.apply(original, this, arguments); } finally { report(); } };
      Object.defineProperty(wrapped, "__atsumiAccountObserved", { value: true });
      window[name] = wrapped;
    }
  };
  const stopProbe = () => {
    serial++;
    if (timer !== null) clearTimeout(timer);
    timer = null;
    active?.abort();
    active = null;
  };
  const readHome = async controller => {
    const endpoint = "https://afevent2.sooplive.com/api/get_private_info.php?_=" + Date.now();
    const response = await fetch(endpoint, { credentials: "include", cache: "no-store", redirect: "error", signal: controller.signal });
    if (!response.ok || response.url !== endpoint || !response.body?.getReader) throw new Error("unconfirmed");
    const length = response.headers.get("content-length");
    if (length !== null && (!/^\d+$/.test(length) || Number(length) > 32768)) throw new Error("unconfirmed");
    const reader = response.body.getReader(), decoder = new TextDecoder("utf-8", { fatal: true });
    let bytes = 0, text = "", done = false;
    try {
      for (let reads = 0; reads < 128; reads++) {
        const part = await reader.read();
        if (part.done) { done = true; break; }
        bytes += part.value.byteLength;
        if (bytes > 32768 || controller.signal.aborted) throw new Error("unconfirmed");
        text += decoder.decode(part.value, { stream: true });
      }
      if (!done) throw new Error("unconfirmed");
      text += decoder.decode();
      const value = JSON.parse(text)?.CHANNEL?.IS_LOGIN;
      if (value === 1) return true;
      if (value === -1) return false;
      throw new Error("unconfirmed");
    } finally {
      if (!done) { try { await reader.cancel(); } catch {} }
      reader.releaseLock();
    }
  };
  const probeHome = async ticket => {
    if (closed || ticket !== serial || !atHome() || active || attempts >= 3) return;
    const remaining = 15000 - (performance.now() - startedAt);
    if (remaining <= 0) return;
    attempts++;
    const controller = new AbortController();
    active = controller;
    const timeout = setTimeout(() => controller.abort(), Math.min(5000, remaining));
    let confirmed = false;
    try {
      const value = await readHome(controller);
      if (closed || ticket !== serial || !atHome() || controller.signal.aborted || performance.now() - startedAt >= 15000) return;
      observed = value;
      confirmed = true;
      flush();
    } catch {} finally {
      controller.abort();
      clearTimeout(timeout);
      if (active === controller) active = null;
      if (!confirmed && !closed && ticket === serial && atHome() && attempts < 3 && performance.now() - startedAt < 14500) {
        timer = setTimeout(() => { timer = null; void probeHome(ticket); }, 500);
      }
    }
  };
  Object.defineProperty(window, "__atsumiSoopAccount", { value: Object.freeze({
    configure(value) {
      if (!Number.isSafeInteger(value) || value < 1 || closed) return false;
      const changed = epoch !== value;
      if (changed) last = null;
      epoch = value;
      install();
      if (homeDocument && changed) {
        stopProbe();
        observed = null;
        attempts = 0;
        startedAt = performance.now();
        void probeHome(serial);
      } else flush();
      return true;
    },
    logout() { install(); if (homeDocument || typeof window.LogOut !== "function") return false; window.LogOut("https://www.sooplive.com/"); return true; },
  }), configurable: false });
  document.addEventListener("DOMContentLoaded", install, { once: true });
  if (homeDocument) window.addEventListener("pagehide", () => { closed = true; stopProbe(); }, { once: true });
})();
