import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockGalleries } from "../../data/mockGalleries";
import { ThumbnailClient, ThumbnailProvider } from "../../thumbnail";
import { DetailWorkspace } from "../../components/DetailWorkspace";
import { createBrowserLibraryApi, snapshotFor, targetFor, type LibraryApi, type Request } from "./api";
import { BookmarkButton } from "./BookmarkButton";
import { PersonalLibraryProvider } from "./PersonalLibraryProvider";
import { PersonalLibraryWorkspace } from "./PersonalLibraryWorkspace";

const gallery = { ...mockGalleries[0]!, pages: 20, relatedIds: [], download: undefined,
  pageDimensions: Array.from({ length: 20 }, (_, index) => ({ sourcePage: index + 1, width: 800, height: 1200 })) };
const save = (page = 0): Request => ({ action: "bookmark_set", target: targetFor(gallery, page), enabled: true, snapshot: snapshotFor(gallery) });
const list = (kind: "albums" | "pages", collectionId: string | null = null): Request => ({ action: "list", kind, collectionId, search: "", offset: 0, limit: 50 });
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));
let root: Root, container: HTMLDivElement, api: LibraryApi, client: ThumbnailClient;
let dialogDescriptors: Array<[string, PropertyDescriptor | undefined]>;
beforeEach(() => {
  localStorage.clear(); sessionStorage.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
  dialogDescriptors = ["showModal", "close"].map((name) => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)]);
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function(this: HTMLDialogElement) { this.setAttribute("open", ""); } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function(this: HTMLDialogElement) { this.removeAttribute("open"); } });
  api = createBrowserLibraryApi(localStorage); container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
});
afterEach(async () => {
  await act(async () => root.unmount()); client.dispose(); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  for (const [name, descriptor] of dialogDescriptors) { if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor); else Reflect.deleteProperty(HTMLDialogElement.prototype, name); }
});
const render = async (children: ReactNode, customApi = api) => act(async () => { root.render(<ThumbnailProvider client={client}><PersonalLibraryProvider api={customApi}>{children}</PersonalLibraryProvider></ThumbnailProvider>); await settle(); });
const button = (label: string, host: ParentNode = document) => {
  const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.getAttribute("aria-label") === label || node.textContent?.trim() === label);
  if (!found) throw new Error(`Button not found: ${label}`); return found;
};
const click = async (label: string, host?: ParentNode) => act(async () => { button(label, host).click(); await settle(); });
const input = async (label: string, value: string) => act(async () => {
  const node = document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, value);
  node.dispatchEvent(new Event("input", { bubbles: true }));
});

