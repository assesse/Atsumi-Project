/** Geometry only. This module owns no platform sessions, credentials or shortcuts. */
export type NativeRect = { x: number; y: number; width: number; height: number };
export type NativeHole = NativeRect & { radii?: [number, number, number, number] };
export type NativeViewport = {
  sequence: number; player: NativeRect | null; chat: NativeRect | null;
  visible: boolean; muted: boolean; occlusions: NativeHole[];
};
const overlays = '[data-platform-native-occlusion="true"], [data-native-overlay="true"], dialog[open], [role="dialog"][aria-modal="true"], [role="alertdialog"], [role="menu"]';
const finite = (r: NativeRect) => [r.x, r.y, r.width, r.height].every(Number.isFinite);
function shown(element: HTMLElement): boolean {
  if (!element.isConnected || element.closest('[hidden], [aria-hidden="true"], [inert]')) return false;
  for (let current: HTMLElement | null = element; current; current = current.parentElement) {
    const style = getComputedStyle(current);
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || style.opacity === '0') return false;
  }
  return true;
}
function fixedContainingBlock(element: HTMLElement): HTMLElement | null {
  for (let current = element.parentElement; current; current = current.parentElement) {
    const style = getComputedStyle(current);
    if (style.display === 'contents') continue;
    const has = (value: string | undefined) => !!value && value !== 'none' && value !== 'normal';
    if ([style.transform, style.translate, style.rotate, style.scale, style.perspective, style.filter, style.backdropFilter].some(has)
      || /\b(layout|paint|strict|content)\b/.test(style.contain)
      || style.contentVisibility === 'auto'
      || style.willChange.split(',').some(value => /^(transform|translate|rotate|scale|perspective|filter|backdrop-filter|contain)$/.test(value.trim()))) return current;
  }
  return null;
}
function measured(element: HTMLElement | null, fullscreenOwner: HTMLElement | null): { rect: NativeRect; clip: NativeRect } | null {
  if (!element || !shown(element)) return null;
  if (fullscreenOwner && !fullscreenOwner.contains(element)) return null;
  const box = element.getBoundingClientRect();
  if (!finite(box) || box.width <= 0 || box.height <= 0) return null;
  let left = Math.max(0, box.left), top = Math.max(0, box.top);
  let right = Math.min(window.innerWidth, box.right), bottom = Math.min(window.innerHeight, box.bottom);
  // Overflow clips follow containing blocks, not every DOM ancestor. A fixed
  // floating detail escapes the workspace scroller, but not a transformed or
  // contained fixed containing block. Keep internal modal scrolling intact.
  let fixedBoundary = getComputedStyle(element).position === 'fixed' ? fixedContainingBlock(element) : undefined;
  for (let current = element.parentElement; current; current = current.parentElement) {
    if (fixedBoundary !== undefined) {
      if (current !== fixedBoundary) continue;
      fixedBoundary = undefined;
    }
    const style = getComputedStyle(current), clip = current.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(style.overflowX || style.overflow)) {
      left = Math.max(left, clip.left + current.clientLeft);
      right = Math.min(right, clip.left + current.clientLeft + current.clientWidth);
    }
    if (/(auto|scroll|hidden|clip)/.test(style.overflowY || style.overflow)) {
      top = Math.max(top, clip.top + current.clientTop);
      bottom = Math.min(bottom, clip.top + current.clientTop + current.clientHeight);
    }
    // A fixed full-window owner is no longer laid out inside the grid cell.
    if (current === fullscreenOwner) break;
    if (style.position === 'fixed') fixedBoundary = fixedContainingBlock(current);
  }
  if (right <= left || bottom <= top) return null;
  return { rect: { x: box.x, y: box.y, width: box.width, height: box.height }, clip: { x: left, y: top, width: right - left, height: bottom - top } };
}
function radius(value: string, rect: NativeRect): number {
  const n = parseFloat(value);
  return Number.isFinite(n) ? Math.max(0, Math.min(value.includes('%') ? Math.min(rect.width, rect.height) * n / 100 : n, rect.width / 2, rect.height / 2)) : 0;
}
export function measureNativePlayerGeometry(
  player: HTMLElement | null, chat: HTMLElement | null,
  options: { sequence: number; visible: boolean; muted: boolean },
): NativeViewport {
  const result: NativeViewport = { ...options, player: null, chat: null, occlusions: [] };
  if (!options.visible) return result;
  const owners = [...document.querySelectorAll<HTMLElement>('[data-platform-native-fullscreen="true"]')].filter(shown);
  if (owners.length > 1) { result.visible = false; return result; }
  const owner = owners[0] ?? null;
  const surfaces = [measured(player, owner), measured(chat, owner)];
  result.player = surfaces[0]?.rect ?? null; result.chat = surfaces[1]?.rect ?? null;
  result.visible = !!(result.player || result.chat);
  if (!result.visible) return result;
  // Clipping must not shrink the document's player box: doing so changes its
  // aspect ratio and reflows controls during a scroll/overlay transition.
  for (const surface of surfaces) {
    if (!surface) continue;
    const { rect: r, clip: c } = surface;
    const holes = [
      { x:r.x, y:r.y, width:r.width, height:c.y-r.y },
      { x:r.x, y:c.y+c.height, width:r.width, height:r.y+r.height-c.y-c.height },
      { x:r.x, y:c.y, width:c.x-r.x, height:c.height },
      { x:c.x+c.width, y:c.y, width:r.x+r.width-c.x-c.width, height:c.height },
    ];
    result.occlusions.push(...holes.filter(h => h.width > 0 && h.height > 0));
  }
  for (const overlay of document.querySelectorAll<HTMLElement>(overlays)) {
    if (!shown(overlay)) continue;
    const isModal = overlay.tagName === 'DIALOG' || overlay.getAttribute('aria-modal') === 'true' || overlay.getAttribute('role') === 'alertdialog';
    // A floating detail is allowed to host its own official player/chat. A
    // different dialog must still hide every native pane (including input).
    if (isModal) {
      const slots = [player, chat].filter((element, index): element is HTMLElement => !!element && !!surfaces[index]);
      if (slots.length && slots.every(element => overlay.contains(element))) continue;
      result.visible = false; result.occlusions = []; return result;
    }
    const surface = overlay.querySelector<HTMLElement>('[data-native-dialog-surface="true"]') ?? overlay;
    const box = surface.getBoundingClientRect();
    if (!finite(box) || box.width <= 0 || box.height <= 0) { result.visible = false; return result; }
    const rect = { x: box.x, y: box.y, width: box.width, height: box.height };
    if (![result.player, result.chat].some(p => p && rect.x < p.x + p.width && rect.x + rect.width > p.x && rect.y < p.y + p.height && rect.y + rect.height > p.y)) continue;
    const style = getComputedStyle(surface);
    const radii = [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius].map(r => radius(r, rect)) as [number, number, number, number];
    result.occlusions.push({ ...rect, radii });
    if (result.occlusions.length > 16) { result.visible = false; result.occlusions = []; return result; }
  }
  return result;
}

