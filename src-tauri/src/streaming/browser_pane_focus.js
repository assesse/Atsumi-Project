// A pointer-only fallback for WebView2 children that suppress WM_PARENTNOTIFY.
// Native code independently checks physical input, visibility and actual hit HWND.
((expectedUrl, token) => {
  "use strict";
  const eligible = () => window.top === window && window.location.href === expectedUrl;
  if (!eligible() || !window.chrome?.webview?.postMessage || window.__atsumiPanePointer) return;
  window.__atsumiPanePointer = true;
  window.addEventListener("pointerdown", (event) => {
    if (!eligible() || !event.isTrusted || event.button !== 0 || event.isPrimary === false) return;
    try { window.chrome.webview.postMessage("ATSUMI_PANE_POINTER:" + token); } catch (_) {}
  }, { capture: true, passive: true });
})
