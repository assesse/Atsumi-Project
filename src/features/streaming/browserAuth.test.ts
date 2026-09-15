import { describe, expect, it, vi } from "vitest";
import script from "../../../src-tauri/src/streaming/browser_auth.js?raw";

const endpoint = "https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus";
const evaluate = (fetch: typeof globalThis.fetch, href = endpoint, child = false) => {
  const frame: { top?: unknown } = {};
  frame.top = child ? {} : frame;
  return new Function("fetch", "location", "window", `return (\n${script}\n);`)(fetch, { href }, frame) as Promise<string>;
};
describe("minimal official-profile account observation", () => {
  it.each([true, false])("returns only the loggedIn boolean projection (%s)", async (loggedIn) => {
    const content = { loggedIn, get nickname() { throw new Error("profile field must not be read"); }, get userIdHash() { throw new Error("identity must not be read"); } };
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ code: 200, content }) });
    expect(await evaluate(fetch)).toBe(loggedIn ? "signed_in" : "signed_out");
    expect(fetch).toHaveBeenCalledExactlyOnceWith("https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus", expect.objectContaining({ method: "GET", credentials: "include", cache: "no-store", redirect: "error" }));
    expect(fetch.mock.calls[0]?.[1]).not.toHaveProperty("headers");
    expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|userIdHash|nickname/);
  });
  it.each([{ code: 200, content: { loggedIn: "true" } }, { code: 200, content: {} }, { code: 401, content: { loggedIn: false } }, null])("does not guess account state from an unexpected response", async (body) => {
    expect(await evaluate(vi.fn().mockResolvedValue({ ok: true, json: async () => body }))).toBe("unknown");
  });
  it("keeps network/HTTP failure unknown and never runs outside an official top-level document", async () => {
    expect(await evaluate(vi.fn().mockRejectedValue(new Error("offline")))).toBe("unknown");
    expect(await evaluate(vi.fn().mockResolvedValue({ ok: false }))).toBe("unknown");
    const fetch = vi.fn();
    expect(await evaluate(fetch, "https://nid.naver.com")).toBe("unknown");
    expect(await evaluate(fetch, endpoint, true)).toBe("unknown");
    expect(await evaluate(fetch, "https://chzzk.naver.com")).toBe("unknown");
    expect(await evaluate(fetch, `${endpoint}?redirect=1`)).toBe("unknown");
    expect(fetch).not.toHaveBeenCalled();
  });
});
