import { afterEach, describe, expect, it, vi } from "vitest";
import type { OfficialBrowserViewport } from "../../api/officialBrowser";
import { hasBlockingNativeOverlay, hasNativeOverlay, nativeModalOcclusion, observeNativeOverlayGeometry } from "./nativeOverlayGeometry";

function rect(x: number, y: number, width: number, height: number): DOMRect { return { x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON: () => ({}) }; }
function stage(x = 100, y = 100, width = 800, height = 600) {
  const element = document.createElement("div"); document.body.append(element);
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue(rect(x, y, width, height));
  const viewport: OfficialBrowserViewport = { x, y, width, height, visible: true, clip: { x: 0, y: 0, width, height } };
  return { element, viewport };
}
function popup(x: number, y: number, width: number, height: number) {
  const overlay = document.createElement("div"); overlay.dataset.nativeOverlay = "true"; overlay.dataset.nativePreserveVideo = "true";
  const surface = document.createElement("div"); surface.dataset.nativeDialogSurface = "true"; overlay.append(surface); document.body.append(overlay);
  vi.spyOn(surface, "getBoundingClientRect").mockReturnValue(rect(x, y, width, height));
  return { overlay, surface };
}
function panel(x: number, y: number, width: number, height: number) {
  const result = popup(x, y, width, height);
  result.overlay.setAttribute("role", "dialog");
  result.overlay.setAttribute("aria-modal", "false");
  result.overlay.dataset.nativeInteractiveBackground = "true";
  return result;
}
afterEach(() => { document.body.replaceChildren(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("native popup occlusion geometry", () => {
  it("measures the lifting tab instead of erasing its larger stationary hitbox", () => {
    const { element, viewport } = stage();
    const { overlay, surface } = popup(420, 656, 170, 58);
    overlay.remove();
    // Keep a real button/face hierarchy as rendered by MadoWorkspace.
    const handle = document.createElement('button'); handle.className = 'live-bottom-handle';
    handle.dataset.nativeOverlay = handle.dataset.nativePreserveVideo = handle.dataset.nativeInteractiveBackground = 'true';
    handle.append(surface); document.body.append(handle);
    vi.spyOn(handle, 'getBoundingClientRect').mockReturnValue(rect(420, 642, 170, 58));
    expect(nativeModalOcclusion(element, viewport)).toMatchObject({ occluded: false, occlusions: [{ x: 320, y: 556, width: 170, height: 44 }] });
    vi.mocked(surface.getBoundingClientRect).mockReturnValue(rect(420, 642, 170, 58));
    expect(nativeModalOcclusion(element, viewport)).toMatchObject({ occluded: false, occlusions: [{ x: 320, y: 542, width: 170, height: 58 }] });
  });
  it("bounds one-off surface transitions and ignores repeated animations and child spinners", () => {
    vi.useFakeTimers();
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(performance.now()), 16));
    vi.stubGlobal('cancelAnimationFrame', clearTimeout);
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    const { overlay, surface } = popup(400, 600, 170, 58);
    const update = vi.fn(), stop = observeNativeOverlayGeometry(update);
    surface.dispatchEvent(new Event('animationstart', { bubbles: true }));
    vi.advanceTimersByTime(450); const stopped = update.mock.calls.length;
    expect(stopped).toBeGreaterThan(0);
    vi.advanceTimersByTime(1000); expect(update).toHaveBeenCalledTimes(stopped);
    surface.dispatchEvent(new Event('animationiteration', { bubbles: true }));
    const spinner = document.createElement('i'); surface.append(spinner);
    spinner.dispatchEvent(new Event('animationstart', { bubbles: true }));
    vi.advanceTimersByTime(2000); expect(update).toHaveBeenCalledTimes(stopped);
    surface.style.animationIterationCount = 'infinite';
    surface.dispatchEvent(new Event('animationstart', { bubbles: true }));
    vi.advanceTimersByTime(2000); expect(update).toHaveBeenCalledTimes(stopped);
    const beforeReveal = update.mock.calls.length;
    overlay.dispatchEvent(new Event('transitionrun', { bubbles: true }));
    vi.advanceTimersByTime(32); expect(update.mock.calls.length).toBeGreaterThan(beforeReveal);
    stop(); const final = update.mock.calls.length;
    surface.dispatchEvent(new Event('animationiteration', { bubbles: true }));
    vi.advanceTimersByTime(5000); expect(update).toHaveBeenCalledTimes(final);
  });
  it("cuts only the visible pull handle without blocking the remaining video", () => {
    const { element, viewport } = stage();
    const handle = document.createElement('button');
    handle.className = 'live-bottom-handle';
    handle.dataset.nativeOverlay = handle.dataset.nativePreserveVideo = handle.dataset.nativeInteractiveBackground = 'true';
    document.body.append(handle);
    vi.spyOn(handle, 'getBoundingClientRect').mockReturnValue(rect(420, 684, 140, 34));
    expect(hasBlockingNativeOverlay()).toBe(false);
    expect(nativeModalOcclusion(element, viewport)).toMatchObject({ occluded: false, preserveBackground: true, occlusions: [{ x: 320, y: 584, width: 140, height: 16 }] });
  });
  it("keeps the measured curved outline across native pane edges", () => {
    const { element, viewport } = stage();
    const { surface } = panel(80, 660, 760, 80);
    surface.style.borderTopLeftRadius = '18px'; surface.style.borderTopRightRadius = '18px';
    expect(nativeModalOcclusion(element, viewport)).toMatchObject({ occluded: false, occlusions: [],
      roundedOcclusions: [{ x: -20, y: 560, width: 760, height: 80, radii: [18, 18, 0, 0] }] });
  });
  it("cuts only the popup rectangle, retaining both sides and everything below it", () => {
    const { element, viewport } = stage(); popup(400, 300, 200, 150);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip,
      occlusions: [{ x: 300, y: 200, width: 200, height: 150 }] });
    expect(viewport).toMatchObject({ width: 800, height: 600, clip: { height: 600 } });
  });
  it("uses each of four panes' local coordinates instead of erasing a shared horizontal strip", () => {
    const panes = [stage(0, 0, 500, 300), stage(500, 0, 500, 300), stage(0, 300, 500, 300), stage(500, 300, 500, 300)];
    popup(450, 250, 100, 100);
    expect(panes.map(({ element, viewport }) => nativeModalOcclusion(element, viewport).occlusions)).toEqual([
      [{ x: 450, y: 250, width: 50, height: 50 }], [{ x: 0, y: 250, width: 50, height: 50 }],
      [{ x: 450, y: 0, width: 50, height: 50 }], [{ x: 0, y: 0, width: 50, height: 50 }],
    ]);
  });
  it("keeps a nonoverlapping pane painted but input-disabled while a global modal is open", () => {
    const { element, viewport } = stage(); popup(1000, 100, 100, 100);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip, occlusions: [] });
  });
  it("keeps video and chat interactive outside an explicitly nonmodal panel", () => {
    const { element, viewport } = stage(); panel(400, 300, 200, 150);
    expect(hasNativeOverlay()).toBe(true);
    expect(hasBlockingNativeOverlay()).toBe(false);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: false, preserveBackground: true, clip: viewport.clip,
      occlusions: [{ x: 300, y: 200, width: 200, height: 150 }] });
  });
  it("does not block a native pane outside the nonmodal panel's bounds", () => {
    const { element, viewport } = stage(); panel(1000, 100, 100, 100);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: false, preserveBackground: true, clip: viewport.clip, occlusions: [] });
  });
  it("lets a real modal override the nonmodal panel without trusting an interactive flag", () => {
    const { element, viewport } = stage(); panel(400, 300, 200, 150);
    const { overlay } = panel(200, 200, 100, 100);
    for (const role of ["alertdialog", "menu"]) {
      overlay.setAttribute("role", role);
      expect(hasBlockingNativeOverlay()).toBe(true);
      expect(nativeModalOcclusion(element, viewport).occluded).toBe(true);
    }
    overlay.setAttribute("role", "dialog"); overlay.setAttribute("aria-modal", "true");
    expect(hasBlockingNativeOverlay()).toBe(true);
    expect(nativeModalOcclusion(element, viewport).occluded).toBe(true);
    overlay.remove();
    expect(hasBlockingNativeOverlay()).toBe(false);
    expect(nativeModalOcclusion(element, viewport).occluded).toBe(false);
  });
  it("still masks invalid or untrusted nonmodal panel geometry", () => {
    const { element, viewport } = stage(); const { overlay, surface } = panel(200, 200, 100, 100);
    vi.mocked(surface.getBoundingClientRect).mockReturnValue(rect(200, 200, 0, 100));
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true });
    overlay.dataset.nativePreserveVideo = "false";
    expect(hasBlockingNativeOverlay()).toBe(true);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true });
  });
  it("intersects holes with an existing scroll clip without changing the original video geometry", () => {
    const { element, viewport } = stage(); viewport.clip = { x: 50, y: 100, width: 500, height: 400 }; popup(100, 100, 200, 200);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip,
      occlusions: [{ x: 50, y: 100, width: 150, height: 100 }] });
  });
  it("preserves fail-closed masking for unknown, unmeasured, hidden-workspace and excessive overlays", () => {
    const { element, viewport } = stage(); const { overlay, surface } = popup(200, 200, 200, 200);
    overlay.dataset.nativePreserveVideo = "false"; expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true });
    overlay.dataset.nativePreserveVideo = "true"; vi.mocked(surface.getBoundingClientRect).mockReturnValue(rect(NaN, 200, 200, 200));
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true });
    vi.mocked(surface.getBoundingClientRect).mockReturnValue(rect(200, 200, 200, 200));
    expect(nativeModalOcclusion(element, { ...viewport, visible: false })).toEqual({ occluded: true });
    for (let i = 0; i < 8; i++) popup(200 + i, 210 + i, 100, 100);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true });
  });
  it("ignores hidden overlays and restores the plain viewport after popup dismissal", () => {
    const { element, viewport } = stage(); const { overlay } = popup(200, 200, 100, 100);
    expect(hasNativeOverlay()).toBe(true); overlay.hidden = true;
    expect(hasNativeOverlay()).toBe(false); expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: false });
    overlay.hidden = false; overlay.remove(); expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: false });
  });
  it("tracks popup-only size changes and disconnects after teardown", async () => {
    let callback!: ResizeObserverCallback;
    const observed = new Set<Element>();
    const disconnect = vi.fn(() => observed.clear());
    class Observer {
      constructor(next: ResizeObserverCallback) { callback = next; }
      observe(element: Element) { observed.add(element); }
      unobserve(element: Element) { observed.delete(element); }
      disconnect = disconnect;
    }
    vi.stubGlobal("ResizeObserver", Observer);
    const update = vi.fn(); const stop = observeNativeOverlayGeometry(update);
    const { surface, overlay } = popup(200, 200, 100, 100);
    await Promise.resolve(); expect(observed.has(surface)).toBe(true);
    callback([{ target: surface } as unknown as ResizeObserverEntry], {} as ResizeObserver); expect(update).toHaveBeenCalledTimes(1);
    overlay.remove(); await Promise.resolve(); expect(observed.has(surface)).toBe(false);
    stop(); callback([], {} as ResizeObserver);
    expect(disconnect).toHaveBeenCalledOnce(); expect(update).toHaveBeenCalledTimes(1);
  });
});
