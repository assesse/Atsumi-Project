import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { YouTubeViewingControls } from './YouTubeViewingControls';
import { targetKey, type YouTubeMetadata } from './model';

const items: YouTubeMetadata[] = ['abcdefghijk', 'bcdefghijkl', 'cdefghijklm', 'defghijklmn', 'efghijklmno'].map((id, index) => ({ kind: 'video', id, url: `https://www.youtube.com/watch?v=${id}`, title: `제목 ${index + 1}`, author: `채널 ${index + 1}`, isLive: true }));
let root: Root, container: HTMLDivElement;
const actions = { onApply: vi.fn(), onStop: vi.fn(), onExplore: vi.fn(), onCloseItem: vi.fn(), onFavorite: vi.fn(), onDownload: vi.fn() };
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); Object.values(actions).forEach(action => action.mockReset()); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function Fixture({ privacy = false, active = true, playing = [] as string[] } = {}) { const [open, setOpen] = useState(false); return <YouTubeViewingControls active={active} open={open} onOpenChange={setOpen} privacyMode={privacy} busy={false} items={items} playingIds={playing} favoriteIds={[]} mode="all" lead={playing[0] ?? ''} {...actions} />; }
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('button')].find(item => item.getAttribute('aria-label') === label || item.textContent === label)!;
const click = async (element: Element) => act(async () => element.dispatchEvent(new MouseEvent('click', { bubbles: true })));
const render = async (privacy = false, active = true, playing: string[] = []) => act(async () => root.render(<Fixture privacy={privacy} active={active} playing={playing} />));
const pointer = async (element: Element, type: string, y: number) => act(async () => { const event = new MouseEvent(type, { bubbles: true, button: 0, clientY: y }); Object.defineProperty(event, 'pointerId', { value: 1 }); element.dispatchEvent(event); });

it('opens/closes by drag without toggling again on the generated click', async () => {
  await render(); const handle = container.querySelector('.youtube-bottom-handle')!;
  await pointer(handle, 'pointerdown', 100); await pointer(handle, 'pointermove', 70); await pointer(handle, 'pointerup', 70); await click(handle);
  expect(handle).toHaveAttribute('aria-expanded', 'true'); expect(container.querySelector('[role=dialog]')).not.toBeNull();
  await pointer(handle, 'pointerdown', 70); await pointer(handle, 'pointermove', 100); await pointer(handle, 'pointerup', 100); await click(handle);
  expect(handle).toHaveAttribute('aria-expanded', 'false'); expect(container.querySelector('[role=dialog]')).toBeNull();
  expect(actions.onApply).not.toHaveBeenCalled(); expect(actions.onStop).not.toHaveBeenCalled();
});
it('keeps draft changes unapplied and returns focus on Escape without leaking it to shortcuts', async () => {
  await render(); const handle = button('방송 보기 열기'); await click(handle); await click(button('제목 1 시청 선택'));
  expect(actions.onApply).not.toHaveBeenCalled(); const outer = vi.fn(); window.addEventListener('keydown', outer);
  const target = button('선택 보기'); target.focus(); await act(async () => target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  window.removeEventListener('keydown', outer); expect(outer).not.toHaveBeenCalled(); expect(document.activeElement).toBe(handle); expect(container.querySelector('[role=dialog]')).toBeNull();
  await click(handle); expect(button('제목 1 시청 선택')).toHaveAttribute('aria-pressed', 'false');
});
it('does not swallow keyboard activation after a cancelled pointer gesture', async () => {
  await render(); const handle = button('방송 보기 열기');
  await pointer(handle, 'pointerdown', 100); await pointer(handle, 'pointercancel', 100);
  handle.focus(); await act(async () => handle.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  await click(handle); expect(handle).toHaveAttribute('aria-expanded', 'true'); expect(container.querySelector('[role=dialog]')).not.toBeNull();
});
it('applies at most four distinct opened videos and exposes an explicit fifth-selection error', async () => {
  await render(); await click(button('방송 보기 열기')); for (let index = 0; index < 5; index++) await click(button(`제목 ${index + 1} 시청 선택`));
  expect(container.querySelector('[role=alert]')?.textContent).toContain('네 개'); expect(button('제목 5 시청 선택')).toHaveAttribute('aria-pressed', 'false');
  await click(button('시청')); expect(actions.onApply).toHaveBeenCalledWith(items.slice(0, 4).map(targetKey), 'all', targetKey(items[0]!)); expect(actions.onStop).not.toHaveBeenCalled();
});
it('masks private labels and disables revealing/start actions without disabling explicit stop', async () => {
  await render(true, true, [targetKey(items[0]!)]); await click(button('방송 조작 열기'));
  expect(container.textContent).not.toContain('제목 1'); expect(container.textContent).not.toContain('채널 1');
  expect(button('변경')).toBeDisabled(); expect(button('탐색에서 영상 찾기')).toBeDisabled();
  await click(button('종료')); expect(actions.onStop).toHaveBeenCalledOnce(); expect(actions.onApply).not.toHaveBeenCalled();
});
it('has a precise four-line help overlay and nonmodal sheet rather than flow-expanding instructions', async () => {
  await render(); await click(button('방송 보기 열기')); const dialog = container.querySelector('[role=dialog]')!, help = container.querySelector('[role=tooltip]')!;
  expect(dialog).toHaveAttribute('aria-modal', 'false'); expect(help.querySelectorAll('br')).toHaveLength(3); expect(button('시청 도움말')).toHaveAttribute('aria-describedby', help.id);
  expect(container.querySelector('[data-gallery-shortcuts-suspended]')).not.toBeNull();
});
it('provides an in-app exploration action and does not trigger fullscreen or media actions', async () => {
  await render(); await click(button('방송 보기 열기')); await click(button('탐색에서 영상 찾기'));
  expect(actions.onExplore).toHaveBeenCalledOnce(); expect(actions.onApply).not.toHaveBeenCalled(); expect(actions.onStop).not.toHaveBeenCalled(); expect(container.querySelector('[role=dialog]')).toBeNull();
});
