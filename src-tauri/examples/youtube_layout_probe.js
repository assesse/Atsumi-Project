// Diagnostics-only read of this example's own anonymous official documents.
// Never returns page text, dynamic identifiers, URLs, credentials or media bytes.
(() => {
  "use strict";
  if (location.origin !== "https://www.youtube.com" || !["/watch", "/live_chat"].includes(location.pathname)) return { supported: false };
  const number = value => Number.isFinite(value) && Math.abs(value) <= 1e12 ? Math.round(value * 1000) / 1000 : null;
  const rect = element => { const r = element?.getBoundingClientRect(); return r ? { x: number(r.x), y: number(r.y), width: number(r.width), height: number(r.height) } : null; };
  const enumValue = (value, values) => values.includes(value) ? value : "other";
  // Fixed numeric projection only; never return an arbitrary computed CSS string.
  const rgba = value => {
    if (typeof value !== "string" || value.length > 64) return null;
    const s = value.trim().toLowerCase();
    if (s === "transparent") return [0, 0, 0, 0];
    if (s === "white") return [255, 255, 255, 1000];
    if (s === "black") return [0, 0, 0, 1000];
    const hex = /^#([\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$/.exec(s);
    if (hex) {
      const h = hex[1].length <= 4 ? [...hex[1]].map(v => v + v).join("") : hex[1];
      return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16), h.length === 8 ? Math.round(parseInt(h.slice(6, 8), 16) * 1000 / 255) : 1000];
    }
    const rgb = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?\s*\)$/.exec(s);
    if (!rgb) return null;
    const values = rgb.slice(1, 4).map(Number), alpha = rgb[4] === undefined ? 1 : Number(rgb[4]);
    return values.every(v => Number.isFinite(v) && v >= 0 && v <= 255) && Number.isFinite(alpha) && alpha >= 0 && alpha <= 1
      ? [...values.map(Math.round), Math.round(alpha * 1000)] : null;
  };
  const tags = ["HTML", "BODY", "DIV", "VIDEO", "IFRAME", "YTD-APP", "YTD-PAGE-MANAGER", "YTD-WATCH-FLEXY", "YTD-PLAYER", "YT-LIVE-CHAT-APP"];
  const ids = ["movie_player", "columns", "primary", "primary-inner", "player", "player-container", "player-container-outer", "player-container-inner", "full-bleed-container", "cinematics", "chat", "secondary", "secondary-inner"];
  const matrix = value => {
    if (value === "none") return [];
    if (!/^matrix(?:3d)?\([-\d.eE+,\s]+\)$/.test(value ?? "")) return null;
    const values = value.slice(value.indexOf("(") + 1, -1).split(",").map(Number);
    return [6, 16].includes(values.length) && values.every(v => number(v) !== null) ? values.map(number) : null;
  };
  const metrics = element => {
    if (!element) return null;
    const s = getComputedStyle(element);
    return { rect: rect(element), position: enumValue(s.position, ["static", "relative", "absolute", "fixed", "sticky"]),
      display: enumValue(s.display, ["none", "block", "inline", "inline-block", "flex", "grid", "contents"]),
      visibility: enumValue(s.visibility, ["visible", "hidden", "collapse"]), opacity: number(Number(s.opacity)),
      transform: matrix(s.transform), filterActive: !!s.filter && s.filter !== "none", backdropFilterActive: !!s.backdropFilter && s.backdropFilter !== "none",
      perspective: s.perspective === "none" ? 0 : number(parseFloat(s.perspective)),
      containFlags: ["layout", "paint", "size", "style", "content", "strict"].reduce((mask, name, index) => mask | (s.contain?.split(/\s+/).includes(name) ? 1 << index : 0), 0),
      willChangeFlags: ["transform", "filter", "perspective", "contain", "opacity"].reduce((mask, name, index) => mask | (s.willChange?.split(/,\s*/).includes(name) ? 1 << index : 0), 0),
      contentVisibility: enumValue(s.contentVisibility, ["visible", "hidden", "auto"]),
      backgroundTransparent: s.backgroundColor === "rgba(0, 0, 0, 0)" || s.backgroundColor === "transparent",
      backgroundWhite: ["rgb(255, 255, 255)", "rgba(255, 255, 255, 1)"].includes(s.backgroundColor),
      foregroundRgba: rgba(s.color), backgroundRgba: rgba(s.backgroundColor),
      zIndex: s.zIndex === "auto" ? null : number(Number(s.zIndex)) };
  };
  const player = document.getElementById("movie_player"), video = player?.querySelector("video"), chat = document.querySelector("yt-live-chat-app");
  const rootStyle = getComputedStyle(document.documentElement);
  const variables = Object.fromEntries(["x", "y", "w", "h"].map(name => [name, number(parseFloat(rootStyle.getPropertyValue(`--atsumi-youtube-${name}`)))]));
  const style = document.getElementById("atsumi-youtube-original-layout");
  let sheetRules = null; try { if (style?.sheet) sheetRules = number(style.sheet.cssRules.length); } catch { /* Fixed null, no exception text. */ }
  const ancestors = []; let current = player?.parentElement;
  while (current && ancestors.length < 12) { ancestors.push({ tag: enumValue(current.tagName, tags), id: enumValue(current.id, ids), ...metrics(current) }); current = current.parentElement; }
  const knownSiblings = ["#secondary", "#chat", "ytd-watch-metadata", "#below", "ytd-masthead", "ytd-popup-container"];
  const chatMessages = location.pathname === "/live_chat" ? document.querySelectorAll("yt-live-chat-text-message-renderer #message") : null;
  const chatTheme = location.pathname === "/live_chat" ? {
    systemDark: typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)").matches : null,
    tokens: {
      newBackground: rgba(rootStyle.getPropertyValue("--t3e41d7b17b187f69")),
      baselineBackground: rgba(rootStyle.getPropertyValue("--yt-sys-color-baseline--base-background")),
      baselineText: rgba(rootStyle.getPropertyValue("--yt-sys-color-baseline--text-primary")),
      primaryText: rgba(rootStyle.getPropertyValue("--yt-live-chat-primary-text-color")),
    },
    renderer: metrics(document.querySelector("yt-live-chat-renderer")),
    header: metrics(document.querySelector("yt-live-chat-header-renderer")),
    messages: Array.from({ length: Math.min(3, chatMessages?.length ?? 0) }, (_, index) => metrics(chatMessages[index])),
  } : null;
  return { supported: true, documentKind: location.pathname === "/watch" ? "watch" : "chat", bridge: !!window.__atsumiYouTubeOriginal,
    stylePresent: !!style, sheetPresent: !!style?.sheet, sheetRules, variables,
    viewport: { width: number(innerWidth), height: number(innerHeight), dpr: number(devicePixelRatio), visualWidth: number(window.visualViewport?.width), visualHeight: number(window.visualViewport?.height), visualScale: number(window.visualViewport?.scale) },
    rootDark: document.documentElement.hasAttribute("dark"), root: metrics(document.documentElement), body: metrics(document.body),
    chatTheme,
    player: metrics(player), video: metrics(video), chrome: metrics(player?.querySelector(".ytp-chrome-bottom")), chat: metrics(chat),
    currentTime: number(video?.currentTime), readyState: number(video?.readyState), decodedFrames: number(video?.getVideoPlaybackQuality?.().totalVideoFrames),
    ancestors, ancestorTruncated: !!current,
    watchSiblings: knownSiblings.map((selector, index) => ({ kind: index, ...metrics(document.querySelector(selector)) })),
    embeddedChatCount: Math.min(8, document.querySelectorAll("iframe#chatframe").length) };
})();
