// Read-only public SOOP display observations. No source setters or account data.
(() => {
  "use strict";
  if (window.top !== window || location.origin !== "https://play.sooplive.com" || window.__atsumiSoopAppearanceObserver) return;
  const route = /^\/([A-Za-z0-9_-]{1,40})\/([0-9]{1,20})\/embed\/?$/.exec(location.pathname);
  if (!route) return;
  const names = ["theme", "fontLevel", "randomNicknameColor", "fanMessageColor", "showEmoticons", "animateEmoticons", "smallOgq"];
  const preferences = { fanMessageColor: "st1_fanchatcolor", showEmoticons: "st1_emoticon", animateEmoticons: "st1_emoticon_ani", smallOgq: "st1_ogq" };
  const roles = ["normal", "streamer", "manager", "vip", "subscribe", "fan", "support", "nightbot"];
  const color = raw => {
    if (typeof raw !== "string" || raw.length > 64) return null;
    const value = raw.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "");
    if (/^#[0-9a-f]{6}$/i.test(value)) return value.toLowerCase();
    // CSS variables retain token text. Accept only exact opaque integer RGB,
    // never evaluate arbitrary CSS, clamp, or round a different source color.
    const match = /^rgba?\([\t\n\f\r ]*(\d{1,3})[\t\n\f\r ]*,[\t\n\f\r ]*(\d{1,3})[\t\n\f\r ]*,[\t\n\f\r ]*(\d{1,3})(?:[\t\n\f\r ]*,[\t\n\f\r ]*1(?:\.0+)?)?[\t\n\f\r ]*\)$/i.exec(value)
      ?? /^rgba?\([\t\n\f\r ]*(\d{1,3})[\t\n\f\r ]+(\d{1,3})[\t\n\f\r ]+(\d{1,3})(?:[\t\n\f\r ]*\/[\t\n\f\r ]*1(?:\.0+)?)?[\t\n\f\r ]*\)$/i.exec(value);
    if (!match) return null;
    const components = match.slice(1, 4).map(Number);
    return components.every(component => component <= 255) ? `#${components.map(component => component.toString(16).padStart(2, "0")).join("")}` : null;
  };
  const paletteAt = viewer => {
    try {
      const style = window.getComputedStyle(viewer);
      if (!style || typeof style.getPropertyValue !== "function") return null;
      const read = key => { try { return color(style.getPropertyValue(key)); } catch { return null; } };
      return { version: 1, basis: "computed-css-variable",
        nickname: Array.from({ length: 20 }, (_, index) => read(`--nickname-ramdom-color${index + 1}`)),
        roles: Object.fromEntries(roles.map(role => [role, read(`--${role}-color`)])) };
    } catch { return null; }
  };
  const settingsSelector = "#st1_fanchatcolor,#st1_emoticon,#st1_emoticon_ani,#st1_ogq,#st1_randomnickname,.chat_layer.setting_chat .font-level button,#setbox_set";
  const surfaceAt = viewer => {
    const result = { version: 1, basis: "computed-dom", foreground: null, background: null };
    const plain = style => style.display !== "none" && style.visibility === "visible" && style.opacity === "1"
      && style.filter === "none" && (style.backdropFilter || "none") === "none" && style.mixBlendMode === "normal";
    const visible = node => { const rect = node.getBoundingClientRect(); return rect.width > 0 && rect.height > 0; };
    try {
      if (!visible(viewer)) return result;
      let background = null, node = viewer, depth = 0, plainAncestors = true, simpleBackground = true;
      for (; node && depth < 12; node = node.parentElement, depth++) {
        const style = window.getComputedStyle(node);
        if (!plain(style)) plainAncestors = false;
        // Effects above the first opaque layer still affect its displayed color.
        if (background !== null) continue;
        if (style.backgroundImage !== "none" || style.backgroundBlendMode !== "normal") simpleBackground = false;
        const opaque = color(style.backgroundColor);
        if (opaque !== null) background = opaque;
        else if (!/^rgba\(\s*0\s*,\s*0\s*,\s*0\s*,\s*0\s*\)$/.test(style.backgroundColor)) simpleBackground = false;
      }
      if (node || !plainAncestors) return result;
      if (simpleBackground) result.background = background;
      const colors = [];
      // Fixed ordinary structure only; no text, previews or translated .msg.
      for (let row = viewer.lastElementChild, scanned = 0; row && scanned < 32 && colors.length < 3; row = row.previousElementSibling, scanned++) {
        if (!row.classList.contains("chatting-list-item") || row.getAttribute("user-type") === "fan") continue;
        const msg = row.querySelector(":scope > .message-container > .message-text > .msg");
        if (!msg || !visible(msg)) continue;
        let eligible = true;
        for (let part = msg; part && part !== viewer; part = part.parentElement) {
          if (part.style.color || !plain(window.getComputedStyle(part))) { eligible = false; break; }
        }
        if (!eligible) continue;
        const value = color(window.getComputedStyle(msg).color);
        if (value === null) return result;
        colors.push(value);
      }
      if (colors.length && colors.every(value => value === colors[0])) result.foreground = colors[0];
    } catch { /* A missing or ambiguous DOM observation remains unknown. */ }
    return result;
  };
  const unknown = () => Object.fromEntries(names.map(name => [name, null]));
  Object.defineProperty(window, "__atsumiSoopAppearanceObserver", { configurable: false, value: Object.freeze({
    create({ binding, emit }) {
      if (typeof binding !== "function" || typeof emit !== "function") return null;
      let epoch = 0, sequence = 0, ready = false, alive = true, last = null, frame = 0, observed = [];
      const source = () => {
        try {
          const value = binding();
          return value?.actualChannel === route[1].toLowerCase() && value.actualBroadcast === route[2] ? value : null;
        } catch { return null; }
      };
      const node = id => { const value = document.getElementById(id); return value?.isConnected ? value : null; };
      const targets = () => {
        const result = [], add = value => { if (value?.isConnected && !result.includes(value)) result.push(value); };
        add(document.documentElement); add(document.body);
        // Relevant ancestors can hold the official dark-mode class. This is a
        // fixed-depth attribute observation, never a chat-content subtree scan.
        for (const id of ["chatting_area", "chat_area"]) {
          let value = node(id);
          for (let depth = 0; value && depth < 16; depth++, value = value.parentElement) add(value);
        }
        return result;
      };
      const cancelFrame = () => { if (frame) window.cancelAnimationFrame(frame); frame = 0; };
      const schedule = () => {
        if (!alive || !epoch || !ready || frame) return;
        const scheduledEpoch = epoch;
        frame = window.requestAnimationFrame(() => { frame = 0; if (alive && epoch === scheduledEpoch) sample(ready); });
      };
      const observer = new window.MutationObserver(schedule);
      const refreshTargets = () => {
        const next = targets();
        if (next.length === observed.length && next.every((value, index) => value === observed[index])) return;
        observer.disconnect(); observed = next;
        for (const target of observed) observer.observe(target, { attributes: true, attributeFilter: ["dark", "class", "font-level"] });
      };
      function sample(isReady) {
        if (!alive || !epoch) return false;
        ready = isReady === true;
        const actual = source();
        if (!actual) { last = null; observer.disconnect(); observed = []; cancelFrame(); return false; }
        const values = unknown(), basis = unknown(); let palette = null, surface = null;
        if (ready) {
          refreshTargets();
          const area = node("chatting_area"), viewer = node("chat_area");
          // Explicit dark markers override a light ancestor in official CSS.
          if (area && viewer) {
            if (observed.some(value => value.getAttribute("dark") === "true" || ["thema_dark", "smode", "chat_thema_dark"].some(name => value.classList.contains(name)))) values.theme = "dark";
            else if (observed.some(value => value.getAttribute("dark") === "false")) values.theme = "light";
            if (values.theme !== null) basis.theme = "dom";
            const level = area.getAttribute("font-level");
            if (/^[1-8]$/.test(level ?? "")) { values.fontLevel = Number(level); basis.fontLevel = "dom"; }
            values.randomNicknameColor = viewer.classList.contains("random-color-on"); basis.randomNicknameColor = "dom";
            palette = paletteAt(viewer);
            surface = surfaceAt(viewer);
          }
          // Official checkboxes are initialized only when opening settings.
          // Read four explicit preference booleans, never initial checked=false
          // or a guessed default; no other storage value leaves this function.
          try {
            const raw = window.localStorage.getItem("chatOptionSetting");
            if (typeof raw === "string" && raw.length <= 4096) {
              const saved = JSON.parse(raw);
              if (saved && typeof saved === "object" && !Array.isArray(saved)) {
                for (const [name, key] of Object.entries(preferences)) if (Object.prototype.hasOwnProperty.call(saved, key) && typeof saved[key] === "boolean") {
                  values[name] = saved[key]; basis[name] = "saved-option";
                }
              }
            }
          } catch { /* Unavailable, malformed or oversized history stays unknown. */ }
        }
        const encoded = JSON.stringify({ values, basis, palette, surface });
        if (encoded === last || sequence >= Number.MAX_SAFE_INTEGER) return false;
        const fields = { sequence: ++sequence, values, basis, palette, surface, actualChannel: actual.actualChannel, actualBroadcast: actual.actualBroadcast };
        try { if (emit(fields, epoch) === false) return false; } catch { return false; }
        last = encoded;
        return true;
      }
      const onSetting = event => {
        // The official li handler stops propagation and writes .checked (not
        // an attribute). Capture only schedules a read after that handler.
        const target = event.target?.closest?.(settingsSelector);
        if (target) schedule();
        else if (event.target?.closest?.(".chat_layer.sub li")) schedule();
      };
      document.addEventListener("click", onSetting, true);
      document.addEventListener("change", onSetting, true);
      return Object.freeze({
        configure(value) {
          if (!alive || !Number.isSafeInteger(value) || value < 1) return false;
          if (value === epoch) return true;
          cancelFrame(); observer.disconnect(); observed = []; epoch = value; sequence = 0; ready = false; last = null;
          sample(false);
          return true;
        },
        sample,
        dispose() {
          if (!alive) return;
          alive = false; ready = false; cancelFrame(); observer.disconnect(); observed = []; last = null;
          document.removeEventListener("click", onSetting, true);
          document.removeEventListener("change", onSetting, true);
        },
      });
    },
  }) });
})();
