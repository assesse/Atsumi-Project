// Account completion facts only. Never read credentials, input values or text.
(() => {
  const unknown = { authenticated: null, interactive: true };
  if (window.top !== window || location.origin !== "https://www.youtube.com" || location.pathname !== "/" || location.search || location.hash) return unknown;
  try {
    const value = window.ytcfg?.get?.("LOGGED_IN");
    if (typeof value !== "boolean") return unknown;
    const editable = document.activeElement?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"]');
    const dialogs = document.querySelectorAll('dialog[open],[role="dialog"],[role="alertdialog"],ytd-consent-bump-v2-lightbox');
    if (document.readyState !== "complete" || editable || dialogs.length > 64) return { authenticated: value, interactive: true };
    for (const dialog of dialogs) {
      const rect = dialog.getBoundingClientRect(), style = getComputedStyle(dialog);
      if (rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden") return { authenticated: value, interactive: true };
    }
    return { authenticated: value, interactive: false };
  } catch { return unknown; }
})();
