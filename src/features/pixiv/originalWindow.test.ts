import { afterEach, describe, expect, it, vi } from "vitest";
import { createOriginalWindow, originalPageWindow, PAGE_BYTES } from "./originalWindow";
import type { PixivMedia } from "./model";
const key = (page: number) => `atsumi-pixiv-public:123:${page}`;
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
afterEach(() => vi.unstubAllGlobals());
describe("Pixiv original reading window", () => {
  it("requests visible pages first and only two neighbours either side, never black blanks", () => {
    expect(originalPageWindow([3, 2], 8)).toEqual({ visible: [3, 2], prefetch: [1, 4, 0, 5] });
    expect(originalPageWindow([null, 0], 5)).toEqual({ visible: [0], prefetch: [1, 2] });
    expect(originalPageWindow([4, null], 5)).toEqual({ visible: [4], prefetch: [3, 2] });
  });
  it("limits concurrency to two, cancels stale queued reads and disposes late originals", async () => {
    const revoke = vi.fn(); vi.stubGlobal("URL", { revokeObjectURL: revoke });
    const resolvers = new Map<number, (value: PixivMedia) => void>();
    const pageMedia = vi.fn((_id: string, page: number) => new Promise<PixivMedia>(resolve => resolvers.set(page, resolve)));
    const pool = createOriginalWindow({ pageMedia, localMedia: vi.fn() });
    pool.setWindow([key(2), key(3)], [key(1), key(4), key(0), key(5)]);
    expect(pageMedia.mock.calls.map(call => call[1])).toEqual([2, 3]);
    pool.setWindow([key(8)], [key(7), key(9)]);
    resolvers.get(2)!({ url: "blob:old2", bytes: 20 }); resolvers.get(3)!({ url: "blob:old3", bytes: 20 }); await flush();
    expect(pageMedia.mock.calls.map(call => call[1])).toEqual([2, 3, 8, 7]);
    expect(revoke).toHaveBeenCalledWith("blob:old2"); expect(pool.get(key(2))).toBeUndefined();
    pool.clear(); resolvers.get(8)!({ url: "blob:old8", bytes: 20 }); resolvers.get(7)!({ url: "blob:old7", bytes: 20 }); await flush();
    expect(pageMedia).toHaveBeenCalledTimes(4); expect(revoke).toHaveBeenCalledWith("blob:old8");
  });
  it("counts binary bytes, evicts speculative pages and never revokes visible pages", async () => {
    const revoke = vi.fn(); vi.stubGlobal("URL", { revokeObjectURL: revoke });
    const pageMedia = vi.fn(async (_id: string, page: number) => ({ url: `blob:${page}`, bytes: PAGE_BYTES }));
    const pool = createOriginalWindow({ pageMedia, localMedia: vi.fn() });
    pool.setWindow([key(2), key(3)], [key(1), key(4), key(0), key(5)]); await flush();
    expect(pool.get(key(2))?.state).toBe("ready"); expect(pool.get(key(3))?.state).toBe("ready");
    expect(revoke).not.toHaveBeenCalledWith("blob:2"); expect(revoke).not.toHaveBeenCalledWith("blob:3");
    const retained = [0, 1, 2, 3, 4, 5].reduce((sum, page) => sum + (pool.get(key(page))?.bytes ?? 0), 0);
    expect(retained).toBeLessThanOrEqual(96 * 1024 * 1024);
    pool.setWindow([key(2)], []); await flush(); expect(pageMedia.mock.calls.filter(call => call[1] === 2)).toHaveLength(1);
    pool.clear(); expect(revoke).toHaveBeenCalledWith("blob:2");
  });
  it("keeps failed local originals local and supports an explicit retry", async () => {
    const localMedia = vi.fn().mockRejectedValueOnce(new Error("missing")).mockResolvedValue({ url: "data:image/png;base64,eA==", bytes: 1 });
    const pageMedia = vi.fn(); const pool = createOriginalWindow({ localMedia, pageMedia }); const local = "atsumi-pixiv:123:0";
    pool.setWindow([local], []); await flush(); expect(pool.get(local)?.state).toBe("error"); expect(pageMedia).not.toHaveBeenCalled();
    pool.retry(local); await flush(); expect(pool.get(local)?.state).toBe("ready"); expect(localMedia).toHaveBeenLastCalledWith("123", 0, true); pool.clear();
  });
  it("deduplicates overlapping windows and rejects oversized native responses", async () => {
    const revoke = vi.fn(); vi.stubGlobal("URL", { revokeObjectURL: revoke }); let resolve!: (value: PixivMedia) => void;
    const pageMedia = vi.fn(() => new Promise<PixivMedia>(done => { resolve = done; }));
    const pool = createOriginalWindow({ pageMedia, localMedia: vi.fn() });
    pool.setWindow([key(0)], []); pool.setWindow([key(0)], []); expect(pageMedia).toHaveBeenCalledTimes(1);
    resolve({ url: "blob:large", bytes: PAGE_BYTES + 1 }); await flush(); expect(pool.get(key(0))?.state).toBe("error"); expect(revoke).toHaveBeenCalledWith("blob:large"); pool.clear();
  });
  it("reports a failed browser decode and retries instead of leaving a broken image", async () => {
    const revoke = vi.fn(); vi.stubGlobal("URL", { revokeObjectURL: revoke });
    const pageMedia = vi.fn().mockResolvedValue({ url: "blob:decode", bytes: 20 }); const pool = createOriginalWindow({ pageMedia, localMedia: vi.fn() });
    pool.setWindow([key(0)], []); await flush(); pool.failedImage(key(0)); expect(pool.get(key(0))?.state).toBe("error"); expect(revoke).toHaveBeenCalledWith("blob:decode");
    pool.retry(key(0)); await flush(); expect(pageMedia).toHaveBeenCalledTimes(2); expect(pool.get(key(0))?.state).toBe("ready"); pool.clear();
  });
});
