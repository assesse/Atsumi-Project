import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SoopApi, SoopChannel } from "../../api/soop";
import { SoopPortrait } from "./SoopPortrait";
import { scheduleSoopImage, subscribeSoopPortrait } from "./soopImageQueue";

const image = "data:image/png;base64,AQ==";
const channel = (channelId = "creator", portrait: string | null = image): SoopChannel => ({ channelId, name: "채널", image: portrait, title: null, status: "offline", viewers: null });
let host: HTMLDivElement, root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
describe("SOOP exact public portraits", () => {
  it("deduplicates one channel across cards and clears immediately in privacy or inactive views", async () => {
    const api: SoopApi = { inspect: vi.fn().mockResolvedValue(channel()) };
    const render = (active = true, privacy = false) => <><SoopPortrait api={api} channelId="creator" active={active} privacy={privacy} /><SoopPortrait api={api} channelId="creator" active={active} privacy={privacy} /></>;
    await act(async () => root.render(render()));
    expect(api.inspect).toHaveBeenCalledTimes(1); expect(host.querySelectorAll("img")).toHaveLength(2);
    await act(async () => root.render(render(false))); expect(host.querySelector("img")).toBeNull();
    await act(async () => root.render(render(true, true))); expect(api.inspect).toHaveBeenCalledTimes(1);
    await act(async () => root.render(render())); expect(host.querySelectorAll("img")).toHaveLength(2); expect(api.inspect).toHaveBeenCalledTimes(1);
  });
  it("rejects wrong identity, arbitrary URLs and oversized images", async () => {
    for (const result of [channel("other"), channel("creator", "https://evil.test/avatar"), channel("creator", `data:image/png;base64,${"A".repeat(180_000)}`)]) {
      const api: SoopApi = { inspect: vi.fn().mockResolvedValue(result) };
      await act(async () => root.render(<SoopPortrait api={api} channelId="creator" active privacy={false} />));
      expect(host.querySelector("img")).toBeNull();
    }
  });
  it("does not reveal a late response after privacy and retains exact-channel cache isolation", async () => {
    let resolve!: (value: SoopChannel) => void;
    const api: SoopApi = { inspect: vi.fn().mockImplementation(() => new Promise<SoopChannel>(done => { resolve = done; })) };
    await act(async () => root.render(<SoopPortrait api={api} channelId="creator" active privacy={false} />));
    await act(async () => root.render(<SoopPortrait api={api} channelId="creator" active privacy />));
    await act(async () => resolve(channel())); expect(host.querySelector("img")).toBeNull();
    const otherApi: SoopApi = { inspect: vi.fn().mockResolvedValue(channel("creator", null)) };
    await act(async () => root.render(<SoopPortrait api={otherApi} channelId="creator" active privacy={false} />));
    expect(otherApi.inspect).toHaveBeenCalledTimes(1); expect(host.querySelector("img")).toBeNull();
  });
  it("admits at most two image operations and cancels queued portrait work", async () => {
    const finish: (() => void)[] = [];
    scheduleSoopImage(() => new Promise<void>(resolve => finish.push(resolve)));
    scheduleSoopImage(() => new Promise<void>(resolve => finish.push(resolve)));
    const api: SoopApi = { inspect: vi.fn().mockResolvedValue(channel()) };
    const stop = subscribeSoopPortrait(api, "creator", vi.fn());
    await act(async () => {}); expect(finish).toHaveLength(2); expect(api.inspect).not.toHaveBeenCalled();
    stop();
    await act(async () => finish.forEach(resolve => resolve())); expect(api.inspect).not.toHaveBeenCalled();
  });
  it("evicts settled cache entries even while128 previous portraits remain mounted", async () => {
    const api: SoopApi = { inspect: vi.fn().mockImplementation(async id => channel(id)) };
    const render = (count: number) => <>{Array.from({ length: count }, (_, id) => <SoopPortrait key={id} api={api} channelId={`creator${id}`} active privacy={false} />)}</>;
    await act(async () => root.render(render(128)));
    expect(api.inspect).toHaveBeenCalledTimes(128); expect(host.querySelectorAll("img")).toHaveLength(128);
    await act(async () => root.render(render(129)));
    expect(api.inspect).toHaveBeenCalledTimes(129); expect(api.inspect).toHaveBeenLastCalledWith("creator128");
    expect(host.querySelectorAll("img")).toHaveLength(129);
  });
  it("waits for visibility before requesting a portrait", async () => {
    let callback!: IntersectionObserverCallback;
    const observe = vi.fn(), disconnect = vi.fn();
    vi.stubGlobal("IntersectionObserver", class { constructor(fn: IntersectionObserverCallback) { callback = fn; } observe = observe; disconnect = disconnect; });
    try {
      const api: SoopApi = { inspect: vi.fn().mockResolvedValue(channel()) };
      await act(async () => root.render(<SoopPortrait api={api} channelId="creator" active privacy={false} />));
      expect(observe).toHaveBeenCalled(); expect(api.inspect).not.toHaveBeenCalled();
      await act(async () => callback([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
      expect(api.inspect).toHaveBeenCalledTimes(1); expect(host.querySelector("img")).toHaveAttribute("src", image);
    } finally { vi.unstubAllGlobals(); }
  });
});
