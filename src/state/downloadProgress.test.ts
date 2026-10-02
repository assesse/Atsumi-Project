import { afterEach, describe, expect, it, vi } from "vitest";
import { DownloadProgressStore } from "./downloadProgress";
import type { DownloadChangedEvent } from "../api/contracts";
import { galleryId } from "../core/types";

const event = (revision: number, state: DownloadChangedEvent["state"] = "downloading"): DownloadChangedEvent => ({
  galleryId: galleryId(1), entryId: "one", revision, state, progress: revision, attempt: 1,
});
afterEach(() => vi.useRealTimers());
describe("isolated download progress", () => {
  it("updates only subscribed IDs once per burst without gallery metadata changes", () => {
    vi.useFakeTimers(); const store = new DownloadProgressStore();
    const one = vi.fn(), other = vi.fn(); store.subscribe(1, one); store.subscribe(2, other);
    const fallback = event(1);
    for (let i=2; i<=50; i++) expect(store.apply(event(i), fallback).structural).toBe(false);
    expect(one).not.toHaveBeenCalled(); vi.advanceTimersByTime(100);
    expect(one).toHaveBeenCalledOnce(); expect(other).not.toHaveBeenCalled();
    expect(store.read(1, fallback)?.progress).toBe(50); expect(fallback.progress).toBe(1);
  });
  it("delivers terminal transitions immediately and rejects late progress", () => {
    vi.useFakeTimers(); const store = new DownloadProgressStore(), listener=vi.fn(); store.subscribe(1,listener);
    store.apply(event(2),event(1));
    expect(store.apply(event(3,"failed"),event(1)).structural).toBe(true);
    expect(listener).toHaveBeenCalledOnce();
    expect(store.apply(event(2),event(1)).applied).toBe(false);
    vi.advanceTimersByTime(100); expect(listener).toHaveBeenCalledOnce();
    expect(store.read(1,event(1))?.state).toBe("failed");
  });
  it("does not override a newer authoritative snapshot or another entry", () => {
    const store = new DownloadProgressStore(); store.apply(event(2),event(1));
    expect(store.read(1,event(5))).toEqual(event(5));
    const replacement={...event(1),entryId:"new"}; expect(store.read(1,replacement)).toBe(replacement);
    store.clear();
  });
});
