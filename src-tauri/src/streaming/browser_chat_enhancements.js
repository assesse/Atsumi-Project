// SPDX-License-Identifier: MIT
// Verified against CHZZK's public index-C4sif-4p.js (2026-09-12). Unknown
// markup stays untouched. No network, React internals, account or chat scraping.
(() => {
  "use strict";
  const channel = () => location.origin === "https://chzzk.naver.com" && /^\/live\/[a-f0-9]{32}\/?$/i.test(location.pathname);
  if (!channel() || window.top !== window || window.__atsumiChatEnhancements) return;
  // The official component reads this exact key at mount, and its own confirm
  // handler persists true/false. Set a default only; never undo a user's choice.
  try { if (localStorage.getItem("cleanbot") === null) localStorage.setItem("cleanbot", "false"); } catch { /* Storage unavailable: leave the official setting alone. */ }
  const NOTICE = "쾌적한 시청 환경을 위해 일부 메시지는 필터링 됩니다. 클린 라이브 채팅 문화 만들기에 동참해 주세요.";
  const collapseSpace = (text) => (text || "").replace(/\s+/g, " ").trim();
  let timer = null, observer = null, queued = false, enabled = true;
  const boards = new Map();
  const restore = (entry) => {
    entry.original.style.removeProperty("display"); entry.original.removeAttribute("data-atsumi-rank-hidden");
    entry.button.remove();
  };
  const ranking = (root) => {
    for (const [original, entry] of boards) {
      if (!original.isConnected || original.getAttribute("aria-expanded") !== "false" || !root?.contains(original)) { restore(entry); boards.delete(original); }
    }
    if (!root) return;
    for (const board of root.querySelectorAll("._container_wl8bq_2")) {
      // Do not convert the separate log-power leaderboard into a donation board.
      const title = board.querySelector("strong._title_wl8bq_22");
      const original = board.querySelector('button._ranking_button_wl8bq_141[aria-expanded="false"]');
      if (!original) continue;
      let rows = title && collapseSpace(title.textContent).includes("주간 후원") ? [...original.querySelectorAll("div._item_qxsox_34")].flatMap((node) => {
        const rank = /^(1|2|3)\s*등$/.exec(collapseSpace(node.querySelector("i > .blind")?.textContent));
        const name = collapseSpace(node.querySelector("._nickname_qxsox_42")?.textContent);
        const amount = collapseSpace(node.querySelector("._number_qxsox_43")?.textContent);
        return rank && name && name.length <= 128 && amount.length <= 32 ? [{ rank: Number(rank[1]), name, amount }] : [];
      }) : [];
      if (!rows.length) {
        // Official shrunk mode duplicates its marquee for a seamless animation.
        // Only cheese-marked donation rows qualify, never log-power values.
        const seen = new Set();
        rows = [...original.querySelectorAll("span._box_19i63_15")].flatMap((node) => {
          if (!node.querySelector("._icon_cheese_19i63_56")) return [];
          const icon = node.querySelector("i._icon_ranking_19i63_24");
          if (!icon) return [];
          const rank = icon.classList.contains("_third_19i63_37") ? 3 : icon.classList.contains("_second_19i63_34") ? 2 : 1;
          const name = collapseSpace(node.querySelector("._nickname_19i63_41")?.textContent);
          const amount = collapseSpace(node.querySelector("._number_19i63_68")?.textContent);
          if (seen.has(rank) || !name || name.length > 128 || amount.length > 32) return [];
          seen.add(rank); return [{ rank, name, amount }];
        });
      }
      rows.sort((a, b) => a.rank - b.rank);
      if (!rows.length || rows.length > 3 || new Set(rows.map((row) => row.rank)).size !== rows.length) {
        const stale = boards.get(original);
        if (stale) { restore(stale); boards.delete(original); }
        continue;
      }
      let entry = boards.get(original);
      if (!entry) {
        if (original.style.display) continue;
        const button = document.createElement("button"); button.type = "button";
        const content = document.createElement("span"); content.style.display = "block"; button.appendChild(content);
        button.className = "atsumi-weekly-donation";
        button.setAttribute("aria-expanded", "false");
        button.title = "주간 후원 순위 펼치기";
        entry = { original, button, content, index: 0, rows, last: Date.now(), paused: false };
        const current = entry;
        button.addEventListener("click", () => {
          if (!channel() || !original.isConnected || original.getAttribute("aria-expanded") !== "false") return;
          // Keep the official React-owned element/handler, not a clone.
          restore(current); boards.delete(original); original.click();
        });
        button.addEventListener("mouseenter", () => { current.paused = true; });
        button.addEventListener("mouseleave", () => { current.paused = false; });
        button.addEventListener("focus", () => { current.paused = true; });
        button.addEventListener("blur", () => { current.paused = false; });
        original.insertAdjacentElement("afterend", button);
        original.style.display = "none"; original.setAttribute("data-atsumi-rank-hidden", "");
        boards.set(original, entry);
      }
      entry.rows = rows; entry.index %= rows.length;
      if (!entry.paused && !document.hidden && !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches && Date.now() - entry.last >= 4000) {
        entry.index = (entry.index + 1) % rows.length; entry.last = Date.now();
      }
      const row = rows[entry.index], label = `${row.rank}등  ${row.name}  ·  ${row.amount} 치즈`;
      if (entry.content.textContent !== label) {
        const changed = !!entry.content.textContent;
        entry.content.textContent = label;
        if (changed && !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches)
          entry.content.animate?.([{ transform: "translateY(10px)", opacity: 0 }, { transform: "translateY(0)", opacity: 1 }], { duration: 240, easing: "ease-out" });
      }
      entry.button.setAttribute("aria-label", `${label} · 주간 후원 전체 순위 펼치기`);
    }
  };
  const render = () => {
    queued = false;
    for (const node of document.querySelectorAll("[data-atsumi-filter-guide]")) {
      if (!channel() || !node.matches("#live-chatting ._container_s1cb2_1._filter_s1cb2_22") ||
          ![...node.querySelectorAll("p")].some((p) => collapseSpace(p.textContent) === NOTICE)) node.removeAttribute("data-atsumi-filter-guide");
    }
    if (!enabled || !channel()) { for (const entry of boards.values()) restore(entry); boards.clear(); return; }
    const root = document.getElementById("live-chatting");
    for (const node of root?.querySelectorAll("._container_s1cb2_1._filter_s1cb2_22") ?? []) {
      // Require the official guide node AND its complete paragraph. A user
      // quoting the notice is not a guide; unrelated notices remain visible.
      if ([...node.querySelectorAll("p")].some((p) => collapseSpace(p.textContent) === NOTICE)) node.setAttribute("data-atsumi-filter-guide", "");
    }
    ranking(root);
  };
  const schedule = () => { if (!queued && enabled) { queued = true; queueMicrotask(render); } };
  const install = () => {
    if (!document.documentElement || observer) return;
    const style = document.getElementById("atsumi-chat-enhancement-style") ?? document.createElement("style"); style.id = "atsumi-chat-enhancement-style";
    style.textContent = '[data-atsumi-filter-guide]{display:none!important}.atsumi-weekly-donation{display:block;width:100%;height:36px;text-align:left;padding:0 12px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;border:0;border-radius:4px;background:transparent;color:inherit;cursor:pointer;font:inherit}.atsumi-weekly-donation:hover,.atsumi-weekly-donation:focus-visible{background:#8882;outline:1px solid #00ffa3}';
    document.documentElement.appendChild(style);
    observer = new MutationObserver(schedule);
    observer.observe(document.documentElement, { childList: true, subtree: true });
    timer = setInterval(render, 1000); render();
  };
  const readViewerCount = () => {
    if (!enabled || !channel()) return null;
    // Public video-info component uses this strong only for OPEN && cvExposure.
    // Exclude VOD, recommendations, uptime spans and abbreviated/unknown text.
    const nodes = document.querySelectorAll("._container_17x81_2 ._data_17x81_70:not(._type_vod_17x81_76) > strong._count_17x81_83");
    if (nodes.length !== 1) return null;
    if ((nodes[0].textContent || "").length > 64) return null;
    const text = collapseSpace(nodes[0].textContent);
    const match = /^(\d{1,3}(?:,\d{3})*|\d+)명(?:\s*시청(?:\s*중)?)?$/.exec(text);
    const value = match ? Number(match[1].replaceAll(",", "")) : NaN;
    return Number.isSafeInteger(value) && value >= 0 && value <= 100000000 ? value : null;
  };
  Object.defineProperty(window, "__atsumiChatEnhancements", { value: Object.freeze({ readViewerCount }) });
  window.addEventListener("pagehide", () => { enabled = false; clearInterval(timer); observer?.disconnect(); observer = null; for (const entry of boards.values()) restore(entry); boards.clear(); });
  window.addEventListener("pageshow", () => { enabled = true; install(); });
  document.addEventListener("DOMContentLoaded", install, { once: true }); install();
})();
