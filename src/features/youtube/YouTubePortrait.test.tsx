import { act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { YouTubePortrait } from './YouTubePortrait';
import type { YouTubeApi } from './api';
import { readYouTubeFavorites, saveYouTubeFavorites, type YouTubeMetadata } from './model';

const image = 'data:image/png;base64,AQ==';
const cover = 'data:image/jpeg;base64,Ag==';
const channelId = 'UC' + 'a'.repeat(22);
const item: YouTubeMetadata = { kind: 'channel', id: 'channel/' + channelId, url: 'https://www.youtube.com/channel/' + channelId, title: 'Channel', author: '' };
const makeApi = (thumbnail = vi.fn(async (_id: string): Promise<string | null> => image)): YouTubeApi => ({ thumbnail, metadata: vi.fn(async target => ({ ...target, title: '', author: '' })), openExternal: vi.fn(async () => {}) });
const roots = new Map<Root, HTMLDivElement>();
async function render(element: ReactNode) {
  const container=document.createElement('div');document.body.append(container);const root=createRoot(container);roots.set(root,container);
  await act(async()=>root.render(element));
  return {container,rerender:async(next:ReactNode)=>{await act(async()=>root.render(next));},unmount:async()=>{await act(async()=>root.unmount());container.remove();roots.delete(root);}};
}
async function waitFor(check:()=>void) { await act(async()=>{}); check(); }
beforeEach(() => { vi.stubGlobal('IntersectionObserver', undefined); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true); });
afterEach(async () => { for(const [root,container] of roots) {await act(async()=>root.unmount());container.remove();}roots.clear();vi.unstubAllGlobals(); });

describe('YouTube channel portrait', () => {
  it('reloads a stored channel favorite without persisting image bytes', async () => {
    let saved = '';
    saveYouTubeFavorites({ setItem: (_key, value) => { saved = value; } }, [{ ...item, thumbnail: image, savedAt: 1 }]);
    expect(saved).not.toContain('data:image');
    const [restored] = readYouTubeFavorites({ getItem: () => saved });
    expect(restored!.thumbnail).toBeUndefined();
    const api = makeApi(), view = await render(<YouTubePortrait item={restored} api={api} active privacy={false} />);
    await waitFor(() => expect(view.container.querySelector('img')?.getAttribute('src')).toBe(image));
    expect(api.thumbnail).toHaveBeenCalledWith(item.id);
  });
  it('uses a video author identity and never its video cover', async () => {
    const api = makeApi(), video: YouTubeMetadata = { kind: 'video', id: 'abcdefghijk', url: 'https://www.youtube.com/watch?v=abcdefghijk', title: 'Video', author: 'Channel', authorUrl: item.url, thumbnail: cover };
    const view = await render(<YouTubePortrait item={video} api={api} active privacy={false} />);
    await waitFor(() => expect(view.container.querySelector('img')?.getAttribute('src')).toBe(image));
    expect(api.thumbnail).toHaveBeenCalledWith(item.id);
    expect(api.thumbnail).not.toHaveBeenCalledWith(video.id);
  });
  it('does not fetch while inactive/private and hides a previous portrait', async () => {
    const api = makeApi(), view = await render(<YouTubePortrait item={item} api={api} active={false} privacy={false} />);
    await act(async () => {}); expect(api.thumbnail).not.toHaveBeenCalled();
    await view.rerender(<YouTubePortrait item={item} api={api} active privacy />);
    await act(async () => {}); expect(api.thumbnail).not.toHaveBeenCalled();
    await view.rerender(<YouTubePortrait item={item} api={api} active privacy={false} />);
    await waitFor(() => expect(view.container.querySelector('img')).not.toBeNull());
    await view.rerender(<YouTubePortrait item={item} api={api} active privacy />);
    expect(view.container.querySelector('img')).toBeNull();
  });
  it('rejects an obsolete identity response', async () => {
    let resolve!: (value: string | null) => void;
    const api = makeApi(vi.fn(() => new Promise<string | null>(done => { resolve = done; })));
    const view = await render(<YouTubePortrait item={item} api={api} active privacy={false} />);
    await waitFor(() => expect(api.thumbnail).toHaveBeenCalledTimes(1));
    await view.rerender(<YouTubePortrait item={undefined} api={api} active privacy={false} />);
    await act(async () => { resolve(image); });
    expect(view.container.querySelector('img')).toBeNull();
  });
  it('does not accept remote URLs or poison the next mount after a failure', async () => {
    const thumbnail = vi.fn().mockResolvedValueOnce('https://example.invalid/profile.png').mockResolvedValue(image), api = makeApi(thumbnail);
    const view = await render(<YouTubePortrait item={item} api={api} active privacy={false} />);
    await act(async () => {}); expect(view.container.querySelector('img')).toBeNull();
    await view.unmount();
    const again = await render(<YouTubePortrait item={item} api={api} active privacy={false} />);
    await waitFor(() => expect(again.container.querySelector('img')?.getAttribute('src')).toBe(image));
    expect(thumbnail).toHaveBeenCalledTimes(2);
  });
  it('limits concurrent reads and cancels queued work on unmount', async () => {
    const pending: ((value: string | null) => void)[] = [];
    const api = makeApi(vi.fn(() => new Promise<string | null>(done => pending.push(done))));
    const view = await render(<>{Array.from({ length: 8 }, (_, n) => { const id = 'channel/UC' + String(n).padStart(22, '0'); return <YouTubePortrait key={id} item={{ ...item, id, url: 'https://www.youtube.com/' + id }} api={api} active privacy={false} />; })}</>);
    await waitFor(() => expect(api.thumbnail).toHaveBeenCalledTimes(4));
    await view.unmount();
    await act(async () => { pending.forEach(done => done(image)); });
    expect(api.thumbnail).toHaveBeenCalledTimes(4);
  });
});
