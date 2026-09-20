// SPDX-License-Identifier: MIT
// Presentation controls only. Remote intent is NOT permission to write a file.
(() => {
  "use strict";
  const channel = () => window.location.origin === "https://chzzk.naver.com" &&
    /^\/live\/([a-f0-9]{32})\/?$/i.exec(window.location.pathname)?.[1].toLowerCase();
  if (!channel() || window.top !== window || !window.chrome?.webview?.postMessage || window.__atsumiPlayerUI) return;
  const PREFIX = "ATSUMI_BROWSER_CAPTURE:";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const pending = new Map();
  let state = { ready: false, recording: false, detail: "unavailable" };
  let controls = null, record = null, screenshot = null, speed = null, focus = null, player = null, style = null;
  let topControls = null, settings = null, rateButton = null, rateMenu = null;
  let recordOnly = null;
  const context = { multiview: false, automaticWatch: false };
  let catchup = null;
  let selectedRate = null;
  const RATES = [.5, .75, 1, 1.25, 1.5, 2];
  let pageActive = true, renderFrame = null;
  let snapshotBusy = false, intentBusy = false, notice = "", noticeUntil = 0;
  const request = (kind, fields) => new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("응답을 받지 못했습니다")); }, 8000);
    pending.set(id, { resolve, reject, timer });
    try { window.chrome.webview.postMessage(PREFIX + JSON.stringify({ atsumiBrowserCapture: 1, id, kind, ...fields })); }
    catch { clearTimeout(timer); pending.delete(id); reject(new Error("연결을 확인해 주세요")); }
  });
  window.addEventListener("atsumi-browser-reply", (event) => {
    const reply = event.detail, waiter = pending.get(reply?.id);
    if (!waiter) return;
    pending.delete(reply.id); clearTimeout(waiter.timer);
    if (reply.ok === true) waiter.resolve(reply.data);
    else waiter.reject(new Error("Atsumi에서 상태를 확인해 주세요"));
  });
  const videoSource = () => [...document.querySelectorAll("video")]
    .filter((v) => v.isConnected && v.readyState >= 2 && v.videoWidth > 0 && v.videoHeight > 0 && !v.ended)
    .sort((a, b) => { const x = a.getBoundingClientRect(), y = b.getBoundingClientRect(); return y.width * y.height - x.width * x.height; })[0];
  const liveDistance = (video) => {
    if (!video || !Number.isFinite(video.currentTime)) return null;
    try {
      const range = video.seekable?.length ? video.seekable : video.buffered;
      if (!range?.length) return null;
      const end = range.end(range.length - 1), delta = end - video.currentTime;
      return Number.isFinite(delta) && delta >= 0 ? delta : null;
    } catch { return null; }
  };
  const tell = (text) => { notice = text; noticeUntil = Date.now() + 6000; render(); };
  const bufferedAhead = (video) => {
    try {
      for (let index = 0; index < (video?.buffered?.length ?? 0); index++) {
        if (video.buffered.start(index) <= video.currentTime && video.currentTime < video.buffered.end(index))
          return video.buffered.end(index) - video.currentTime;
      }
    } catch { /* Unknown ranges cannot authorize a speed increase. */ }
    return null;
  };
  const preservePitch = (video) => {
    try { video.preservesPitch = true; if ("webkitPreservesPitch" in video) video.webkitPreservesPitch = true; } catch { /* Older runtimes may not expose this preference. */ }
  };
  const viewIntent = async (action, event) => {
    if (!event.isTrusted || intentBusy || !channel()) return;
    const sourceChannel = channel();
    intentBusy = true; render();
    try {
      await request("view_intent", { channelId: sourceChannel, action });
      if (channel() !== sourceChannel) throw new Error("시청 채널이 변경되었습니다");
      if (action === "record_only") {
        const video = videoSource();
        if (video) { video.muted = true; if (video.paused) video.play().catch(() => {}); }
      }
    } catch (error) { tell(error.message); }
    finally { intentBusy = false; render(); }
  };
  const rateAllowed = (video) => {
    if (!video || video.paused || video.ended || video.seeking || !channel()) return false;
    if (state.detail === "saving" || state.detail === "starting") return false;
    // Output-stream recording still needs 1x. Only the exact native-approved
    // encoded session may decouple playback speed from recorded timestamps.
    return !state.recording || window.__atsumiEncodedCapture?.canChangePlaybackRate(video) === true;
  };
  const stopCatchup = () => {
    const previous = catchup; catchup = null;
    // Do not undo an explicit rate choice by the official player or user.
    if (previous && previous.video.playbackRate === 1.2) previous.video.playbackRate = previous.originalRate;
  };
  const stopSelectedRate = () => {
    const previous = selectedRate; selectedRate = null;
    if (previous && previous.video.playbackRate === previous.rate) previous.video.playbackRate = previous.originalRate;
  };
  const updateSelectedRate = (video) => {
    if (!selectedRate) return;
    const ahead = bufferedAhead(video), distance = liveDistance(video);
    if (video !== selectedRate.video || (video?.currentSrc || video?.src || "") !== selectedRate.source ||
        !rateAllowed(video) || document.hidden || video.playbackRate !== selectedRate.rate || ahead === null ||
        (selectedRate.rate > 1 && (ahead <= 1.25 || distance === null || distance <= 1.25))) stopSelectedRate();
  };
  const selectRate = (rate, event) => {
    if (!event.isTrusted || !RATES.includes(rate)) return;
    const video = videoSource(), ahead = bufferedAhead(video), distance = liveDistance(video);
    if (!rateAllowed(video)) { tell("현재 녹화 방식에서는 배속을 사용할 수 없습니다"); return; }
    if (rate !== 1 && (ahead === null || (rate > 1 && (ahead < 2 || distance === null || distance < 2)))) {
      tell("배속 재생에 필요한 영상 버퍼가 부족합니다"); return;
    }
    stopCatchup(); stopSelectedRate();
    if (rate !== 1) selectedRate = { video, rate, originalRate: 1, source: video.currentSrc || video.src || "" };
    preservePitch(video); video.playbackRate = rate;
    rateMenu.hidden = true; render();
  };
  const updateCatchup = (video) => {
    if (!catchup) return;
    const distance = liveDistance(video);
    if (video !== catchup.video || (video?.currentSrc || video?.src || "") !== catchup.source || !rateAllowed(video) || distance === null || distance <= 1 ||
        video.playbackRate !== 1.2 || document.hidden) { stopCatchup(); return; }
    const ahead = bufferedAhead(video);
    if (ahead === null || ahead <= 1.25) stopCatchup();
  };
  const toggleCatchup = (event) => {
    if (!event.isTrusted) return;
    if (catchup) { stopCatchup(); render(); return; }
    const video = videoSource(), distance = liveDistance(video);
    if (!rateAllowed(video)) { tell("현재 녹화 방식에서는 배속을 사용할 수 없습니다"); return; }
    if (distance === null || distance < 2 || video.playbackRate !== 1) {
      tell("이미 최신 지점에 가깝거나 다른 배속이 설정되어 있습니다"); return;
    }
    catchup = { video, originalRate: video.playbackRate, source: video.currentSrc || video.src || "" };
    preservePitch(video); video.playbackRate = 1.2; render();
  };
  const intent = async (action, event) => {
    // Synthetic clicks are ignored; native source, generation and nonce checks
    // remain authoritative. Record start/stop no longer open a confirmation.
    if (!event.isTrusted || intentBusy || !channel()) return;
    intentBusy = true; render();
    try { await request("control_intent", { channelId: channel(), action }); if (action === "screenshot") tell("Atsumi에서 확인해 주세요"); }
    catch (error) { tell(error.message); }
    finally { intentBusy = false; render(); }
  };
  const visibleRect = (node) => {
    if (!node?.isConnected || node.hidden || node.getAttribute("aria-hidden") === "true") return null;
    const rect = node.getBoundingClientRect(), css = window.getComputedStyle(node);
    return [rect.left, rect.top, rect.width, rect.height].every(Number.isFinite) && rect.width > 0 && rect.height > 0 &&
      css.display !== "none" && css.visibility !== "hidden" ? rect : null;
  };
  const layoutTopControls = (nextPlayer) => {
    const box = visibleRect(nextPlayer);
    // Find only the official player's small visible LIVE badge. Its text,
    // attributes, click handler and playback-state meaning remain untouched.
    const live = box ? [...nextPlayer.querySelectorAll("span,strong,em,button,div,[role=status]")]
      .slice(0, 512).filter((node) => node.textContent?.trim() === "LIVE" && !topControls.contains(node))
      .map((node) => ({ node, rect: visibleRect(node) })).filter(({ rect }) => rect && rect.width <= 160 && rect.height <= 80 &&
        rect.left >= box.left && rect.top >= box.top && rect.top <= box.top + Math.min(96, box.height / 3) &&
        rect.right <= box.right && rect.bottom <= box.bottom)
      .sort((a, b) => a.rect.width * a.rect.height - b.rect.width * b.rect.height)[0] : null;
    const anchor = live?.node.closest("button,a,[role=button]") ?? live?.node;
    const host = anchor?.parentElement;
    const css = host ? window.getComputedStyle(host) : null;
    const anchorCss = anchor ? window.getComputedStyle(anchor) : null;
    const inline = host && host !== nextPlayer && host !== document.body && nextPlayer?.contains(host) &&
      !host.closest("button,a,label,[role=button]") && ["flex", "inline-flex"].includes(css.display) &&
      !["column", "column-reverse"].includes(css.flexDirection) && !["absolute", "fixed"].includes(anchorCss.position);
    if (inline) {
      // Insert our node beside (never inside/reparent) the LIVE control. Respect
      // an existing flex gap and leave the original row's styles unchanged.
      topControls.setAttribute("data-inline", "");
      topControls.removeAttribute("data-bootstrap");
      const before = css.flexDirection === "row-reverse" ? anchor.nextSibling : anchor;
      if (topControls.parentElement !== host || (css.flexDirection === "row-reverse" ? topControls.previousSibling !== anchor : topControls.nextSibling !== anchor))
        host.insertBefore(topControls, before);
      const gap = Math.max(0, 8 - (Number.parseFloat(css.columnGap) || 0));
      topControls.style.marginInlineEnd = css.flexDirection === "row-reverse" ? "0px" : `${gap}px`;
      topControls.style.marginInlineStart = css.flexDirection === "row-reverse" ? `${gap}px` : "0px";
      topControls.style.left = ""; topControls.style.top = "";
      topControls.style.maxWidth = "";
      topControls.setAttribute("data-placement", "inline-live");
      const actual = visibleRect(topControls), currentLive = visibleRect(live.node);
      if (actual && currentLive && actual.left >= box.left && actual.right <= box.right &&
          actual.top >= box.top && actual.bottom <= box.bottom &&
          (actual.right <= currentLive.left || actual.left >= currentLive.right)) return;
    }
    // Unknown/absolute layouts get a small viewport-anchored fallback, not
    // injected spacing or a replacement official control strip.
    if (topControls.parentElement !== document.body) document.body.appendChild(topControls);
    topControls.removeAttribute("data-inline");
    topControls.toggleAttribute("data-bootstrap", !box);
    topControls.style.marginInlineStart = ""; topControls.style.marginInlineEnd = "";
    const measured = topControls.getBoundingClientRect();
    const width = measured.width > 0 ? measured.width : 38;
    const height = measured.height > 0 ? measured.height : 38;
    const availableWidth = box?.width ?? window.innerWidth ?? 960;
    let left = 12, top = 12, placement = box ? "player-left" : "page-fallback";
    if (live) {
      left = live.rect.left - box.left - width - 8;
      top = Math.max(8, live.rect.top - box.top + (live.rect.height - height) / 2);
      placement = "before-live";
      if (left < 8) { left = 8; top = live.rect.bottom - box.top + 8; placement = "below-live"; }
    }
    left = Math.max(8, Math.min(left, Math.max(8, availableWidth - width - 8)));
    if (box) {
      const fits = (x, y) => x >= 8 && y >= 8 && x + width <= box.width - 8 && y + height <= box.height - 8;
      const hitsLive = (x, y) => live && x < live.rect.right - box.left + 8 && x + width > live.rect.left - box.left - 8 &&
        y < live.rect.bottom - box.top + 8 && y + height > live.rect.top - box.top - 8;
      top = Math.max(8, Math.min(top, box.height - height - 8));
      if (!fits(left, top) || hitsLive(left, top)) {
        const selected = [
          [8, live ? live.rect.top - box.top - height - 8 : 8, "above-live"],
          [8, live ? live.rect.bottom - box.top + 8 : 8, "below-live"],
          [8, 8, "player-left"], [box.width - width - 8, 8, "player-right"],
          [8, box.height - height - 8, "player-bottom"],
        ].find(([x, y]) => fits(x, y) && !hitsLive(x, y));
        if (selected) [left, top, placement] = selected;
        else topControls.hidden = true;
      }
    }
    const nextLeft = `${Math.round(left + (box?.left ?? 0))}px`, nextTop = `${Math.round(top + (box?.top ?? 0))}px`;
    if (topControls.style.left !== nextLeft) topControls.style.left = nextLeft;
    if (topControls.style.top !== nextTop) topControls.style.top = nextTop;
    if (topControls.getAttribute("data-placement") !== placement) topControls.setAttribute("data-placement", placement);
  };
  const removeControls = () => {
    controls?.remove(); topControls?.remove();
  };
  const scheduleRender = () => {
    if (!pageActive || renderFrame !== null) return;
    renderFrame = window.requestAnimationFrame(() => { renderFrame = null; render(); });
  };
  const render = () => {
    if (!pageActive || !document.body) return;
    const video = videoSource();
    updateSelectedRate(video);
    updateCatchup(video);
    if (!channel()) { stopCatchup(); removeControls(); return; }
    const nextPlayer = video?.closest(".u_rmcplayer,.pzp-pc,.webplayer") ??
      [...document.querySelectorAll(".u_rmcplayer,.pzp-pc,.webplayer")].find((node) => visibleRect(node));
    if (!style) {
      style = document.createElement("style"); style.id = "atsumi-player-ui-style";
      style.textContent = `
        #atsumi-player-controls{display:flex;align-items:center;gap:2px;flex:0 0 auto;pointer-events:auto;z-index:2147483646}
        #atsumi-player-controls[data-floating]{position:absolute;right:12px;bottom:12px;top:auto;opacity:0;transition:opacity .15s;background:#111a;border-radius:6px}
        .u_rmcplayer:hover #atsumi-player-controls,.pzp-pc:hover #atsumi-player-controls,.webplayer:hover #atsumi-player-controls,#atsumi-player-controls:focus-within{opacity:1}
        #atsumi-player-controls button{position:relative;display:inline-flex;align-items:center;justify-content:center;width:36px;height:36px;padding:7px;border:0;border-radius:5px;background:transparent;color:#fff;cursor:pointer;filter:drop-shadow(0 1px 2px #000)}
        #atsumi-player-controls button:hover,#atsumi-player-controls button:focus-visible{background:#ffffff24;color:#00ffa3;outline:1px solid #ffffff80;outline-offset:-2px}
        #atsumi-player-controls button:disabled{opacity:.4;cursor:default}
        #atsumi-player-controls button[data-recording=true]{color:#ff5b70}
        #atsumi-player-controls .atsumi-record-stop{display:none}
        #atsumi-player-controls button[data-recording=true] .atsumi-record-idle{display:none}
        #atsumi-player-controls button[data-recording=true] .atsumi-record-stop{display:inline}
        #atsumi-player-controls button[aria-pressed=true]{color:#00ffa3}
        #atsumi-player-controls button[hidden]{display:none!important}
        #atsumi-player-controls [data-playback-rate]{width:44px;padding:4px;font:12px/1.2 system-ui,sans-serif}
        #atsumi-player-controls [role=menu]{position:absolute;bottom:42px;right:0;display:flex;gap:2px;padding:5px;background:#18201ff5;border:1px solid #ffffff40;border-radius:7px;box-shadow:0 4px 16px #0008}
        #atsumi-player-controls [role=menu][hidden]{display:none!important}
        #atsumi-player-controls [role=menu] button{width:42px;padding:4px;font:12px/1.2 system-ui,sans-serif}
        #atsumi-player-controls [aria-checked=true]{color:#00ffa3;background:#ffffff16}
        #atsumi-player-controls svg{width:22px;height:22px;pointer-events:none}
        #atsumi-player-top-controls{position:fixed;display:flex;align-items:center;gap:6px;max-width:calc(100% - 16px);box-sizing:border-box;pointer-events:auto;z-index:2147483646;padding:3px;border-radius:6px;background:#111b;color:#fff;opacity:0;transition:opacity .15s}
        #atsumi-player-top-controls[data-bootstrap]{position:fixed;opacity:1}
        #atsumi-player-top-controls[data-inline]{position:relative;flex:0 0 auto;align-self:center;inset:auto;max-width:none;padding:0;background:transparent;opacity:1}
        #atsumi-player-top-controls[hidden]{display:none!important}
        body:has(.u_rmcplayer:hover) #atsumi-player-top-controls,body:has(.pzp-pc:hover) #atsumi-player-top-controls,body:has(.webplayer:hover) #atsumi-player-top-controls,#atsumi-player-top-controls:hover,#atsumi-player-top-controls:focus-within{opacity:1}
        #atsumi-player-top-controls button{display:inline-flex;align-items:center;justify-content:center;flex:0 0 auto;min-width:32px;height:32px;padding:0 9px;border:0;border-radius:4px;background:transparent;color:inherit;font:12px/1.3 system-ui,sans-serif;white-space:nowrap;cursor:pointer}
        #atsumi-player-top-controls button:hover,#atsumi-player-top-controls button:focus-visible{background:#ffffff24;outline:1px solid #ffffff80;outline-offset:-1px}
        #atsumi-player-top-controls button:disabled{opacity:.4;cursor:default}
        #atsumi-player-top-controls button[hidden]{display:none!important}
        #atsumi-player-top-controls [data-settings]{width:32px;padding:6px}
        #atsumi-player-top-controls svg{width:20px;height:20px;pointer-events:none}
        body:has(#atsumi-player-controls) #atsumi-browser-capture-status{display:none!important}
        @media(prefers-reduced-motion:reduce){#atsumi-player-controls,#atsumi-player-top-controls{transition:none}}
      `;
      document.head.appendChild(style);
    }
    if (!topControls) {
      topControls = document.createElement("div"); topControls.id = "atsumi-player-top-controls";
      topControls.setAttribute("role", "group"); topControls.setAttribute("aria-label", "시청 보기 설정");
      settings = document.createElement("button"); settings.type = "button";
      settings.setAttribute("aria-label", "시청 설정"); settings.title = "시청 설정 · 로그인 · 고화질 연결";
      settings.setAttribute("data-settings", "");
      settings.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="m9 3-.6 2.2-2 .9L4.3 5.5 2.8 8l1.5 1.7-.2 2.2L2.5 13.5 4 16l2.2-.4 1.8 1.2.3 2.2h3l.8-2.1 2-.8 2 .8 1.7-2.4-1.3-1.8.3-2.2 1.7-1.5L19 6.4l-2.2.2L15 5.3 14.9 3z" transform="translate(1 1)"/><circle cx="12" cy="12" r="3.2"/></svg>';
      settings.addEventListener("click", (event) => { event.stopPropagation(); void viewIntent("open_settings", event); });
      topControls.appendChild(settings);
    }
    settings.disabled = intentBusy;
    settings.title = Date.now() < noticeUntil ? notice : "시청 설정 · 로그인 · 고화질 연결";
    topControls.hidden = context.multiview || context.automaticWatch;
    if (context.multiview) topControls.remove();
    else layoutTopControls(nextPlayer);
    if (!nextPlayer || !video) { controls?.remove(); return; }
    if (!controls) {
      controls = document.createElement("div"); controls.id = "atsumi-player-controls";
      controls.setAttribute("role", "group"); controls.setAttribute("aria-label", "방송 조작");
      const button = (name, shape) => {
        const element = document.createElement("button"); element.type = "button";
        element.setAttribute("aria-label", name); element.title = name;
        // Constant owned SVG only. No page content is interpolated into HTML.
        element.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">${shape}</svg>`;
        controls.appendChild(element); return element;
      };
      record = button("녹화", '<circle cx="12" cy="12" r="8" stroke-width="1.4"/><circle class="atsumi-record-idle" cx="12" cy="12" r="4.5"/><rect class="atsumi-record-stop" x="8" y="8" width="8" height="8" rx="1" fill="currentColor" stroke="none"/>');
      record.addEventListener("click", (event) => { event.stopPropagation(); void intent(state.recording ? "record_stop" : "record_start", event); });
      recordOnly = button("녹화만 계속", '<path d="M3 8h4l5-4v16l-5-4H3zM16 9l6 6m0-6-6 6"/>');
      recordOnly.title = "음소거하고 시청 화면만 닫기 · 영상과 채팅 녹화는 계속됩니다";
      recordOnly.addEventListener("click", (event) => { event.stopPropagation(); void viewIntent("record_only", event); });
      screenshot = button("스크린샷", '<path d="M4 6h4l2-2h4l2 2h4v14H4z"/><circle cx="12" cy="13" r="4"/>');
      screenshot.setAttribute("aria-keyshortcuts", "S");
      screenshot.addEventListener("click", (event) => { event.stopPropagation(); void intent("screenshot", event); });
      speed = button("따라잡기", '<path d="m4 5 8 7-8 7zm9 0 8 7-8 7z"/>');
      speed.addEventListener("click", (event) => { event.stopPropagation(); toggleCatchup(event); });
      rateButton = button("재생 배속", ""); rateButton.setAttribute("data-playback-rate", "");
      rateButton.setAttribute("aria-haspopup", "menu"); rateButton.setAttribute("aria-controls", "atsumi-rate-menu");
      rateMenu = document.createElement("div"); rateMenu.id = "atsumi-rate-menu"; rateMenu.hidden = true;
      rateMenu.setAttribute("role", "menu"); rateMenu.setAttribute("aria-label", "배속 선택");
      for (const rate of RATES) {
        const choice = document.createElement("button"); choice.type = "button"; choice.textContent = `${rate}×`;
        choice.setAttribute("role", "menuitemradio"); choice.setAttribute("aria-label", `${rate}배속`); choice.setAttribute("data-rate", String(rate));
        choice.addEventListener("click", (event) => { event.stopPropagation(); selectRate(rate, event); }); rateMenu.appendChild(choice);
      }
      controls.appendChild(rateMenu);
      rateButton.addEventListener("click", (event) => {
        event.stopPropagation(); if (!event.isTrusted || !rateAllowed(videoSource())) return;
        rateMenu.hidden = !rateMenu.hidden; render();
        if (!rateMenu.hidden) rateMenu.querySelector('[aria-checked="true"]')?.focus();
      });
      rateMenu.addEventListener("keydown", (event) => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); rateMenu.hidden = true; rateButton.focus(); render(); }
        if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
          event.preventDefault(); const choices = [...rateMenu.querySelectorAll("button")];
          const index = choices.indexOf(document.activeElement), delta = event.key === "ArrowRight" ? 1 : -1;
          choices[(index + delta + choices.length) % choices.length]?.focus();
        }
      });
      focus = button("기본 보기", '<path d="M4 9V4h5m6 0h5v5M4 15v5h5m6 0h5v-5"/>');
      focus.addEventListener("click", (event) => { event.stopPropagation(); if (context.multiview) void viewIntent("exit_focus", event); });
    }
    player = nextPlayer;
    // Cheese-PIP uses this official control strip as well. Unknown versions get
    // a small player-local hover strip, never a layout-changing extra row.
    const strip = player.querySelector(".pzp-pc__bottom-buttons-right");
    const parent = strip ?? player;
    if (controls.parentElement !== parent) parent.appendChild(controls);
    controls.toggleAttribute("data-floating", !strip);
    record.disabled = intentBusy || (!state.recording && !state.ready) || state.detail === "saving";
    screenshot.disabled = intentBusy || snapshotBusy || !video;
    // A foreground automatic session uses these exact controls/approval paths.
    // Only off-canvas receivers suppress them.
    record.hidden = context.automaticWatch;
    screenshot.hidden = context.automaticWatch;
    recordOnly.hidden = !state.recording || context.multiview || context.automaticWatch;
    recordOnly.disabled = intentBusy;
    rateButton.disabled = !rateAllowed(video);
    if (rateButton.disabled) rateMenu.hidden = true;
    rateButton.textContent = `${Number.isFinite(video.playbackRate) ? video.playbackRate : 1}×`;
    rateButton.setAttribute("aria-expanded", String(!rateMenu.hidden));
    rateButton.title = Date.now() < noticeUntil ? notice : state.recording && rateButton.disabled ? "영상 재생과 원본 수신 상태를 확인하세요" : "시청 배속 · 원본 녹화 속도에는 영향을 주지 않습니다";
    for (const choice of rateMenu.querySelectorAll("button")) choice.setAttribute("aria-checked", String(Number(choice.getAttribute("data-rate")) === video.playbackRate));
    speed.disabled = !catchup && (!rateAllowed(video) || liveDistance(video) === null);
    speed.setAttribute("aria-pressed", String(Boolean(catchup)));
    const distance = liveDistance(video);
    const latencyText = distance === null ? "재생 가능 끝점 거리 확인 전" : `재생 가능 끝점까지 ${distance.toFixed(1)}초`;
    speed.title = `${catchup ? "1.2배로 따라잡는 중 · 다시 누르면 해제" : "최신 지점까지 1.2배로 따라잡기 · 영상 재다운로드 없음"} · ${latencyText}`;
    focus.hidden = !context.multiview;
    focus.setAttribute("aria-label", "기본 보기");
    focus.title = "넓게 보기 종료 · Esc";
    record.setAttribute("data-recording", String(state.recording));
    const text = state.recording ? "녹화 중지" : "녹화";
    const resolution = video.videoWidth > 0 && video.videoHeight > 0 ? ` · ${video.videoWidth}×${video.videoHeight}` : "";
    record.setAttribute("aria-label", text); record.title = Date.now() < noticeUntil ? notice : `${text}${resolution} · 저장 전에 Atsumi에서 확인합니다`;
    screenshot.title = Date.now() < noticeUntil ? notice : "스크린샷 · S · 저장 전에 Atsumi에서 확인합니다";

  };
  const saveScreenshot = async (command) => {
    if (snapshotBusy || !UUID.test(command.requestId ?? "") || command.channelId !== channel()) return;
    const requestId = command.requestId;
    snapshotBusy = true; render();
    const deadline = Date.now() + 19000;
    const check = () => { if (Date.now() > deadline || command.channelId !== channel()) throw new Error("스크린샷 요청이 만료됐습니다"); };
    let canvas;
    try {
      const video = videoSource();
      if (!video || video.videoWidth > 4096 || video.videoHeight > 4096 || video.videoWidth * video.videoHeight > 16 * 1024 * 1024) throw new Error("저장할 수 없는 영상 크기입니다");
      canvas = document.createElement("canvas"); canvas.width = video.videoWidth; canvas.height = video.videoHeight;
      const context = canvas.getContext("2d");
      if (!context) throw new Error("스크린샷을 만들 수 없습니다");
      context.drawImage(video, 0, 0);
      const blob = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error("스크린샷 생성 시간이 초과됐습니다")), 5000);
        canvas.toBlob((value) => { clearTimeout(timeout); if (value) resolve(value); else reject(new Error("영상이 스크린샷을 허용하지 않습니다")); }, "image/png");
      });
      check();
      if (!blob.size || blob.size > 16 * 1024 * 1024) throw new Error("스크린샷 파일이 너무 큽니다");
      await request("screenshot_begin", { requestId, channelId: channel(), mimeType: "image/png", size: blob.size, width: canvas.width, height: canvas.height });
      const bytes = new Uint8Array(await blob.arrayBuffer());
      for (let offset = 0, chunkIndex = 0; offset < bytes.length; offset += 128 * 1024, chunkIndex++) {
        check(); const part = bytes.subarray(offset, offset + 128 * 1024); let binary = "";
        for (let i = 0; i < part.length; i += 8192) binary += String.fromCharCode(...part.subarray(i, i + 8192));
        await request("screenshot_chunk", { requestId, chunkIndex, data: btoa(binary) });
      }
      check(); await request("screenshot_finish", { requestId }); tell("스크린샷 저장 완료");
    } catch {
      void request("screenshot_abort", { requestId }).catch(() => {});
      tell("스크린샷을 저장하지 못했습니다. Atsumi에서 확인해 주세요");
    } finally {
      if (canvas) { canvas.width = 0; canvas.height = 0; }
      snapshotBusy = false; render();
    }
  };
  window.addEventListener("atsumi-browser-command", (event) => {
    if (event.detail?.kind === "screenshot") void saveScreenshot(event.detail);
  });
  const editing = (event) => {
    const nodes = [...(event.composedPath?.() ?? [event.target]), document.activeElement];
    return nodes.some((node) => node?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"],[role="dialog"],[role="alertdialog"],dialog[open]'));
  };
  document.addEventListener("keydown", (event) => {
    if (!event.isTrusted || event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229 ||
        event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || editing(event) || !channel()) return;
    if ([...document.querySelectorAll('dialog[open],[role="dialog"],[role="alertdialog"],[aria-modal="true"]')].some((node) => {
      if (node.hidden || node.getAttribute("aria-hidden") === "true") return false;
      const box = node.getBoundingClientRect(), css = window.getComputedStyle(node);
      return box.width > 0 && box.height > 0 && css.display !== "none" && css.visibility !== "hidden";
    })) return;
    const isScreenshot = event.code === "KeyS" || event.key?.toLowerCase() === "s";
    if (isScreenshot && !context.automaticWatch && videoSource() && !snapshotBusy && !intentBusy) {
      event.preventDefault(); event.stopPropagation(); void intent("screenshot", event);
    } else if (event.key === "Escape" && context.multiview && !document.fullscreenElement && !intentBusy) {
      event.preventDefault(); event.stopPropagation(); void viewIntent("exit_focus", event);
    }
  }, true);
  window.addEventListener("pagehide", () => {
    pageActive = false;
    if (renderFrame !== null) { window.cancelAnimationFrame(renderFrame); renderFrame = null; }
    stopCatchup(); stopSelectedRate(); removeControls();
  });
  window.addEventListener("pageshow", () => { pageActive = true; scheduleRender(); });
  window.addEventListener("resize", scheduleRender);
  // Capture also observes nested non-bubbling scroll events on the full page.
  window.addEventListener("scroll", scheduleRender, true);
  for (const name of ["seeking", "emptied", "pause", "waiting"]) document.addEventListener(name, (event) => {
    if (event.target === selectedRate?.video) stopSelectedRate();
    if (event.target === catchup?.video) stopCatchup();
  }, true);
  document.addEventListener("visibilitychange", () => { if (document.hidden) { stopCatchup(); stopSelectedRate(); } });
  Object.defineProperty(window, "__atsumiPlayerUI", { value: Object.freeze({
    update: (next) => { state = next; render(); },
    configure: (next) => {
      if (typeof next?.multiview === "boolean") context.multiview = next.multiview;
      if (typeof next?.automaticWatch === "boolean") context.automaticWatch = next.automaticWatch;
      render();
    },
    stopCatchup,
  }) });
  setInterval(render, 1000);
  setInterval(() => { if (selectedRate || catchup) { const video = videoSource(); updateSelectedRate(video); updateCatchup(video); } }, 250);
  document.addEventListener("DOMContentLoaded", render, { once: true });
  render();
})();
