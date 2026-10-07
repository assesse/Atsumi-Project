import type { WindowPlacement } from "../api/contracts";

type Size = { width: number; height: number };
type Position = { x: number; y: number };
export type PlacementMonitor = {
  workArea: { position: Position; size: Size };
  scaleFactor: number;
};
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** Saved placements are physical OUTER bounds; Tauri setSize takes INNER bounds. */
export function fitWindowToWorkArea(saved: WindowPlacement, monitors: PlacementMonitor[], fallback: PlacementMonitor, frame: Size) {
  const area = (monitor: PlacementMonitor) => {
    if (saved.x === null || saved.y === null) return 0;
    const { position: p, size: s } = monitor.workArea;
    return Math.max(0, Math.min(saved.x + saved.width, p.x + s.width) - Math.max(saved.x, p.x))
      * Math.max(0, Math.min(saved.y + saved.height, p.y + s.height) - Math.max(saved.y, p.y));
  };
  const monitor = monitors.reduce((best, item) => area(item) > area(best) ? item : best, fallback);
  const { position: origin, size: available } = monitor.workArea;
  const decoration = { width: Math.max(0, frame.width), height: Math.max(0, frame.height) };
  const maxInner = { width: Math.max(1, available.width - decoration.width), height: Math.max(1, available.height - decoration.height) };
  const minimum = { width: Math.min(maxInner.width, Math.round(960 * monitor.scaleFactor)), height: Math.min(maxInner.height, Math.round(640 * monitor.scaleFactor)) };
  const inner = { width: clamp(saved.width - decoration.width, minimum.width, maxInner.width), height: clamp(saved.height - decoration.height, minimum.height, maxInner.height) };
  const outer = { width: inner.width + decoration.width, height: inner.height + decoration.height };
  const position = {
    x: Math.round(clamp(saved.x ?? origin.x + (available.width - outer.width) / 2, origin.x, origin.x + available.width - outer.width)),
    y: Math.round(clamp(saved.y ?? origin.y + (available.height - outer.height) / 2, origin.y, origin.y + available.height - outer.height)),
  };
  return { position, inner, outer, minimum };
}
