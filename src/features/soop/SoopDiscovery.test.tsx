import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { SoopDiscovery } from "./SoopDiscovery";
import type { SoopApi, SoopSearchItem, SoopSearchPage } from "../../api/soop";
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
function deferredPage() {
  let resolve!: (value: SoopSearchPage) => void, reject!: (reason: string) => void;
  const promise = new Promise<SoopSearchPage>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
describe("SOOP discovery interactions", () => {
  it("renders official four sections with real viewer badges, tags and deduplicated channel portraits", async () => {
    const portrait = "data:image/png;base64,AQ==";
    const official: SoopSearchItem[] = [
      { ...items[0]!, kind: "channel", id: "artist", section: "streamers", favoriteCount: 22894 },
      { ...items[1]!, kind: "live", section: "live", views: 0, tags: ["한국어", "버추얼", "공개 태그"] },
      { ...items[2]!, section: "related" },
      { ...items[2]!, section: "catch" },
    ];
    const api: SoopApi = { inspect: vi.fn().mockResolvedValue({ channelId: "artist", image: portrait }), search: vi.fn().mockResolvedValue({ items: official, page: 1, hasMore: false }) };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await query("곰이");
    expect([...host.querySelectorAll(".soop-result-section h2")].map(node => node.textContent)).toEqual(["관련 스트리머", "LIVE", "관련 콘텐츠", "Catch"]);
    expect(host.querySelectorAll("article")).toHaveLength(4);
    expect(host.querySelector(".soop-viewer-badge")).toHaveTextContent("시청 0명");
    expect(host.querySelector(".soop-viewer-badge i")).not.toBeNull();
    expect(host.querySelector(".soop-card-tags")).toHaveTextContent("한국어버추얼공개 태그");
    expect(host).toHaveTextContent("즐겨찾기 22,894명");
    expect(api.inspect).toHaveBeenCalledTimes(1); expect(host.querySelectorAll(".soop-channel-portrait img")).toHaveLength(4);
    expect([...host.querySelector('[data-section="live"] .soop-result-text')!.children].slice(0, 2).map(node => node.tagName)).toEqual(["BUTTON", "STRONG"]);
    expect(host.querySelector('[data-section="streamers"] .soop-result-text strong')).toBeNull();
  });
  it("uses displayed section order for Shift ranges even when provider rows arrive in another order", async () => {
    const api: SoopApi = { inspect: vi.fn(), search: vi.fn().mockResolvedValue({ items: [
      { ...items[0], kind: "channel", id: "artist", section: "streamers" },
      { ...items[1], section: "related" },
      { ...items[2], kind: "live", section: "live" },
      { ...items[0], section: "catch" },
    ], page: 1, hasMore: false }) };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await query("곰이");
    await click(host.querySelector('[data-section="streamers"] article')!);
    await click(host.querySelector('[data-section="live"] article')!, { shiftKey: true });
    expect(host.querySelectorAll("article.is-focused")).toHaveLength(2);
    expect(host.querySelector('[data-section="related"] article')).not.toHaveClass("is-focused");
    expect(host.querySelector(".soop-multi-bar")).toHaveTextContent("2개 선택");
  });
  it("keeps repeated related/Catch content separate but downloads a canonical video only once", async () => {
    const download = vi.fn().mockResolvedValue({ jobId: "new" });
    const first = { ...items[0], section: "related" as const };
    const second = { ...items[0], section: "catch" as const };
    const api: SoopApi = { inspect: vi.fn(), search: vi.fn().mockResolvedValue({ items: [first, second], page: 1, hasMore: false }) };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} mediaApi={{ download } as unknown as PlatformMediaApi} />));
    await query("곰이");
    const cards = host.querySelectorAll("article");
    await click(cards[0]!); await click(cards[1]!, { ctrlKey: true });
    expect(host.querySelector(".soop-multi-bar")).toHaveTextContent("2개 선택");
    await click(host.querySelector(".soop-multi-bar button")!);
    expect(download).toHaveBeenCalledTimes(1); expect(download.mock.calls[0]![0]).toEqual({ provider: "soop", kind: "video", externalId: "1" });
  });
  it("appends live/Catch pages without replacing first-page editorial groups or their duplicates", async () => {
    const search = vi.fn().mockResolvedValueOnce({ items: [{ ...items[0], kind: "channel", id: "artist", section: "streamers" }, { ...items[0], section: "related" }, { ...items[0], section: "catch" }], page: 1, hasMore: true })
      .mockResolvedValueOnce({ items: [{ ...items[0], section: "catch" }, { ...items[1], section: "catch" }], page: 2, hasMore: false, warnings: ["LIVE: 일시적 오류"] });
    const api: SoopApi = { inspect: vi.fn(), search };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await query("곰이"); await click(host.querySelector(".soop-load-more")!);
    expect(host.querySelectorAll('[data-section="streamers"] article')).toHaveLength(1);
    expect(host.querySelectorAll('[data-section="related"] article')).toHaveLength(1);
    expect(host.querySelectorAll('[data-section="catch"] article')).toHaveLength(2);
    expect(host.querySelector('[role="alert"]')).toHaveTextContent("LIVE: 일시적 오류");
  });
  it("retains earlier partial failures after successful append and deduplicates later warnings", async () => {
    const warning = "관련 스트리머·콘텐츠: 일시적 오류";
    const search = vi.fn().mockResolvedValueOnce({ items: [items[0]], page: 1, hasMore: true, warnings: [warning, "LIVE: 요청 제한"] })
      .mockResolvedValueOnce({ items: [items[1]], page: 2, hasMore: true })
      .mockResolvedValueOnce({ items: [items[2]], page: 3, hasMore: false, warnings: [warning, "Catch: 요청 제한"] });
    const api: SoopApi = { inspect: vi.fn(), search };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await query("곰이"); await click(host.querySelector(".soop-load-more")!);
    expect([...host.querySelectorAll('[role="alert"]')].map(node => node.textContent)).toEqual([warning, "LIVE: 요청 제한"]);
    await click(host.querySelector(".soop-load-more")!);
    expect([...host.querySelectorAll('[role="alert"]')].map(node => node.textContent)).toEqual([warning, "LIVE: 요청 제한", "Catch: 요청 제한"]);
    expect(host.querySelectorAll("article")).toHaveLength(3);
  });
  it("clears old partial warnings only after a true same-tab replacement refetch", async () => {
    const search = vi.fn().mockResolvedValueOnce({ items: [], page: 1, hasMore: false, warnings: ["관련 스트리머·콘텐츠: 일시적 오류"] })
      .mockResolvedValueOnce({ items, page: 1, hasMore: false });
    const api: SoopApi = { inspect: vi.fn(), search };
    await act(async () => root.render(<SoopDiscovery api={api} active privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await query("곰이"); expect(host.querySelector('[role="alert"]')).not.toBeNull();
    await query("곰이");
    expect(search).toHaveBeenCalledTimes(2); expect(search.mock.calls[1]![0]).toMatchObject({ page: 1 });
    expect(host.querySelector('[role="alert"]')).toBeNull(); expect(host.querySelectorAll("article")).toHaveLength(3);
    expect(host.querySelectorAll('[aria-label="탐색 닫기"]')).toHaveLength(1);
  });
  it('loads exact channel portraits without trusting an expired thumbnail key', async () => {
    const portrait='data:image/png;base64,AQ==';
    const api: SoopApi={inspect:vi.fn().mockResolvedValue({channelId:'artist',name:'작가',image:portrait,status:'offline'}),thumbnail:vi.fn(),home:vi.fn().mockResolvedValue({sections:[{title:'채널',items:[{...items[0],id:'artist',kind:'channel',thumbnailKey:'expired'}]}],personalized:false,notice:null})};
    const render=(active:boolean,privacyMode=false)=><SoopDiscovery api={api} active={active} privacyMode={privacyMode} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={()=>false}/>;
    await act(async()=>root.render(render(true)));
    expect(api.inspect).toHaveBeenCalledWith('artist'); expect(api.thumbnail).not.toHaveBeenCalled();
    expect(host.querySelector('.soop-result-image img')).toHaveAttribute('src',portrait);
    await act(async()=>root.render(render(false))); await act(async()=>root.render(render(true,true)));
    expect(api.inspect).toHaveBeenCalledTimes(1); expect(host.querySelector('.soop-result-image img')).toBeNull();
  });
  it("loads the official home without a query and keeps integrated search with channel/media groups", async () => {
    const home = vi.fn().mockResolvedValue({ sections: [{ title: "인기 LIVE", items: [{ ...items[0], kind: "live" }] }], personalized: false, notice: null });
    const search = vi.fn().mockResolvedValue({ items: [{ ...items[0], kind: "channel", id: "artist" }, { ...items[1], kind: "live" }, items[2]], page: 1, hasMore: false });
    const api: SoopApi = { inspect: vi.fn(), home, search };
    const render = (width: number) => <SoopDiscovery api={api} active privacyMode={false} previewWidth={width} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />;
    await act(async () => root.render(render(320)));
    expect(home).toHaveBeenCalledTimes(1); expect(search).not.toHaveBeenCalled();
    expect(host).toHaveTextContent("인기 LIVE"); expect(host.querySelectorAll("article")).toHaveLength(1);
    await query("음악"); expect(search).toHaveBeenCalledWith({ kind: "all", keyword: "음악", page: 1 });
    expect([...host.querySelectorAll(".soop-result-section h2")].map(node => node.textContent)).toEqual(["채널", "라이브", "영상"]);
    expect(host.querySelector('[data-kind="channel"]')).not.toBeNull();
    await act(async () => root.render(render(400)));
    expect((host.querySelector(".soop-discovery") as HTMLElement).style.getPropertyValue("--soop-preview-width")).toBe("400px");
    expect(search).toHaveBeenCalledTimes(1); expect(host.querySelectorAll("article")).toHaveLength(3);
    await click([...host.querySelectorAll('[role="tab"]')].find(node => node.textContent === "홈")!);
    expect(host).toHaveTextContent("인기 LIVE"); expect(host.querySelectorAll("article")).toHaveLength(1);
  });
  it("does not read home in privacy and shows home and partial-search failures honestly", async () => {
    const home = vi.fn().mockRejectedValue("메인 연결 실패");
    const search = vi.fn().mockResolvedValue({ items, page: 1, hasMore: false, warnings: ["채널: 요청 제한"] });
    const api: SoopApi = { inspect: vi.fn(), home, search };
    const render = (privacy: boolean) => <SoopDiscovery api={api} active privacyMode={privacy} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />;
    await act(async () => root.render(render(true))); expect(home).not.toHaveBeenCalled();
    await act(async () => root.render(render(false))); expect(host.querySelector('[role="alert"]')).toHaveTextContent("메인 연결 실패");
    home.mockResolvedValue({ sections: [], personalized: false, notice: "인기 콘텐츠를 표시합니다." });
    await click(host.querySelector('[aria-label="메인 새로고침"]')!); expect(home).toHaveBeenCalledTimes(2);
    await query("음악"); expect(host.querySelector('[role="alert"]')).toHaveTextContent("채널: 요청 제한");
    expect(host.querySelectorAll("article")).toHaveLength(3);
  });
  it("retains completed home across visibility/privacy changes without additional requests", async () => {
    const home = vi.fn().mockResolvedValue({ sections: [{ title: "추천 LIVE", items }], personalized: true, notice: null });
    const api: SoopApi = { inspect: vi.fn(), home };
    const render = (active: boolean, privacy: boolean) => <SoopDiscovery api={api} active={active} privacyMode={privacy} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />;
    await act(async () => root.render(render(true, false)));
    await act(async () => root.render(render(false, false)));
    await act(async () => root.render(render(true, false)));
    await act(async () => root.render(render(true, true)));
    expect(host).not.toHaveTextContent("추천 LIVE"); expect(host.querySelector("input")).toHaveValue("");
    await act(async () => root.render(render(true, false)));
    expect(home).toHaveBeenCalledTimes(1); expect(host).toHaveTextContent("추천 LIVE");
  });
  it("ignores home replies cancelled by leaving and permits one fresh read on return", async () => {
    let resolveOld: (value: unknown) => void = () => {};
    const home = vi.fn().mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; })).mockResolvedValue({ sections: [{ title: "새 추천", items }], personalized: true, notice: null });
    const api: SoopApi = { inspect: vi.fn(), home };
    const render = (active: boolean) => <SoopDiscovery api={api} active={active} privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />;
    await act(async () => root.render(render(true)));
    await act(async () => root.render(render(false)));
    await act(async () => root.render(render(true)));
    await act(async () => resolveOld({ sections: [{ title: "오래된 추천", items }], personalized: true, notice: null }));
    expect(home).toHaveBeenCalledTimes(2); expect(host).toHaveTextContent("새 추천"); expect(host).not.toHaveTextContent("오래된 추천");
  });
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
  it("restores completed home after hidden layout clamps the shared scroller without refetching", async () => {
    host.className = "soop-workspace";
    const home = vi.fn().mockResolvedValue({ sections: [{ title: "추천 영상", items }], personalized: false, notice: null });
    const api: SoopApi = { inspect: vi.fn(), home };
    const render = async (active: boolean) => act(async () => root.render(<SoopDiscovery api={api} active={active} privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    await render(true); const card = host.querySelector("article");
    host.scrollTop = 340; host.dispatchEvent(new Event("scroll"));
    // Browser hiding/clamping can happen before effect cleanup and can queue a scroll.
    host.hidden = true; host.scrollTop = 0; host.dispatchEvent(new Event("scroll"));
    await render(false);
    host.hidden = false; host.scrollTop = 120; host.dispatchEvent(new Event("scroll"));
    await render(true);
    expect(host.scrollTop).toBe(340); expect(host.querySelector("article")).toBe(card); expect(home).toHaveBeenCalledTimes(1);
    // A rapid second leave/return must not replace the retained offset with the clamp.
    host.scrollTop = 0; await render(false); host.dispatchEvent(new Event("scroll")); await render(true);
    expect(host.scrollTop).toBe(340); expect(home).toHaveBeenCalledTimes(1);
  });
  it("retains independent home search creator and saved offsets across inactive shared scrolling", async () => {
    host.className = "soop-workspace"; localStorage.setItem("atsumi:soop:video-favorites:v1", JSON.stringify(items));
    const home = vi.fn().mockResolvedValue({ sections: [{ title: "추천 영상", items }], personalized: false, notice: null });
    const search = vi.fn().mockResolvedValue({ items, page: 1, hasMore: false });
    const api: SoopApi = { inspect: vi.fn(), home, search };
    const render = async (active: boolean) => act(async () => root.render(<SoopDiscovery api={api} active={active} privacyMode={false} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    const scroll = (at: number) => { host.scrollTop = at; host.dispatchEvent(new Event("scroll")); };
    const leaveReturn = async (at: number) => { await render(false); scroll(0); scroll(90); await render(true); expect(host.scrollTop).toBe(at); };
    const tab = (title: string) => [...host.querySelectorAll('[role="tab"]')].find(node => node.textContent === title)!;
    await render(true); scroll(80); await leaveReturn(80);
    await query("음악"); scroll(340); await leaveReturn(340);
    await click(host.querySelector(".soop-result-copy button")!); scroll(460); await leaveReturn(460);
    expect(search).toHaveBeenLastCalledWith({ kind: "video", keyword: "작가", channelId: "artist", page: 1 });
    await chooseKind("saved"); scroll(75); await leaveReturn(75);
    await chooseKind("all"); expect(host.scrollTop).toBe(460);
    await click(tab("음악 · 통합검색")); expect(host.scrollTop).toBe(340);
    await click(tab("홈")); expect(host.scrollTop).toBe(80);
    expect(home).toHaveBeenCalledTimes(1); expect(search).toHaveBeenCalledTimes(2);
  });
  it("keeps a genuine visible zero but ignores private layout scroll without resetting results", async () => {
    host.className = "soop-workspace";
    const api: SoopApi = { inspect: vi.fn(), search: vi.fn().mockResolvedValue({ items, page: 1, hasMore: false }) };
    const render = async (active: boolean, privacy = false) => act(async () => root.render(<SoopDiscovery api={api} active={active} privacyMode={privacy} onLive={vi.fn()} onFavoriteChannel={vi.fn()} isChannelFavorite={() => false} />));
    const scroll = (at: number) => { host.scrollTop = at; host.dispatchEvent(new Event("scroll")); };
    await render(true); await query("음악"); scroll(340); scroll(0);
    await render(false); scroll(99); await render(true); expect(host.scrollTop).toBe(0);
    scroll(220); await render(true, true); scroll(0); await render(true);
    expect(host.scrollTop).toBe(220); expect(host.querySelectorAll("article")).toHaveLength(3); expect(api.search).toHaveBeenCalledTimes(1);
  });
  it("distinguishes live viewers, video views, zero and unknown counts", async () => {
    const { search } = await setup();
    search.mockResolvedValueOnce({ items: [
      { ...items[0]!, kind: "live", id: "live", views: 12 },
      { ...items[0]!, id: "zero", views: 0 },
      { ...items[0]!, kind: "live", id: "unknown", views: null },
      { ...items[0]!, kind: "channel", id: "channel", views: 100 },
    ], page: 1, hasMore: false });
    await query("인원"); const live = host.querySelectorAll('article[data-kind="live"]');
    expect(live[0]).toHaveTextContent("시청 12명"); expect(live[0]).not.toHaveTextContent("12회");
    expect(host.querySelector('article[data-kind="video"]')).toHaveTextContent("조회 0회"); expect(live[1]).not.toHaveTextContent("시청 0명");
    expect(host.querySelector('article[data-kind="channel"]')).not.toHaveTextContent("100회"); expect(host.querySelector('article[data-kind="channel"]')).not.toHaveTextContent("100명");
  });
  it("loads real API results on submission and reuses a populated query without clearing it", async () => {
    const { search } = await setup(); await query("음악"); expect(host.querySelectorAll("article")).toHaveLength(3);
    await query("게임"); await query("음악"); expect(search).toHaveBeenCalledTimes(2); expect(host.querySelectorAll("article")).toHaveLength(3);
  });
  it.each([
    ["success", true], ["error", true], ["success", false], ["error", false],
  ] as const)("ignores a closed first-page %s when the identical replacement is settled=%s", async (outcome, settled) => {
    const { search } = await setup(), old = deferredPage(), replacement = deferredPage();
    const latest = { ...items[0]!, id: "40", title: "다시 연 검색의 최신 영상" };
    search.mockReset().mockImplementationOnce(() => old.promise).mockImplementationOnce(() => replacement.promise);
    await query("같은 검색");
    await click(host.querySelector('[aria-label="탐색 닫기"]')!);
    await query("같은 검색");
    if (settled) await act(async () => replacement.resolve({ items: [latest], page: 1, hasMore: false }));
    await act(async () => {
      if (outcome === "success") old.resolve({ items: [{ ...items[1]!, title: "닫은 탭의 오래된 영상" }], page: 1, hasMore: true, warnings: ["닫은 탭의 경고"] });
      else old.reject("닫은 탭의 오래된 오류");
    });
    expect(search).toHaveBeenCalledTimes(2);
    expect(host.querySelectorAll('[aria-label="탐색 닫기"]')).toHaveLength(1);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host).not.toHaveTextContent("닫은 탭의 오래된 영상");
    expect(host).not.toHaveTextContent("닫은 탭의 경고");
    expect(host.querySelector('[aria-label="불러오는 중"]') !== null).toBe(!settled);
    if (!settled) {
      expect(host.querySelectorAll("article")).toHaveLength(0);
      await act(async () => replacement.resolve({ items: [latest], page: 1, hasMore: false }));
    }
    expect(host.querySelectorAll("article")).toHaveLength(1);
    expect(host.querySelector("article")).toHaveTextContent(latest.title);
    expect(host.querySelector('[aria-label="불러오는 중"]')).toBeNull();
    expect(host.querySelector(".soop-load-more")).toBeNull();
  });
  it.each(["success", "error"] as const)("ignores a closed page-two %s while the replacement page-two is pending", async outcome => {
    const { search } = await setup(), oldPage = deferredPage(), replacementPage = deferredPage();
    const first = { ...items[0]!, id: "40", title: "다시 연 첫 페이지" }, next = { ...items[1]!, id: "41", title: "다시 연 다음 페이지" };
    await query("같은 페이지 검색");
    search.mockImplementationOnce(() => oldPage.promise);
    await click(host.querySelector(".soop-load-more")!);
    await click(host.querySelector('[aria-label="탐색 닫기"]')!);
    search.mockResolvedValueOnce({ items: [first], page: 1, hasMore: true });
    await query("같은 페이지 검색");
    search.mockImplementationOnce(() => replacementPage.promise);
    await click(host.querySelector(".soop-load-more")!);
    await act(async () => {
      if (outcome === "success") oldPage.resolve({ items: [{ ...items[2]!, title: "닫은 탭의 다음 페이지" }], page: 2, hasMore: false });
      else oldPage.reject("닫은 탭의 페이지 오류");
    });
    expect(search).toHaveBeenCalledTimes(4);
    expect(search).toHaveBeenNthCalledWith(4, { kind: "all", keyword: "같은 페이지 검색", page: 2 });
    expect(host.querySelectorAll("article")).toHaveLength(1);
    expect(host.querySelector("article")).toHaveTextContent(first.title);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector(".soop-load-more")).toBeDisabled();
    expect(host.querySelector('[aria-label="불러오는 중"]')).not.toBeNull();
    await act(async () => replacementPage.resolve({ items: [next], page: 2, hasMore: false }));
    expect([...host.querySelectorAll("article strong")].map(node => node.textContent)).toEqual([first.title, next.title]);
    expect(host.querySelector(".soop-load-more")).toBeNull();
    expect(host.querySelector('[aria-label="불러오는 중"]')).toBeNull();
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
