import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CommunityWorkPreview } from "./CommunityWorkPreview";
import { loadDanbooruPreview } from "./workPreview";
vi.mock("./workPreview", () => ({ loadDanbooruPreview: vi.fn().mockResolvedValue("https://cdn.donmai.us/preview/test.jpg") }));
vi.mock("../../components/GalleryThumbnail", () => ({ GalleryThumbnail: ({ alt, thumbnailKey }: { alt: string; thumbnailKey: unknown }) => <img alt={alt} data-key={JSON.stringify(thumbnailKey)} /> }));
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => { vi.clearAllMocks(); vi.unstubAllGlobals(); });
it("uses the shared Hitomi cover coordinator and never fetches in privacy mode", async () => {
  const host = document.createElement("div"); const root = createRoot(host);
  try {
    await act(async () => root.render(<CommunityWorkPreview work={{ source: "hitomi", workId: "123" }} />));
    expect(host.querySelector("img")).toHaveAttribute("data-key", JSON.stringify({ kind: "gallery-cover", galleryId: 123 }));
    await act(async () => root.render(<CommunityWorkPreview work={{ source: "danbooru", workId: "123" }} privacyMode />));
    expect(host.querySelector("img")).toBeNull(); expect(loadDanbooruPreview).not.toHaveBeenCalled();
    await act(async () => root.render(<CommunityWorkPreview work={{ source: "hitomi", workId: "99999999999999999999" }} />));
    expect(host.textContent).toContain("미리보기 없음");
  } finally { await act(async () => root.unmount()); }
});
it("defers Danbooru metadata until near viewport and settles image failures", async () => {
  let observe!: IntersectionObserverCallback;
  vi.stubGlobal("IntersectionObserver", class { constructor(callback: IntersectionObserverCallback) { observe = callback; } observe() {} disconnect() {} });
  const host = document.createElement("div"); const root = createRoot(host);
  try {
    await act(async () => root.render(<CommunityWorkPreview work={{ source: "danbooru", workId: "123" }} />));
    expect(loadDanbooruPreview).not.toHaveBeenCalled();
    await act(async () => { observe([{ isIntersecting: true }] as IntersectionObserverEntry[], {} as IntersectionObserver); });
    expect(loadDanbooruPreview).toHaveBeenCalledExactlyOnceWith("123");
    expect(host.querySelector("img")).toHaveAttribute("referrerpolicy", "no-referrer");
    await act(async () => host.querySelector("img")!.dispatchEvent(new Event("error")));
    expect(host.textContent).toContain("미리보기 없음");
    expect(host.firstElementChild).toHaveAttribute("aria-busy", "false");
  } finally { await act(async () => root.unmount()); }
});
