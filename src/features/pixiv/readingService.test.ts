import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { createReadingService } from "./readingService";

class FakeWorker {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessageerror: (() => void) | null = null;
  postMessage = vi.fn(); terminate = vi.fn();
  reply(result: unknown, id = this.postMessage.mock.calls.at(-1)![0].id) { this.onmessage?.({ data: { id, result } }); }
}
describe("Pixiv disposable reading worker", () => {
  let worker: FakeWorker, factory: Mock<() => Worker>, service: ReturnType<typeof createReadingService>;
  beforeEach(() => { vi.useFakeTimers(); worker = new FakeWorker(); factory = vi.fn(() => worker as unknown as Worker); service = createReadingService(factory); });
  afterEach(() => { service.clear(); vi.useRealTimers(); });
  it("never starts for plain Latin, Kana-only, invalid or aborted input", async () => {
    const controller = new AbortController(); controller.abort();
    for (const text of ["Artist", "ねこ", "", "山".repeat(301), "山\n"]) expect(await service.read(text)).toBeUndefined();
    expect(await service.read("山田", controller.signal)).toBeUndefined(); expect(factory).not.toHaveBeenCalled();
  });
  it("starts lazily, caches bounded successful readings, and terminates after idle", async () => {
    expect(factory).not.toHaveBeenCalled(); const pending = service.read("山田");
    worker.reply({ text: "yamada", partial: false }); expect(await pending).toEqual({ text: "yamada", partial: false });
    expect(await service.read("山田")).toEqual({ text: "yamada", partial: false }); expect(worker.postMessage).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(20000); expect(worker.terminate).toHaveBeenCalledTimes(1);
  });
  it("a newer target or cached target supersedes pending work and stale replies are ignored", async () => {
    const first = service.read("山田"), firstId = worker.postMessage.mock.calls[0]![0].id;
    const second = service.read("田中"); expect(await first).toBeUndefined();
    worker.reply({ text: "stale", partial: false }, firstId);
    worker.reply({ text: "tanaka", partial: false }); expect(await second).toEqual({ text: "tanaka", partial: false });
    const third = service.read("山本"); await service.read("田中"); expect(await third).toBeUndefined();
  });
  it("aborts on leave and clear immediately releases the worker and private cache", async () => {
    const controller = new AbortController(), pending = service.read("山田", controller.signal); controller.abort();
    expect(await pending).toBeUndefined();
    const next = service.read("田中"); service.clear(); expect(await next).toBeUndefined(); expect(worker.terminate).toHaveBeenCalledTimes(1);
    const fresh = service.read("田中"); expect(factory).toHaveBeenCalledTimes(2); worker.reply({ text: "tanaka", partial: false }); await fresh;
    service.clear(); const afterPrivacy = service.read("田中"); expect(factory).toHaveBeenCalledTimes(3); worker.reply({ text: "tanaka", partial: false }); await afterPrivacy;
  });
  it("times out safely and backs off after worker failure", async () => {
    const pending = service.read("山田"); vi.advanceTimersByTime(15000); expect(await pending).toBeUndefined();
    expect(worker.terminate).toHaveBeenCalled(); expect(await service.read("田中")).toBeUndefined(); expect(factory).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(30001); const retry = service.read("田中"); expect(factory).toHaveBeenCalledTimes(2);
    worker.onerror?.(); expect(await retry).toBeUndefined();
  });
  it("discards malformed results without caching them", async () => {
    for (const result of [{ text: "bad\n", partial: false }, { text: "good", partial: 1 }, null]) {
      const pending = service.read("山田"); worker.reply(result); expect(await pending).toBeUndefined();
    }
    expect(worker.postMessage).toHaveBeenCalledTimes(3);
  });
  it("evicts older entries above 128", async () => {
    for (let i = 0; i < 129; i++) { const pending = service.read(`山${i}`); worker.reply({ text: `yama ${i}`, partial: false }); await pending; }
    const old = service.read("山0"); expect(worker.postMessage).toHaveBeenCalledTimes(130); worker.reply({ text: "yama 0", partial: false }); await old;
  });
});
