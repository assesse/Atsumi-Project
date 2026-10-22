import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IMAGE_PREVIEW_WIDTHS, loadImagePreviewWidth, normalizeImagePreviewWidth, saveImagePreviewWidth, useImagePreviewWidth } from "./imagePreviewPreferences";

beforeEach(() => {
  localStorage.removeItem("atsumi.pinterest.preview-width.v1");
  localStorage.removeItem("atsumi.pixiv.preview-width.v1");
});
afterEach(() => vi.restoreAllMocks());

it("persists every bounded size independently for each image provider", () => {
  expect(loadImagePreviewWidth("pinterest")).toBe(180);
  expect(loadImagePreviewWidth("pixiv")).toBe(180);
  for (const width of IMAGE_PREVIEW_WIDTHS) {
    expect(saveImagePreviewWidth("pinterest", width)).toBe(true);
    expect(loadImagePreviewWidth("pinterest")).toBe(width);
    expect(loadImagePreviewWidth("pixiv")).toBe(180);
  }
  expect(saveImagePreviewWidth("pixiv", 220)).toBe(true);
  expect(loadImagePreviewWidth("pixiv")).toBe(220);
  expect(loadImagePreviewWidth("pinterest")).toBe(340);
});

it.each([null, undefined, "220", 0, -1, 9999, NaN, Infinity, 140.5])("normalizes invalid prop %s without accepting a new preset", value => {
  expect(normalizeImagePreviewWidth(value)).toBe(180);
});

it.each(["", "NaN", "null", "9999", "-1", "140.5"])("recovers malformed stored value %s", value => {
  localStorage.setItem("atsumi.pixiv.preview-width.v1", value);
  expect(loadImagePreviewWidth("pixiv")).toBe(180);
});

it("rejects invalid saves without writing or notifying and retains saved values on storage failure", () => {
  saveImagePreviewWidth("pinterest", 220);
  const changed = vi.fn();
  window.addEventListener("atsumi:image-preview-preference-changed", changed);
  try {
    expect(saveImagePreviewWidth("pinterest", 9999)).toBe(false);
    expect(loadImagePreviewWidth("pinterest")).toBe(220);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
    expect(saveImagePreviewWidth("pinterest", 340)).toBe(false);
    expect(loadImagePreviewWidth("pinterest")).toBe(220);
    expect(changed).not.toHaveBeenCalled();
  } finally { window.removeEventListener("atsumi:image-preview-preference-changed", changed); }
});

it("falls back safely when storage cannot be read", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("storage unavailable"); });
  expect(loadImagePreviewWidth("pixiv")).toBe(180);
});

it("updates existing image surfaces without remounting or touching video preferences", async () => {
  const mount = vi.fn(), unmount = vi.fn();
  const videoWidth = localStorage.getItem("atsumi.youtube.preview-width.v1");
  function Surface() {
    const pinterest = useImagePreviewWidth("pinterest"), pixiv = useImagePreviewWidth("pixiv");
    const inactive = useImagePreviewWidth(null);
    useEffect(() => { mount(); return unmount; }, []);
    return <section data-pinterest={pinterest} data-pixiv={pixiv} data-inactive={inactive}><div data-testid="card" /></section>;
  }
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<Surface />));
    const card = host.querySelector('[data-testid="card"]');
    await act(async () => { saveImagePreviewWidth("pinterest", 340); });
    expect(host.querySelector("section")).toHaveAttribute("data-pinterest", "340");
    expect(host.querySelector("section")).toHaveAttribute("data-pixiv", "180");
    expect(host.querySelector("section")).toHaveAttribute("data-inactive", "180");
    localStorage.setItem("atsumi.pixiv.preview-width.v1", "140");
    await act(async () => { window.dispatchEvent(new StorageEvent("storage", { key: "atsumi.pixiv.preview-width.v1" })); });
    expect(host.querySelector("section")).toHaveAttribute("data-pixiv", "140");
    expect(host.querySelector('[data-testid="card"]')).toBe(card);
    expect(mount).toHaveBeenCalledTimes(1);
    expect(unmount).not.toHaveBeenCalled();
    expect(localStorage.getItem("atsumi.youtube.preview-width.v1")).toBe(videoWidth);
  } finally { await act(async () => root.unmount()); }
});