describe("private favorites and collections", () => {
  it("keeps album/page favorites independent, idempotent, and persistent across clients", async () => {
    await api(save()); await api(save(7)); await api(save(8)); await api(save(7));
    expect((await api({ action: "summary" })).summary.keys).toHaveLength(3);
    const reopened = createBrowserLibraryApi(localStorage);
    expect((await reopened(list("pages"))).items.map((item) => item.page).sort()).toEqual([7, 8]);
    await reopened({ action: "bookmark_set", target: targetFor(gallery), enabled: false, snapshot: null });
    expect((await api({ action: "summary" })).summary.keys).toHaveLength(2);
    expect(localStorage.getItem("atsumi.favorites")).toBeNull();
  });
  it("supports multiple collections, rename, unfiled filtering and non-destructive collection deletion", async () => {
    await api(save()); await api(save(7));
    const a = (await api({ action: "collection_save", id: null, name: "장면" })).collectionId!;
    const b = (await api({ action: "collection_save", id: null, name: "다시 보기" })).collectionId!;
    for (const id of [a, b]) await api({ action: "membership_set", target: targetFor(gallery, 7), collectionId: id, enabled: true });
    expect((await api(list("pages", a))).total).toBe(1);
    expect((await api(list("albums", "unfiled"))).total).toBe(1);
    await api({ action: "collection_save", id: a, name: "기억할 장면" });
    await api({ action: "collection_delete", id: a });
    expect((await api(list("pages", b))).total).toBe(1);
    expect((await api({ action: "summary" })).summary.keys).toHaveLength(2);
  });
  it("saves in place with one button and no collection menu or navigation", async () => {
    const parentClick = vi.fn();
    await render(<div onClick={parentClick}><BookmarkButton gallery={gallery} /></div>);
    await click("앨범 즐겨찾기 저장");
    expect(button("앨범 즐겨찾기 해제")).toHaveAttribute("aria-pressed", "true");
    expect(parentClick).not.toHaveBeenCalled();
    expect(document.querySelector('[aria-label="즐겨찾기 컬렉션"]')).toBeNull();
    expect(container.querySelectorAll("button")).toHaveLength(1);
    expect(container.querySelector(".bookmark-actions")).not.toHaveClass("is-split");
    expect((await api({ action: "summary" })).summary.keys).toHaveLength(1);
  });
  it("does not show a successful save on storage failure and permits retry", async () => {
    let fail = true;
    const unreliable: LibraryApi = (request) => request.action === "bookmark_set" && fail ? Promise.reject(new Error("디스크 저장 실패")) : api(request);
    await render(<BookmarkButton gallery={gallery} />, unreliable);
    await click("앨범 즐겨찾기 저장"); expect(button("앨범 즐겨찾기 저장")).toHaveAttribute("aria-pressed", "false");
    fail = false; await click("앨범 즐겨찾기 저장"); expect(button("앨범 즐겨찾기 해제")).toHaveAttribute("aria-pressed", "true");
  });
  it("shows legacy collected and unfiled items together in exactly two categories", async () => {
    const collectionId = (await api({ action: "collection_save", id: null, name: "장면 모음" })).collectionId!;
    await api(save()); await api(save(7)); await api(save(8));
    await api({ action: "membership_set", target: targetFor(gallery, 7), collectionId, enabled: true });
    const calls = vi.fn(api);
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={vi.fn()} />, calls);
    expect(container.querySelectorAll('[aria-label="즐겨찾기 종류"] button')).toHaveLength(2);
    expect(container.textContent).not.toMatch(/컬렉션|미분류|장면 모음/);
    expect(container.querySelectorAll(".saved-card")).toHaveLength(1);
    await click("페이지 2");
    expect(container.querySelectorAll(".saved-card")).toHaveLength(2);
    expect(calls.mock.calls.filter(([r]) => r.action === "list").every(([r]) => r.action === "list" && r.collectionId === null)).toBe(true);
    expect((await api({ action: "summary" })).summary.keys).toHaveLength(3);
  });
  it("keeps the compact card control centered and single-purpose with a saved-state marker", async () => {
    await render(<BookmarkButton gallery={gallery} compact />);
    expect(container.querySelector(".bookmark-actions")).toHaveAttribute("data-saved", "false");
    expect(container.querySelectorAll("button")).toHaveLength(1);
    await click("앨범 즐겨찾기 저장");
    expect(container.querySelector(".bookmark-actions")).toHaveAttribute("data-saved", "true");
    expect(container.querySelectorAll("button")).toHaveLength(1);
  });
  it("uses the common toolbar with independent search and working activity without duplicate sidebar actions", async () => {
    await api(save());
    const onActivity = vi.fn();
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={vi.fn()} onActivity={onActivity} />);
    expect(container).not.toHaveTextContent("탐색으로 돌아가기");
    const header = container.querySelector(".view-header")!;
    expect(header.querySelector(".search-box input")).toHaveAccessibleName("즐겨찾기 검색");
    expect(header.querySelector('button[type="submit"]')).toHaveAttribute("form", "personal-library-search");
    await click("활동 기록", header);
    expect(header.querySelector('[aria-label="프라이버시 모드"], [aria-label="설정"]')).toBeNull();
    expect(onActivity).toHaveBeenCalledOnce();
    expect(container.querySelector(".personal-collections")).toBeNull();
    await input("즐겨찾기 검색", "없는 작품");
    await act(async () => { header.querySelector<HTMLButtonElement>('button[type="submit"]')!.click(); await settle(); });
    expect(container.querySelectorAll(".saved-card")).toHaveLength(0);
  });
  it("disables repeated mutation while a save is in flight", async () => {
    let resolve!: (value: Awaited<ReturnType<LibraryApi>>) => void;
    const slow: LibraryApi = (request) => request.action === "bookmark_set" ? new Promise((done) => { resolve = done; }) : api(request);
    const calls = vi.fn(slow); await render(<BookmarkButton gallery={gallery} />, calls);
    await click("앨범 즐겨찾기 저장"); expect(button("앨범 즐겨찾기 저장")).toBeDisabled();
    await click("앨범 즐겨찾기 저장"); expect(calls.mock.calls.filter(([r]) => r.action === "bookmark_set")).toHaveLength(1);
    await act(async () => { resolve(await api(save())); });
    expect(button("앨범 즐겨찾기 해제")).toBeEnabled();
  });
  it("overlays a single bookmark control and date on the cover, with the Korean title first", async () => {
    const saved = { ...gallery, title: "Original title | 한글 제목" };
    await api({ ...save(), action: "bookmark_set", target: targetFor(saved), enabled: true, snapshot: snapshotFor(saved) });
    const onOpen = vi.fn();
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={onOpen} />);
    const card = container.querySelector(".saved-card")!;
    expect(card.querySelector(".saved-card-info strong")).toHaveTextContent(/^한글 제목$/);
    expect(card.querySelector(".saved-card-info strong")).toHaveAttribute("title", saved.title);
    expect(card.querySelector("footer")).toBeNull();
    expect(card.querySelector(".saved-card-cover .saved-card-bookmark .bookmark-actions")).toHaveAttribute("data-saved", "true");
    expect(card.querySelector(".saved-card-cover time")).toBeInTheDocument();
    // Siblings, not nested buttons: bookmark clicks must not open the gallery.
    expect(card.querySelector(".saved-card-preview button")).toBeNull();
    expect(card.querySelectorAll(".saved-card-bookmark button")).toHaveLength(1);
    await click("앨범 즐겨찾기 해제", card);
    expect(onOpen).not.toHaveBeenCalled();
    expect(container.querySelector(".saved-card")).toBeNull();
  });
  it.each([0, 7])("offers a menu on right-click for saved page %i, while Enter opens the saved target", async (page) => {
    await api(save(page)); const onOpen = vi.fn(), calls = vi.fn(api);
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={onOpen} />, calls);
    if (page) await click("페이지 1");
    const card = container.querySelector<HTMLElement>(".saved-card")!;
    for (const target of [card.querySelector(".saved-card-preview")!, card.querySelector(".saved-card-info strong")!]) {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 });
      await act(async () => { target.dispatchEvent(event); await settle(); });
      expect(event.defaultPrevented).toBe(true);
      expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
      await act(async () => { [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((item) => item.textContent === (page ? "앨범 상세 열기" : "상세 열기"))!.click(); await settle(); });
      expect(onOpen).toHaveBeenLastCalledWith(expect.objectContaining({ galleryId: gallery.id, page }), page ? { detailOnly: true } : undefined);
      expect(document.activeElement).toBe(card);
    }
    expect(calls.mock.calls.filter(([request]) => request.action === "get")).toHaveLength(2);
    await act(async () => { card.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); await settle(); });
    expect(onOpen).toHaveBeenLastCalledWith(expect.objectContaining({ page }), undefined);
    await act(async () => { card.querySelector(".saved-card-info")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })); await settle(); });
    expect(onOpen).toHaveBeenCalledTimes(4);
    // The bookmark controls have their own keyboard and context-menu scope.
    await act(async () => { card.querySelector(".bookmark-toggle")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2 })); });
    expect(onOpen).toHaveBeenCalledTimes(4);
  });
  it("loads a bounded list, opens the precise page and checks its state again before opening", async () => {
    for (const page of [5, 6, 7]) await api(save(page));
    const onOpen = vi.fn(); const calls = vi.fn(api);
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={2} privacyMode={false} onBack={vi.fn()} onOpen={onOpen} />, calls);
    await click("페이지 3");
    expect(container.querySelectorAll(".saved-card")).toHaveLength(2);
    const opening = container.querySelector<HTMLButtonElement>(".saved-card-preview")!;
    await act(async () => { opening.click(); await settle(); });
    expect(calls.mock.calls.some(([request]) => request.action === "get")).toBe(true);
    expect(onOpen.mock.calls[0]![0].page).toBeGreaterThan(0);
    await click("더 보기 · 2/3"); expect(container.querySelectorAll(".saved-card")).toHaveLength(3);
  });
  it("keeps excluded and changed bookmarks visible without loading their images", async () => {
    await api(save(7)); const onOpen = vi.fn();
    const excluded: LibraryApi = async (request) => { const result = await api(request); result.items.forEach((item) => { item.reference.status = "changed"; }); return result; };
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={onOpen} />, excluded);
    await click("페이지 1"); expect(container.querySelector(".saved-card-preview")).toBeDisabled();
    expect(container.querySelector(".saved-thumbnail")).toBeNull(); expect(container.textContent).toContain("저장 당시 페이지와 달라짐");
    await act(async () => {
      container.querySelector(".saved-card")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      container.querySelector(".saved-card")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await settle();
    });
    expect(onOpen).not.toHaveBeenCalled(); expect((await api({ action: "summary" })).summary.keys).toHaveLength(1);
  });
  it("retains the warning if an item becomes excluded between listing and opening", async () => {
    await api(save()); const onOpen = vi.fn(); let removed = false;
    const changing: LibraryApi = async (request) => {
      const result = await api(request);
      if (request.action === "get") removed = true;
      if (removed) result.items.forEach((item) => { item.reference.status = "excluded"; });
      return result;
    };
    await render(<PersonalLibraryWorkspace previewWidth={220} pageSize={50} privacyMode={false} onBack={vi.fn()} onOpen={onOpen} />, changing);
    await act(async () => { container.querySelector<HTMLButtonElement>(".saved-card-preview")!.click(); await settle(); });
    expect(onOpen).not.toHaveBeenCalled();
    expect(container.querySelector('.personal-library-error[role="status"]')).toHaveTextContent("원본 상태를 먼저 확인해 주세요");
    expect(container.querySelector(".saved-card-preview")).toBeDisabled();
  });
  it("opens a saved page in the existing preview and never reopens it on metadata rerenders", async () => {
    const props = { tabs: [gallery.id], activeId: gallery.id, minimized: false, galleries: new Map([[gallery.id, gallery]]), favoriteMetadata: new Set<string>(), thumbnailClient: client,
      onActivate: vi.fn(), onClose: vi.fn(), onCloseAll: vi.fn(), onMinimize: vi.fn(), onRestore: vi.fn(), onOpenRelated: vi.fn(), onQueue: vi.fn(), onMetadataSearch: vi.fn(), onMetadataFavorite: vi.fn() };
    await render(<DetailWorkspace {...props} pageOpenRequest={{ galleryId: gallery.id, page: 7, sequence: 1 }} />);
    expect(container.querySelector("#page-preview-title")?.textContent).toContain("7페이지");
    await click("7페이지 즐겨찾기 저장");
    expect((await api({ action: "summary" })).summary.keys.map((key) => key.page)).toEqual([7]);
    await click("페이지 미리보기 닫기");
    await render(<DetailWorkspace {...props} galleries={new Map([[gallery.id, { ...gallery }]])} pageOpenRequest={{ galleryId: gallery.id, page: 7, sequence: 1 }} />);
    expect(container.querySelector(".page-preview-dialog")).not.toHaveAttribute("open");
    await render(<DetailWorkspace {...props} pageOpenRequest={{ galleryId: gallery.id, page: 7, sequence: 2 }} />);
    expect(container.querySelector("#page-preview-title")?.textContent).toContain("7페이지");
    await click("두쪽 보기"); await click("8페이지 즐겨찾기 저장");
    expect((await api({ action: "summary" })).summary.keys.map((key) => key.page)).toEqual([7, 8]);
    expect(container.querySelector(".page-preview-dialog .bookmark-collections")).toBeNull();
    expect(container.querySelector(".page-preview-dialog")).toHaveAttribute("open");
  });
});
