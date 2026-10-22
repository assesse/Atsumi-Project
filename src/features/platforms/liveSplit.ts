export const LIVE_SPLITTER_SIZE = 7;
const safeSize = (value: number) => Number.isFinite(value) ? Math.max(0, value) : 0;
export const clampLiveChat = (value: number, min: number, max: number) => Math.max(min, Math.min(max, Number.isFinite(value) ? value : min));

export function liveSplit(width: number, height: number, preferredWidth: number, compactFraction: number, sideBySide = false) {
  const compact = !sideBySide && safeSize(width) > 0 && width < 580;
  const available = Math.max(0, safeSize(compact ? height : width) - LIVE_SPLITTER_SIZE);
  // Very short multi-view cells must still fit both panes, rather than overflowing
  // their parent to satisfy the usual readable minimum.
  const min = Math.min(96, available * .35);
  const max = Math.max(min, Math.min(available - min, compact ? available : available * .6));
  const desired = compact ? available * compactFraction : preferredWidth;
  return { compact, available, min, max, pixels: clampLiveChat(desired, min, max) };
}

export function liveSplitKey(key: string, current: number, min: number, max: number, compact: boolean) {
  if (key === 'Home') return min;
  if (key === 'End') return max;
  if (key === (compact ? 'ArrowUp' : 'ArrowLeft')) return clampLiveChat(current + 20, min, max);
  if (key === (compact ? 'ArrowDown' : 'ArrowRight')) return clampLiveChat(current - 20, min, max);
  return null;
}
