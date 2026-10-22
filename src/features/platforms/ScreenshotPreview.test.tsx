import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ScreenshotPreview } from './ScreenshotPreview';

let container: HTMLDivElement, root: Root;
const image = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] });
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div'); document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove();
  vi.useRealTimers(); vi.unstubAllGlobals();
});
const preview = (id = 'capture') => ({ id, url: image, undoing: false, error: '', undoDeadline: performance.now() + 45000 });

it('mounts the saved preview and dismisses presentation after three seconds without undo', async () => {
  const dismiss = vi.fn(), undo = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview()} onDismiss={dismiss} onUndo={undo} />));
  expect(container.querySelector('img')?.getAttribute('src')).toBe(image);
  expect(container.querySelector('[aria-label="스크린샷 저장 취소"]')).not.toBeNull();
  await act(async () => vi.advanceTimersByTime(2999)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => vi.advanceTimersByTime(1)); expect(dismiss).toHaveBeenCalledExactlyOnceWith('capture');
  expect(undo).not.toHaveBeenCalled();
});

it('holds the mounted preview on hover and gives one and a half seconds after leaving', async () => {
  const dismiss = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview()} onDismiss={dismiss} onUndo={vi.fn()} />));
  const surface = container.querySelector('[role="group"]')!;
  await act(async () => surface.dispatchEvent(new MouseEvent('pointerover', { bubbles: true })));
  await act(async () => vi.advanceTimersByTime(8000)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => surface.dispatchEvent(new MouseEvent('pointerout', { bubbles: true, relatedTarget: document.body })));
  await act(async () => vi.advanceTimersByTime(1499)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => vi.advanceTimersByTime(1)); expect(dismiss).toHaveBeenCalledExactlyOnceWith('capture');
});

it('disposes the old capture clock when replaced or unmounted', async () => {
  const dismiss = vi.fn(), undo = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview('old')} onDismiss={dismiss} onUndo={undo} />));
  await act(async () => vi.advanceTimersByTime(2000));
  await act(async () => root.render(<ScreenshotPreview preview={preview('new')} onDismiss={dismiss} onUndo={undo} />));
  await act(async () => vi.advanceTimersByTime(1000)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => root.render(null));
  await act(async () => vi.advanceTimersByTime(60000)); expect(dismiss).not.toHaveBeenCalled();
  expect(undo).not.toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
});

it('separates explicit preview dismissal from exact saved-file undo', async () => {
  const dismiss = vi.fn(), undo = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview('owned')} onDismiss={dismiss} onUndo={undo} />));
  const cancel = container.querySelector<HTMLButtonElement>('.platform-live-capture-cancel')!;
  expect(cancel.textContent?.trim()).toBe('취소');
  await act(async () => cancel.click());
  expect(undo).toHaveBeenCalledOnce(); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="스크린샷 미리보기 닫기"]')!.click());
  expect(dismiss).toHaveBeenCalledExactlyOnceWith('owned'); expect(undo).toHaveBeenCalledOnce();
});

it('keeps one lifetime while focus moves between dismiss and cancel', async () => {
  const dismiss = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview()} onDismiss={dismiss} onUndo={vi.fn()} />));
  const close = container.querySelector<HTMLButtonElement>('.platform-live-capture-dismiss')!;
  const cancel = container.querySelector<HTMLButtonElement>('.platform-live-capture-cancel')!;
  await act(async () => close.focus());
  await act(async () => vi.advanceTimersByTime(8000));
  await act(async () => cancel.focus());
  await act(async () => vi.advanceTimersByTime(8000)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => cancel.blur());
  await act(async () => vi.advanceTimersByTime(1499)); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => vi.advanceTimersByTime(1)); expect(dismiss).toHaveBeenCalledExactlyOnceWith('capture');
});

it('blocks both actions while undo is pending and allows explicit dismissal of failure', async () => {
  const dismiss = vi.fn(), undo = vi.fn(), shown = preview();
  await act(async () => root.render(<ScreenshotPreview preview={{ ...shown, undoing: true }} onDismiss={dismiss} onUndo={undo} />));
  await act(async () => { for (const button of container.querySelectorAll('button')) button.click(); });
  await act(async () => vi.advanceTimersByTime(46000));
  expect(undo).not.toHaveBeenCalled(); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => root.render(<ScreenshotPreview preview={{ ...shown, error: '취소 실패' }} onDismiss={dismiss} onUndo={undo} />));
  expect(container.querySelector('[role="alert"]')?.textContent).toBe('취소 실패');
  await act(async () => container.querySelector<HTMLButtonElement>('.platform-live-capture-cancel')!.click());
  expect(undo).not.toHaveBeenCalled();
  await act(async () => container.querySelector<HTMLButtonElement>('.platform-live-capture-dismiss')!.click());
  expect(dismiss).toHaveBeenCalledExactlyOnceWith('capture');
});

it('does not undo an expired focused preview and dismisses only the current replacement ID', async () => {
  const dismiss = vi.fn(), undo = vi.fn();
  await act(async () => root.render(<ScreenshotPreview preview={preview('old')} onDismiss={dismiss} onUndo={undo} />));
  await act(async () => container.querySelector<HTMLButtonElement>('.platform-live-capture-cancel')!.focus());
  await act(async () => vi.advanceTimersByTime(45000));
  await act(async () => container.querySelector<HTMLButtonElement>('.platform-live-capture-cancel')!.click());
  expect(undo).not.toHaveBeenCalled(); expect(dismiss).not.toHaveBeenCalled();
  await act(async () => root.render(<ScreenshotPreview preview={preview('new')} onDismiss={dismiss} onUndo={undo} />));
  await act(async () => container.querySelector<HTMLButtonElement>('.platform-live-capture-dismiss')!.click());
  expect(dismiss).toHaveBeenCalledExactlyOnceWith('new');
});
