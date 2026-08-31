import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { ProgressiveGallerySlot } from "./ProgressiveGallerySlot";

const gallery: Gallery = {
  id: galleryId(1234567),
  title: "요약 앨범",
  subtitle: "",
  artist: "작가 이름",
  pages: 24,
  score: 0,
  publishedAt: "2026-09-01",
  coverIndex: 0,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
};

describe("ProgressiveGallerySlot", () => {
  it("keeps local summary fields visible while the full card is outside the overscan window", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(
      <ProgressiveGallerySlot gallery={gallery} active={false}><button>상세 카드</button></ProgressiveGallerySlot>,
    ));

    expect(container.textContent).toContain("요약 앨범");
    expect(container.textContent).toContain("작가 이름");
    expect(container.textContent).toContain("24p");
    expect(container.querySelector("button")).toBeNull();
    await act(async () => root.unmount());
  });

  it("mounts the interactive card when the slot enters the viewport window", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(
      <ProgressiveGallerySlot gallery={gallery} active><button>상세 카드</button></ProgressiveGallerySlot>,
    ));

    expect(container.querySelector("button")?.textContent).toBe("상세 카드");
    expect(container.querySelector("[aria-busy='true']")).toBeNull();
    await act(async () => root.unmount());
  });

  it("uses the same fixed preview footprint while compact details are deferred", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(
      <ProgressiveGallerySlot gallery={gallery} active={false} displayMode="compact"><button>상세 카드</button></ProgressiveGallerySlot>,
    ));

    expect(container.querySelector(".gallery-card")).toHaveClass("is-compact");
    expect(container.querySelector(".card-content")).toBeNull();
    expect(container.textContent).toContain("요약 앨범");
    await act(async () => root.unmount());
  });
});
