// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_chat_enhancements.js?raw";
const CHANNEL = "0123456789abcdef0123456789abcdef";
const NOTICE = "쾌적한 시청 환경을 위해 일부 메시지는 필터링 됩니다. 클린 라이브 채팅 문화 만들기에 동참해 주세요.";
const windows: EventTarget[] = [];
function fixture(initial: string | null = null, url = `https://chzzk.naver.com/live/${CHANNEL}`) {
  const values = new Map<string, string>(initial === null ? [] : [["cleanbot", initial]]);
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: vi.fn((key: string, value: string) => values.set(key, value)) };
  const view = Object.assign(new EventTarget(), { top: null as unknown, matchMedia: () => ({ matches: false }), __atsumiChatEnhancements: undefined as { readViewerCount(): number | null } | undefined });
  view.top = view; windows.push(view);
  new Function("window", "location", "localStorage", source)(view, new URL(url), storage);
  return { api: view.__atsumiChatEnhancements, values, storage, view };
}
beforeEach(() => { vi.useFakeTimers(); document.body.innerHTML = ""; });
afterEach(() => { for (const view of windows.splice(0)) view.dispatchEvent(new Event("pagehide")); document.getElementById("atsumi-chat-enhancement-style")?.remove(); vi.clearAllTimers(); vi.useRealTimers(); });
const row = (rank: number) => `<div class="_item_qxsox_34"><i><span class="blind">${rank}등</span></i><span class="_nickname_qxsox_42">합성 ${rank}</span><span class="_number_qxsox_43">${rank},000</span></div>`;
describe("verified official chat enhancements", () => {
  it("defaults only a missing official CleanBot preference and never alters another origin", () => {
    const fresh = fixture(); expect(fresh.values.get("cleanbot")).toBe("false");
    for (const choice of ["true", "false", "unknown"]) { const saved = fixture(choice); expect(saved.storage.setItem).not.toHaveBeenCalled(); }
    const foreign = fixture(null, `https://evil.test/live/${CHANNEL}`); expect(foreign.api).toBeUndefined(); expect(foreign.storage.setItem).not.toHaveBeenCalled();
  });
  it("hides only the exact official filter guide, not quoted chat, other notices or consent", () => {
    document.body.innerHTML = `<div id="live-chatting"><div class="_container_s1cb2_1 _filter_s1cb2_22"><p>${NOTICE}</p></div><div class="chat-message"><p>${NOTICE}</p></div><div class="_container_s1cb2_1 _filter_s1cb2_22"><p>다른 운영 공지</p></div></div><div role="dialog"><p>${NOTICE}</p></div>`;
    fixture(); expect(document.querySelectorAll("[data-atsumi-filter-guide]")).toHaveLength(1);
    expect(document.querySelector(".chat-message")?.hasAttribute("data-atsumi-filter-guide")).toBe(false);
    expect(document.querySelector('[role="dialog"]')?.hasAttribute("data-atsumi-filter-guide")).toBe(false);
  });
  it("cycles donated top three, then uses the surviving official expand handler", () => {
    document.body.innerHTML = `<div id="live-chatting"><div class="_container_wl8bq_2"><strong class="_title_wl8bq_22">주간 후원 랭킹</strong><button class="_ranking_button_wl8bq_141" aria-expanded="false">${row(1)}${row(2)}${row(3)}</button></div></div>`;
    const official = document.querySelector("button")!; const expanded = vi.fn(() => official.setAttribute("aria-expanded", "true")); official.addEventListener("click", expanded);
    fixture(); const compact = document.querySelector<HTMLButtonElement>(".atsumi-weekly-donation")!;
    const animate = vi.fn(); compact.firstElementChild!.animate = animate;
    expect(compact.textContent).toContain("1등"); vi.advanceTimersByTime(4000); expect(compact.textContent).toContain("2등");
    expect(animate).toHaveBeenCalledOnce();
    compact.dispatchEvent(new Event("mouseenter")); vi.advanceTimersByTime(4000); expect(compact.textContent).toContain("2등");
    compact.click(); expect(expanded).toHaveBeenCalledOnce(); expect(official.style.display).toBe(""); expect(compact.isConnected).toBe(false);
  });
  it("does not call a power ranking donation or invent missing rows", () => {
    document.body.innerHTML = `<div id="live-chatting"><div class="_container_wl8bq_2"><strong class="_title_wl8bq_22">주간 통나무 파워</strong><button class="_ranking_button_wl8bq_141" aria-expanded="false">${row(1)}</button></div></div>`;
    fixture(); expect(document.querySelector(".atsumi-weekly-donation")).toBeNull();
  });
  it("restores official nodes reused for an unrelated notice or non-donation board", () => {
    document.body.innerHTML = `<div id="live-chatting"><div class="_container_s1cb2_1 _filter_s1cb2_22"><p>${NOTICE}</p></div><div class="_container_wl8bq_2"><strong class="_title_wl8bq_22">주간 후원 랭킹</strong><button class="_ranking_button_wl8bq_141" aria-expanded="false">${row(1)}</button></div></div>`;
    fixture(); const official = document.querySelector<HTMLButtonElement>("._ranking_button_wl8bq_141")!;
    expect(official.style.display).toBe("none"); expect(document.querySelector("[data-atsumi-filter-guide]")).not.toBeNull();
    document.querySelector("p")!.textContent = "다른 운영 공지";
    document.querySelector("strong")!.textContent = "주간 통나무 파워";
    vi.advanceTimersByTime(1000);
    expect(official.style.display).toBe(""); expect(document.querySelector(".atsumi-weekly-donation")).toBeNull();
    expect(document.querySelector("[data-atsumi-filter-guide]")).toBeNull();
  });
  it("deduplicates the official shrunk donation marquee and excludes its power rows", () => {
    const boxes = [1,2,3].map((rank) => `<span class="_box_19i63_15"><i class="_icon_ranking_19i63_24 ${rank === 2 ? "_second_19i63_34" : rank === 3 ? "_third_19i63_37" : ""}"></i><span class="_nickname_19i63_41">합성${rank}</span><i class="_icon_cheese_19i63_56"></i><span class="_number_19i63_68">100</span></span>`).join("");
    document.body.innerHTML = `<div id="live-chatting"><div class="_container_wl8bq_2"><button class="_ranking_button_wl8bq_141 _is_shrunk_wl8bq_148" aria-expanded="false">${boxes}${boxes}</button></div></div>`;
    fixture(); const compact = document.querySelector(".atsumi-weekly-donation")!; expect(compact.textContent).toContain("1등"); vi.advanceTimersByTime(8000); expect(compact.textContent).toContain("3등");
  });
  it("samples only the verified live-count element and preserves missing versus real zero", () => {
    document.body.innerHTML = '<div class="_container_17x81_2"><div class="_data_17x81_70"><strong class="_count_17x81_83">1,234명 시청 중</strong><span class="_count_17x81_83">01:20:00 방송 중</span></div></div>';
    const { api } = fixture(); const count = document.querySelector("strong")!;
    expect(api!.readViewerCount()).toBe(1234); count.textContent = "0명 시청 중"; expect(api!.readViewerCount()).toBe(0);
    for (const text of ["1.2만명 시청 중", "조회수 123", "-1명", "100000001명", "알 수 없음"]) { count.textContent = text; expect(api!.readViewerCount()).toBeNull(); }
    count.textContent = "12명 시청 중"; count.parentElement!.classList.add("_type_vod_17x81_76"); expect(api!.readViewerCount()).toBeNull();
  });
});
