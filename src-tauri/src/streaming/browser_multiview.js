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
  const MODAL = "data-atsumi-mado-dialog";
  const MODAL_PATH = "data-atsumi-mado-dialog-path";
  const DIALOG = '[role="dialog"],[role="alertdialog"],[aria-modal="true"]';
  const SIDE = "data-atsumi-mado-side";
  const LIVE_CHAT = "#aside-chatting";
  let sideRequested = false, sideWasOpen = false;
  const PLAYER_SELECTOR = ".chzzk_player,.u_rmcplayer,.pzp-pc,.webplayer";
  // The official page mounts its pre-roll/startup view before the PZP player.
  // Hiding that slot makes its geometry 0x0 and can strand startup forever.
  // Keep this verified live-player slot (not the site's main/sidebar) visible.
  const STARTUP_SELECTOR = "main ._player_jwedy_23,main #preAdPlayerWrapper";
  const WIDE_BUTTON = 'button.pzp-pc__viewmode-button[aria-label="넓은 화면"]';
  const wideAttempts = new WeakMap();
  let wideRetry = null;
  let expandedPlayer = null;
  let audioEnabled = false;
  let requested = true;
  let active = false;
  let reason = chat ? "chat_only" : "loading";
  let timer = null;
  let style = null;
  let currentPlayer = null;
  let currentVideo = null;
  let placeholder = null;
  const CHAT_HEADER = "._container_1e2su_2";
  const CHAT_TITLE = "h2._title_1e2su_12";
  let chatContext = null, chatHeading = null, chatLabel = null, chatStyle = null;
  let chatLayoutStyle = null, chatScaleControl = null, chatScaleOriginal = null, chatScaleRoot = null;
  let chatScale = 100;
  const clearChatLayout = () => {
    document.body?.removeAttribute("data-atsumi-chat-fit");
    document.body?.removeAttribute("data-atsumi-chat-compact");
    document.body?.style.removeProperty("--atsumi-chat-zoom");
    for (const element of document.querySelectorAll("[data-atsumi-chat-layout]")) element.removeAttribute("data-atsumi-chat-layout");
    chatScaleOriginal?.removeAttribute("data-atsumi-chat-scale-original");
    chatScaleControl?.remove(); chatScaleControl = chatScaleOriginal = chatScaleRoot = null;
  };
  const renderChatLayout = () => {
    if (!chat) return;
    if (!allowed() || !document.body) { clearChatLayout(); return; }
    // The official popup's CSS-module hash changes between site deployments.
    // Keep the exact route/unique ID/type and popup-role token checks, without
    // tying the layout repair to one obsolete build hash.
    const roots = document.querySelectorAll("#aside-chatting");
    if (roots.length !== 1 || roots[0].tagName !== "ASIDE" ||
        ![...roots[0].classList].some(name => /^_is_popup_chat_[a-z0-9]+_\d+$/.test(name))) {
      clearChatLayout(); return;
    }
    const root = roots[0];
    if (!chatLayoutStyle) {
      chatLayoutStyle = document.createElement("style");
      chatLayoutStyle.textContent = `
        html:has(body[data-atsumi-chat-fit]) {min-width:0!important;min-height:0!important;height:100%!important;overflow:hidden!important;}
        body[data-atsumi-chat-fit] {min-width:0!important;min-height:0!important;margin:0!important;width:100%!important;height:calc(100dvh / var(--atsumi-chat-zoom,1))!important;zoom:var(--atsumi-chat-zoom,1);overflow:hidden!important;}
        body[data-atsumi-chat-fit] [data-atsumi-chat-layout] {min-width:0!important;min-height:0!important;width:100%!important;height:100%!important;box-sizing:border-box!important;}
        body[data-atsumi-chat-fit] #aside-chatting {min-width:0!important;min-height:0!important;width:100%!important;height:100%!important;box-sizing:border-box!important;}
        body[data-atsumi-chat-fit] #aside-chatting > ._container_1e2su_2 {flex:none;}
        body[data-atsumi-chat-fit] [data-atsumi-chat-scale-original] {display:none!important;}
        body[data-atsumi-chat-fit] .atsumi-chat-scale {position:absolute;right:40px;top:10px;display:flex;align-items:center;z-index:1001;}
        body[data-atsumi-chat-fit] .atsumi-chat-scale output {min-width:30px;text-align:center;}
        /* Keep every original control reachable at the user's narrow width.
           This changes layout only, never native input/menu ownership or zoom. */
        body[data-atsumi-chat-compact] #aside-chatting > ._container_1e2su_2 {height:72px!important;}
        body[data-atsumi-chat-compact] #aside-chatting > ._container_1e2su_2 > h2 {height:32px!important;padding-right:0!important;}
        body[data-atsumi-chat-compact] [data-atsumi-mado-chat-heading] > .atsumi-mado-chat-channel {height:32px!important;width:calc(50% - 36px)!important;}
        body[data-atsumi-chat-compact] #aside-chatting > ._container_1e2su_2 > ._wrapper_1e2su_28._menu_1e2su_36 {top:32px!important;}
        body[data-atsumi-chat-compact] .atsumi-chat-scale {top:40px!important;}
        body[data-atsumi-chat-compact] #aside-chatting > ._area_nu1j9_49 {padding:6px 8px!important;}
        body[data-atsumi-chat-compact] #aside-chatting ._input_19u4u_59 {max-height:clamp(20px,calc(100dvh / var(--atsumi-chat-zoom,1) - 180px),60px)!important;}
        body[data-atsumi-chat-compact] #aside-chatting ._tools_19u4u_125 {flex-wrap:wrap;align-items:flex-end;gap:6px;}
        body[data-atsumi-chat-compact] #aside-chatting ._donation_19u4u_132 {flex:1 1 90px;min-width:0;flex-wrap:wrap;row-gap:4px;}
        body[data-atsumi-chat-compact] #aside-chatting ._action_19u4u_140 {flex-basis:100%;padding-left:0;}
        body[data-atsumi-chat-compact] #aside-chatting ._action_19u4u_140:before {display:none;}
      `;
      document.documentElement.appendChild(chatLayoutStyle);
    }
    if (!document.body.hasAttribute("data-atsumi-chat-fit")) document.body.setAttribute("data-atsumi-chat-fit", "");
    for (let parent = root.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
      if (!parent.hasAttribute("data-atsumi-chat-layout")) parent.setAttribute("data-atsumi-chat-layout", "");
    }
    const original = root.querySelector(`${CHAT_HEADER} > ._scale_1e2su_39`);
    const originalButtons = original?.querySelectorAll("button");
    if (originalButtons?.length === 2 && (chatScaleOriginal !== original || !chatScaleControl?.isConnected)) {
      chatScaleControl?.remove(); chatScaleOriginal?.removeAttribute("data-atsumi-chat-scale-original");
      chatScaleOriginal = original; chatScaleRoot = root;
      chatScaleControl = document.createElement("div"); chatScaleControl.className = "atsumi-chat-scale";
      const minus = document.createElement("button"), plus = document.createElement("button"), value = document.createElement("output");
      value.className = "_scale_text_1e2su_46"; value.setAttribute("aria-label", "채팅 배율");
      for (const [button, direction, text] of [[minus, -1, "−"], [plus, 1, "+"]]) {
        button.type = "button"; button.className = "_scale_button_1e2su_55"; button.textContent = text;
        button.setAttribute("aria-label", direction < 0 ? "채팅 크기 마이너스" : "채팅 크기 플러스");
        button.title = direction < 0 ? "채팅 축소 (최소 50%)" : "채팅 확대";
        button.addEventListener("click", () => {
          if (!allowed() || chatScaleRoot !== root || chatScaleOriginal !== original) return;
          const nativeScale = Number.parseInt(original.querySelector("._scale_text_1e2su_46")?.textContent || "100", 10);
          if (chatScale < 100 || (direction < 0 && nativeScale === 100)) {
            chatScale = Math.max(50, Math.min(100, chatScale + direction * 10));
            renderChatLayout(); renderChatHeader();
            // Notify the official list/virtualizer of its new logical size.
            window.dispatchEvent(new Event("resize"));
          } else originalButtons[direction < 0 ? 0 : 1].click();
        });
      }
      chatScaleControl.append(minus, value, plus); original.parentElement.appendChild(chatScaleControl);
      original.setAttribute("data-atsumi-chat-scale-original", "");
    }
    const factor = chatScale / 100;
    if (document.body.style.getPropertyValue("--atsumi-chat-zoom") !== String(factor)) document.body.style.setProperty("--atsumi-chat-zoom", String(factor));
    // clientWidth is logical CSS width after the user's selected zoom. Thus
    // 175px at 50% can retain the normal 350px layout instead of double-shrinking.
    const compact = root.clientWidth > 0 && root.clientWidth < 300;
    if (document.body.hasAttribute("data-atsumi-chat-compact") !== compact) document.body.toggleAttribute("data-atsumi-chat-compact", compact);
    if (chatScaleControl?.isConnected) {
      const nativeScale = Number.parseInt(chatScaleOriginal.querySelector("._scale_text_1e2su_46")?.textContent || "100", 10);
      const value = chatScaleControl.querySelector("output"), text = `${chatScale < 100 ? chatScale : nativeScale}%`;
      if (value.textContent !== text) value.textContent = text;
      const minusDisabled = chatScale <= 50;
      const plusDisabled = chatScale === 100 && chatScaleOriginal.lastElementChild.disabled;
      // Reassigning disabled=true emits a mutation even when unchanged. The
      // header observer would otherwise repaint every 150ms at either limit.
      if (chatScaleControl.firstElementChild.disabled !== minusDisabled) chatScaleControl.firstElementChild.disabled = minusDisabled;
      if (chatScaleControl.lastElementChild.disabled !== plusDisabled) chatScaleControl.lastElementChild.disabled = plusDisabled;
    }
  };
  const clearChatHeader = () => {
    chatLabel?.remove();
    chatHeading?.removeAttribute("data-atsumi-mado-chat-heading");
    chatHeading?.style.removeProperty("--atsumi-chat-title-inset");
    chatHeading?.style.removeProperty("--atsumi-chat-label-width");
    chatHeading = null; chatLabel = null;
  };
  const renderChatHeader = () => {
    if (!chat || !chatContext || !allowed()) { clearChatHeader(); return; }
    // Only augment the verified popup-chat heading, never message content.
    // The menu/input and native scale above 100% keep their original owners.
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
    const header = heading.parentElement, bounds = header.getBoundingClientRect(), factor = chatScale / 100;
    const width = bounds.width / factor;
    const fontSize = Number.parseFloat(window.getComputedStyle(heading).fontSize) || 15;
    let right = width;
    for (const control of header.querySelectorAll("._wrapper_1e2su_28,.atsumi-chat-scale")) {
      const rect = control.getBoundingClientRect();
      if (rect.width > 0 && rect.left >= bounds.left + bounds.width / 3) right = Math.min(right, (rect.left - bounds.left) / factor);
    }
    // Keep the original centered title at normal widths. In a narrow chat,
    // reserve room for the original scale/menu controls and ellipsize only the
    // channel name, rather than shrinking either heading's typography.
    const center = Math.min(width / 2, right - fontSize - 8);
    const values = { "--atsumi-chat-title-inset": `${Math.max(0, width - center * 2)}px`, "--atsumi-chat-label-width": `${Math.max(0, center - fontSize - 22)}px` };
    for (const [property, value] of Object.entries(values)) if (heading.style.getPropertyValue(property) !== value) heading.style.setProperty(property, value);
  };
  const configureChat = (value) => {
    if (!chat || !allowed() || value?.channelId !== window.location.pathname.split("/")[2] ||
        !Number.isInteger(value.number) || value.number < 1 || value.number > 4 ||
        typeof value.channelName !== "string" || value.channelName.length > 160 || /[\u0000-\u001f\u007f]/.test(value.channelName)) return;
    chatContext = { number: value.number, channelName: value.channelName.trim() };
    renderChatLayout();
    renderChatHeader();
  };
  const changed = new Map();
  const mark = (element, name) => {
    if (!changed.has(element)) changed.set(element, new Map());
    const previous = changed.get(element);
    if (!previous.has(name)) previous.set(name, element.getAttribute(name));
    if (!element.hasAttribute(name)) element.setAttribute(name, "");
  };
  const reconcile = (desired) => {
    for (const [element, attributes] of changed) {
      for (const [name, previous] of attributes) if (!desired.get(element)?.has(name)) {
        if (previous === null) element.removeAttribute(name);
        else element.setAttribute(name, previous);
        attributes.delete(name);
      }
      if (!attributes.size) changed.delete(element);
    }
    for (const [element, names] of desired) for (const name of names) mark(element, name);
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
    sideRequested = sideWasOpen = false;
    placeholder?.remove(); placeholder = null;
  };
  const showOfficialInformation = (player) => {
    // CHZZK renders avatar/title/channel/viewers/uptime only in its own wide
    // mode. Let that original component populate and update its existing
    // header; CSS alone cannot reveal metadata that the site did not render.
    const button = player.querySelector(WIDE_BUTTON);
    if (!button) { clearTimeout(wideRetry); wideRetry = null; wideAttempts.delete(player); return; }
    // A disabled control may stay disabled throughout the pre-roll. Its
    // eventual disabled/aria-label mutation will schedule a fresh attempt.
    if (button.disabled) return;
    const previous = wideAttempts.get(player);
    const attempt = previous?.button === button ? previous : { count: 0, at: -Infinity };
    // The first click can precede the site's event binding. Verify the native
    // wide-state label, with bounded retries, rather than remembering an
    // attempted click forever. Never toggle an already-wide player.
    if (attempt.count >= 4 || Date.now() - attempt.at < 1000) return;
    wideAttempts.set(player, { button, count: attempt.count + 1, at: Date.now() });
    clearTimeout(wideRetry);
    wideRetry = setTimeout(() => { wideRetry = null; if (requested && allowed() && player.isConnected) schedule(); }, 1200);
    expandedPlayer = player;
    button.click();
  };
  const restoreOfficialView = () => {
    clearTimeout(wideRetry); wideRetry = null;
    const player = expandedPlayer;
    expandedPlayer = null;
    if (!player?.isConnected || !allowed()) return;
    const button = player.querySelector('button.pzp-pc__viewmode-button[aria-label="좁은 화면"]');
    if (button && !button.disabled) {
      wideAttempts.delete(player);
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
  const update = () => {
    timer = null;
    muteAll();
    // Never change chat input, focus, key handlers, DOM parents or navigation.
    // In particular, failure here must not load a full live page.
    if (chat) { renderChatLayout(); renderChatHeader(); return; }
    if (!document.body) return;
    if (!style) {
      style = document.createElement("style");
      style.textContent = `
        @media(prefers-reduced-motion:reduce) {body[${ROOT}] .player_header .header_info {transition:none!important;}}
        body[${ROOT}] {overflow:hidden!important;background:#000!important;transform:none!important;min-width:0!important;}
        body[${ROOT}] > :not([${PATH}]):not([${PLAYER}]):not([${MODAL}]),
        body[${ROOT}] [${PATH}] > :not([${PATH}]):not([${PLAYER}]):not([${MODAL}]) {display:none!important;}
        body[${ROOT}] [${PATH}] {transform:none!important;translate:none!important;rotate:none!important;scale:none!important;filter:none!important;perspective:none!important;overflow:visible!important;clip:auto!important;clip-path:none!important;contain:none!important;}
        body[${ROOT}] [${PLAYER}] {position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;box-sizing:border-box!important;aspect-ratio:auto!important;margin:0!important;padding:0!important;border:0!important;border-radius:0!important;transform:none!important;translate:none!important;rotate:none!important;scale:none!important;z-index:2147483600!important;background:#000!important;}
        body[${ROOT}] [${MEDIA}],body[${ROOT}] [${VIDEO}] {position:absolute!important;inset:0!important;width:100%!important;height:100%!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;box-sizing:border-box!important;aspect-ratio:auto!important;margin:0!important;padding:0!important;border:0!important;transform:none!important;translate:none!important;rotate:none!important;scale:none!important;clip:auto!important;clip-path:none!important;}
        body[${ROOT}] [${MEDIA}] {overflow:visible!important;}
        body[${ROOT}] [${VIDEO}] {object-fit:contain!important;object-position:center!important;}
        body[${ROOT}] [${MODAL}],body[${ROOT}] [${MODAL_PATH}] {z-index:2147483640!important;}
        body[${ROOT}]:has([${SIDE}]) [${PLAYER}] {width:calc(100vw - min(380px, 48vw))!important;right:auto!important;}
        body[${ROOT}] [${SIDE}] {display:flex!important;flex-direction:column!important;position:fixed!important;inset:0 0 0 auto!important;width:min(380px,48vw)!important;max-width:100vw!important;min-width:0!important;height:100dvh!important;min-height:0!important;max-height:100dvh!important;box-sizing:border-box!important;overflow:hidden!important;z-index:2147483641!important;}
        body[${ROOT}] [${SIDE}] > * {min-width:0!important;max-width:100%!important;box-sizing:border-box!important;}
        body[${ROOT}] [${SIDE}] [class*="_popup_"],body[${ROOT}] [${SIDE}] [class*="_layer_"] {min-width:0!important;max-width:100%!important;max-height:100dvh!important;box-sizing:border-box!important;overflow:auto!important;}
        body[${ROOT}] [data-atsumi-mado-placeholder] {display:grid!important;place-items:center;color:#a6abb6;font:14px/1.6 sans-serif;}
        /* Native PZP owns visibility. Its children fade individually; fading
           their parent a second time produces an earlier/darker fade. */
        body[${ROOT}] [${PLAYER}] .player_header .header_info{opacity:0!important;transition:opacity .2s ease-in!important;pointer-events:none!important;}
        body[${ROOT}]:has(.pzp-pc.pzp-pc--controls) [${PLAYER}] .player_header .header_info{opacity:1!important;pointer-events:auto!important;}
        [data-atsumi-redundant-action]{display:none!important;}
        body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons,
        body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-left,
        body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-right {align-items:center!important;}
        @media(max-width:640px) {
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom {left:8px!important;right:8px!important;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons {height:36px!important;flex-wrap:nowrap!important;gap:4px;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-left {flex:0 0 auto;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-right {flex:0 0 auto;flex-wrap:nowrap;gap:2px;margin-left:auto!important;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-right .pzp-button {margin-left:0!important;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-right > button,
          body[${ROOT}] [${PLAYER}] #atsumi-player-controls > button {width:32px!important;height:36px!important;min-width:0!important;}
          body[${ROOT}] [${PLAYER}] #atsumi-player-controls > [data-playback-rate] {width:40px!important;}
        }
        /* Truly small tiles need two rows; both rows stay centered and the
           secondary tools remain reachable without spilling outside the tile. */
        @media(max-width:400px) {
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons {height:auto!important;flex-wrap:wrap!important;row-gap:2px;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-buttons-right {flex:1 1 100%;flex-wrap:wrap;justify-content:flex-end;}
          body[${ROOT}] [${PLAYER}] .pzp-pc__bottom-shadow {height:120px!important;}
        }
        body[${ROOT}] button.pzp-pc__viewmode-button[aria-label="넓은 화면"],
        body[${ROOT}] button.pzp-pc__viewmode-button[aria-label="좁은 화면"] {display:none!important;}
      `;
      document.body.appendChild(style);
    }
    if (!allowed() || !requested) {
      restore(); reason = allowed() ? "original" : "navigation_blocked"; return;
    }
    // Keep the presentation stable across viewport/receiver updates. Tearing
    // these attributes down briefly exposes the site and resets native controls.
    const desired = new Map();
    const want = (element, name) => {
      if (!desired.has(element)) desired.set(element, new Set());
      desired.get(element).add(name);
    };
    const path = element => {
      for (let parent = element.parentElement; parent && parent !== document.body; parent = parent.parentElement) want(parent, PATH);
    };
    want(document.body, ROOT);
    const video = [...document.querySelectorAll("video")]
        .filter((item) => item.isConnected && item.closest(PLAYER_SELECTOR))
        .sort((a, b) => {
          const x = a.getBoundingClientRect(), y = b.getBoundingClientRect();
          return y.width * y.height - x.width * x.height;
        })[0];
    // The official outer player owns controls AND offline/access notices.
    // Do not crop to an inner media wrapper or require a decoded video frame.
    const decodedPlayer = video?.closest(".chzzk_player") || video?.closest(PLAYER_SELECTOR) || document.querySelector(PLAYER_SELECTOR);
    const player = decodedPlayer || document.querySelector(STARTUP_SELECTOR);
    if (player && player !== document.body) {
      placeholder?.remove(); placeholder = null;
      path(player); want(player, PLAYER);
      if (video) {
        for (let parent = video.parentElement; parent && parent !== player; parent = parent.parentElement) want(parent, MEDIA);
        want(video, VIDEO);
      }
      currentPlayer = player; currentVideo = video || null;
      active = true; reason = video ? "video_only" : decodedPlayer ? "player_notice" : "player_preparing";
    } else {
      if (!placeholder) {
        placeholder = document.createElement("div");
        placeholder.setAttribute("data-atsumi-mado-placeholder", "");
        placeholder.setAttribute("role", "status");
        placeholder.textContent = "방송 화면을 기다리고 있습니다.";
        document.body.appendChild(placeholder);
      }
      want(placeholder, PLAYER);
      active = false; reason = "player_unavailable"; currentPlayer = currentVideo = null;
    }
    // Preserve official authentication/quality dialogs without revealing the
    // site's navigation/sidebar. Never click or bypass an access dialog.
    for (const dialog of document.querySelectorAll(DIALOG)) {
      if (dialog.closest('[hidden],[aria-hidden="true"]')) continue;
      if (!player?.contains(dialog)) {
        path(dialog);
        for (let parent = dialog.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
          if (!player || !parent.contains(player)) want(parent, MODAL_PATH);
        }
      }
      want(dialog, MODAL);
    }
    // No second aside inside a video tile. The existing app chat owns these
    // functions; its visibility is toggled by a channel-scoped native intent.
    for (const button of document.querySelectorAll('.player_header .header_info button')) {
      const label = button.getAttribute('aria-label') || button.title || button.textContent || '';
      if (/채팅/.test(label)) { button.setAttribute('data-atsumi-chat-toggle', ''); button.setAttribute('title', 'Atsumi 채팅창 켜기/끄기'); }
      else if (button.closest('[class*="_control_"]') || /후원|미션/.test(label)) button.setAttribute('data-atsumi-redundant-action', '');
    }
    reconcile(desired);
    if (player) showOfficialInformation(player);
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
  document.addEventListener("fullscreenchange", schedule);
  document.addEventListener("click", event => {
    const button = event.target?.closest?.(".player_header .header_info button");
    if (!chat && requested && allowed() && button &&
        (button.closest('[class*="_control_"]') || /채팅|후원|미션/.test(button.getAttribute("aria-label") || button.title || button.textContent || ""))) {
      if (!button.hasAttribute('data-atsumi-chat-toggle')) { event.preventDefault(); event.stopImmediatePropagation(); }
    }
  }, true);
  const structural = (node) => node.nodeType === 1 &&
    (node.matches(`video,${PLAYER_SELECTOR},${STARTUP_SELECTOR},${DIALOG},${WIDE_BUTTON},${LIVE_CHAT}`) || node.querySelector(`video,${PLAYER_SELECTOR},${STARTUP_SELECTOR},${DIALOG},${WIDE_BUTTON},${LIVE_CHAT}`));
  const observer = new MutationObserver((records) => {
    muteAll();
    if (chat) {
      // Ignore high-volume message appends; only layout/header replacement and
      // the original scale label need another presentation pass.
      const rootChanged = records.some(record => record.type === "attributes"
        ? (record.attributeName === "class" || record.attributeName === "id") &&
          (record.target === chatScaleRoot || record.target.id === "aside-chatting")
        : record.type === "childList" && [...record.addedNodes, ...record.removedNodes].some(node =>
          node.nodeType === 1 && (node.matches(LIVE_CHAT) || node.querySelector(LIVE_CHAT))));
      if (!chatScaleRoot?.isConnected || !chatScaleControl?.isConnected ||
          (chatContext && (!chatHeading?.isConnected || !chatLabel?.isConnected)) ||
          rootChanged ||
          records.some(record => chatHeading?.parentElement?.contains(record.target) || chatScaleOriginal?.contains(record.target))) schedule();
      return;
    }
    const stable = active && allowed() && currentPlayer?.isConnected &&
      (!currentVideo || currentVideo.isConnected && currentPlayer.contains(currentVideo));
    if (stable && records.every((record) => {
      const target = record.target;
      // Site progress bars, volume controls and live-chat classes can update
      // many times per second. They must not restore/reapply the entire pane.
      // Existing dialogs may become visible via an ancestor's class/style.
      if (record.type === "attributes") return !(target.matches(DIALOG) || target.querySelector(DIALOG) || target.matches(WIDE_BUTTON) || target.matches(LIVE_CHAT));
      return ![...record.addedNodes, ...record.removedNodes].some(structural);
    })) return;
    if (!stable || records.some((record) => record.type === "attributes" ||
      [...record.addedNodes, ...record.removedNodes].some(structural))) schedule();
  });
  const observe = () => {
    observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true,
      characterData: chat,
      attributeFilter: ["class", "style", "hidden", "role", "aria-modal", "aria-label", "disabled", ...(chat ? ["id"] : [])] });
    schedule();
  };
  if (document.documentElement) observe();
  else document.addEventListener("DOMContentLoaded", observe, { once: true });
  Object.defineProperty(window, "__atsumiMultiView", { value: Object.freeze({
    getState: () => ({ kind: chat ? "chat" : "video", active, reason, audioEnabled }),
    configureChat,
    // Presentation only: preserve the video, chat connection and capture session.
    setVideoOnly: (enabled) => {
      if (chat || typeof enabled !== "boolean" || requested === enabled) return;
      if (requested && !enabled) restoreOfficialView();
      requested = enabled;
      update();
    },
  }) });
})();
