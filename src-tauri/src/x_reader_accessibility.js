// Reader-only: the off-client transport is not a second accessible interface.
// Inject from create_reader only, never into the interactive account window.
(() => {
  if (window !== window.top || location.protocol !== 'https:' ||
      !['x.com', 'www.x.com'].includes(location.hostname) || location.port) return;

  const diagnostics = __ATSUMI_X_READER_DIAGNOSTICS__;
  let focusedBeforeCover = false;
  const coverRoots = () => {
    // Chromium ignores aria-hidden on html/body. Use valid content roots while
    // leaving layout/rendering and programmatic controls unchanged. ARIA alone
    // is also ignored when a descendant takes focus; inert prevents that focus.
    for (const root of document.body?.children || []) {
      if (diagnostics && root.contains(document.activeElement)) focusedBeforeCover = true;
      if (!root.hasAttribute('inert')) root.setAttribute('inert', '');
      if (root.getAttribute('aria-hidden') !== 'true') root.setAttribute('aria-hidden', 'true');
    }
  };
  const observer = new MutationObserver(records => {
    const body = document.body;
    if (records.some(record => record.type === 'attributes'
      ? record.target.parentElement === body
      : record.target === document || record.target === document.documentElement || record.target === body)) {
      coverRoots();
    }
  });
  // A body/root may be created after initialization or replaced by navigation.
  // Observe only our two attributes; idempotence prevents a self-write loop.
  // BFCache restores this same document rather than re-running initialization.
  let observing = false;
  const start = () => {
    if (!observing) {
      observer.observe(document, {subtree: true, childList: true, attributes: true, attributeFilter: ['aria-hidden', 'inert']});
      observing = true;
    }
    coverRoots();
  };
  window.addEventListener('pagehide', () => { observer.disconnect(); observing = false; });
  window.addEventListener('pageshow', start);
  if (diagnostics) Object.defineProperty(window, '__atsumiXReaderAccessibility', {
    value: Object.freeze({snapshot: () => {
      const roots = [...document.body?.children || []], active = document.activeElement;
      return {
        installed: true, observing, bodyPresent: !!document.body,
        roots: Math.min(roots.length, 64),
        ariaHiddenRoots: Math.min(roots.filter(root => root.getAttribute('aria-hidden') === 'true').length, 64),
        inertRoots: Math.min(roots.filter(root => root.hasAttribute('inert')).length, 64),
        documentHasFocus: document.hasFocus(),
        activeIsDocumentRoot: active === document.body || active === document.documentElement,
        focusInContent: roots.some(root => root.contains(active)), focusedBeforeCover,
      };
    }}), configurable: false, writable: false,
  });
  start();
})()
