import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient, type ThumbnailRequest } from "../thumbnail";
import { PersonalLibraryProvider } from "../features/personalLibrary/PersonalLibraryProvider";
import { createBrowserLibraryApi } from "../features/personalLibrary/api";
import { DetailWorkspace } from "./DetailWorkspace";

const dispose: (() => Promise<void>)[] = [];
let dialogDescriptors: (readonly [string, PropertyDescriptor | undefined])[] = [];
beforeEach(() => {
  localStorage.clear(); sessionStorage.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
  dialogDescriptors = ["showModal", "close"].map((name) => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)] as const);
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.open = true; } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function (this: HTMLDialogElement) { this.open = false; } });
});
afterEach(async () => {
  for (const cleanup of dispose.splice(0)) await cleanup();
  vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  for (const [name, descriptor] of dialogDescriptors) {
    if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, name);
  }
});

async function mount(start = 1, count = 5) {
  const gallery = { ...mockGalleries[0]!, pages: count, relatedIds: [], download: undefined,
    pageDimensions: Array.from({ length: count }, (_, i) => ({ sourcePage: i + 1, width: i === 2 ? 1800 : 800, height: 1200 })) };
  const resolve = vi.fn((_request: ThumbnailRequest) => ({ kind: "missing" as const, reason: "offline fixture" }));
  const client = new ThumbnailClient({ resolve });
  const storage = new Map<string, string>();
  const api = createBrowserLibraryApi({ getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => { storage.set(key, value); } });
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  dispose.push(async () => { await act(async () => root.unmount()); container.remove(); client.dispose(); });
  await act(async () => root.render(<PersonalLibraryProvider api={api}><DetailWorkspace
    tabs={[gallery.id]} activeId={gallery.id} minimized={false} galleries={new Map([[gallery.id, gallery]])}
    favoriteMetadata={new Set()} thumbnailClient={client} pageOpenRequest={{ galleryId: gallery.id, page: start, sequence: 1 }}
    onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()}
    onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />
  </PersonalLibraryProvider>));
  const dialog = container.querySelector<HTMLDialogElement>(".page-preview-dialog")!;
  const button = (label: string) => dialog.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
  const click = async (label: string) => { await act(async () => button(label).click()); };
  const key = async (key: string, target: EventTarget = window) => { await act(async () => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }))); };
  const pages = () => [...dialog.querySelectorAll<HTMLButtonElement>(".bookmark-toggle")].map((b) => b.querySelector("svg text")?.textContent);
  return { dialog, button, click, key, pages, resolve };
}

describe("page preview overlay and resilient spread", () => {
  it("keeps landscape/portrait pairs and trailing black space in two-page mode", async () => {
    const f = await mount();
    await f.click("두쪽 보기");
    expect(f.pages()).toEqual(["1", "2"]);
    expect(f.dialog.querySelector(".bookmark-toggle small")).toBeNull();
    await f.key("ArrowRight");
    expect(f.dialog).toHaveAttribute("data-page-preview-view", "spread");
    expect(f.pages()).toEqual(["3", "4"]);
    expect(f.button("두쪽 보기")).toHaveAttribute("aria-pressed", "true");
    expect(f.dialog.querySelector(".is-right")).toHaveClass("is-turning");
    expect(f.dialog.querySelector(".page-preview-chevron-pulse")).not.toBeNull();
    await f.key("d");
    expect(f.pages()).toEqual(["5"]);
    expect(f.dialog.querySelector(".page-preview-media-stage")?.lastElementChild).toHaveClass("page-preview-empty-slot");
    expect(f.dialog.querySelector(".page-preview-navigation")).toHaveTextContent("5 / 5");
    expect(f.button("다음 페이지")).toBeDisabled();
    await f.key("d");
    expect(f.pages()).toEqual(["5"]);
    await f.key("a");
    expect(f.pages()).toEqual(["3", "4"]);
    expect(f.resolve.mock.calls.every(([r]) => r.key.kind !== "source-page" || (r.key.page >= 1 && r.key.page <= 5))).toBe(true);
  });

  it("preserves a leading blank, RTL order, and real page bookmarks without changing the pair offset", async () => {
    const f = await mount(2);
    await f.click("두쪽 보기");
    await f.key("a");
    expect(f.dialog.querySelector(".page-preview-media-stage")?.firstElementChild).toHaveClass("page-preview-empty-slot");
    expect(f.pages()).toEqual(["1"]);
    expect(f.button("이전 페이지")).toBeDisabled();
    await f.click("읽기 방향");
    expect(f.dialog.querySelector(".page-preview-media-stage")?.lastElementChild).toHaveClass("page-preview-empty-slot");
    await f.key("a");
    expect(f.pages()).toEqual(["3", "2"]);
    await f.key("d");
    await f.click("두쪽 보기");
    expect(f.dialog).toHaveAttribute("data-page-preview-view", "single");
    expect(f.pages()).toEqual(["1"]);
    expect(f.dialog.querySelector(".page-preview-empty-slot")).toBeNull();
  });

  it("pins all controls beyond the timeout and remembers the preference", async () => {
    const f = await mount();
    vi.useFakeTimers();
    expect(f.dialog).toHaveAttribute("data-controls-visible", "false");
    await act(async () => f.dialog.dispatchEvent(new MouseEvent("pointermove", { bubbles: true })));
    await act(async () => vi.advanceTimersByTime(1900));
    expect(f.dialog).toHaveAttribute("data-controls-visible", "false");
    await f.click("UI 고정");
    await act(async () => vi.advanceTimersByTime(10000));
    expect(f.dialog).toHaveAttribute("data-controls-visible", "true");
    expect(localStorage.getItem("atsumi.pagePreview.controlsPinned")).toBe("true");
    await f.click("UI 고정 해제");
    await act(async () => vi.advanceTimersByTime(1900));
    expect(f.dialog).toHaveAttribute("data-controls-visible", "false");
  });

  it("restores the front preview on Alt-Tab and handles keys from bookmark buttons without disrupting editors", async () => {
    const f = await mount();
    const background = document.createElement("input"); document.body.append(background);
    dispose.push(async () => background.remove());
    background.focus();
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(document.activeElement).toBe(f.dialog);
    const bookmark = f.button("1페이지 즐겨찾기 저장");
    bookmark.focus();
    await f.key("ArrowRight", bookmark);
    expect(f.pages()).toEqual(["2"]);
    const editor = document.createElement("textarea"); f.dialog.append(editor); editor.focus();
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(document.activeElement).toBe(editor);
    await f.key("ArrowRight", editor);
    expect(f.pages()).toEqual(["2"]);
    editor.remove();
    const modal = document.createElement("dialog"); modal.open = true; modal.tabIndex = -1; document.body.append(modal); modal.focus();
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(document.activeElement).toBe(modal);
    await f.key("ArrowRight", modal);
    expect(f.pages()).toEqual(["2"]);
    modal.remove();
    await f.click("페이지 미리보기 닫기");
    const inactiveLibrary = document.createElement("main");
    inactiveLibrary.dataset.galleryShortcutsSuspended = ""; inactiveLibrary.hidden = true;
    document.body.append(inactiveLibrary); background.focus();
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(document.activeElement).toBe(document.querySelector('.detail-tabs [aria-selected="true"]'));
    inactiveLibrary.remove();
  });
});
