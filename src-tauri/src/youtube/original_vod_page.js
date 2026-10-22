// Watch-only YouTube document. No recorder, chat receiver, account inputs or media extraction.
(() => {
  'use strict';
  if (window.top !== window || location.origin !== 'https://www.youtube.com' || location.pathname !== '/watch' || window.__atsumiYouTubeVod) return;
  const videoId = new URLSearchParams(location.search).get('v');
  if (!/^[A-Za-z0-9_-]{11}$/.test(videoId ?? '')) return;
  let epoch = 0, layout = null, layoutKey = '', fullscreen = false;
  let endedCancelled = false, observedPlayer = null, endscreenObserver = null, pageGone = false;
  const notify = (type, fields = {}) => {
    if (!epoch) return;
    try { window.chrome.webview.postMessage('ATSUMI_YOUTUBE_VOD_V1:' + JSON.stringify({ id: crypto.randomUUID(), epoch, type, ...fields })); } catch {}
  };
  function apply() {
    if (!layout || !document.documentElement) return;
    if (!document.getElementById('atsumi-youtube-vod-layout')) {
      const style = document.createElement('style'); style.id = 'atsumi-youtube-vod-layout';
      // Preserve the original VOD player/menus, not the unrelated watch page.
      // Visibility keeps site layout alive without recreating the receiver.
      const watchSiblings = `:is(ytd-watch-flexy #secondary,ytd-watch-flexy #chat,ytd-watch-flexy #below,ytd-watch-flexy ytd-watch-metadata,ytd-masthead):not(:has(#movie_player))`;
      style.textContent = `html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#000!important}${watchSiblings},${watchSiblings} *{visibility:hidden!important;pointer-events:none!important}ytd-app,ytd-page-manager,ytd-watch-flexy,#columns,#primary,#primary-inner,#player-container-outer,#player-container-inner,ytd-player{transform:none!important;contain:none!important;overflow:visible!important}#movie_player{position:fixed!important;left:var(--atsumi-vod-x)!important;top:var(--atsumi-vod-y)!important;width:var(--atsumi-vod-w)!important;height:var(--atsumi-vod-h)!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;background:#000!important;z-index:2147483000!important}#movie_player .html5-video-container{width:100%!important;height:100%!important}#movie_player video.html5-main-video{width:100%!important;height:100%!important;left:0!important;top:0!important;object-fit:contain!important}#movie_player .ytp-chrome-bottom{width:calc(100% - 24px)!important;left:12px!important}#movie_player .ytp-popup{max-width:calc(100% - 24px)!important;max-height:calc(100% - 56px)!important}ytd-popup-container,tp-yt-iron-dropdown,ytd-menu-popup-renderer{z-index:2147483100!important}`;
      (document.head || document.documentElement).appendChild(style);
    }
    const rect = layout.player, key = JSON.stringify(rect);
    if (key === layoutKey) return; layoutKey = key;
    for (const [name, value] of Object.entries({ x: rect?.x ?? 0, y: rect?.y ?? 0, w: rect?.width ?? 0, h: rect?.height ?? 0 })) document.documentElement.style.setProperty('--atsumi-vod-' + name, value + 'px');
    window.dispatchEvent(new Event('resize'));
  }
  function selectedDocument() {
    const query = new URLSearchParams(location.search);
    return location.origin === 'https://www.youtube.com' && location.pathname === '/watch'
      && query.getAll('v').length === 1 && query.get('v') === videoId;
  }
  function retainSelectedEndscreen(player, video, response) {
    if (pageGone || !epoch || !selectedDocument() || response?.videoDetails?.videoId !== videoId || !video) return;
    if (!video.ended) { endedCancelled = false; return; }
    if (endedCancelled || player.classList?.contains('ad-showing') || player.classList?.contains('ad-interrupting')) return;
    const cancel = player.querySelector('.ytp-autonav-endscreen-upnext-cancel-button');
    if (!cancel || cancel.disabled || cancel.getAttribute('aria-disabled') === 'true'
      || cancel.closest('[hidden],[aria-hidden="true"]') || !cancel.getClientRects().length) return;
    const style = window.getComputedStyle(cancel);
    if (style.display === 'none' || style.visibility !== 'visible' || Number(style.opacity) === 0) return;
    // Official one-shot endscreen cancellation, not the persisted autoplay toggle.
    // Do not construct/execute the site's optional cancelCommand ourselves.
    endedCancelled = true;
    try { cancel.click(); } catch { /* Native identity fencing remains the fail-closed fallback. */ }
  }
  function observeEndscreen(player) {
    if (pageGone || player === observedPlayer) return;
    endscreenObserver?.disconnect(); endscreenObserver = null; observedPlayer = player;
    if (player && typeof MutationObserver === 'function') {
      endscreenObserver = new MutationObserver(() => status());
      // Only new/removed nodes inside the original player; no full-page/style observer.
      endscreenObserver.observe(player, { childList: true, subtree: true });
    }
  }
  function status() {
    const player = document.getElementById('movie_player'), video = player?.querySelector('video');
    let response = null, authenticated = null;
    try { response = player?.getPlayerResponse?.() ?? window.ytInitialPlayerResponse; } catch {}
    try { const known = window.ytcfg?.get?.('LOGGED_IN'); if (typeof known === 'boolean') authenticated = known; } catch {}
    observeEndscreen(player);
    retainSelectedEndscreen(player, video, response);
    const correct = selectedDocument() && response?.videoDetails?.videoId === videoId;
    const unavailable = ['ERROR', 'UNPLAYABLE', 'LOGIN_REQUIRED', 'LIVE_STREAM_OFFLINE'].includes(response?.playabilityStatus?.status);
    const ready = correct && video?.readyState >= 2 && video.videoWidth > 0;
    // End is a presentation state, not teardown. The official replay and seek controls remain active.
    return { state: ready ? video.ended ? 'ended' : 'playing' : unavailable ? 'error' : 'loading', authenticated, unavailable };
  }
  const timer = setInterval(() => notify('status', status()), 1500);
  document.addEventListener('ended', () => status(), true);
  document.addEventListener('playing', () => status(), true);
  document.addEventListener('seeked', () => status(), true);
  document.addEventListener('pointerdown', event => {
    if (event.isTrusted && event.button === 0 && event.target?.closest?.('.ytp-fullscreen-button')) notify('fullscreen', { enabled: !fullscreen, inputKey: 1 });
  }, true);
  document.addEventListener('click', event => {
    if (event.isTrusted && event.target?.closest?.('.ytp-fullscreen-button')) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  document.addEventListener('keydown', event => {
    if (!event.isTrusted || event.repeat || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey || event.isComposing || event.keyCode === 229 || event.target?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]')) return;
    if (event.key === 'Escape' && fullscreen || event.key.toLowerCase() === 'f') {
      event.preventDefault(); event.stopImmediatePropagation();
      notify('fullscreen', { enabled: event.key === 'Escape' ? false : !fullscreen, inputKey: event.key === 'Escape' ? 0x1B : 0x46 });
    }
  }, true);
  window.addEventListener('pagehide', () => { pageGone = true; clearInterval(timer); endscreenObserver?.disconnect(); endscreenObserver = null; observedPlayer = null; }, { once: true });
  Object.defineProperty(window, '__atsumiYouTubeVod', { value: Object.freeze({
    configure(value) { if (!value || !Number.isSafeInteger(value.epoch) || value.epoch < 1 || value.videoId !== videoId) return false; epoch = value.epoch; fullscreen = value.fullscreen === true; notify('status', status()); return true; },
    layout(value) { layout = value; apply(); },
    fullscreen(enabled) { fullscreen = enabled === true; const button = document.querySelector('.ytp-fullscreen-button'); if (button) { button.setAttribute('aria-label', fullscreen ? '전체화면 종료 (f)' : '전체화면 (f)'); button.setAttribute('aria-pressed', String(fullscreen)); } },
    status,
  }), configurable: false });
  document.addEventListener('DOMContentLoaded', apply, { once: true });
})();
