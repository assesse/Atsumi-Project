import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { loadVideoPreviewWidth, saveVideoPreviewWidth, useVideoPreviewWidth } from "./previewPreferences";

beforeEach(() => {
  localStorage.removeItem("atsumi.soop.preview-width.v1");
  localStorage.removeItem("atsumi.youtube.preview-width.v1");
});
afterEach(() => vi.restoreAllMocks());

it("validates stored sizes and keeps independent provider preferences", () => {
  expect(loadVideoPreviewWidth("soop")).toBe(320);
  expect(saveVideoPreviewWidth("soop", 400)).toBe(true);
  expect(loadVideoPreviewWidth("soop")).toBe(400);
  expect(loadVideoPreviewWidth("youtube")).toBe(320);
  expect(saveVideoPreviewWidth("youtube", 240)).toBe(true);
  expect(loadVideoPreviewWidth("youtube")).toBe(240);
  expect(saveVideoPreviewWidth("soop", 9999)).toBe(false);
  expect(loadVideoPreviewWidth("soop")).toBe(400);
  localStorage.setItem("atsumi.youtube.preview-width.v1", "NaN");
  expect(loadVideoPreviewWidth("youtube")).toBe(320);
});

it("retains the last saved size when storage rejects the change", () => {
  saveVideoPreviewWidth("soop", 240);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
  expect(saveVideoPreviewWidth("soop", 400)).toBe(false);
  expect(loadVideoPreviewWidth("soop")).toBe(240);
});

it("reactively resizes existing surfaces without remounting media", async () => {
  const mount = vi.fn(), unmount = vi.fn();
  function Surface() {
    const soop = useVideoPreviewWidth("soop"), youtube = useVideoPreviewWidth("youtube");
    useEffect(() => { mount(); return unmount; }, []);
    return <section data-soop={soop} data-youtube={youtube}><video /></section>;
  }
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<Surface />));
    const video = host.querySelector("video");
    await act(async () => { saveVideoPreviewWidth("soop", 400); });
    expect(host.querySelector("section")).toHaveAttribute("data-soop", "400");
    expect(host.querySelector("section")).toHaveAttribute("data-youtube", "320");
    expect(host.querySelector("video")).toBe(video);
    expect(mount).toHaveBeenCalledTimes(1); expect(unmount).not.toHaveBeenCalled();
    localStorage.setItem("atsumi.youtube.preview-width.v1", "240");
    await act(async () => { window.dispatchEvent(new StorageEvent("storage", { key: "atsumi.youtube.preview-width.v1" })); });
    expect(host.querySelector("section")).toHaveAttribute("data-youtube", "240");
  } finally { await act(async () => root.unmount()); }
});
