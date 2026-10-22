import { describe, expect, it, vi } from "vitest";
import { createPixivApi, createPixivMediaLoader } from "./api";
import { invoke } from "@tauri-apps/api/core";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
describe("Pixiv bounded media", () => {
  it("falls back only for an explicitly classified oversized original, preserving a visible quality marker", async () => {
    const original = URL.createObjectURL; URL.createObjectURL = vi.fn().mockReturnValue("blob:preview");
    try {
      vi.mocked(invoke).mockRejectedValueOnce("PIXIV_DISPLAY_LIMIT: fixture").mockResolvedValueOnce(new ArrayBuffer(12));
      expect(await createPixivApi("tauri").pageMedia("123", 0)).toEqual({ url: "blob:preview", bytes: 12, preview: true });
      expect(vi.mocked(invoke).mock.calls.slice(-2)).toEqual([["pixiv_page_media", { id: "123", page: 0 }], ["pixiv_page_preview", { id: "123", page: 0 }]]);
      const count = vi.mocked(invoke).mock.calls.length;
      vi.mocked(invoke).mockRejectedValueOnce("HTTP 403");
      await expect(createPixivApi("tauri").pageMedia("123", 0)).rejects.toBe("HTTP 403");
      expect(vi.mocked(invoke).mock.calls.length).toBe(count + 1);
    } finally { if (original) URL.createObjectURL = original; else Reflect.deleteProperty(URL, "createObjectURL"); }
  });
  it("dispatches isolated account commands without cookie or account values", async () => {
    vi.mocked(invoke).mockResolvedValue({ authenticated: false, open: false, busy: false, revision: 1 });
    const api = createPixivApi("tauri");
    await api.account!(); await api.login!(); await api.logout!();
    expect(vi.mocked(invoke).mock.calls.slice(-3)).toEqual([["pixiv_account_status", undefined], ["pixiv_account_login", undefined], ["pixiv_account_logout", undefined]]);
  });
  it("never fabricates data outside desktop runtime", async () => { await expect(createPixivApi("browser-mock").search({ query: "", page: 1, mode: "daily" })).rejects.toThrow("데스크톱"); });
  it("deduplicates identical pending requests and caches results", async () => {
    const media = vi.fn().mockResolvedValue("data:image/jpeg;base64,ZmFrZQ=="); const loader = createPixivMediaLoader({ media });
    const first = loader("url"); const second = loader("url"); expect(first).toBe(second); await first; await loader("url"); expect(media).toHaveBeenCalledTimes(1);
    await loader("url", true); expect(media).toHaveBeenCalledTimes(2);
  });
  it("never starts more than four image requests concurrently", async () => {
    const completions: ((value: string) => void)[] = [];
    const media = vi.fn(() => new Promise<string>(resolve => completions.push(resolve))); const loader = createPixivMediaLoader({ media });
    const promises = Array.from({ length: 8 }, (_, index) => loader(String(index)));
    expect(media).toHaveBeenCalledTimes(4);
    completions.splice(0).forEach(resolve => resolve("data:image/jpeg;base64,x")); await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(media).toHaveBeenCalledTimes(8); completions.splice(0).forEach(resolve => resolve("data:image/jpeg;base64,x")); await Promise.all(promises);
  });
  it("revokes late blob responses after dispose and permits a clean reload", async () => {
    const completions: ((value: string) => void)[] = [];
    const originalRevoke = URL.revokeObjectURL, revoke = vi.fn(); URL.revokeObjectURL = revoke;
    const loader = createPixivMediaLoader({ media: vi.fn(() => new Promise<string>(resolve => completions.push(resolve))) });
    const old = loader("url", true).catch(error => error.message); loader.dispose();
    const current = loader("url", true);
    completions[0]!("blob:old"); completions[1]!("blob:current");
    expect(await old).toContain("취소"); expect(await current).toBe("blob:current"); expect(revoke).toHaveBeenCalledWith("blob:old");
    loader.dispose(); expect(revoke).toHaveBeenCalledWith("blob:current");
    if (originalRevoke) URL.revokeObjectURL = originalRevoke; else Reflect.deleteProperty(URL, "revokeObjectURL");
  });
});
