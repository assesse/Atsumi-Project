import { invoke } from "@tauri-apps/api/core";

export type PinterestBrowserSnapshot = { epoch: number; open: boolean; loading: boolean; currentPinId: string | null; error: string | null };
export type PinterestBrowserViewport = { epoch: number; sequence: number; x: number; y: number; width: number; height: number; visible: boolean };
export type PinterestBrowserAction = "back" | "forward" | "reload" | "stop" | "close";
export type PinterestBrowserApi = {
  open(query: string): Promise<PinterestBrowserSnapshot>;
  snapshot(epoch: number): Promise<PinterestBrowserSnapshot>;
  viewport(value: PinterestBrowserViewport): Promise<void>;
  select(epoch: number, expectedPinId: string): Promise<string>;
  action(epoch: number, action: PinterestBrowserAction): Promise<void>;
};
export function createPinterestBrowserApi(runtime: "tauri" | "browser-mock"): PinterestBrowserApi {
  const unavailable = async (): Promise<never> => { throw new Error("공식 Pinterest 탐색은 Windows 데스크톱 앱에서 사용할 수 있습니다."); };
  if (runtime !== "tauri") return { open: unavailable, snapshot: unavailable, viewport: unavailable, select: unavailable, action: unavailable };
  return {
    open: query => invoke("pinterest_browser_open", { query }),
    snapshot: epoch => invoke("pinterest_browser_snapshot", { epoch }),
    viewport: viewport => invoke("pinterest_browser_set_viewport", { viewport }),
    select: (epoch, expectedPinId) => invoke("pinterest_browser_select_current", { epoch, expectedPinId }),
    action: (epoch, action) => invoke("pinterest_browser_action", { epoch, action }),
  };
}

/** Clip to the actual app viewport. Never put a native surface over app chrome. */
export function pinterestBrowserRect(rect: Pick<DOMRect, "left" | "top" | "right" | "bottom">, width: number, height: number) {
  const x = Math.max(0, rect.left), y = Math.max(0, rect.top);
  const w = Math.max(0, Math.min(rect.right, width) - x), h = Math.max(0, Math.min(rect.bottom, height) - y);
  return { x, y, width: w, height: h, visible: [x, y, w, h].every(Number.isFinite) && w >= 1 && h >= 1 };
}

/** A native child does not inherit CSS overflow clipping from its placeholder. */
export function pinterestBrowserSurfaceRect(surface: HTMLElement, width: number, height: number) {
  const bounds = surface.getBoundingClientRect();
  let left = bounds.left, top = bounds.top, right = bounds.right, bottom = bounds.bottom;
  for (let ancestor = surface.parentElement; ancestor; ancestor = ancestor.parentElement) {
    const style = getComputedStyle(ancestor), rect = ancestor.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(style.overflowX || style.overflow)) { left = Math.max(left, rect.left); right = Math.min(right, rect.right); }
    if (/(auto|scroll|hidden|clip)/.test(style.overflowY || style.overflow)) { top = Math.max(top, rect.top); bottom = Math.min(bottom, rect.bottom); }
  }
  return pinterestBrowserRect({ left, top, right, bottom }, width, height);
}