/** Event-driven measurement; infinite spinner animations never reclip video. */
export function observeNativePlayerGeometry(elements: (HTMLElement | null)[], onChange: () => void): () => void {
  let disposed = false, frame = 0, motionEnd = 0;
  const observed = new Set<Element>();
  const queue = () => { if (!disposed && !frame) frame = requestAnimationFrame(tick); };
  const tick = () => { frame = 0; if (disposed) return; onChange(); if (performance.now() < motionEnd) queue(); };
  const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(queue);
  const sync = () => {
    const current = new Set<Element>([...elements.filter((el): el is HTMLElement => !!el), ...document.querySelectorAll(overlays)]);
    for (const old of observed) if (!current.has(old)) { resize?.unobserve(old); observed.delete(old); }
    for (const el of current) if (!observed.has(el)) { resize?.observe(el); observed.add(el); }
    queue();
  };
  const mutation = new MutationObserver(sync);
  mutation.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['hidden', 'inert', 'aria-hidden', 'aria-modal', 'class', 'style', 'open', 'data-native-overlay', 'data-platform-native-occlusion', 'data-platform-native-fullscreen'] });
  const motion = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || !target.matches(overlays) || getComputedStyle(target).animationIterationCount.split(',').some(v => v.trim() === 'infinite')) return;
    motionEnd = performance.now() + 400; queue();
  };
  window.addEventListener('resize', queue); document.addEventListener('scroll', queue, true);
  document.addEventListener('transitionrun', motion, true); document.addEventListener('animationstart', motion, true);
  sync();
  return () => { disposed = true; if (frame) cancelAnimationFrame(frame); mutation.disconnect(); resize?.disconnect(); observed.clear(); window.removeEventListener('resize', queue); document.removeEventListener('scroll', queue, true); document.removeEventListener('transitionrun', motion, true); document.removeEventListener('animationstart', motion, true); };
}
