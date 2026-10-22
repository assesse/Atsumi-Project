import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ScreenshotLibrary } from "./ScreenshotLibrary";
import type { ScreenshotsApi, ScreenshotItem } from "../../api/screenshots";

const item = (index: number): ScreenshotItem => ({ id: index.toString().padStart(32, '0'), channelId: '', fileName: `${index}.png`, createdAt: 1800000000000 + index * 1000, bytes: 250000, width: 1920, height: 1080 });
let root: Root, container: HTMLDivElement;
const api = { list: vi.fn<ScreenshotsApi['list']>(), image: vi.fn<ScreenshotsApi['image']>(), delete: vi.fn<ScreenshotsApi['delete']>(), folder: vi.fn<ScreenshotsApi['folder']>() };
const cards = () => [...container.querySelectorAll<HTMLButtonElement>('.screenshot-card')];
const click = (index: number, options: MouseEventInit = {}) => act(async () => { cards()[index]!.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1, ...options })); });
const render = (privacy = false) => act(async () => root.render(<ScreenshotLibrary runtime="browser-mock" privacy={privacy} api={api} />));
beforeEach(() => {
  vi.resetAllMocks(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  HTMLDialogElement.prototype.showModal = function () { this.open = true; }; HTMLDialogElement.prototype.close = function () { this.open = false; };
  api.list.mockResolvedValue([item(1), item(2), item(3)]); api.image.mockResolvedValue('data:image/png;base64,c3ludGhldGlj');
  api.delete.mockResolvedValue({ deleted: [], failures: [] }); api.folder.mockResolvedValue();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
describe('screenshot library', () => {
  it('uses recording-card selection gestures and requires confirmation to delete', async () => {
    await render(); expect(cards()).toHaveLength(3); expect(api.delete).not.toHaveBeenCalled();
    await click(0); expect(cards()[0]).toHaveAttribute('aria-pressed', 'true'); expect(container.querySelector('input[type=checkbox]')).toBeNull();
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    await click(2, { shiftKey: true }); expect(container.querySelectorAll('input:checked')).toHaveLength(3);
    await click(1, { ctrlKey: true }); expect(container.querySelectorAll('input:checked')).toHaveLength(2);
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'Delete' })));
    expect(container.querySelector('dialog')).toHaveAttribute('open'); expect(api.delete).not.toHaveBeenCalled();
    api.delete.mockResolvedValue({ deleted: [item(1).id, item(3).id], failures: [] });
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>('dialog button')].find(button => button.textContent === '삭제')!.click());
    expect(api.delete).toHaveBeenCalledExactlyOnceWith([item(1).id, item(3).id]); expect(cards()).toHaveLength(1); expect(container.querySelector('dialog')).not.toHaveAttribute('open');
  });
  it('opens full resolution on double click without toggling selection on the second click', async () => {
    await render(); await click(0); await click(0, { detail: 2 });
    await act(async () => cards()[0]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, detail: 2 })));
    expect(cards()[0]).toHaveAttribute('aria-pressed', 'true'); expect(api.image).toHaveBeenCalledWith(item(1).id, true);
    expect(container.querySelector('.screenshot-viewer')).toHaveAttribute('open'); expect(api.delete).not.toHaveBeenCalled();
    await act(async () => container.querySelector('.screenshot-viewer')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })));
    expect(api.image).toHaveBeenCalledWith(item(2).id, true);
  });
  it('keeps image data unloaded in privacy mode and closes an already open viewer', async () => {
    await render(true); expect(api.image).not.toHaveBeenCalled(); expect(container.querySelector('img')).toBeNull();
    await render(false); await act(async () => cards()[0]!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    expect(container.querySelector('.screenshot-viewer')).not.toBeNull();
    await render(true); expect(container.querySelector('.screenshot-viewer')).toBeNull(); expect(container.querySelector('img')).toBeNull();
  });
  it('supports Space, Ctrl+A, Ctrl+Shift+A and Escape without a selection-mode button', async () => {
    await render(); await click(0); await click(1, { ctrlKey: true });
    await act(async () => cards()[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true })));
    expect(cards()[0]).toHaveAttribute('aria-pressed', 'true'); expect(cards()[1]).toHaveAttribute('aria-pressed', 'false');
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true })));
    expect(container.querySelectorAll('input:checked')).toHaveLength(3);
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, shiftKey: true, bubbles: true })));
    expect(container.querySelector('input[type=checkbox]')).toBeNull(); expect(cards().every(card => card.getAttribute('aria-pressed') === 'false')).toBe(true);
  });
  it('keeps failed deletions selected and reports the failure', async () => {
    await render(); await click(0); await click(1, { ctrlKey: true });
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'Delete' })));
    api.delete.mockResolvedValue({ deleted: [item(1).id], failures: [{ id: item(2).id, message: '파일 사용 중' }] });
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>('dialog button')].find(button => button.textContent === '삭제')!.click());
    expect(cards()).toHaveLength(2); expect(cards()[0]).toHaveAttribute('aria-pressed', 'true');
    expect(container.querySelector('.recording-library-delete-errors')).toHaveTextContent('1개 삭제 실패');
  });
});
