export type TourRect = { left: number; top: number; width: number; height: number };
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(value, Math.max(min, max)));
export function spotlightRect(rect: TourRect, width: number, height: number): TourRect | null {
  const left = clamp(rect.left - 6, 4, width - 4), top = clamp(rect.top - 6, 4, height - 4);
  const right = clamp(rect.left + rect.width + 6, 4, width - 4), bottom = clamp(rect.top + rect.height + 6, 4, height - 4);
  return right > left && bottom > top ? { left, top, width: right - left, height: bottom - top } : null;
}
export function tourPlacement(target: TourRect | null, viewport: { width: number; height: number }, size: { width: number; height: number }) {
  const margin = 16, gap = 30;
  const width = Math.min(size.width, Math.max(0, viewport.width - margin * 2)), height = Math.min(size.height, Math.max(0, viewport.height - margin * 2));
  let left = (viewport.width - width) / 2, top = (viewport.height - height) / 2;
  let side: "right" | "left" | "bottom" | "top" = "bottom";
  if (target) {
    const right = target.left + target.width, bottom = target.top + target.height;
    if (viewport.width - right >= width + gap + margin) { side = "right"; left = right + gap; top = target.top + target.height / 2 - height / 2; }
    else if (target.left >= width + gap + margin) { side = "left"; left = target.left - gap - width; top = target.top + target.height / 2 - height / 2; }
    else if (viewport.height - bottom >= height + gap + margin) { side = "bottom"; left = target.left + target.width / 2 - width / 2; top = bottom + gap; }
    else { side = "top"; left = target.left + target.width / 2 - width / 2; top = target.top - gap - height; }
  }
  left = clamp(left, margin, viewport.width - width - margin); top = clamp(top, margin, viewport.height - height - margin);
  let line: string | null = null;
  if (target) {
    const x = target.left + target.width / 2, y = target.top + target.height / 2;
    const points = side === "right" ? [target.left + target.width, y, left, clamp(y, top + 20, top + height - 20)]
      : side === "left" ? [target.left, y, left + width, clamp(y, top + 20, top + height - 20)]
      : side === "bottom" ? [x, target.top + target.height, clamp(x, left + 20, left + width - 20), top]
      : [x, target.top, clamp(x, left + 20, left + width - 20), top + height];
    line = `M ${points[0]} ${points[1]} L ${points[2]} ${points[3]}`;
  }
  return { bubble: { left, top, width, height }, line };
}
