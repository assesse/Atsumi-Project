import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SoopLivePaneGrid } from '../soop/SoopLivePaneGrid';
import { SoopConnectionSetup } from '../soop/SoopConnectionSetup';
import { YouTubePaneGrid } from '../youtube/YouTubePaneGrid';
import { YouTubeViewingControls } from '../youtube/YouTubeViewingControls';
import type { YouTubeMetadata } from '../youtube/model';
import soopShellCss from '../soop/SoopLiveShell.css?raw';
import youtubeShellCss from '../youtube/YouTubeConnection.css?raw';
import soopSource from '../soop/SoopWorkspace.tsx?raw';
import youtubeSource from '../youtube/YouTubeWorkspace.tsx?raw';
import soopGridSource from '../soop/SoopLivePaneGrid.tsx?raw';
import youtubeGridSource from '../youtube/YouTubePaneGrid.tsx?raw';
import soopSetupSource from '../soop/SoopConnectionSetup.tsx?raw';
import youtubeSetupSource from '../youtube/YouTubeViewingControls.tsx?raw';

let host: HTMLDivElement, root: Root;
beforeEach(() => { localStorage.clear(); vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const click = async (node: Element) => act(async () => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
const button = (scope: ParentNode, label: string) => [...scope.querySelectorAll<HTMLButtonElement>('button')].find(node => node.textContent === label || node.getAttribute('aria-label') === label)!;

it('keeps copied split preferences and keyboard resize effects separate while preserving receiver elements', async () => {
  vi.useFakeTimers();
  const soopKey = 'atsumi.soop.live.panes.same-layout.v1', youtubeKey = 'atsumi.youtube.live.panes.same-layout.v1';
  localStorage.setItem(soopKey, JSON.stringify({ columns: 32, rows: 50 }));
  localStorage.setItem(youtubeKey, JSON.stringify({ columns: 68, rows: 50 }));
  const soopResizing = vi.fn(), youtubeResizing = vi.fn();
  const render = async (grouped: boolean, hidden: boolean) => act(async () => root.render(<>
    <div hidden={hidden}><SoopLivePaneGrid kind="pair" className="soop-channel-pairs" storageId="same-layout" grouped={grouped} onResizing={soopResizing}>{[<video key="same-a" data-receiver="soop-a" />, <video key="same-b" data-receiver="soop-b" />]}</SoopLivePaneGrid></div>
    <YouTubePaneGrid kind="pair" className="youtube-playing-grid" storageId="same-layout" arrangement={grouped ? 'grouped' : 'channels'} live hidden={hidden} onResizing={youtubeResizing}>{[<video key="same-a" data-receiver="youtube-a" />, <video key="same-b" data-receiver="youtube-b" />]}</YouTubePaneGrid>
  </>));
  await render(false, false);
  const soop = host.querySelector('.soop-live-pane-grid')!, youtube = host.querySelector('.youtube-pane-grid')!;
  expect(soop.querySelector('[role=separator]')).toHaveAttribute('aria-valuenow', '32');
  expect(youtube.querySelector('[role=separator]')).toHaveAttribute('aria-valuenow', '68');
  const receivers = [...host.querySelectorAll('video')], leakedKey = vi.fn();
  window.addEventListener('keydown', leakedKey);
  try { await act(async () => soop.querySelector('[role=separator]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))); }
  finally { window.removeEventListener('keydown', leakedKey); }
  expect(leakedKey).not.toHaveBeenCalled();
  expect(soop.querySelector('[role=separator]')).toHaveAttribute('aria-valuenow', '34');
  expect(youtube.querySelector('[role=separator]')).toHaveAttribute('aria-valuenow', '68');
  await render(true, true);
  expect([...host.querySelectorAll('video')]).toEqual(receivers);
  expect(soop.querySelectorAll('[role=separator]')).toHaveLength(3);
  expect(youtube.querySelectorAll('[role=separator]')).toHaveLength(3);
  await act(async () => vi.advanceTimersByTime(220));
  expect(JSON.parse(localStorage.getItem(soopKey)!).columns).toBe(34);
  expect(JSON.parse(localStorage.getItem(youtubeKey)!).columns).toBe(68);
  expect(Object.keys(localStorage).sort()).toEqual([soopKey, youtubeKey]);
  expect(soopResizing).not.toHaveBeenCalled(); expect(youtubeResizing).not.toHaveBeenCalled();
});

it('keeps stop, mode and help controls local when both provider drawers exist', async () => {
  const soopStop = vi.fn(), youtubeStop = vi.fn(), soopMode = vi.fn(), youtubeApply = vi.fn();
  const item: YouTubeMetadata = { kind: 'video', id: 'abcdefghijk', url: 'https://www.youtube.com/watch?v=abcdefghijk', title: '테스트 영상', author: '테스트 채널', isLive: true };
  const render = async (active: boolean) => act(async () => root.render(<>
    <div className="soop-test"><SoopConnectionSetup active={active} favorites={<div>SOOP 즐겨찾기</div>} watching disabled={false} ready pending={false} rowCount={1} mode="all" onMode={soopMode} channels={[{ channelId: 'sample', name: '샘플' }]} lead="sample" onLead={vi.fn()} arrangement="channels" onArrangement={vi.fn()} onAdd={vi.fn()} onStart={vi.fn()} onStop={soopStop} onClose={vi.fn()}><input aria-label="SOOP 초안" /></SoopConnectionSetup></div>
    <YouTubeViewingControls active={active} open onOpenChange={vi.fn()} privacyMode={false} busy={false} items={[item]} playingIds={['video:abcdefghijk']} favoriteIds={[]} mode="all" lead="video:abcdefghijk" onApply={youtubeApply} onStop={youtubeStop} onExplore={vi.fn()} onCloseItem={vi.fn()} onFavorite={vi.fn()} onDownload={vi.fn()} />
  </>));
  await render(true);
  const soop = host.querySelector('.soop-test')!, youtube = host.querySelector('.youtube-connection-dock')!;
  await click(button(soop, '선택 보기')); expect(soopMode).toHaveBeenCalledExactlyOnceWith('selected'); expect(youtubeApply).not.toHaveBeenCalled();
  await click(button(soop, '종료')); expect(soopStop).toHaveBeenCalledOnce(); expect(youtubeStop).not.toHaveBeenCalled();
  await click(button(youtube, '종료')); expect(youtubeStop).toHaveBeenCalledOnce(); expect(soopStop).toHaveBeenCalledOnce();
  await act(async () => { host.querySelector('.soop-connection-help')!.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); host.querySelector('.youtube-connection-help')!.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
  const tips = [...document.querySelectorAll<HTMLElement>('[role=tooltip]')];
  expect(tips).toHaveLength(2); expect(new Set(tips.map(tip => tip.id)).size).toBe(2);
  await render(false);
  expect(document.querySelector('[role=tooltip]')).toBeNull();
  expect(youtube).toHaveAttribute('hidden'); expect(youtube).toHaveAttribute('inert');
});

it('does not transplant CHZZK domain state, events, commands or storage into the UI copies', () => {
  for (const source of [soopSource, youtubeSource, soopGridSource, youtubeGridSource, soopSetupSource, youtubeSetupSource]) {
    const imports = [...source.matchAll(/from\s+['"]([^'"]+)['"]/g)].map(match => match[1]!);
    expect(imports.filter(path => path.includes('/streaming/'))).toEqual(imports.includes('../streaming/usePanelHandleGesture') ? ['../streaming/usePanelHandleGesture'] : []);
    expect(source).not.toMatch(/chzzk-presentation|atsumi\.(?:chzzk|mado|live)\.|\b(?:officialBrowserApi|multiviewApi|liveChannelsApi|recordingProfileApi)\b/);
    expect(source).not.toMatch(/(?:dispatchEvent|emit|listen)\s*\(/);
  }
  expect(soopGridSource).toContain('atsumi.soop.live.panes.'); expect(youtubeGridSource).toContain('atsumi.youtube.live.panes.');
  expect(soopSource).toContain('atsumi.soop.live.arrangement.v1'); expect(youtubeSource).toContain('atsumi.youtube.live.arrangement.v1');
  expect(soopSource).not.toMatch(/atsumi\.youtube|provider:\s*["'](?:youtube|chzzk)["']/);
  expect(youtubeSource).not.toMatch(/atsumi\.soop|provider:\s*["'](?:soop|chzzk)["']/);
});

it('scopes every copied shell selector and animation to its provider', () => {
  for (const [prefix, css] of [['soop', soopShellCss], ['youtube', youtubeShellCss]] as const) {
    const clean = css.replace(/\/\*[\s\S]*?\*\//g, '');
    for (const match of clean.matchAll(/([^{}]+)\{/g)) {
      const selector = match[1]!.trim();
      if (selector.startsWith('@') || /^(?:from|to|\d+%)$/.test(selector)) continue;
      for (const arm of selector.split(',')) expect(arm.trim(), `Unscoped ${prefix} selector: ${arm}`).toMatch(new RegExp(`^\\.${prefix}-`));
    }
    for (const match of clean.matchAll(/@keyframes\s+([\w-]+)/g)) expect(match[1]).toMatch(new RegExp(`^${prefix}-`));
  }
});
