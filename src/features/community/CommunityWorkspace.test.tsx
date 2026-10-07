import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommunityWorkspace } from "./CommunityWorkspace";
import { CommonNavigationContext } from "../../app/CommonNavigation";
import type { CommunityApi, CommunitySource, Review, WorkKey, Writer } from "./api";

vi.mock("../../components/GalleryThumbnail", () => ({ GalleryThumbnail: ({ alt }: { alt: string }) => <img alt={alt} /> }));
vi.mock("./workPreview", () => ({ loadDanbooruPreview: vi.fn().mockResolvedValue(null) }));
const review: Review = { id: "review-1", source: "hitomi", workId: "3657124", nickname: "방문자", rating: 4, recommended: true, comment: "짧은 감상", createdAt: "2026-09-14T00:00:00Z", updatedAt: "2026-09-14T00:00:00Z" };
const writer: Writer = { profile: { id: "member-1", nickname: "이용자-1234" }, mine: null };
const own = { ...review, id: "my-review", nickname: writer.profile.nickname, comment: "내 이전 후기", hidden: false };
const ownPage = { profile: writer.profile, identityIssued: true, items: [own], nextCursor: null };
const apiMock = () => ({ feed: vi.fn().mockResolvedValue({ items: [review], nextCursor: null }), work: vi.fn(), beginWriting: vi.fn().mockResolvedValue(writer),
  myReviews: vi.fn().mockResolvedValue(ownPage), save: vi.fn().mockResolvedValue(undefined), delete: vi.fn().mockResolvedValue(undefined), report: vi.fn().mockResolvedValue(undefined) });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const button = (host: HTMLElement, text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.getAttribute("aria-label") === text || item.textContent?.trim() === text)!;
const click = (node: HTMLElement) => act(async () => { node.click(); await settle(); });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }
const originalShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
const originalClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.setAttribute("open", ""); } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function (this: HTMLDialogElement) { this.removeAttribute("open"); } });
});
afterEach(() => {
  vi.restoreAllMocks(); vi.unstubAllGlobals();
  for (const [key, descriptor] of [["showModal", originalShowModal], ["close", originalClose]] as const) {
    if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
  }
});
async function mount(api: CommunityApi, initialReview?: WorkKey, privacyMode = false, source: CommunitySource = initialReview?.source ?? "hitomi") {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host); const navigate = vi.fn();
  const openFavorites = vi.fn();
  await act(async () => { root.render(<StrictMode><CommonNavigationContext.Provider value={{ communityOpen: true, openCommunity: vi.fn() }}><CommunityWorkspace initialReview={initialReview} privacyMode={privacyMode} source={source} collapsed={false} attentionCount={2} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onNavigate={navigate} onOpenPersonalLibrary={source === "hitomi" ? openFavorites : undefined} onSettings={vi.fn()} api={api} /></CommonNavigationContext.Provider></StrictMode>); await settle(); });
  return { host, navigate, openFavorites, close: async () => { await act(async () => root.unmount()); host.remove(); } };
}

