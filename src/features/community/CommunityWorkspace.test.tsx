import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommunityWorkspace } from "./CommunityWorkspace";
import { CommonNavigationContext } from "../../app/CommonNavigation";
import type { CommunityApi, Review, WorkKey, Writer } from "./api";

const review: Review = { id: "review-1", source: "hitomi", workId: "3657124", nickname: "방문자", rating: 4, recommended: true, comment: "짧은 감상", createdAt: "2026-09-14T00:00:00Z", updatedAt: "2026-09-14T00:00:00Z" };
const writer: Writer = { profile: { id: "member-1", nickname: "이용자-1234" }, mine: null };
const apiMock = () => ({ feed: vi.fn().mockResolvedValue({ items: [review], nextCursor: null }), work: vi.fn().mockResolvedValue({ items: [review], nextCursor: null }), beginWriting: vi.fn().mockResolvedValue(writer), save: vi.fn().mockResolvedValue(undefined), delete: vi.fn().mockResolvedValue(undefined), report: vi.fn().mockResolvedValue(undefined) });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const button = (host: HTMLElement, text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === text)!;

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function mount(api: CommunityApi, initialReview?: WorkKey) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host); const navigate = vi.fn();
  await act(async () => { root.render(<StrictMode><CommonNavigationContext.Provider value={{ communityOpen: true, openCommunity: vi.fn() }}><CommunityWorkspace initialReview={initialReview} source="hitomi" collapsed={false} attentionCount={2} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onNavigate={navigate} onSettings={vi.fn()} api={api} /></CommonNavigationContext.Provider></StrictMode>); await settle(); });
  return { host, navigate, close: async () => { await act(async () => root.unmount()); host.remove(); } };
}
describe("CommunityWorkspace", () => {
  it.each(["hitomi", "danbooru"] as const)("opens a %s review shortcut directly with the work number and the owner's existing review", async (source) => {
    const api = apiMock();
    api.beginWriting.mockResolvedValue({ ...writer, mine: { id: "own-shortcut-review", rating: 3, recommended: true, comment: "기존에 남긴 후기", hidden: false } });
    const work: WorkKey = { source, workId: "1234567" };
    const ui = await mount(api, work);
    try {
      expect(ui.host.querySelector('[aria-label="후기 작품번호"]')).toHaveValue(work.workId);
      expect(api.feed).not.toHaveBeenCalled();
      expect(api.work).toHaveBeenLastCalledWith(work, null);
      expect(api.beginWriting).toHaveBeenCalledExactlyOnceWith(work);
      expect(ui.host.querySelector('.community-editor h2')).toHaveTextContent(`${source === "hitomi" ? "Hitomi" : "Danbooru"} #1234567`);
      expect(ui.host.querySelector('[aria-label="짧은 후기"]')).toHaveValue("기존에 남긴 후기");
      expect(ui.host.querySelector('[aria-label="3점"]')).toHaveAttribute("aria-pressed", "true");
      expect(button(ui.host, "후기 수정")).toBeEnabled();
      expect(api.save).not.toHaveBeenCalled();
      await act(async () => button(ui.host, "닫기").click());
      expect(ui.host.querySelector('.community-editor')).toBeNull();
      expect(api.beginWriting).toHaveBeenCalledTimes(1);
    } finally { await ui.close(); }
  });

  it("reads publicly without issuing a key and has an independent left tab", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      expect(ui.host.textContent).toContain("짧은 감상"); expect(api.beginWriting).not.toHaveBeenCalled();
      expect(ui.host.querySelector('[aria-label="커뮤니티"]')).toHaveAttribute("aria-current", "page");
      expect(ui.host.querySelector('[aria-label="Explore"]')).not.toHaveAttribute("aria-current");
      await act(async () => ui.host.querySelector<HTMLButtonElement>('[aria-label="Downloads"]')!.click());
      expect(ui.navigate).toHaveBeenCalledWith("downloads");
      expect(api.save).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("issues only on a writing attempt, once in StrictMode, then writes the immutable work key", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      await act(async () => { button(ui.host, "내 후기 작성 / 수정").click(); await settle(); });
      expect(api.beginWriting).toHaveBeenCalledTimes(1); expect(api.beginWriting).toHaveBeenCalledWith({ source: "hitomi", workId: "3657124" });
      expect(ui.host.querySelector('[aria-label="후기 닉네임"]')).toHaveValue("이용자-1234");
      await act(async () => { ui.host.querySelector<HTMLFormElement>('.community-editor form')!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); await settle(); });
      expect(api.save).toHaveBeenCalledWith({ source: "hitomi", workId: "3657124", nickname: "이용자-1234", rating: 5, recommended: false, comment: "" });
      expect(ui.host.textContent).toContain("후기를 저장했습니다.");
    } finally { await ui.close(); }
  });
  it("keeps storage errors visible without retrying issuance or enabling submission", async () => {
    const api = apiMock(); api.beginWriting.mockRejectedValue("기존 키 읽기 실패"); const ui = await mount(api);
    try {
      await act(async () => { button(ui.host, "내 후기 작성 / 수정").click(); await settle(); });
      expect(ui.host.textContent).toContain("기존 키 읽기 실패"); expect(api.beginWriting).toHaveBeenCalledTimes(1);
      expect(ui.host.querySelector('.community-editor form')).toBeNull(); expect(api.save).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("loads the owner's existing review and does not suggest editing the displayed author's review", async () => {
    const api = apiMock(); api.beginWriting.mockResolvedValue({ ...writer, mine: { id: "my-review", rating: 2, recommended: false, comment: "내 이전 후기", hidden: true } }); const ui = await mount(api);
    try {
      await act(async () => { button(ui.host, "내 후기 작성 / 수정").click(); await settle(); });
      expect(ui.host.querySelector('[aria-label="짧은 후기"]')).toHaveValue("내 이전 후기");
      expect(ui.host.textContent).toContain("수정해도 공개 상태로 바뀌지 않습니다.");
      vi.spyOn(window, "confirm").mockReturnValue(true);
      await act(async () => { button(ui.host, "내 후기 삭제").click(); await settle(); });
      expect(api.delete).toHaveBeenCalledWith({ source: "hitomi", workId: "3657124" });
      expect(ui.host.textContent).toContain("작성자 키는 그대로 유지됩니다.");
    } finally { await ui.close(); }
  });
  it("appends a bounded next page without duplicate reviews or authentication", async () => {
    const api = apiMock(); const cursor = { createdAt: review.createdAt, id: review.id };
    api.feed.mockImplementation(async (_source, after) => after ? { items: [review, { ...review, id: "review-2", comment: "다음 후기" }], nextCursor: null } : { items: [review], nextCursor: cursor });
    const ui = await mount(api);
    try {
      await act(async () => { button(ui.host, "후기 더 보기").click(); await settle(); });
      expect(api.feed).toHaveBeenLastCalledWith(null, cursor);
      expect(ui.host.querySelectorAll('.community-review')).toHaveLength(2); expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("renders comments as plain text, never executable markup", async () => {
    const api = apiMock(); api.feed.mockResolvedValue({ items: [{ ...review, comment: '<img src=x onerror="alert(1)">' }], nextCursor: null }); const ui = await mount(api);
    try { expect(ui.host.querySelector('.community-comment img')).toBeNull(); expect(ui.host.querySelector('.community-comment')?.textContent).toContain('<img'); }
    finally { await ui.close(); }
  });
});
