import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { SoopDiscovery } from "./SoopDiscovery";
import type { SoopApi, SoopSearchItem } from "../../api/soop";
import type { PlatformMediaApi } from "../platforms/platformMedia";
let root: Root, host: HTMLDivElement;
beforeEach(() => { localStorage.clear(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const items: SoopSearchItem[] = [1, 2, 3].map(id => ({ id: String(id), kind: "video", title: `영상 ${id}`, channelId: "artist", channelName: "작가", thumbnailKey: null, duration: "0:28", views: 100, restricted: false }));
async function setup() {
  const search = vi.fn<NonNullable<SoopApi["search"]>>().mockResolvedValue({ items, page: 1, hasMore: true });
  const download = vi.fn().mockResolvedValue({ jobId: "new" });
  const api: SoopApi = { inspect: vi.fn(), search };
  await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} mediaApi={{ download } as unknown as PlatformMediaApi} />));
  return { search, download };
}
async function query(value: string) {
  await act(async () => { const input = host.querySelector("input")!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
}
const click = async (node: Element, init: MouseEventInit = {}) => act(async () => node.dispatchEvent(new MouseEvent("click", { bubbles: true, ...init })));
async function chooseKind(value: string) {
  await act(async () => { const select = host.querySelector<HTMLSelectElement>('[aria-label="SOOP 검색 종류"]')!; select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); });
}
describe("SOOP discovery interactions", () => {
  it("reopens existing saved videos in the search filter without a new page or search request", async () => {
    const stored = JSON.stringify(items.slice(0, 2)); localStorage.setItem("atsumi:soop:video-favorites:v1", stored);
    const { search, download } = await setup(); const select = host.querySelector("select"), input = host.querySelector("input");
    await chooseKind("saved");
    expect(host.querySelector("select")).toBe(select); expect(host.querySelector("input")).toBe(input);
    expect(host.querySelectorAll("article")).toHaveLength(2); expect(search).not.toHaveBeenCalled();
    expect(localStorage.getItem("atsumi:soop:video-favorites:v1")).toBe(stored);
    await act(async () => host.querySelector("article")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(host.querySelector('[role="dialog"]')).toHaveTextContent("영상 1");
    await click([...host.querySelectorAll('.soop-detail-actions button')].find(button => button.textContent === "다운로드")!);
    expect(download).toHaveBeenCalledWith({ provider: "soop", kind: "video", externalId: "1" }, expect.any(String));
    await click(host.querySelector('.soop-detail-close')!);
    await click(host.querySelector(".soop-result-favorite")!);
    expect(host.querySelectorAll("article")).toHaveLength(1);
    expect(JSON.parse(localStorage.getItem("atsumi:soop:video-favorites:v1")!)).toHaveLength(1);
    expect(search).not.toHaveBeenCalled();
  });
  it("searches saved videos locally on submit and preserves the remote query and results", async () => {
    localStorage.setItem("atsumi:soop:video-favorites:v1", JSON.stringify(items));
    const { search } = await setup(); await query("음악"); const tab = host.querySelector('[role="tab"]');
    await click(host.querySelector("article")!);
    await chooseKind("saved"); expect(host.querySelector(".soop-result.is-focused")).toBeNull();
    await query("영상 2"); expect(host.querySelectorAll("article")).toHaveLength(1); expect(host.querySelector("article")).toHaveTextContent("영상 2");
    expect(search).toHaveBeenCalledTimes(1);
    await query(""); expect(host.querySelectorAll("article")).toHaveLength(3);
    await chooseKind("live"); expect(host.querySelector("input")).toHaveValue("음악");
    expect(host.querySelector('[role="tab"]')).toBe(tab); expect(host.querySelectorAll("article")).toHaveLength(3);
    expect(search).toHaveBeenCalledTimes(1);
  });
  it("keeps remote errors and loading outside the saved-video list", async () => {
    const { search } = await setup(); search.mockRejectedValueOnce("연결 실패"); await query("음악");
    expect(host.querySelector('[role="alert"]')).toHaveTextContent("연결 실패");
    await chooseKind("saved"); expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host).toHaveTextContent("영상의 별표를 눌러 저장하세요.");
    await chooseKind("live"); expect(host.querySelector('[role="alert"]')).toHaveTextContent("연결 실패");
  });
  it("restores separate scroll positions for the search and saved lists", async () => {
    host.className = "soop-workspace";
    localStorage.setItem("atsumi:soop:video-favorites:v1", JSON.stringify(items));
    await setup(); await query("음악");
    host.scrollTop = 340; host.dispatchEvent(new Event("scroll"));
    await chooseKind("saved"); expect(host.scrollTop).toBe(0);
    host.scrollTop = 75; host.dispatchEvent(new Event("scroll"));
    await chooseKind("live"); expect(host.scrollTop).toBe(340);
    await chooseKind("saved"); expect(host.scrollTop).toBe(75);
  });
  it("distinguishes live viewers, video views, zero and unknown counts", async () => {
    const { search } = await setup();
    search.mockResolvedValueOnce({ items: [
      { ...items[0]!, kind: "live", id: "live", views: 12 },
      { ...items[0]!, id: "zero", views: 0 },
      { ...items[0]!, kind: "live", id: "unknown", views: null },
      { ...items[0]!, kind: "channel", id: "channel", views: 100 },
    ], page: 1, hasMore: false });
    await query("인원"); const cards = host.querySelectorAll("article");
    expect(cards[0]).toHaveTextContent("시청 12명"); expect(cards[0]).not.toHaveTextContent("12회");
    expect(cards[1]).toHaveTextContent("조회 0회"); expect(cards[2]).not.toHaveTextContent("시청 0명");
    expect(cards[3]).not.toHaveTextContent("100회"); expect(cards[3]).not.toHaveTextContent("100명");
  });
  it("loads real API results on submission and reuses a populated query without clearing it", async () => {
    const { search } = await setup(); await query("음악"); expect(host.querySelectorAll("article")).toHaveLength(3);
    await query("게임"); await query("음악"); expect(search).toHaveBeenCalledTimes(2); expect(host.querySelectorAll("article")).toHaveLength(3);
  });
  it("uses single focus, Ctrl/Shift selection, double-click detail and nested-button precedence", async () => {
    await setup(); await query("음악"); const cards = host.querySelectorAll("article");
    await click(cards[0]!); expect(host.querySelector(".soop-result-check")).toBeNull(); expect(host.querySelector('[role="dialog"]')).toBeNull();
    await click(cards[1]!, { ctrlKey: true }); expect(host.querySelectorAll('[aria-label="선택됨"]')).toHaveLength(2);
    await click(cards[2]!, { shiftKey: true }); expect(host.querySelectorAll('[aria-label="선택됨"]')).toHaveLength(2);
    await click(cards[0]!.querySelector(".soop-result-favorite")!); expect(host.querySelector('[role="dialog"]')).toBeNull(); expect(cards[0]!.querySelector(".soop-result-favorite")).toHaveAttribute("aria-pressed", "true");
    await act(async () => cards[0]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))); expect(host.querySelector('[role="dialog"]')).toHaveTextContent("영상 1");
  });
  it("retains results on a failed next page and offers retry instead of no-results", async () => {
    const { search } = await setup(); await query("음악"); search.mockRejectedValueOnce("요청 제한");
    await click(host.querySelector(".soop-load-more")!); expect(host.querySelectorAll("article")).toHaveLength(3); expect(host.querySelector('[role="alert"]')).toHaveTextContent("요청 제한");
  });
  it("downloads from card context menu using only provider IDs", async () => {
    const { download } = await setup(); await query("음악"); await act(async () => host.querySelector("article")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
    await click([...host.querySelectorAll('[role="menuitem"]')].find(node => node.textContent === "다운로드")!);
    expect(download).toHaveBeenCalledWith({ provider: "soop", kind: "video", externalId: "1" }, expect.any(String));
  });
  it("deselects the only focused card and releases focus", async () => {
    await setup(); await query("음악"); const card = host.querySelector("article")!;
    card.focus(); await click(card); expect(card).toHaveClass("is-focused");
    await click(card); expect(card).not.toHaveClass("is-focused"); expect(document.activeElement).not.toBe(card);
  });
  it("does not consume Enter from nested actions and clears selection when opening detail", async () => {
    await setup(); await query("음악"); const card = host.querySelector("article")!;
    await click(card);
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    await act(async () => card.querySelector("button")!.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(false); expect(host.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => card.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(host.querySelector('[role="dialog"]')).not.toBeNull(); expect(card).not.toHaveClass("is-focused");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
  });
  it("preserves restrictions in saved video snapshots", async () => {
    const { search } = await setup(); search.mockResolvedValueOnce({ items: [{ ...items[0]!, restricted: true }], page: 1, hasMore: false });
    await query("제한"); await click(host.querySelector(".soop-result-favorite")!);
    const stored = JSON.parse(localStorage.getItem("atsumi:soop:video-favorites:v1")!);
    expect(stored[0].restricted).toBe(true);
  });
});
