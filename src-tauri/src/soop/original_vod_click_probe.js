  // Opt-in isolated QA only. Observations never authorize or modify input.
  (() => {
    let count = 0, stopped = false;
    const stop = () => {
      stopped = true;
      try { document.removeEventListener('pointerdown', observe, true); } catch {}
      try { document.removeEventListener('click', observe, true); } catch {}
      try { window.removeEventListener('pagehide', stop); } catch {}
    };
    const observe = event => {
      if (stopped || count >= 16) return;
      count++;
      try {
        const node = event.target;
        const chat = !!node?.closest?.('#chatting_area');
        const excluded = !!node?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]');
        const player = !!node?.closest?.('#player');
        const target = chat ? 'chat' : excluded || !player ? 'excluded'
          : node?.closest?.('.btn_fullScreen_mode,#btnExtendMode') ? 'fullscreen_button' : 'player_other';
        window.chrome.webview.postMessage('ATSUMI_SOOP_VOD_CLICK_PROBE:' + JSON.stringify({
          kind: event.type === 'pointerdown' ? 'pointerdown' : 'click', target,
          trusted: event.isTrusted === true, left: event.button === 0,
          configured: epoch > 0, fullscreen: fullscreen === true,
        }));
      } catch {}
      finally { if (count >= 16) stop(); }
    };
    try {
      document.addEventListener('pointerdown', observe, { capture: true, passive: true });
      document.addEventListener('click', observe, { capture: true, passive: true });
      window.addEventListener('pagehide', stop, { once: true });
    } catch { stop(); }
  })();
