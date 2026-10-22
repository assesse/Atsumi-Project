import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PixivPageReader } from "./PixivPageReader";
import { createOriginalWindow } from "./originalWindow";
import type { PixivDetail } from "./model";
const detail = (count = 5): PixivDetail => ({ work: { id: "123", title: "Original title", artist: "Artist", artistId: "4", width: 600, height: 900, pageCount: count, tags: [], createdAt: "", bookmarkCount: 0, type: "manga" }, pages: Array.from({ length: count }, (_, index) => ({ index, width: index === 2 ? 1400 : 600, height: 900, previewUrl: "", originalUrl: "" })) });
let host: HTMLDivElement, root: Root;
let pool: ReturnType<typeof createOriginalWindow>;
const onClose = vi.fn(), onFavorite = vi.fn();
async function render(page = 0, count = 5, privacy = false, options: Partial<ComponentProps<typeof PixivPageReader>> = {}) { await act(async () => { root.render(<PixivPageReader detail={detail(count)} initialPage={page} pool={pool} privacyMode={privacy} isFavorite={() => false} onFavorite={onFavorite} onClose={onClose} {...options} />); }); }
const control = (label: string) => document.querySelector<HTMLButtonElement>(`.pixiv-reader button[aria-label="${label}"]`)!;
async function click(label: string) { await act(async () => { control(label).dispatchEvent(new MouseEvent("click", { bubbles: true })); }); }
async function key(value: string, options?: KeyboardEventInit) { await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, ...options })); }); }
const pages = () => [...document.querySelectorAll(".pixiv-reader [data-page]")].map(node => node.getAttribute("data-page"));
const reader = () => document.querySelector<HTMLElement>(".pixiv-reader")!;
const handle = (edge: string) => document.querySelector<HTMLElement>(`[data-pixiv-resize-edge="${edge}"]`)!;
const originalViewport = { width: window.innerWidth, height: window.innerHeight };
function viewport(width: number, height: number) { Object.defineProperty(window, "innerWidth", { value: width, configurable: true }); Object.defineProperty(window, "innerHeight", { value: height, configurable: true }); }
async function pointer(target: EventTarget, type: string, x: number, y: number, id = 1, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button });
  Object.defineProperties(event, { pointerId: { value: id }, isPrimary: { value: true } });
  await act(async () => target.dispatchEvent(event));
}
describe("Pixiv separate reader", () => {
  beforeEach(() => { viewport(1000, 800); Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); localStorage.clear(); vi.useFakeTimers(); onClose.mockClear(); onFavorite.mockClear(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); pool = createOriginalWindow({ localMedia: vi.fn().mockResolvedValue({ url: "data:image/png;base64,eA==", bytes: 1 }), pageMedia: vi.fn().mockResolvedValue({ url: "data:image/png;base64,eA==", bytes: 1 }) }); });
  afterEach(async () => { await act(async () => root.unmount()); pool.clear(); host.remove(); viewport(originalViewport.width, originalViewport.height); vi.useRealTimers(); vi.restoreAllMocks(); });
  it("puts related works after the full-height page and keeps related keyboard controls independent", async () => {
    const onScrollPosition = vi.fn();
    await render(1, 5, false, { related: <button className="fixture-related">Related card</button>, onScrollPosition });
    const body = reader().querySelector<HTMLElement>(".pixiv-reader-scroll")!;
    expect(body.children[0]?.className).toBe("pixiv-reader-page"); expect(body.children[1]?.className).toBe("pixiv-reader-related");
    expect(body.querySelector(".pixiv-reader-page .pixiv-original-pages")).not.toBeNull();
    await act(async () => { document.querySelector(".fixture-related")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })); });
    expect(pages()).toEqual(["2"]);
    body.scrollTop = 600; await act(async () => body.dispatchEvent(new Event("scroll")));
    expect(onScrollPosition).toHaveBeenLastCalledWith(600);
    expect(pages()).toEqual(["2"]); expect(onClose).not.toHaveBeenCalled();
  });
  it("starts on the chosen original, reveals only on movement and hides stationary controls", async () => {
    await render(2); const reader = document.querySelector(".pixiv-reader")!; expect(pages()).toEqual(["3"]); expect(reader.getAttribute("data-controls-visible")).toBe("false");
    await act(async () => reader.dispatchEvent(new MouseEvent("pointermove", { bubbles: true, clientX: 10 })));
    expect(reader.getAttribute("data-controls-visible")).toBe("true");
    await act(async () => vi.advanceTimersByTime(1900)); expect(reader.getAttribute("data-controls-visible")).toBe("false");
    await act(async () => reader.dispatchEvent(new MouseEvent("pointermove", { bubbles: true, clientX: 10 })));
    expect(reader.getAttribute("data-controls-visible")).toBe("false");
    await click("미리보기 UI 고정"); await act(async () => vi.advanceTimersByTime(1900)); expect(reader.getAttribute("data-controls-visible")).toBe("true");
    expect(JSON.parse(localStorage.getItem("atsumi.pixiv.reader.v1")!)).toEqual({ reading: "ltr", pinned: true });
  });
  it("pins direct entry without losing reading direction, and manual unpin survives parent updates", async () => {
    localStorage.setItem("atsumi.pixiv.reader.v1", JSON.stringify({ reading: "rtl", pinned: false }));
    await render(1, 5, false, { initialPinned: true });
    expect(reader().getAttribute("data-controls-visible")).toBe("true"); expect(reader().getAttribute("data-reading")).toBe("rtl");
    await click("미리보기 UI 고정"); await act(async () => vi.advanceTimersByTime(1900));
    expect(reader().getAttribute("data-controls-visible")).toBe("false");
    await render(1, 5, false, { initialPinned: true, toolbar: <button>다운로드</button> });
    expect(reader().getAttribute("data-controls-visible")).toBe("false"); expect(control("미리보기 UI 고정").getAttribute("aria-pressed")).toBe("false");
    await act(async () => root.render(null)); await render(1, 5, false, { initialPinned: true });
    expect(reader().getAttribute("data-controls-visible")).toBe("true");
  });
  it("keeps numbered page favorites next to the left pin and a separate centered page count", async () => {
    await render(1); await click("두쪽 보기");
    const left = reader().querySelector(".pixiv-reader-left-tools")!;
    expect(left.contains(control("미리보기 UI 고정"))).toBe(true);
    expect(left.contains(control("2쪽 즐겨찾기"))).toBe(true); expect(left.contains(control("3쪽 즐겨찾기"))).toBe(true);
    expect(control("2쪽 즐겨찾기").querySelector("span")?.textContent).toBe("2");
    expect(reader().querySelector('[aria-label="현재 쪽"]')?.textContent).toBe("2–3 / 5");
    expect(reader().querySelector(".pixiv-reader-page-status button")).toBeNull();
    await click("3쪽 즐겨찾기"); expect(onFavorite).toHaveBeenCalledWith(2);
  });
  it("reports every changed real page using the latest callback without resetting page, resize or focus", async () => {
    const previous = vi.fn(), latest = vi.fn();
    await render(1, 5, false, { onPageChange: previous }); expect(previous).toHaveBeenLastCalledWith(1);
    await key("d"); expect(previous).toHaveBeenLastCalledWith(2);
    await pointer(handle("right"), "pointerdown", 500, 400); await pointer(window, "pointermove", 540, 400); await pointer(window, "pointerup", 540, 400);
    const width = reader().style.width; await act(async () => control("두쪽 보기").focus());
    await render(0, 5, false, { onPageChange: latest });
    expect(pages()).toEqual(["3"]); expect(reader().style.width).toBe(width); expect(document.activeElement).toBe(control("두쪽 보기")); expect(latest).not.toHaveBeenCalled();
    await key("d"); expect(latest).toHaveBeenLastCalledWith(3); expect(previous).toHaveBeenCalledTimes(2);
    await click("두쪽 보기"); await key("a"); expect(latest).toHaveBeenLastCalledWith(1);
    await key("d"); expect(latest).toHaveBeenLastCalledWith(3);
  });
  it("renders supplied work controls below title and cycles works only with an active callback", async () => {
    const cycle = vi.fn(); await render(1, 5, false, { initialPinned: true, toolbar: <><button>앨범</button><button>다운로드</button><button>웹에서 열기</button></>, navigation: <button role="tab">다른 작품</button>, onCycleWork: cycle });
    const header = reader().querySelector(".pixiv-reader-header")!;
    expect([...header.children].map(node => node.className)).toEqual(["pixiv-reader-title", "pixiv-reader-toolbar", "pixiv-reader-navigation"]);
    await key("q"); await key("E"); expect(cycle.mock.calls).toEqual([[-1], [1]]); expect(pages()).toEqual(["2"]);
    await key("q", { ctrlKey: true }); await key("e", { altKey: true }); await key("q", { metaKey: true }); await key("e", { isComposing: true });
    const editor = document.createElement("div"); editor.contentEditable = "true"; editor.setAttribute("contenteditable", "true"); editor.innerHTML = "<span>composing</span>"; reader().append(editor);
    await act(async () => editor.firstElementChild!.dispatchEvent(new KeyboardEvent("keydown", { key: "e", bubbles: true, cancelable: true })));
    expect(cycle).toHaveBeenCalledTimes(2); editor.remove();
    const overlay = document.createElement("section"); overlay.setAttribute("aria-modal", "true"); document.body.append(overlay); await key("e"); expect(cycle).toHaveBeenCalledTimes(2); overlay.remove();
    await render(1); const event = new KeyboardEvent("keydown", { key: "q", bubbles: true, cancelable: true }); await act(async () => window.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(false); expect(cycle).toHaveBeenCalledTimes(2);
  });
  it("vetoes close before animation and gives a confirmation exclusive Escape and backdrop ownership", async () => {
    const beforeClose = vi.fn().mockReturnValue(false); await render(2, 5, false, { onBeforeClose: beforeClose });
    await click("미리보기 닫기"); expect(beforeClose).toHaveBeenCalledWith(2);
    expect(document.querySelector(".pixiv-reader-backdrop.is-closing")).toBeNull(); await act(async () => vi.advanceTimersByTime(200)); expect(onClose).not.toHaveBeenCalled();
    const confirmation = document.createElement("section"); confirmation.setAttribute("role", "alertdialog"); confirmation.setAttribute("aria-modal", "true"); document.body.append(confirmation);
    await key("Escape"); await click("미리보기 닫기");
    await act(async () => document.querySelector(".pixiv-reader-backdrop")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    expect(beforeClose).toHaveBeenCalledTimes(1); expect(onClose).not.toHaveBeenCalled(); confirmation.remove();
    beforeClose.mockReturnValue(true); await key("Escape"); expect(document.querySelector(".pixiv-reader-backdrop.is-closing")).not.toBeNull();
    await act(async () => vi.advanceTimersByTime(150)); expect(onClose).toHaveBeenCalledTimes(1); expect(onClose).toHaveBeenCalledWith(2);
  });
  it("restores the Tab cycle from outside after a confirmation closes without stealing another modal focus", async () => {
    await render(); const outside = document.createElement("button"); document.body.append(outside);
    await act(async () => outside.focus()); await key("Tab"); expect(document.activeElement).toBe(control("미리보기 닫기"));
    await act(async () => outside.focus()); await key("Tab", { shiftKey: true }); expect(document.activeElement).toBe(handle("bottom"));
    const confirmation = document.createElement("section"); confirmation.setAttribute("aria-modal", "true"); const cancel = document.createElement("button"); confirmation.append(cancel); document.body.append(confirmation);
    await act(async () => cancel.focus()); await key("Tab"); expect(document.activeElement).toBe(cancel);
    await act(async () => { cancel.blur(); window.dispatchEvent(new Event("focus")); }); expect(document.activeElement).toBe(document.body);
    confirmation.remove(); await key("Tab"); expect(document.activeElement).toBe(control("미리보기 닫기")); outside.remove();
  });
  it.each([1, 2, 5, 6])("preserves final black blank and direction for %i pages", async count => {
    await render(count - 1, count); await click("두쪽 보기"); expect(pages()).toEqual([String(count), "blank"]);
    await click("오른쪽부터 읽기"); expect(pages()).toEqual(["blank", String(count)]);
    expect(document.querySelectorAll(".pixiv-reader-bookmark")).toHaveLength(1);
    await click(`${count}쪽 즐겨찾기`); expect(onFavorite).toHaveBeenCalledWith(count - 1);
    await click("두쪽 보기"); expect(pages()).toEqual([String(count)]);
  });
  it("keeps spread across landscape pages and navigates physical arrows in RTL", async () => {
    await render(1); await click("두쪽 보기"); expect(pages()).toEqual(["2", "3"]);
    await key("a"); expect(pages()).toEqual(["blank", "1"]);
    await click("오른쪽부터 읽기"); expect(pages()).toEqual(["1", "blank"]);
    await key("a"); expect(pages()).toEqual(["3", "2"]);
    expect(document.querySelector(".pixiv-reader")?.getAttribute("data-spread")).toBe("true");
    expect(document.querySelector(".pixiv-reader-arrow.is-left .is-turning")).not.toBeNull();
    await key("d", { ctrlKey: true }); expect(pages()).toEqual(["3", "2"]);
  });
  it("newer dialogs own shortcuts and closing reports only the reader page", async () => {
    await render(1); const overlay = document.createElement("section"); overlay.setAttribute("aria-modal", "true"); document.body.append(overlay);
    await key("d"); await key("Escape"); expect(pages()).toEqual(["2"]); expect(onClose).not.toHaveBeenCalled(); overlay.remove();
    await key("d"); await key("Escape"); await act(async () => vi.advanceTimersByTime(150)); expect(onClose).toHaveBeenCalledWith(2);
  });
  it("privacy clears originals and no content image or title is exposed", async () => {
    await render(0, 5, true); expect(document.querySelector(".pixiv-reader img")).toBeNull(); expect(document.querySelector(".pixiv-reader-header")?.textContent).not.toContain("Original title"); expect(document.querySelector(".pixiv-reader")?.textContent).toContain("프라이버시");
  });
  it.each(["right", "bottom", "corner"])("resizes the %s edge with existing centered geometry only", async edge => {
    await render(); const before = { width: Number.parseFloat(reader().style.width), height: Number.parseFloat(reader().style.height) };
    await pointer(handle(edge), "pointerdown", 500, 600);
    await pointer(window, "pointermove", 525, 575);
    expect(Number.parseFloat(reader().style.width)).toBe(before.width + (edge === "bottom" ? 0 : 50));
    expect(Number.parseFloat(reader().style.height)).toBe(before.height + (edge === "right" ? 0 : -50));
    expect(Number.parseFloat(reader().style.left)).toBe(Math.round((1000 - Number.parseFloat(reader().style.width)) / 2));
    expect(Number.parseFloat(reader().style.top)).toBe(Math.round((800 - Number.parseFloat(reader().style.height)) / 2));
    await pointer(window, "pointerup", 525, 575);
    expect(reader().classList.contains("is-edge-resizing")).toBe(false);
    const width = reader().style.width; await pointer(window, "pointermove", 700, 700); expect(reader().style.width).toBe(width);
    expect(pages()).toEqual(["1"]); expect(onFavorite).not.toHaveBeenCalled(); expect(onClose).not.toHaveBeenCalled();
  });
  it("clamps minimum/viewport and retains chosen size across spread and mixed page turns", async () => {
    await render(1); await pointer(handle("corner"), "pointerdown", 500, 600); await pointer(window, "pointermove", -5000, -5000); await pointer(window, "pointerup", -5000, -5000);
    expect(reader().style.width).toBe("320px"); expect(reader().style.height).toBe("320px");
    await click("두쪽 보기"); await click("오른쪽부터 읽기"); await key("a");
    expect(reader().style.width).toBe("320px"); expect(reader().style.height).toBe("320px"); expect(reader().getAttribute("data-spread")).toBe("true");
    await pointer(handle("corner"), "pointerdown", 500, 600); await pointer(window, "pointermove", 5000, 5000); await pointer(window, "pointerup", 5000, 5000);
    expect(reader().style.width).toBe("976px"); expect(reader().style.height).toBe("776px"); expect(reader().style.left).toBe("12px"); expect(reader().style.top).toBe("12px");
    await act(async () => { viewport(600, 440); window.dispatchEvent(new Event("resize")); });
    expect(reader().style.width).toBe("576px"); expect(reader().style.height).toBe("416px"); expect(reader().style.left).toBe("12px"); expect(reader().style.top).toBe("12px");
  });
  it("ignores other pointers/right button and cancels drag on pointercancel or window blur", async () => {
    await render(); const original = reader().style.width;
    await pointer(handle("right"), "pointerdown", 500, 400, 1, 2); await pointer(window, "pointermove", 560, 400); expect(reader().style.width).toBe(original);
    await pointer(handle("right"), "pointerdown", 500, 400); await pointer(window, "pointermove", 560, 400, 2); expect(reader().style.width).toBe(original);
    await pointer(window, "pointercancel", 500, 400, 2); expect(reader().classList.contains("is-edge-resizing")).toBe(true);
    await pointer(window, "pointercancel", 500, 400); expect(reader().classList.contains("is-edge-resizing")).toBe(false);
    await pointer(handle("right"), "pointerdown", 500, 400); await act(async () => window.dispatchEvent(new Event("blur"))); await pointer(window, "pointermove", 580, 400); expect(reader().style.width).toBe(original);
    await act(async () => { document.body.focus(); window.dispatchEvent(new Event("focus")); }); await key("d"); expect(pages()).toEqual(["2"]);
  });
  it("separator keys resize rather than turn pages and remain in the focus cycle", async () => {
    await render(1); const right = handle("right"), bottom = handle("bottom");
    await act(async () => right.focus()); const width = Number.parseFloat(reader().style.width);
    await act(async () => right.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true })));
    expect(Number.parseFloat(reader().style.width)).toBe(width + 24); expect(pages()).toEqual(["2"]);
    await act(async () => right.dispatchEvent(new KeyboardEvent("keydown", { key: "d", bubbles: true, cancelable: true }))); expect(pages()).toEqual(["3"]);
    await act(async () => bottom.focus()); const height = Number.parseFloat(reader().style.height);
    await act(async () => bottom.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true, shiftKey: true })));
    expect(Number.parseFloat(reader().style.height)).toBe(height - 96); expect(pages()).toEqual(["3"]);
    await key("Tab"); expect(document.activeElement).toBe(control("미리보기 닫기"));
    await key("Tab", { shiftKey: true }); expect(document.activeElement).toBe(bottom);
  });
  it("keeps controls available during drag and releases capture when a newer modal or close takes over", async () => {
    await render(); const right = handle("right"), release = vi.fn();
    right.setPointerCapture = vi.fn(); right.hasPointerCapture = vi.fn().mockReturnValue(true); right.releasePointerCapture = release;
    await pointer(right, "pointerdown", 500, 400); await act(async () => vi.advanceTimersByTime(1900)); expect(reader().getAttribute("data-controls-visible")).toBe("true");
    const overlay = document.createElement("section"); overlay.setAttribute("aria-modal", "true"); document.body.append(overlay);
    const width = reader().style.width; await pointer(window, "pointermove", 560, 400); expect(reader().style.width).toBe(width); expect(release).toHaveBeenCalledWith(1); overlay.remove();
    await pointer(right, "pointerdown", 500, 400); await key("Escape"); expect(release).toHaveBeenCalledTimes(2); await act(async () => vi.advanceTimersByTime(150)); expect(onClose).toHaveBeenCalledWith(0);
  });
  it("ignores obsolete lost-capture signals and releases its pointer on unmount", async () => {
    await render(); const right = handle("right"), release = vi.fn();
    right.setPointerCapture = vi.fn(); right.hasPointerCapture = vi.fn().mockReturnValue(true); right.releasePointerCapture = release;
    await pointer(right, "pointerdown", 500, 400);
    await pointer(right, "lostpointercapture", 500, 400, 2); expect(reader().classList.contains("is-edge-resizing")).toBe(true);
    await pointer(right, "lostpointercapture", 500, 400); expect(reader().classList.contains("is-edge-resizing")).toBe(true);
    vi.mocked(right.hasPointerCapture).mockReturnValue(false); await pointer(right, "lostpointercapture", 500, 400); expect(reader().classList.contains("is-edge-resizing")).toBe(false);
    vi.mocked(right.hasPointerCapture).mockReturnValue(true); await pointer(right, "pointerdown", 500, 400);
    await act(async () => root.render(null)); expect(release).toHaveBeenCalledWith(1);
    await pointer(window, "pointermove", 700, 400); expect(document.querySelector(".pixiv-reader")).toBeNull(); expect(onClose).not.toHaveBeenCalled();
  });
});
