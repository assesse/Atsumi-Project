// SOOP original VOD presentation only. No media capture, account authority or extra requests.
(() => {
  'use strict';
  if (window.top !== window || location.origin !== 'https://vod.sooplive.com' || window.__atsumiSoopVod) return;
  const match = /^\/player\/([1-9][0-9]{0,19})\/embed$/.exec(location.pathname);
  if (!match || location.search !== '?showChat=true&replayType=1') return;
  const videoId = match[1], LIMIT = 1024 * 1024;
  let epoch = 0, fullscreen = false, layout = null, layoutKey = '', chatState = 'pending';
  const notify = (type, fields = {}) => {
    if (epoch) try { window.chrome.webview.postMessage('ATSUMI_SOOP_VOD_V1:' + JSON.stringify({ id: crypto.randomUUID(), epoch, type, ...fields })); } catch {}
  };
  const metadataEndpoint = value => {
    try { const url = new URL(value, location.href); return url.protocol === 'https:' && url.hostname === 'api.m.sooplive.com' && !url.username && !url.password && !url.port && url.pathname === '/station/video/a/view'; } catch { return false; }
  };
  // The original caller still receives its untouched response. Only three non-secret fields survive.
  const observeMetadata = (url, text) => {
    if (!metadataEndpoint(url) || typeof text !== 'string' || text.length > LIMIT) return;
    try {
      if (new TextEncoder().encode(text).byteLength > LIMIT) return;
      const response = JSON.parse(text), data = response?.data;
      if (response?.result !== 1 || !data || !['string', 'number'].includes(typeof data.title_no) || String(data.title_no) !== videoId) return;
      if (data.show_chat === 'Y') chatState = 'available';
      else if (data.show_chat === 'N') chatState = 'unavailable';
      else return;
      notify('status', status());
    } catch {}
  };
  async function observeFetch(response) {
    if (!response?.ok || !metadataEndpoint(response?.url)) return;
    let reader, timer, timedOut = false;
    try {
      const length = Number(response.headers?.get('content-length'));
      if (Number.isFinite(length) && length > LIMIT) return;
      const copy = response.clone(); reader = copy.body?.getReader(); if (!reader) return;
      timer = setTimeout(() => { timedOut = true; void reader.cancel().catch(() => {}); }, 10_000);
      const decoder = new TextDecoder('utf-8', { fatal: true }); let bytes = 0, text = '';
      while (true) { const part = await reader.read(); if (part.done) break; bytes += part.value.byteLength; if (bytes > LIMIT) { void reader.cancel().catch(() => {}); return; } text += decoder.decode(part.value, { stream: true }); }
      text += decoder.decode(); if (!timedOut) observeMetadata(response.url, text);
    } catch {} finally { if (timer) clearTimeout(timer); try { reader?.releaseLock(); } catch {} }
  }
  if (typeof window.fetch === 'function') {
    const originalFetch = window.fetch;
    window.fetch = function (...args) {
      const result = Reflect.apply(originalFetch, this, args);
      // Attach directly before returning: cross-realm Promise.resolve can delay cloning
      // until the caller has already started consuming the original response body.
      result.then(response => { void observeFetch(response); }, () => {});
      return result;
    };
  }
  if (window.XMLHttpRequest?.prototype?.send) {
    const originalSend = window.XMLHttpRequest.prototype.send;
    window.XMLHttpRequest.prototype.send = function (...args) {
      const done = () => { try { if (this.status >= 200 && this.status < 300 && (this.responseType === '' || this.responseType === 'text')) observeMetadata(this.responseURL, this.responseText); } catch {} };
      this.addEventListener('loadend', done, { once: true });
      try { return Reflect.apply(originalSend, this, args); } catch (error) { this.removeEventListener('loadend', done); throw error; }
    };
  }
  function applyLayout() {
    if (!layout || !document.documentElement) return;
    if (!document.getElementById('atsumi-soop-vod-layout')) {
      const style = document.createElement('style'); style.id = 'atsumi-soop-vod-layout';
      style.textContent = `html,body,#root,#webplayer{margin:0!important;padding:0!important;min-width:0!important;min-height:0!important;overflow:hidden!important;background:#000!important}#root,#webplayer,#player_area{transform:none!important;contain:none!important}#player,#chatting_area{position:fixed!important;box-sizing:border-box!important;margin:0!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;transform:none!important;z-index:2147483000!important}#player{left:var(--atsumi-vod-player-x)!important;top:var(--atsumi-vod-player-y)!important;width:var(--atsumi-vod-player-w)!important;height:var(--atsumi-vod-player-h)!important;display:var(--atsumi-vod-player-display)!important}#chatting_area{left:var(--atsumi-vod-chat-x)!important;top:var(--atsumi-vod-chat-y)!important;width:var(--atsumi-vod-chat-w)!important;height:var(--atsumi-vod-chat-h)!important;display:var(--atsumi-vod-chat-display)!important}#chatbox_height{width:100%!important;height:100%!important;min-width:0!important;min-height:0!important}#player .player_ratio{padding:0!important;height:100%!important}#videoLayer,#player .video_area,#player .video_wrap{width:100%!important;height:100%!important}#videoLayer video.af_video{width:100%!important;height:100%!important;object-fit:contain!important}#player .player_ctrlBox{box-sizing:border-box!important;max-width:100%!important}#portalContainer{position:relative;z-index:2147483100!important}`;
      // The official embed rule has two IDs and height:100vh!important. Its
      // specificity must not override the measured floating slot height.
      style.textContent += `:is(:root.embeded_mode,:root .embeded_mode) #webplayer #player{height:var(--atsumi-vod-player-h)!important}`;
      (document.head || document.documentElement).appendChild(style);
    }
    const key = JSON.stringify([layout.player, layout.chat]); if (key === layoutKey) return; layoutKey = key;
    for (const name of ['player', 'chat']) {
      const rect = layout[name];
      for (const [key, value] of Object.entries({ x: rect?.x ?? 0, y: rect?.y ?? 0, w: rect?.width ?? 0, h: rect?.height ?? 0 })) document.documentElement.style.setProperty(`--atsumi-vod-${name}-${key}`, `${value}px`);
      document.documentElement.style.setProperty(`--atsumi-vod-${name}-display`, rect ? 'block' : 'none');
    }
    window.dispatchEvent(new Event('resize'));
  }
  function status() {
    const video = document.querySelector('#videoLayer video#video.af_video');
    const unavailable = !!video?.error || !!document.querySelector('#webplayer.vod_404');
    const ready = video?.readyState >= 2 && video.videoWidth > 0;
    return { state: ready ? video.ended ? 'ended' : video.paused ? 'paused' : 'playing' : unavailable ? 'error' : 'loading', chatState, unavailable };
  }
  const safeInput = event => event.isTrusted && !event.target?.closest?.('#chatting_area,input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]');
  const playerInput = event => safeInput(event) && event.target?.closest?.('#player');
  document.addEventListener('pointerdown', event => {
    if (event.button !== 0 || !playerInput(event) || !event.target?.closest?.('.btn_fullScreen_mode,#btnExtendMode')) return;
    event.preventDefault(); event.stopImmediatePropagation(); notify('fullscreen', { enabled: !fullscreen, inputKey: 1 });
  }, true);
  document.addEventListener('click', event => { if (playerInput(event) && event.target?.closest?.('.btn_fullScreen_mode,#btnExtendMode')) { event.preventDefault(); event.stopImmediatePropagation(); } }, true);
  document.addEventListener('keydown', event => {
    // EOF or an original control can return focus to the document. Only an
    // existing fullscreen exit is document-wide; F and pointer input stay local.
    const exitingFullscreen = event.key === 'Escape' && fullscreen;
    if (!(exitingFullscreen ? safeInput(event) : playerInput(event)) || event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229 || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
    if (event.key.toLowerCase() === 'f' || event.key === 'Escape' && fullscreen) { event.preventDefault(); event.stopImmediatePropagation(); notify('fullscreen', { enabled: event.key === 'Escape' ? false : !fullscreen, inputKey: event.key === 'Escape' ? 0x1B : 0x46 }); }
  }, true);
  const timer = setInterval(() => notify('status', status()), 1500);
  window.addEventListener('pagehide', () => clearInterval(timer), { once: true });
  Object.defineProperty(window, '__atsumiSoopVod', { value: Object.freeze({
    configure(value) { if (!value || !Number.isSafeInteger(value.epoch) || value.epoch < 1 || value.videoId !== videoId) return false; epoch = value.epoch; fullscreen = value.fullscreen === true; notify('status', status()); return true; },
    layout(value) { layout = value; applyLayout(); },
    fullscreen(enabled) { fullscreen = enabled === true; const button = document.querySelector('.btn_fullScreen_mode,#btnExtendMode'); if (button) { button.setAttribute('aria-pressed', String(fullscreen)); button.setAttribute('title', fullscreen ? '전체화면 종료 (F)' : '전체화면 (F)'); } },
    status,
  }), configurable: false });
  document.addEventListener('DOMContentLoaded', applyLayout, { once: true });
})();
