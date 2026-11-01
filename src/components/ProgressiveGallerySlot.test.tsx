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
  it("does not rebuild an offscreen summary for changing card actions, but mounts their latest state on entry", async () => {
    let reads = 0;
    const counted = { ...gallery, get title() { reads++; return gallery.title; } };
    const container = document.createElement("div"), root = createRoot(container);
    try {
      await act(async () => root.render(<ProgressiveGallerySlot gallery={counted} active={false}><button>before</button></ProgressiveGallerySlot>));
      const initialReads = reads;
      await act(async () => root.render(<ProgressiveGallerySlot gallery={counted} active={false}><button>latest</button></ProgressiveGallerySlot>));
      expect(reads).toBe(initialReads);
      expect(container.querySelector("button")).toBeNull();
      await act(async () => root.render(<ProgressiveGallerySlot gallery={counted} active><button>latest</button></ProgressiveGallerySlot>));
      expect(container.querySelector("button")).toHaveTextContent("latest");
    } finally { await act(async () => root.unmount()); }
  });

  it.each(["detail", "compact"] as const)("keeps the Korean-first title even in a %s placeholder", async (displayMode) => {
    const container = document.createElement("div"), root = createRoot(container);
    const bilingual = { ...gallery, title: "Original title | 한글 제목" };
    try {
      await act(async () => root.render(<ProgressiveGallerySlot gallery={bilingual} active={false} displayMode={displayMode}>loaded</ProgressiveGallerySlot>));
      expect(container.querySelector("strong")).toHaveTextContent(/^한글 제목$/);
      expect(container.querySelector('[title="Original title | 한글 제목"]')).not.toBeNull();
    } finally { await act(async () => root.unmount()); }
  });
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
