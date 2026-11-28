import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { groupKeeperFixture } from '../test/groupKeeperFixture';
import { buildOverlapReviewGroup } from '../downloadOverlap/groupReview';
import { OverlapGroupReview, type GroupReviewDraft } from './OverlapGroupReview';
import type { EditionBatchOperation } from '../downloadOverlap/editionBatch';

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.restoreAllMocks(); });
async function mount() {
  const fixture = groupKeeperFixture(), container = document.createElement('div');
  document.body.append(container); const root = createRoot(container);
  const apply = vi.fn(async (operations: EditionBatchOperation[]) => ({ completedKeys: operations.map(o => o.key), total: operations.length }));
  let draft: GroupReviewDraft | undefined;
  const render = async () => act(async () => root.render(<OverlapGroupReview group={buildOverlapReviewGroup(fixture.review, undefined, fixture.reviews)!}
    disabled={false} loading={false} onDraftChange={d => { draft = d; }} onClearPreview={vi.fn()} onDecision={vi.fn()} onMerge={vi.fn()}
    onApplyEditions={apply} renderCover={() => null} renderPage={p => p.page && p.visible ? <button aria-label={p.label + ' page ' + p.page}
      data-selected={p.selection?.pages[p.side].includes(p.page)} onClick={e => { if (e.ctrlKey || e.shiftKey) p.onToggle(p.side, p.page!, p.pair, e.shiftKey); }}>{p.page}</button> : null} />));
  await render(); cleanups.push(async () => { await act(async () => root.unmount()); container.remove(); });
  const click = async (el: HTMLElement) => act(async () => el.click());
  const mode = async (id: number, label: string) => click([...container.querySelectorAll<HTMLButtonElement>(`[aria-label="#${id} 남길 범위"] button`)].find(b => b.textContent === label)!);
  const applyClick = () => click(container.querySelector<HTMLButtonElement>('.overlap-group-actions .primary-button')!);
  const select = async (id: number, page: number, shift = false) => act(async () => container.querySelector(`[aria-label="#${id} page ${page}"]`)!.dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: !shift, shiftKey: shift })));
  return { ...fixture, container, root, render, apply, click, mode, applyClick, select, getDraft: () => draft! };
}

describe('group-wide surviving edition', () => {
  it('applies both removals when only the third edition is kept', async () => {
    const f = await mount(); vi.spyOn(window, 'confirm').mockReturnValue(true);
    await f.mode(2776471, '제거'); await f.mode(2776792, '제거'); await f.mode(3053713, '남기기'); await f.mode(2776471, '제거');
    await f.applyClick();
    expect(f.container.textContent).not.toContain('남길 판본을 하나 이상');
    expect(f.apply).toHaveBeenCalledOnce();
    const operations = f.apply.mock.calls[0]![0];
    expect(operations.map(o => o.galleryId).sort()).toEqual([2776471, 2776792]);
    expect(operations.every(o => o.incomingEntryId === 'entry-3053713' && o.kind === 'decision' && o.request.action === 'remove_existing_continue')).toBe(true);
  });
  it('keeps the baseline mode while activating, closing and reopening another comparison', async () => {
    const f = await mount(); await f.mode(2776471, '제거'); await f.mode(3053713, '남기기');
    expect(f.container.querySelector('[aria-label="#2776471 남길 범위"]')).toHaveAttribute('data-mode', 'remove');
    await f.click(f.container.querySelector('[aria-label="#2776792 비교"]')!); await f.click(f.container.querySelector('[aria-label="#2776792 비교"]')!);
    expect(f.container.querySelector('[aria-label="#2776471 남길 범위"]')).toHaveAttribute('data-mode', 'remove');
  });
  it('shares baseline Ctrl/Shift choices across active peers without losing their separate choices', async () => {
    const f = await mount();
    await f.select(2776471, 1); await f.select(2776792, 5); await f.mode(3053713, '제거');
    expect(f.container.querySelector('[aria-label="#2776471 page 1"]')).toHaveAttribute('data-selected', 'true');
    expect(f.container.querySelector('[aria-label="#2776471 남길 범위"]')).toHaveAttribute('data-mode', 'partial');
    await f.select(2776471, 3, true);
    for (const page of [1, 2, 3]) expect(f.container.querySelector('[aria-label="#2776471 page '+page+'"]')).toHaveAttribute('data-selected', 'true');
    expect(f.container.querySelector('[aria-label="#2776792 page 5"]')).toHaveAttribute('data-selected', 'true');
    await f.click(f.container.querySelector('[aria-label="#2776792 비교"]')!); await f.click(f.container.querySelector('[aria-label="#2776792 비교"]')!);
    expect(f.container.querySelector('[aria-label="#2776471 page 3"]')).toHaveAttribute('data-selected', 'true');
    await f.mode(2776471, '남기기'); await f.mode(2776471, '제거'); await f.mode(2776471, '일부만');
    expect(f.getDraft().keeper?.pages).toEqual([1, 2, 3]);
  });
  it('deselects direct counterparts across peers when choosing a baseline page', async () => {
    const f = await mount(); await f.select(2776792, 2); await f.select(3053713, 2); await f.select(2776471, 2);
    expect(f.container.querySelector('[aria-label="#2776792 page 2"]')).toHaveAttribute('data-selected', 'false');
    expect(f.container.querySelector('[aria-label="#3053713 page 2"]')).toHaveAttribute('data-selected', 'false');
    expect(f.container.querySelector('[aria-label="#2776471 page 2"]')).toHaveAttribute('data-selected', 'true');
  });
  it('blocks an all-remove intent without invoking the batch or confirmation', async () => {
    const f = await mount(), confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    for (const id of f.ids) await f.mode(id, '제거'); await f.applyClick();
    expect(f.container.textContent).toContain('남길 판본을 하나 이상'); expect(f.apply).not.toHaveBeenCalled(); expect(confirm).not.toHaveBeenCalled();
  });
  it('keeps the draft after cancelling confirmation and uses actual removed IDs', async () => {
    const f = await mount(), confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    await f.mode(2776471, '제거'); await f.mode(2776792, '제거'); await f.applyClick();
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('#2776471 제거'));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('#2776792 제거'));
    expect(confirm.mock.calls[0]![0]).not.toContain('#3053713 제거'); expect(f.apply).not.toHaveBeenCalled();
    expect(f.getDraft().keeper?.mode).toBe('remove');
  });

});
