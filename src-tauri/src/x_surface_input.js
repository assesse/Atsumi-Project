// Bounded native-document keyboard adapter. The host arbitrates the last
// user-focused pane; this helper operates only original visible controls.
(() => {
  const config = __ATSUMI_X_INPUT__;
  if (window !== window.top || location.protocol !== 'https:' || !['x.com','www.x.com'].includes(location.hostname) || window.__atsumiXInput) return;
  let visible = false;
  const signal = (kind, value = '') => {
    if (!visible) return;
    try { window.chrome?.webview?.postMessage(`ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/${config.nonce}?kind=${encodeURIComponent(kind)}&value=${encodeURIComponent(value)}`); } catch {}
  };
  const security = 'input[type="password"],input[autocomplete="one-time-code"],[data-testid="ocfEnterTextTextInput"],[data-testid="LoginForm_Login_Button"]';
  const editing = target => target instanceof Element && !!target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="slider"]');
  const shown = node => {
    if (!node || node.closest('[hidden],[aria-hidden="true"],[inert]')) return false;
    const style = window.getComputedStyle(node);
    return style.display !== 'none' && style.visibility !== 'hidden' && style.visibility !== 'collapse' && node.getClientRects().length > 0;
  };
  const photoPath = () => /^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/[1-9]\d{4,24}\/photo\/[1-4]$/.test(location.pathname);
  const photoImage = img => {
    if (!shown(img)) return false;
    try {
      const url = new URL(img.currentSrc || img.getAttribute('src'));
      return url.protocol === 'https:' && url.hostname === 'pbs.twimg.com' && !url.port && !url.username && !url.password
        && /^\/media\/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9]+)?$/.test(url.pathname);
    } catch { return false; }
  };
  const dialogs = () => [...document.querySelectorAll('[role="dialog"],[aria-modal="true"]')].filter(shown).slice(-8);
  const nativeEscape = () => {
    // Only a user-requested Escape in an existing native layer is replayed.
    // Our adapters ignore untrusted events, so it cannot recurse into outer
    // preview close. Original X handlers decide how their own layer closes.
    const target = document.activeElement instanceof Element ? document.activeElement : document.body;
    target.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',code:'Escape',bubbles:true,cancelable:true}));
  };
  const photoRoot = () => {
    if (!photoPath() || document.querySelector(security)) return null;
    const layers = dialogs();
    if (layers.length) {
      const top = layers[layers.length - 1];
      return [...top.querySelectorAll('img')].slice(0,32).some(photoImage) ? top : null;
    }
    return [...document.querySelectorAll('img')].slice(0,128).some(photoImage) ? document.body : null;
  };
  const labels = {
    previous: /^(previous(?: (?:photo|image|slide))?|이전(?: (?:사진|이미지|슬라이드))?|前へ|前の(?:画像|写真))$/i,
    next: /^(next(?: (?:photo|image|slide))?|다음(?: (?:사진|이미지|슬라이드))?|次へ|次の(?:画像|写真))$/i,
    close: /^(close|back|닫기|뒤로|뒤로가기|閉じる|戻る)$/i,
  };
  const ids = {
    previous: new Set(['carouselprevious','previousphoto','previousimage']),
    next: new Set(['carouselnext','nextphoto','nextimage']),
    close: new Set(['app-bar-close','app-bar-back']),
  };
  const control = (root, kind) => {
    if (!root) return null;
    const matches = [...root.querySelectorAll('button,[role="button"]')].slice(0,128).filter(button => shown(button)
      && !button.disabled && button.getAttribute('aria-disabled') !== 'true'
      && !button.closest('[data-atsumi-x-actions],.atsumi-x-native-actions,article,[data-testid="videoPlayer"]')
      && (ids[kind].has((button.getAttribute('data-testid') || '').toLowerCase())
        || labels[kind].test((button.getAttribute('aria-label') || '').trim())));
    return matches.length === 1 ? matches[0] : null;
  };
  function key(value) {
    if (!visible || document.querySelector(security) || editing(document.activeElement)) return false;
    if (value === 'Escape') {
      if (window.__atsumiXSurface?.dismissMenu?.()) return true;
      if (document.fullscreenElement) {
        try { void document.exitFullscreen().catch(() => {}); } catch {}
        return true;
      }
      const layers = dialogs(), photo = photoRoot();
      if (layers.length || photo) {
        const close = control(layers.length ? layers[layers.length - 1] : photo, 'close');
        if (close) close.click(); else nativeEscape();
        return true; // Unknown native layers never dismiss an outer preview.
      }
      if ([...document.querySelectorAll('[role="menu"]')].some(shown)) { nativeEscape(); return true; }
      if (config.feed === 'preview') { signal('close'); return true; }
      return false;
    }
    if (!['ArrowLeft','ArrowRight'].includes(value) || document.fullscreenElement) return false;
    const root = photoRoot();
    if (!root) return false;
    control(root, value === 'ArrowLeft' ? 'previous' : 'next')?.click();
    return true;
  }
  window.__atsumiXInput = Object.freeze({
    visibility(value) { visible = value === true; },
    key,
  });
  const pointerFocus = event => {
    if (event.isTrusted && visible && [0,2].includes(event.button) && !event.ctrlKey && !event.metaKey && !event.altKey) signal('focus');
  };
  document.addEventListener('pointerdown', pointerFocus, true);
  document.addEventListener('contextmenu', pointerFocus, true);
  document.addEventListener('keydown', event => {
    if (!event.isTrusted || !visible || event.defaultPrevented || event.isComposing || event.keyCode === 229
      || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    if (editing(event.target)) { signal('focus'); return; }
    if (event.key === 'Escape' && window.__atsumiXSurface?.dismissMenu?.()) {
      event.preventDefault();event.stopImmediatePropagation();signal('focus');return;
    }
    const imageKey = /^(a|d|ArrowLeft|ArrowRight)$/i.test(event.key);
    const root = photoRoot();
    const targetControl = event.target instanceof Element ? event.target.closest('button,[role="button"]') : null;
    const photoNavigation = targetControl && root && (
      [control(root,'previous'),control(root,'next'),control(root,'close')].includes(targetControl)
      || window.__atsumiXActions?.ownsPhotoControl?.(targetControl) === true
    );
    const unrelatedControl = targetControl && !photoNavigation || event.target instanceof Element
      && event.target.closest('video,audio,[data-testid="videoPlayer"],[role="menu"],[data-atsumi-x-actions],.atsumi-x-native-actions');
    if (event.key !== 'Escape' && (!imageKey || unrelatedControl || !root)) { signal('focus'); return; }
    event.preventDefault();event.stopImmediatePropagation();
    if (!event.repeat) signal('key', event.key === 'Escape' ? 'Escape' : /^(a|ArrowLeft)$/i.test(event.key) ? 'ArrowLeft' : 'ArrowRight');
  }, true);
})();
