import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { YouTubePaneGrid } from './YouTubePaneGrid';
let root: Root, container: HTMLDivElement;
beforeEach(() => { localStorage.clear(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const pointer = async (element: Element, type: string, x: number) => act(async () => { const event = new MouseEvent(type, { bubbles: true, button: 0, clientX: x }); Object.defineProperty(event, 'pointerId', { value: 1 }); element.dispatchEvent(event); });
it('does not jump grouped video/chat dividers on first pointer movement and only writes YouTube preferences', async () => {
  await act(async () => root.render(<YouTubePaneGrid kind="pair" className="youtube-playing-grid" live arrangement="grouped" onResizing={vi.fn()}>{[<div key="a">A</div>, <div key="b">B</div>]}</YouTubePaneGrid>));
  const grid = container.querySelector<HTMLElement>('.youtube-pane-grid')!;
  vi.spyOn(grid, 'getBoundingClientRect').mockReturnValue({ left: 10, top: 0, width: 900, height: 600, right: 910, bottom: 600, x: 10, y: 0, toJSON: () => ({}) });
  const positions = [['columns', 10 + 282.75 + 5, 50], ['group', 10 + 565.5 + 15, 65], ['chats', 10 + 565.5 + 20 + 152.25 + 5, 50]] as const;
  for (const [axis, x, expected] of positions) {
    const divider = grid.querySelector<HTMLElement>('.split-' + axis)!;
    await pointer(divider, 'pointerdown', x); await pointer(divider, 'pointermove', x); await pointer(divider, 'pointerup', x);
    expect(divider.getAttribute('aria-valuenow')).toBe(String(expected));
  }
  await act(async () => new Promise(resolve => setTimeout(resolve, 220)));
  expect(localStorage.getItem('atsumi.youtube.live.panes.pair.v1')).not.toBeNull();
  expect(localStorage.getItem('atsumi.live.panes.pair.v1')).toBeNull(); expect(localStorage.getItem('atsumi.soop.live.panes.pair.v1')).toBeNull();
});
it('keeps the same cells across layouts and hidden presentation and supports keyboard resizing', async () => {
  const children = [<div key="a">A</div>, <div key="b">B</div>];
  await act(async () => root.render(<YouTubePaneGrid kind="pair" className="youtube-playing-grid" live onResizing={vi.fn()}>{children}</YouTubePaneGrid>));
  const first = container.querySelector('.youtube-pane-grid>div')!;
  const divider = container.querySelector('.split-columns')!;
  await act(async () => divider.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true })));
  expect(divider.getAttribute('aria-valuenow')).toBe('52');
  await act(async () => root.render(<YouTubePaneGrid kind="pair" className="youtube-playing-grid" live arrangement="grouped" hidden onResizing={vi.fn()}>{children}</YouTubePaneGrid>));
  expect(container.querySelector('.youtube-pane-grid>div')).toBe(first); expect(container.querySelector('.youtube-pane-grid')).toHaveAttribute('hidden');
});
