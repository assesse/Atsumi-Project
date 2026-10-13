// Playback-only adapter for the official pre-roll component (2026-10-12).
// Its new encrypted transport has its own JSON parser. Do NOT patch JSON,
// XHR/fetch, cryptography or authentication. Use the component's existing
// NO_ADS completion path, with a strict DOM/props/channel boundary instead.
(() => {
  const channel = () => window.top === window
    && window.location.origin === 'https://chzzk.naver.com'
    && !new URLSearchParams(window.location.search).has('chat')
    ? /^\/live\/([a-f0-9]{32})$/i.exec(window.location.pathname)?.[1] : null;
  if (!channel() || window.__atsumiVideoAdPolicy) return;
  const state = { version: 1, completed: 0, unsupported: false };
  Object.defineProperty(window, '__atsumiVideoAdPolicy', { value: state });
  const handled = new WeakSet();
  function finishPreRoll() {
    const id = channel(), root = document.getElementById('preAdPlayerWrapper');
    if (!id || !root?.isConnected || handled.has(root)) return;
    const key = Object.keys(root).find(key => key.startsWith('__reactFiber$'));
    let fiber = key && root[key];
    for (let depth = 0; fiber && depth < 6; depth++, fiber = fiber.return) {
      const props = fiber.memoizedProps;
      if (typeof fiber.type === 'string' || !props || props.channelId !== id
        || !/^\d{1,20}$/.test(String(props.liveId ?? ''))
        || typeof props.onComplete !== 'function'
        || typeof props.onSkip !== 'function'
        || typeof props.onAdStart !== 'function') continue;
      // Record before calling: React reconciliation can synchronously mutate
      // this subtree. Do not emit started/skipped/impression events.
      handled.add(root);
      try {
        props.onComplete('NO_ADS');
        state.completed++;
        state.unsupported = false;
      } catch (_) {
        state.unsupported = true;
      }
      return;
    }
    // Unknown upstream UI stays usable. Never click arbitrary buttons, remove
    // an unknown player, or suppress other errors/account/paid-access prompts.
    state.unsupported = true;
  }
  const observer = new MutationObserver(finishPreRoll);
  observer.observe(document, { childList: true, subtree: true });
  finishPreRoll();
  window.addEventListener('pagehide', () => observer.disconnect());
  window.addEventListener('pageshow', event => {
    if (event.persisted) {
      observer.observe(document, { childList: true, subtree: true });
      finishPreRoll();
    }
  });
})();
