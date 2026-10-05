import { describe, expect, it, vi } from "vitest";
import type { DanbooruApi } from "../../api/featureClients";
import { createDanbooruPreviewLoader } from "./workPreview";

const result = (id: number) => ({ ok: true, data: { items: [{ id, previewUrl: `https://cdn.donmai.us/preview/${id}.jpg` }], page: 1, hasMore: false } });
describe("community preview requests", () => {
  it("shares duplicate works and limits metadata requests to three at a time", async () => {
    const done: (() => void)[] = [];
    const search = vi.fn(({ tags }: { tags: string }) => new Promise((resolve) => done.push(() => resolve(result(Number(tags.slice(3)))))));
    const load = createDanbooruPreviewLoader({ danbooruSearch: search } as unknown as DanbooruApi);
    const first = load("1"); expect(load("1")).toBe(first);
    const jobs = [first, ...[2, 3, 4, 5, 6].map((id) => load(String(id)))];
    expect(search).toHaveBeenCalledTimes(3);
    done[0]!(); await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(4));
    done[1]!(); done[2]!(); await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(6));
    done.slice(3).forEach((resolve) => resolve());
    expect(await Promise.all(jobs)).toHaveLength(6);
    expect(await load("1")).toContain("/1.jpg"); expect(search).toHaveBeenCalledTimes(6);
  });
  it("does not use a different post as a fallback and settles errors", async () => {
    const search = vi.fn().mockResolvedValueOnce(result(123)).mockRejectedValueOnce(new Error("offline"));
    const load = createDanbooruPreviewLoader({ danbooruSearch: search });
    expect(await load("99")).toBeNull(); expect(await load("12")).toBeNull();
    expect(await load("12")).toBeNull(); expect(search).toHaveBeenCalledTimes(2);
  });
});
