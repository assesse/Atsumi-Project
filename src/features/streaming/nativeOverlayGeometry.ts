import type { OfficialBrowserViewport } from "../../api/officialBrowser";

const selectors = 'dialog[open], [role="dialog"][aria-modal="true"], [role="alertdialog"], [role="menu"], [data-native-overlay="true"]';
const MAX_OCCLUSIONS = 8;
const SHADOW_GUTTER = 8;
function visibleOverlays(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>(selectors)].filter(element => {
    const style = getComputedStyle(element);
    return !element.closest('[hidden], [aria-hidden="true"]') && style.display !== "none" && style.visibility !== "hidden";
  });
}
export function hasNativeOverlay(): boolean { return visibleOverlays().length > 0; }

/** Popup text, help expansion and responsive wrapping can resize the dialog
 * without resizing its native pane. Update the hole during that same layout. */
export function observeNativeOverlayGeometry(onResize: () => void): () => void {
  if (typeof ResizeObserver === "undefined") return () => {};
  let disposed = false;
  const surfaces = new Set<HTMLElement>();
  const resize = new ResizeObserver(() => { if (!disposed) onResize(); });
  const sync = () => {
    const current = new Set(visibleOverlays().flatMap(overlay => [...overlay.querySelectorAll<HTMLElement>('[data-native-dialog-surface="true"]')]));
    for (const old of surfaces) if (!current.has(old)) { resize.unobserve(old); surfaces.delete(old); }
    for (const surface of current) if (!surfaces.has(surface)) { surfaces.add(surface); resize.observe(surface); }
  };
  const mutation = new MutationObserver(sync);
  mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["hidden", "aria-hidden", "open", "class", "style", "data-native-overlay", "data-native-dialog-surface"] });
  sync();
  return () => { disposed = true; mutation.disconnect(); resize.disconnect(); surfaces.clear(); };
}

/** Keep the native viewport and scroll clip unchanged; subtract only the actual
 * trusted popup rectangles. Unknown/unmeasurable overlays still fail closed. */
export function nativeModalOcclusion(stage: HTMLElement, viewport: OfficialBrowserViewport): Partial<OfficialBrowserViewport> {
  const overlays = visibleOverlays();
  if (!overlays.length) return { occluded: false };
  const masked = { occluded: true };
  if (!viewport.visible || overlays.some(overlay => overlay.dataset.nativePreserveVideo !== "true")) return masked;
  const bounds = stage.getBoundingClientRect();
  if (![bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite) || bounds.width < 1 || bounds.height < 1) return masked;
  const clip = viewport.clip ?? { x: 0, y: 0, width: viewport.width, height: viewport.height };
  const occlusions: NonNullable<OfficialBrowserViewport["occlusions"]> = [];
  for (const overlay of overlays) {
    const surface = overlay.querySelector<HTMLElement>('[data-native-dialog-surface="true"]');
    const rect = surface?.getBoundingClientRect();
    if (!rect || ![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) || rect.width < 1 || rect.height < 1) return masked;
    const left = Math.max(clip.x, rect.left - bounds.left - SHADOW_GUTTER);
    const top = Math.max(clip.y, rect.top - bounds.top - SHADOW_GUTTER);
    const right = Math.min(clip.x + clip.width, rect.right - bounds.left + SHADOW_GUTTER);
    const bottom = Math.min(clip.y + clip.height, rect.bottom - bounds.top + SHADOW_GUTTER);
    if (right <= left || bottom <= top) continue;
    const hole = { x: left, y: top, width: right - left, height: bottom - top };
    if (!occlusions.some(old => old.x === hole.x && old.y === hole.y && old.width === hole.width && old.height === hole.height)) occlusions.push(hole);
    if (occlusions.length > MAX_OCCLUSIONS) return masked;
  }
  return { occluded: true, preserveBackground: true, clip, occlusions };
}
