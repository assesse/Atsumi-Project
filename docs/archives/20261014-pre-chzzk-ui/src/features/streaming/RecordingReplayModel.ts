import type { ReplayMessage } from "../../api/replay";

export type ReplayTimeLabel = "recording" | "broadcast" | "hidden";
export const REPLAY_PAGE_LIMIT = 200;
export function visibleReplayWarnings(warnings: string[] = []): string[] {
  return warnings.filter(warning => warning !== "수신 시각 기준의 근사 동기화가 포함됩니다. 채팅 보정값으로 조정할 수 있습니다.");
}
export const formatReplayTime = (value: number) => {
  const seconds = Math.max(0, Math.floor(Number.isFinite(value) ? value : 0));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};
export function replayMessageTime(message: ReplayMessage, mode: ReplayTimeLabel): string | null {
  if (mode === "hidden") return null;
  if (mode === "broadcast" && message.broadcastOffsetSeconds == null) return "—";
  return formatReplayTime(mode === "broadcast" ? message.broadcastOffsetSeconds! : message.offsetSeconds);
}
export function safeNicknameColor(color: string | null | undefined): string | undefined {
  return color && /^#[a-f0-9]{6}$/i.test(color) ? color : undefined;
}
export function safeReplayProfile(url: string | null | undefined): boolean {
  return typeof url === "string" && /^https:\/\/chzzk\.naver\.com\/[a-f0-9]{32}$/.test(url);
}
export function replayShortcutAllowed(target: EventTarget | null): boolean {
  return !(target instanceof Element && target.closest('input,textarea,select,button,a,[contenteditable]:not([contenteditable="false"]),[role="textbox"]'));
}
/** The original chat lives in a style-isolated shadow tree, but shares the dialog's focus order. */
export function replayFocusableControls(root: Element | ShadowRoot): HTMLElement[] {
  const controls: HTMLElement[] = [];
  for (const child of root.children) {
    if (child.hasAttribute("hidden") || child.getAttribute("aria-hidden") === "true") continue;
    if (child instanceof HTMLElement && child.matches('button:not(:disabled),input:not(:disabled),select:not(:disabled),iframe,[tabindex="0"]')) controls.push(child);
    if (child.shadowRoot) controls.push(...replayFocusableControls(child.shadowRoot));
    controls.push(...replayFocusableControls(child));
  }
  return controls;
}
/** Every path must originate in a backend asset token, never archived image URLs. */
export function replayAssetToken(token: string | null | undefined): string | null {
  return typeof token === "string" && /^[a-f0-9]{32,128}$/.test(token) ? token : null;
}
export function boundedReplayMessages(items: ReplayMessage[]): ReplayMessage[] {
  const seen = new Set<number>();
  return items.slice(-REPLAY_PAGE_LIMIT).filter((item) => {
    if (!Number.isSafeInteger(item.sequence) || !Number.isFinite(item.mediaTimeSeconds) || seen.has(item.sequence)) return false;
    seen.add(item.sequence);
    return true;
  });
}

/** Variable-height page virtualization; height storage is bounded by one page. */
export function replayVirtualRange(heights: number[], scrollTop: number, viewportHeight: number, following: boolean) {
  const offsets = [0];
  for (const height of heights) offsets.push(offsets[offsets.length - 1]! + height);
  const total = offsets[offsets.length - 1]!;
  const top = following ? Math.max(0, total - viewportHeight) : Math.max(0, scrollTop);
  let first = 0;
  while (first < heights.length && offsets[first + 1]! < top) first++;
  let last = first;
  while (last < heights.length && offsets[last]! < top + viewportHeight) last++;
  const start = Math.max(0, first - 6), end = Math.min(heights.length, last + 6, start + 80);
  return { start, end, before: offsets[start]!, after: total - offsets[end]!, total };
}
