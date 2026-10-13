import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LiveEdgePanel } from "./LiveEdgePanel";
import { hasBlockingNativeOverlay, hasNativeOverlay } from "./nativeOverlayGeometry";

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { vi.useFakeTimers(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
it('keeps the compact toggle exposed and changes the panel only on explicit clicks', async () => {
  await act(async () => root.render(<LiveEdgePanel enabled edge="top" label="도구"><button>방송 보기</button></LiveEdgePanel>));
  const trigger = host.querySelector<HTMLButtonElement>('.live-edge-trigger')!, overlay = host.querySelector<HTMLElement>('.live-edge-overlay')!;
  expect(overlay.hidden).toBe(true); expect(hasNativeOverlay()).toBe(true); expect(hasBlockingNativeOverlay()).toBe(false);
  await act(async () => trigger.dispatchEvent(new MouseEvent('pointerover', { bubbles: true })));
  expect(overlay.hidden).toBe(true);
  await act(async () => trigger.click());
  expect(overlay.hidden).toBe(false); expect(trigger).toHaveAttribute('aria-expanded', 'true');
  expect(hasNativeOverlay()).toBe(true); expect(hasBlockingNativeOverlay()).toBe(false);
  await act(async () => overlay.dispatchEvent(new MouseEvent('pointerout', { bubbles: true, relatedTarget: document.body })));
  await act(async () => vi.advanceTimersByTimeAsync(350));
  expect(overlay.hidden).toBe(false); expect(trigger).toHaveAttribute('aria-expanded', 'true');
  expect(trigger.textContent).toBe(''); expect(trigger).toHaveAccessibleName('도구 닫기');
  await act(async () => trigger.click());
  expect(overlay.hidden).toBe(true); expect(hasNativeOverlay()).toBe(true); expect(hasBlockingNativeOverlay()).toBe(false);
});
it('supports keyboard opening and Escape and does not remount its children', async () => {
  const render = (enabled: boolean) => root.render(<LiveEdgePanel enabled={enabled} edge="left" label="메뉴"><input aria-label="입력" defaultValue="보존" /></LiveEdgePanel>);
  await act(async () => render(true));
  const input = host.querySelector('input')!, trigger = host.querySelector<HTMLButtonElement>('.live-edge-trigger')!;
  await act(async () => { trigger.focus(); trigger.click(); }); expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await act(async () => { input.focus(); input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
  expect(document.activeElement).toBe(trigger); expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await act(async () => render(false));
  expect(host.querySelector('input')).toBe(input); expect(input.value).toBe('보존');
  expect(host.querySelector('.live-edge-trigger')).toBeNull(); expect(hasNativeOverlay()).toBe(false);
});
it('pulls the left toggle in both directions without the subsequent click undoing it', async () => {
  await act(async () => root.render(<LiveEdgePanel enabled edge="left" label="메뉴"><input /></LiveEdgePanel>));
  const trigger = host.querySelector<HTMLButtonElement>('.live-edge-trigger')!;
  const pointer = (type: string, x: number) => {
    const event = new MouseEvent(type, { clientX: x, bubbles: true, button: 0 });
    Object.defineProperty(event, 'pointerId', { value: 1 }); trigger.dispatchEvent(event);
  };
  await act(async () => { pointer('pointermove', 100); pointer('pointerdown', 5); pointer('pointermove', 18); });
  expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await act(async () => { pointer('pointermove', 40); pointer('pointerup', 40); trigger.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })); });
  expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await act(async () => { pointer('pointerdown', 230); pointer('pointermove', 200); pointer('pointerup', 200); trigger.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })); });
  expect(trigger).toHaveAttribute('aria-expanded', 'false');
});