describe("CommunityWorkspace", () => {
  it("shares the initial my-review read under StrictMode and fetches again on reopening", async () => {
    const api = apiMock();
    const pending = deferred<typeof ownPage>();
    api.myReviews.mockReturnValue(pending.promise);
    const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      expect(api.myReviews).toHaveBeenCalledTimes(1);
      expect(ui.host.querySelector('dialog [role="alert"]')).toBeNull();
      await act(async () => pending.resolve(ownPage));
      expect(ui.host.querySelector("dialog")).toHaveTextContent("내 이전 후기");
      await click(ui.host.querySelector<HTMLButtonElement>('[aria-label="내 후기 닫기"]')!);
      api.myReviews.mockResolvedValue({ ...ownPage, items: [] });
      await click(button(ui.host, "내 후기"));
      expect(api.myReviews).toHaveBeenCalledTimes(2);
      expect(ui.host.querySelector("dialog")).toHaveTextContent("아직 작성한 후기가 없습니다.");
    } finally { await ui.close(); }
  });
  it("preserves long existing comments without allowing an over-limit save", async () => {
    const api = apiMock();
    api.myReviews.mockResolvedValue({ ...ownPage, items: [{ ...own, comment: "가".repeat(101) }] });
    const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      await click(button(ui.host.querySelector("dialog")!, "수정"));
      expect(ui.host.querySelector('[aria-label="짧은 후기"]')).toHaveValue("가".repeat(101));
      expect(ui.host.querySelector(".community-editor")).not.toHaveTextContent("등록한 내용은 공개됩니다");
      expect(button(ui.host, "수정")).toBeDisabled();
      expect(button(ui.host, "삭제")).toBeEnabled();
      expect(api.save).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it.each(["hitomi", "danbooru"] as const)("shows only %s cards in both public and own feeds", async source => {
    const api = apiMock();
    api.feed.mockResolvedValue({ items: [review, { ...review, id: "dan-review", source: "danbooru", workId: "123", comment: "단부루 후기" }], nextCursor: null });
    const cursor = { id: "foreign-page", createdAt: own.createdAt };
    api.myReviews.mockImplementation(async after => after ? { ...ownPage, items: [{ ...own, source }] } : { ...ownPage, items: [{ ...own, source: source === "hitomi" ? "danbooru" : "hitomi" }], nextCursor: cursor });
    const ui = await mount(api, undefined, false, source);
    try {
      expect(api.feed).toHaveBeenCalledWith(source, null, "popular");
      expect(api.feed).toHaveBeenCalledWith(source, null, "latest");
      expect(ui.host.querySelectorAll(".community-review")).toHaveLength(2);
      expect(ui.host.querySelector('[aria-label="후기 사이트"]')).toBeNull();
      await click(button(ui.host, "내 후기"));
      expect(api.myReviews).toHaveBeenLastCalledWith(cursor);
      expect(ui.host.querySelectorAll("dialog .community-review")).toHaveLength(1);
      expect(ui.host.querySelector("dialog .community-work-label")).toHaveTextContent(source === "hitomi" ? "Hitomi" : "Danbooru");
      expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("reads publicly with previews, no search/write form, and no identity request", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      expect(ui.host.textContent).toContain("짧은 감상");
      expect(ui.host.querySelector('img[alt="Hitomi #3657124 미리보기"]')).not.toBeNull();
      expect(ui.host.querySelector('[aria-label="후기 작품번호"]')).toBeNull();
      expect(ui.host.textContent).not.toContain("후기 작성 / 수정");
      expect(api.myReviews).not.toHaveBeenCalled(); expect(api.beginWriting).not.toHaveBeenCalled();
      expect(ui.host.querySelector('[aria-label="커뮤니티"]')).toHaveAttribute("aria-current", "page");
      expect(api.feed).toHaveBeenCalledWith("hitomi", null, "popular");
      expect(api.feed).toHaveBeenCalledWith("hitomi", null, "latest");
      expect(ui.host.querySelector(".community-review-row > .community-review .community-review-cover")).not.toBeNull();
      expect(ui.host).not.toHaveTextContent("Hitomi 후기");
      expect(ui.host).not.toHaveTextContent("내 후기 보기");
      expect(ui.host).not.toHaveTextContent("열람은 인증 없이");
      expect(ui.host.querySelector('[aria-label="후기 정렬"]')).toBeNull();
      const favorites = ui.host.querySelector<HTMLButtonElement>('[aria-label="내 즐겨찾기"]')!;
      expect(favorites).not.toHaveAttribute("aria-current");
      await click(favorites); expect(ui.openFavorites).toHaveBeenCalledOnce();
      await click(ui.host.querySelector<HTMLButtonElement>('[aria-label="Downloads"]')!);
      expect(ui.navigate).toHaveBeenCalledWith("downloads");
    } finally { await ui.close(); }
  });
  it("opens own reviews in a floating dialog, shows the non-secret ID, and edits without issuing a key", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      const dialog = ui.host.querySelector("dialog")!;
      expect(dialog).toHaveAttribute("open"); expect(dialog.textContent).toContain("내 이전 후기");
      expect(dialog.querySelector(".community-identity code")).toHaveTextContent("member-1");
      expect(dialog.textContent).toContain("이 ID는 로그인·복구용이 아닙니다.");
      await click(button(dialog, "수정"));
      expect(dialog.querySelector('[aria-label="짧은 후기"]')).toHaveValue("내 이전 후기");
      await act(async () => { dialog.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); await settle(); });
      expect(api.save).toHaveBeenCalledWith({ source: "hitomi", workId: "3657124", nickname: writer.profile.nickname, rating: 4, recommended: true, comment: "내 이전 후기" });
      expect(dialog.textContent).toContain("후기를 저장했습니다.");
      expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("does not issue an identity just to inspect an empty own-history", async () => {
    const api = apiMock(); api.myReviews.mockResolvedValue({ profile: null, identityIssued: false, items: [], nextCursor: null }); const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      expect(ui.host.textContent).toContain("아직 익명 키가 발급되지 않았습니다.");
      expect(ui.host.querySelector(".community-identity")).toBeNull();
      expect(api.beginWriting).not.toHaveBeenCalled(); expect(api.save).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("keeps identity errors visible without silently creating a replacement", async () => {
    const api = apiMock(); api.myReviews.mockRejectedValue("기존 키 읽기 실패"); const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      expect(ui.host.querySelector('dialog [role="alert"]')).toHaveTextContent("기존 키 읽기 실패");
      expect(ui.host.querySelector("dialog form")).toBeNull();
      expect(api.beginWriting).not.toHaveBeenCalled();
      api.myReviews.mockResolvedValue(ownPage);
      await click(button(ui.host, "다시 시도"));
      expect(ui.host.querySelector('dialog [role="alert"]')).toBeNull();
    } finally { await ui.close(); }
  });
  it("includes hidden own reviews and retains moderation when editing or deleting", async () => {
    const api = apiMock(); api.myReviews.mockResolvedValue({ ...ownPage, items: [{ ...own, hidden: true }] }); const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기"));
      expect(ui.host.querySelector("dialog")).toHaveTextContent("비공개 처리됨");
      await click(button(ui.host.querySelector("dialog")!, "수정"));
      expect(ui.host.textContent).toContain("수정해도 공개 상태로 바뀌지 않습니다.");
      vi.spyOn(window, "confirm").mockReturnValue(true);
      await click(button(ui.host, "삭제"));
      expect(api.delete).toHaveBeenCalledWith({ source: "hitomi", workId: "3657124" });
      expect(ui.host.textContent).toContain("작성자 키는 그대로 유지됩니다.");
      expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("paginates my reviews without exposing another service or mixing public reviews", async () => {
    const api = apiMock(); const cursor = { createdAt: own.createdAt, id: own.id };
    api.myReviews.mockImplementation(async (after) => after ? { ...ownPage, items: [own, { ...own, id: "mine-2", workId: "123" }, { ...own, source: "danbooru", id: "mine-foreign", workId: "456", comment: "다른 사이트 내 후기" }] } : { ...ownPage, nextCursor: cursor });
    const ui = await mount(api);
    try {
      await click(button(ui.host, "내 후기")); await click(button(ui.host, "내 후기 더 보기"));
      const dialog = ui.host.querySelector("dialog")!;
      expect(dialog.querySelectorAll(".community-review")).toHaveLength(2);
      expect(dialog).toHaveTextContent("Hitomi #123"); expect(dialog).not.toHaveTextContent("다른 사이트 내 후기"); expect(dialog).not.toHaveTextContent("짧은 감상");
      expect(api.myReviews).toHaveBeenLastCalledWith(cursor);
    } finally { await ui.close(); }
  });
  it("shows a loading signal during refresh even with existing public reviews", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      const pending = deferred<Awaited<ReturnType<CommunityApi["feed"]>>>();
      api.feed.mockReturnValue(pending.promise);
      await click(button(ui.host, "인기 후기 새로고침"));
      expect(button(ui.host, "인기 후기 새로고침")).toHaveAttribute("aria-busy", "true");
      expect(ui.host.querySelector(".community-feed-heading .spinner")).not.toBeNull();
      expect(ui.host.querySelectorAll(".community-review")).toHaveLength(2);
      await act(async () => pending.resolve({ items: [], nextCursor: null }));
      expect(button(ui.host, "인기 후기 새로고침")).toBeEnabled();
    } finally { await ui.close(); }
  });
  it("hides previews in privacy mode, including own-history", async () => {
    const ui = await mount(apiMock(), undefined, true);
    try {
      await click(button(ui.host, "내 후기"));
      expect(ui.host.querySelector(".community-work-preview.is-private")).not.toBeNull();
      expect(ui.host.querySelector(".community-review img")).toBeNull();
    } finally { await ui.close(); }
  });
  it("appends public pages without duplicate reviews or authentication", async () => {
    const api = apiMock(); const cursor = { createdAt: review.createdAt, id: review.id };
    api.feed.mockImplementation(async (_source, after) => after ? { items: [review, { ...review, id: "review-2", workId: "123", comment: "다음 후기" }], nextCursor: null } : { items: [review], nextCursor: cursor });
    const ui = await mount(api);
    try {
      await click(button(ui.host, "인기 후기 더 보기"));
      await click(button(ui.host, "후기 더 보기"));
      expect(api.feed).toHaveBeenLastCalledWith("hitomi", cursor, "popular");
      expect(ui.host.querySelectorAll(".community-feed:not([hidden]) .community-review")).toHaveLength(2); expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it.each(["hitomi", "danbooru"] as const)("preserves the explicit %s writing shortcut with one issuance attempt", async (source) => {
    const api = apiMock(); api.beginWriting.mockResolvedValue({ ...writer, mine: { ...own, comment: "기존에 남긴 후기", rating: 3 } });
    const work: WorkKey = { source, workId: "1234567" }; const ui = await mount(api, work);
    try {
      expect(api.beginWriting).toHaveBeenCalledExactlyOnceWith(work);
      expect(ui.host.querySelector('[aria-label="짧은 후기"]')).toHaveValue("기존에 남긴 후기");
      expect(ui.host.querySelector('[aria-label="3점"]')).toHaveAttribute("aria-pressed", "true");
      await click(button(ui.host, "목록으로"));
      expect(ui.host.querySelector(".community-editor")).toBeNull();
    } finally { await ui.close(); }
  });
  it("renders comments as plain text, never executable markup", async () => {
    const api = apiMock(); api.feed.mockResolvedValue({ items: [{ ...review, comment: '<img src=x onerror="alert(1)">' }], nextCursor: null }); const ui = await mount(api);
    try { expect(ui.host.querySelector(".community-comment img")).toBeNull(); expect(ui.host.querySelector(".community-comment")?.textContent).toContain("<img"); }
    finally { await ui.close(); }
  });
  it("limits overview rows to six reviews, keeps the full page in dedicated view and separates feed failures", async () => {
    const api = apiMock();
    api.feed.mockImplementation(async (_source, _cursor, order) => {
      if (order === "latest") throw new Error("최신 목록 연결 실패");
      return { items: Array.from({ length: 9 }, (_, index) => ({ ...review, id: `item-${index}`, workId: String(index + 1) })), nextCursor: null };
    });
    const ui = await mount(api);
    try {
      expect(ui.host.querySelectorAll('[aria-label="인기 후기 목록"] .community-review')).toHaveLength(6);
      expect(ui.host.querySelector('[aria-label="최신 후기 목록"] [role="alert"]')).toHaveTextContent("최신 목록 연결 실패");
      const before = api.feed.mock.calls.length;
      await click(button(ui.host, "인기 후기 더 보기"));
      expect(api.feed).toHaveBeenCalledTimes(before);
      expect(ui.host.querySelectorAll('.community-feed:not([hidden]) .community-review')).toHaveLength(9);
      await click(button(ui.host, "돌아가기"));
      expect(ui.host.querySelectorAll('[aria-label="인기 후기 목록"] .community-review')).toHaveLength(6);
    } finally { await ui.close(); }
  });
  it("shows both rows, expands one without refetching and isolates late responses between rankings", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      expect(ui.host.querySelectorAll(".community-feed:not([hidden])")).toHaveLength(2);
      const before = api.feed.mock.calls.length;
      await click(button(ui.host, "최신 후기 더 보기"));
      expect(api.feed).toHaveBeenCalledTimes(before);
      expect(ui.host.querySelectorAll(".community-feed:not([hidden])")).toHaveLength(1);
      const pending = deferred<Awaited<ReturnType<CommunityApi["feed"]>>>();
      api.feed.mockReturnValueOnce(pending.promise);
      await click(button(ui.host, "최신 후기 새로고침"));
      expect(api.feed).toHaveBeenLastCalledWith("hitomi", null, "latest");
      expect(ui.host.querySelectorAll(".community-feed:not([hidden]) .community-review")).toHaveLength(1);
      api.feed.mockResolvedValue({ items: [{ ...review, id: "worst", comment: "최저 평균 작품", workSummary: { averageRating: 1.5, reviewCount: 8 } }], nextCursor: null });
      await act(async () => {
        const select = ui.host.querySelector<HTMLSelectElement>('[aria-label="후기 목록"]')!;
        select.value = "worst"; select.dispatchEvent(new Event("change", { bubbles: true })); await settle();
      });
      expect(api.feed).toHaveBeenLastCalledWith("hitomi", null, "worst");
      await act(async () => pending.resolve({ items: [{ ...review, comment: "늦게 도착한 최신 후기" }], nextCursor: null }));
      expect(ui.host).toHaveTextContent("최저 평균 작품");
      expect(ui.host).toHaveTextContent("작품 평균 1.5 · 후기 8개");
      expect(ui.host.querySelector(".community-feed:not([hidden])")).not.toHaveTextContent("늦게 도착한 최신 후기");
      expect(ui.host.querySelector('[aria-label="후기 목록"]')).toHaveValue("worst");
      await click(button(ui.host, "돌아가기"));
      expect(ui.host.querySelectorAll(".community-feed:not([hidden])")).toHaveLength(2);
      expect(ui.host.querySelector('[aria-label="최신 후기 목록"]')).toHaveTextContent("늦게 도착한 최신 후기");
      expect(api.beginWriting).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("keeps rating/comment in the fixed overlay, and puts metadata and reporting in hover details", async () => {
    const api = apiMock(); const ui = await mount(api);
    try {
      const card = ui.host.querySelector<HTMLElement>(".community-review")!;
      expect(card).toHaveAttribute("tabindex", "0");
      expect(card.querySelector(".community-review-body footer")).toBeNull();
      expect(card.querySelector(".community-review-body")).toHaveTextContent("짧은 감상");
      expect(card.querySelector(".community-comment")).toHaveAttribute("title", "짧은 감상");
      expect(card.querySelector(".community-review-details")).toHaveTextContent("방문자");
      await click(button(card, "신고"));
      expect(card).toHaveAttribute("data-reporting", "true");
      expect(button(card, "닫기")).toHaveAttribute("aria-expanded", "true");
      await click(button(card, "닫기"));
      expect(card).not.toHaveAttribute("data-reporting");
      expect(card.querySelector("form")).toBeNull();
    } finally { await ui.close(); }
  });
});
