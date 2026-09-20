import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommunityReviewButton } from "./CommunityReviewButton";
import type { CommunityApi, Review, WorkKey, Writer } from "./api";

const work: WorkKey = { source: "hitomi", workId: "1234567" };
const writer: Writer = { profile: { id: "private-writer", nickname: "이용자-1234" }, mine: null };
const review: Review = { ...work, id: "r1", nickname: "다른 독자", rating: 4, recommended: false, comment: "마지막 장면이 좋았어요.", createdAt: "2026-09-17T00:00:00Z", updatedAt: "2026-09-17T00:00:00Z" };
const mockApi = () => ({ feed: vi.fn(), work: vi.fn().mockResolvedValue({ items: [review], nextCursor: null, summary: { ...work, reviewCount: 1, averageRating: 4, recommendationCount: 0 } }),
  beginWriting: vi.fn().mockResolvedValue(writer), save: vi.fn().mockResolvedValue(undefined), delete: vi.fn(), report: vi.fn() });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const panel = () => document.querySelector<HTMLElement>('[aria-label="앨범 코멘트"]')!;
const trigger = (host: HTMLElement) => host.querySelector<HTMLButtonElement>('[aria-label="코멘트 남기기"]')!;
const click = (node: HTMLElement) => act(async () => { node.click(); await settle(); });
async function input(value: string) {
  const node = panel().querySelector<HTMLTextAreaElement>("textarea")!;
  await act(async () => { node.focus(); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(node, value); node.dispatchEvent(new Event("input", { bubbles: true })); await settle(); });
}
const submit = () => act(async () => { panel().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); await settle(); });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }
async function mount(api: CommunityApi, modal = false) {
  const host = document.createElement(modal ? "dialog" : "div"); document.body.append(host); if (modal) host.setAttribute("open", "");
  const root = createRoot(host);
  const render = (key = work) => act(async () => { root.render(<StrictMode><CommunityReviewButton work={key} api={api} /></StrictMode>); await settle(); });
  await render();
  return { host, render, close: async () => { await act(async () => root.unmount()); host.remove(); } };
}
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("in-place album comments", () => {
  it("shows the requested tooltip, reads only after opening, and never issues a key just to read", async () => {
    const api = mockApi(); api.work.mockResolvedValue({ items: Array.from({ length: 5 }, (_, i) => ({ ...review, id: `r${i}` })), nextCursor: null });
    const ui = await mount(api);
    try {
      expect(trigger(ui.host)).toHaveAttribute("title", "코멘트 남기기"); expect(api.work).not.toHaveBeenCalled();
      await click(trigger(ui.host));
      expect(panel().querySelectorAll(".album-comment")).toHaveLength(3);
      expect(panel()).toHaveTextContent(review.comment);
      expect(api.work).toHaveBeenCalledExactlyOnceWith(work, null);
      expect(api.beginWriting).not.toHaveBeenCalled(); expect(api.feed).not.toHaveBeenCalled(); expect(api.save).not.toHaveBeenCalled();
      expect(panel().querySelector('[type="submit"]')).toBeDisabled();
      await click(panel().querySelector<HTMLButtonElement>(".album-comments-more")!);
      expect(panel().querySelectorAll(".album-comment")).toHaveLength(5); expect(api.work).toHaveBeenCalledTimes(1);
    } finally { await ui.close(); }
  });
  it("keeps the first typed comment and selected rating while lazy identity lookup is pending", async () => {
    const api = mockApi(), pending = deferred<Writer>(); api.beginWriting.mockReturnValue(pending.promise);
    const ui = await mount(api);
    try {
      await click(trigger(ui.host)); await click(panel().querySelector<HTMLButtonElement>('[aria-label="4점"]')!);
      await input("지금 남기는 한마디");
      expect(api.beginWriting).toHaveBeenCalledExactlyOnceWith(work); expect(panel().querySelector('[type="submit"]')).toBeDisabled();
      await act(async () => { pending.resolve({ ...writer, mine: { id: "mine", rating: 2, comment: "예전 코멘트", recommended: true, hidden: false } }); await settle(); });
      expect(panel().querySelector("textarea")).toHaveValue("지금 남기는 한마디");
      expect(panel().querySelector('[aria-label="4점"]')).toHaveAttribute("aria-pressed", "true");
      await submit();
      expect(api.save).toHaveBeenCalledExactlyOnceWith({ ...work, nickname: writer.profile.nickname, rating: 4, comment: "지금 남기는 한마디", recommended: true });
      expect(panel()).toHaveTextContent("저장했어요."); expect(trigger(ui.host)).toHaveAttribute("aria-expanded", "true");
    } finally { await ui.close(); }
  });
  it("loads the existing owner's review only when writing and leaves an unchanged recommendation intact", async () => {
    const api = mockApi(); api.beginWriting.mockResolvedValue({ ...writer, mine: { id: "mine", rating: 3, comment: "내 코멘트", recommended: true, hidden: true } });
    const ui = await mount(api);
    try {
      await click(trigger(ui.host)); await act(async () => { panel().querySelector<HTMLTextAreaElement>("textarea")!.focus(); await settle(); });
      expect(panel().querySelector("textarea")).toHaveValue("내 코멘트"); expect(panel()).toHaveTextContent("운영자가 숨긴 코멘트");
      await submit(); expect(api.save).toHaveBeenCalledWith({ ...work, nickname: writer.profile.nickname, rating: 3, comment: "내 코멘트", recommended: true });
      expect(panel()).toHaveTextContent("운영자가 숨긴 상태는 유지됩니다."); expect(api.delete).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("keeps a draft when dismissed and closes only the popover on Escape inside page preview", async () => {
    const api = mockApi(); const ui = await mount(api, true);
    try {
      await click(trigger(ui.host)); await input("아직 작성 중"); expect(ui.host.contains(panel())).toBe(true);
      const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      await act(async () => { panel().querySelector("textarea")!.dispatchEvent(event); await settle(); });
      expect(event.defaultPrevented).toBe(true); expect(panel()).toBeNull(); expect(ui.host).toHaveAttribute("open");
      expect(document.activeElement).toBe(trigger(ui.host));
      await click(trigger(ui.host)); expect(panel().querySelector("textarea")).toHaveValue("아직 작성 중");
      expect(api.beginWriting).toHaveBeenCalledTimes(1); expect(api.save).not.toHaveBeenCalled();
      await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(panel()).toBeNull(); expect(api.save).not.toHaveBeenCalled();
    } finally { await ui.close(); }
  });
  it("keeps the draft on save failure and serializes duplicate submit gestures", async () => {
    const api = mockApi(); const request = deferred<void>(); api.save.mockReturnValueOnce(request.promise).mockRejectedValueOnce("연결이 끊겼어요");
    const ui = await mount(api);
    try {
      await click(trigger(ui.host)); await click(panel().querySelector<HTMLButtonElement>('[aria-label="5점"]')!); await input("짧은 코멘트");
      await submit(); await submit(); expect(api.save).toHaveBeenCalledTimes(1);
      await act(async () => { request.resolve(); await settle(); });
      await submit(); expect(panel()).toHaveTextContent("연결이 끊겼어요"); expect(panel().querySelector("textarea")).toHaveValue("짧은 코멘트");
      expect(panel().querySelector('[type="submit"]')).toBeEnabled();
    } finally { await ui.close(); }
  });
  it("allows star-only comments without manufacturing a default rating or publishing before submit", async () => {
    const api = mockApi(); const ui = await mount(api);
    try {
      await click(trigger(ui.host)); expect(panel().querySelectorAll('[aria-pressed="true"]')).toHaveLength(0);
      await click(panel().querySelector<HTMLButtonElement>('[aria-label="4점"]')!); expect(api.save).not.toHaveBeenCalled();
      await submit(); expect(api.save).toHaveBeenCalledWith({ ...work, nickname: writer.profile.nickname, rating: 4, comment: "", recommended: false });
    } finally { await ui.close(); }
  });
  it("does not move a late read or writer response to a different album", async () => {
    const api = mockApi(); const read = deferred<Awaited<ReturnType<CommunityApi["work"]>>>(), identity = deferred<Writer>();
    api.work.mockReturnValueOnce(read.promise); api.beginWriting.mockReturnValueOnce(identity.promise);
    const ui = await mount(api);
    try {
      await click(trigger(ui.host)); await click(panel().querySelector<HTMLButtonElement>('[aria-label="2점"]')!);
      const other: WorkKey = { source: "danbooru", workId: "7654321" }; await ui.render(other); await click(trigger(ui.host));
      await act(async () => { read.resolve({ items: [{ ...review, comment: "늦게 온 이전 앨범" }], nextCursor: null }); identity.resolve(writer); await settle(); });
      expect(panel()).not.toHaveTextContent("늦게 온 이전 앨범"); expect(panel().querySelectorAll('[aria-pressed="true"]')).toHaveLength(0);
      expect(panel().querySelector('[type="submit"]')).toBeDisabled(); expect(api.save).not.toHaveBeenCalled();
      expect(api.work).toHaveBeenLastCalledWith(other, null);
    } finally { await ui.close(); }
  });
  it("shows read/identity errors without automatic identity retries and renders untrusted comments as text", async () => {
    const api = mockApi(); api.work.mockRejectedValueOnce("읽기 실패"); api.beginWriting.mockRejectedValue("작성자 키 읽기 실패");
    const ui = await mount(api);
    try {
      await click(trigger(ui.host)); expect(panel()).toHaveTextContent("읽기 실패"); expect(api.beginWriting).not.toHaveBeenCalled();
      api.work.mockResolvedValue({ items: [{ ...review, comment: '<img src=x onerror="alert(1)">' }], nextCursor: null });
      await click(panel().querySelector<HTMLButtonElement>(".album-comments-error button")!);
      expect(panel().querySelector(".album-comment img")).toBeNull(); expect(panel()).toHaveTextContent("<img");
      await click(panel().querySelector<HTMLButtonElement>('[aria-label="1점"]')!); expect(panel()).toHaveTextContent("작성자 키 읽기 실패");
      expect(api.beginWriting).toHaveBeenCalledTimes(1); expect(panel().querySelector('[type="submit"]')).toBeDisabled();
    } finally { await ui.close(); }
  });
});
