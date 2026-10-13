import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BackendClient } from '../api/backend';
import type { ApiResult, DetailOriginalPrepared } from '../api/contracts';
import { mockGalleries } from '../data/mockGalleries';
import { OriginalPagePool } from './OriginalPageWindow';

const gallery = { ...mockGalleries[0]!, download: undefined };
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function fixture() {
  const pending: Array<{ request: Parameters<BackendClient['detailOriginalPrepare']>[0]; finish(result?: ApiResult<DetailOriginalPrepared>): void }> = [];
  let active = 0, peak = 0;
  const backend = {
    detailOriginalPrepare: vi.fn<BackendClient['detailOriginalPrepare']>(request => {
      active++; peak = Math.max(peak, active);
      return new Promise(resolve => pending.push({ request, finish: result => {
        active--;
        resolve(result ?? { ok: true, data: { ...request, mediaUrl: `http://fixture/${request.sourcePage}`, contentType: 'image/png', width: 3000, height: 3000 } });
      } }));
    }),
    detailOriginalDispose: vi.fn().mockResolvedValue({ ok: true }),
  };
  return { backend, pending, peak: () => peak, pool: new OriginalPagePool(backend as unknown as BackendClient, gallery) };
}
afterEach(() => { vi.useRealTimers(); });
describe('original page preview window', () => {
  it('warms only three neighbours on either side, with visible pages first and two concurrent reads', async () => {
    const f = fixture(); f.pool.setRange([7], 30);
    expect(f.pending.map(item => item.request.sourcePage)).toEqual([7, 8]);
    for (let i = 0; i < 7; i++) { f.pending[i]!.finish(); await settle(); }
    expect(f.pending.map(item => item.request.sourcePage)).toEqual([7, 8, 6, 9, 5, 10, 4]);
    expect(f.peak()).toBe(2);
    expect(f.pool.get(3)).toBeUndefined(); expect(f.pool.get(11)).toBeUndefined();
    await expect(f.pool.get(7)).resolves.toMatchObject({ sourcePage: 7 });
    f.pool.dispose();
    expect(f.backend.detailOriginalDispose).toHaveBeenCalledTimes(7);
  });
  it('cancels old leases on a distant jump and discards late results after close', async () => {
    const f = fixture(); f.pool.setRange([1], 30);
    const old = f.pool.get(1)!; f.pool.setRange([20], 30);
    await expect(old).rejects.toThrow('preview closed');
    expect(f.pool.get(1)).toBeUndefined();
    f.pending[0]!.finish(); f.pending[1]!.finish(); await settle();
    expect(f.pending.slice(2).map(item => item.request.sourcePage)).toEqual([20, 21]);
    const current = f.pool.get(20)!; f.pool.dispose();
    await expect(current).rejects.toThrow('preview closed');
    f.pending[2]!.finish(); f.pending[3]!.finish(); await settle();
    expect(f.pending).toHaveLength(4);
    expect(f.backend.detailOriginalDispose).toHaveBeenCalledWith(f.pending[2]!.request.requestId);
  });
  it('times out without accepting a late original and never starts more than two reads', async () => {
    vi.useFakeTimers(); const f = fixture(); f.pool.setRange([5], 15);
    const visible = f.pool.get(5)!;
    const failed = expect(visible).rejects.toThrow('original timeout');
    await vi.advanceTimersByTimeAsync(60_000); await failed;
    expect(f.pending).toHaveLength(2);
    f.pool.dispose(); f.pending[0]!.finish(); f.pending[1]!.finish(); await settle();
    expect(f.peak()).toBe(2); expect(f.pool.get(5)).toBeUndefined();
  });
});
