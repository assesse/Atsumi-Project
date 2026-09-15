import { afterEach, describe, expect, it, vi } from "vitest";
import type { OfficialBrowserViewport } from "../../api/officialBrowser";
import { hasNativeOverlay, nativeModalOcclusion, observeNativeOverlayGeometry } from "./nativeOverlayGeometry";

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
afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("native popup occlusion geometry", () => {
  it("cuts only the popup rectangle, retaining both sides and everything below it", () => {
    const { element, viewport } = stage(); popup(400, 300, 200, 150);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip,
      occlusions: [{ x: 292, y: 192, width: 216, height: 166 }] });
    expect(viewport).toMatchObject({ width: 800, height: 600, clip: { height: 600 } });
  });
  it("uses each of four panes' local coordinates instead of erasing a shared horizontal strip", () => {
    const panes = [stage(0, 0, 500, 300), stage(500, 0, 500, 300), stage(0, 300, 500, 300), stage(500, 300, 500, 300)];
    popup(450, 250, 100, 100);
    expect(panes.map(({ element, viewport }) => nativeModalOcclusion(element, viewport).occlusions)).toEqual([
      [{ x: 442, y: 242, width: 58, height: 58 }], [{ x: 0, y: 242, width: 58, height: 58 }],
      [{ x: 442, y: 0, width: 58, height: 58 }], [{ x: 0, y: 0, width: 58, height: 58 }],
    ]);
  });
  it("keeps a nonoverlapping pane painted but input-disabled while a global modal is open", () => {
    const { element, viewport } = stage(); popup(1000, 100, 100, 100);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip, occlusions: [] });
  });
  it("intersects holes with an existing scroll clip without changing the original video geometry", () => {
    const { element, viewport } = stage(); viewport.clip = { x: 50, y: 100, width: 500, height: 400 }; popup(100, 100, 200, 200);
    expect(nativeModalOcclusion(element, viewport)).toEqual({ occluded: true, preserveBackground: true, clip: viewport.clip,
      occlusions: [{ x: 50, y: 100, width: 158, height: 108 }] });
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
