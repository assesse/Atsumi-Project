// SPDX-License-Identifier: MIT
// Watch-only presentation: no capture, IPC, network interception or chat logging.
(() => {
  "use strict";
  const allowed = () => window.location.origin === "https://chzzk.naver.com" &&
    !window.location.search && !window.location.hash &&
    /^\/live\/[a-f0-9]{32}(\/chat)?$/.test(window.location.pathname);
  if (!allowed() || window.top !== window || window.__atsumiMultiView) return;
  const chat = window.location.pathname.endsWith("/chat");
  const ROOT = "data-atsumi-mado";
  const PATH = "data-atsumi-mado-path";
  const PLAYER = "data-atsumi-mado-player";
  const MEDIA = "data-atsumi-mado-media";
  const VIDEO = "data-atsumi-mado-video";
  const DIALOG = '[role="dialog"],[role="alertdialog"],[aria-modal="true"]';
  const WIDE_BUTTON = 'button.pzp-pc__viewmode-button[aria-label="넓은 화면"]';
  const wideAttempts = new WeakSet();
  let expandedPlayer = null;
  let audioEnabled = false;
  let requested = true;
  let active = false;
  let reason = chat ? "chat_only" : "loading";
  let timer = null;
  let style = null;
  let toggle = null;
  let currentPlayer = null;
  let currentVideo = null;
  const CHAT_HEADER = "._container_1e2su_2";
  const CHAT_TITLE = "h2._title_1e2su_12";
  let chatContext = null, chatHeading = null, chatLabel = null, chatStyle = null;
  const clearChatHeader = () => {
    chatLabel?.remove();
    chatHeading?.removeAttribute("data-atsumi-mado-chat-heading");
    chatHeading?.style.removeProperty("--atsumi-chat-title-inset");
    chatHeading?.style.removeProperty("--atsumi-chat-label-width");
    chatHeading = null; chatLabel = null;
  };
  const renderChatHeader = () => {
    if (!chat || !chatContext || !allowed()) { clearChatHeader(); return; }
    // Only augment the verified official popup-chat heading, never message
    // content. Keep its original text, scaling/menu controls and event owners.
    const titles = document.querySelectorAll(`${CHAT_HEADER} > ${CHAT_TITLE}`);
    if (titles.length !== 1) { clearChatHeader(); return; }
    const heading = titles[0];
    if (chatHeading !== heading || !chatLabel?.isConnected) {
      clearChatHeader(); chatHeading = heading;
      chatLabel = document.createElement("span");
      chatLabel.className = "atsumi-mado-chat-channel";
      heading.appendChild(chatLabel);
      heading.setAttribute("data-atsumi-mado-chat-heading", "");
    }
    if (!chatStyle) {
      chatStyle = document.createElement("style");
      chatStyle.textContent = `
        [data-atsumi-mado-chat-heading] {padding-left:0!important;padding-right:var(--atsumi-chat-title-inset,0px)!important;}
        [data-atsumi-mado-chat-heading] > .atsumi-mado-chat-channel {position:absolute;left:12px;top:0;height:100%;width:var(--atsumi-chat-label-width,calc(50% - 42px));display:block;align-content:center;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:left;font:inherit;letter-spacing:inherit;color:inherit;}
      `;
      document.documentElement.appendChild(chatStyle);
    }
    const name = `${chatContext.number}.${chatContext.channelName || "방송"}`;
    if (chatLabel.textContent !== name) { chatLabel.textContent = name; chatLabel.title = name; }
    const header = heading.parentElement, bounds = header.getBoundingClientRect();
    const fontSize = Number.parseFloat(window.getComputedStyle(heading).fontSize) || 15;
    let right = bounds.width;
    for (const control of header.querySelectorAll("._wrapper_1e2su_28")) {
      const rect = control.getBoundingClientRect();
      if (rect.width > 0 && rect.left >= bounds.left + bounds.width / 3) right = Math.min(right, rect.left - bounds.left);
    }
    // Keep the original centered title at normal widths. In a narrow chat,
    // reserve room for the original scale/menu controls and ellipsize only the
    // channel name, rather than shrinking either heading's typography.
    const center = Math.min(bounds.width / 2, right - fontSize - 8);
    const values = { "--atsumi-chat-title-inset": `${Math.max(0, bounds.width - center * 2)}px`, "--atsumi-chat-label-width": `${Math.max(0, center - fontSize - 22)}px` };
    for (const [property, value] of Object.entries(values)) if (heading.style.getPropertyValue(property) !== value) heading.style.setProperty(property, value);
  };
  const configureChat = (value) => {
    if (!chat || !allowed() || value?.channelId !== window.location.pathname.split("/")[2] ||
        !Number.isInteger(value.number) || value.number < 1 || value.number > 4 ||
        typeof value.channelName !== "string" || value.channelName.length > 160 || /[\u0000-\u001f\u007f]/.test(value.channelName)) return;
    chatContext = { number: value.number, channelName: value.channelName.trim() };
    renderChatHeader();
  };
  const changed = new Map();
  const mark = (element, name) => {
    if (!changed.has(element)) changed.set(element, new Map());
    const previous = changed.get(element);
    if (!previous.has(name)) previous.set(name, element.getAttribute(name));
    element.setAttribute(name, "");
  };
  const restore = () => {
    for (const [element, attributes] of changed) for (const [name, previous] of attributes) {
      if (previous === null) element.removeAttribute(name);
      else element.setAttribute(name, previous);
    }
    changed.clear();
    active = false;
    currentPlayer = null;
    currentVideo = null;
  };
  const showOfficialInformation = (player) => {
    // CHZZK renders avatar/title/channel/viewers/uptime only in its own wide
    // mode. Let that original component populate and update its existing
    // header; CSS alone cannot reveal metadata that the site did not render.
    const button = player.querySelector(WIDE_BUTTON);
    if (!button || button.disabled || wideAttempts.has(button)) return;
    wideAttempts.add(button);
    expandedPlayer = player;
    button.click();
  };
  const restoreOfficialView = () => {
    const player = expandedPlayer;
    expandedPlayer = null;
    if (!player?.isConnected || !allowed()) return;
    const button = player.querySelector('button.pzp-pc__viewmode-button[aria-label="좁은 화면"]');
    if (button && !button.disabled) {
      wideAttempts.delete(button);
      button.click();
    }
  };
  const mute = (element) => {
    if (!element || !/^(VIDEO|AUDIO)$/.test(element.tagName)) return;
    // The host can deny audio while loading/closing, and chat is always silent.
    // Once allowed, the official volume UI owns muted/volume: never undo the
    // user's mute choice on play, volumechange, resize or a DOM update.
    if ((chat || !audioEnabled || !allowed()) && !element.muted) element.muted = true;
    if (chat && !element.paused) element.pause();
  };
  const muteAll = () => { for (const element of document.querySelectorAll("video,audio")) mute(element); };
  const visible = (element) => {
    const rect = element.getBoundingClientRect();
    const css = window.getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && css.display !== "none" && css.visibility !== "hidden";
  };
  const update = () => {
    timer = null;
    muteAll();
    // Never change chat input, focus, key handlers, DOM parents or navigation.
    // In particular, failure here must not load a full live page.
    if (chat) { renderChatHeader(); return; }
    if (!document.body) return;
    if (!style) {
      style = document.createElement("style");
      style.textContent = `
        #atsumi-mado-toggle {opacity:0;pointer-events:none;transition:opacity .16s ease;}
        body:hover #atsumi-mado-toggle,#atsumi-mado-toggle:focus-visible {opacity:1;pointer-events:auto;}
        @media(prefers-reduced-motion:reduce) {#atsumi-mado-toggle,body[${ROOT}] .player_header .header_info {transition:none!important;}}
        body[${ROOT}] {overflow:hidden!important;background:#000!important;transform:none!important;min-width:0!important;}
        body[${ROOT}] > :not([${PATH}]):not([${PLAYER}]):not(#atsumi-mado-toggle),
        body[${ROOT}] [${PATH}] > :not([${PATH}]):not([${PLAYER}]) {display:none!important;}
        body[${ROOT}] [${PATH}] {transform:none!important;translate:none!important;rotate:none!important;scale:none!important;filter:none!important;perspective:none!important;overflow:visible!important;clip:auto!important;clip-path:none!important;contain:none!important;}
        body[${ROOT}] [${PLAYER}] {position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;box-sizing:border-box!important;aspect-ratio:auto!important;margin:0!important;padding:0!important;border:0!important;border-radius:0!important;transform:none!important;translate:none!important;rotate:none!important;scale:none!important;z-index:2147483600!important;background:#000!important;}
        body[${ROOT}] [${MEDIA}],body[${ROOT}] [${VIDEO}] {position:absolute!important;inset:0!important;width:100%!important;height:100%!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;box-sizing:border-box!important;aspect-ratio:auto!important;margin:0!important;padding:0!important;border:0!important;transform:none!important;translate:none!important;rotate:none!important;scale:none!important;clip:auto!important;clip-path:none!important;}
        body[${ROOT}] [${MEDIA}] {overflow:visible!important;}
        body[${ROOT}] [${VIDEO}] {object-fit:contain!important;object-position:center!important;}
        body[${ROOT}] [${PLAYER}]:focus-within .player_header .header_info {opacity:1!important;}
      `;
      document.body.appendChild(style);
      toggle = document.createElement("button");
      toggle.id = "atsumi-mado-toggle";
      toggle.type = "button";
      toggle.style.cssText = "position:fixed;right:8px;top:8px;z-index:2147483647;background:#18212ed9;color:white;border:1px solid #526170;border-radius:5px;font:11px system-ui;padding:4px 6px";
      toggle.addEventListener("click", () => { requested = !requested; if (!requested) restoreOfficialView(); update(); });
      document.body.appendChild(toggle);
    }
    restore();
    reason = "original";
    if (!allowed()) reason = "navigation_blocked";
    else if (!requested) reason = "original";
    else if (window.innerWidth < 160 || window.innerHeight < 90) reason = "viewport_too_small";
    else if ([...document.querySelectorAll(DIALOG)].some(visible)) reason = "dialog_visible";
    else {
      const video = [...document.querySelectorAll("video")]
        .filter((item) => item.isConnected && item.readyState >= 1 && item.videoWidth > 0 && visible(item))
        .sort((a, b) => {
          const x = a.getBoundingClientRect(), y = b.getBoundingClientRect();
          return y.width * y.height - x.width * x.height;
        })[0];
      const player = video?.closest(".u_rmcplayer,.pzp-pc,.webplayer");
      if (!player || player === document.body) reason = "player_unavailable";
      else {
        for (let parent = player.parentElement; parent && parent !== document.body; parent = parent.parentElement) mark(parent, PATH);
        for (let parent = video.parentElement; parent && parent !== player; parent = parent.parentElement) mark(parent, MEDIA);
        mark(player, PLAYER); mark(video, VIDEO); mark(document.body, ROOT);
        currentPlayer = player; currentVideo = video;
        active = true; reason = "video_only";
        showOfficialInformation(player);
      }
    }
    const text = active ? "전체 보기" : "영상만";
    if (toggle.textContent !== text) toggle.textContent = text;
    toggle.setAttribute("aria-pressed", String(active));
  };
  const schedule = () => { if (timer === null) timer = setTimeout(update, 150); };
  window.addEventListener("atsumi-multiview-audio", (event) => {
    if (typeof event.detail?.enabled !== "boolean") return;
    audioEnabled = !chat && event.detail.enabled;
    // Only an explicit host audio command overrides the media's mute state.
    // Initial readiness merely opens the gate, preserving the original UI.
    if (event.detail.applyToMedia === true && allowed()) {
      for (const element of document.querySelectorAll("video,audio")) {
        if (element.muted !== !audioEnabled) element.muted = !audioEnabled;
      }
    }
    muteAll();
  });
  for (const event of ["play", "playing", "loadedmetadata", "volumechange"]) {
    document.addEventListener(event, (event) => { mute(event.target); if (event.type === "loadedmetadata") schedule(); }, true);
  }
  window.addEventListener("resize", schedule);
  const structural = (node) => node.nodeType === 1 &&
    (node.matches(`video,${DIALOG},${WIDE_BUTTON}`) || node.querySelector(`video,${DIALOG},${WIDE_BUTTON}`));
  const observer = new MutationObserver((records) => {
    muteAll();
    if (chat) {
      if (chatContext && (!chatHeading?.isConnected || !chatLabel?.isConnected || records.some(record => chatHeading.parentElement?.contains(record.target)))) schedule();
      return;
    }
    const stable = active && allowed() && currentPlayer?.isConnected &&
      currentVideo?.isConnected && currentPlayer.contains(currentVideo);
    if (stable && records.every((record) => {
      const target = record.target;
      if (target === toggle || toggle?.contains(target)) return true;
      // Site progress bars, volume controls and live-chat classes can update
      // many times per second. They must not restore/reapply the entire pane.
      // Existing dialogs may become visible via an ancestor's class/style.
      if (record.type === "attributes") return !(target.matches(DIALOG) || target.querySelector(DIALOG));
      return ![...record.addedNodes, ...record.removedNodes].some(structural);
    })) return;
    if (!stable || records.some((record) => record.type === "attributes" ||
      [...record.addedNodes, ...record.removedNodes].some(structural))) schedule();
  });
  const observe = () => {
    observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true,
      attributeFilter: ["class", "style", "hidden", "role", "aria-modal"] });
    schedule();
  };
  if (document.documentElement) observe();
  else document.addEventListener("DOMContentLoaded", observe, { once: true });
  Object.defineProperty(window, "__atsumiMultiView", { value: Object.freeze({
    getState: () => ({ kind: chat ? "chat" : "video", active, reason, audioEnabled }),
    configureChat,
    // Presentation only: preserve the video, chat connection and capture session.
    setVideoOnly: (enabled) => {
      if (chat || typeof enabled !== "boolean") return;
      if (requested && !enabled) restoreOfficialView();
      requested = enabled;
      update();
    },
  }) });
})();
