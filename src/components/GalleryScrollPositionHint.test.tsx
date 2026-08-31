import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import type { GalleryGroup } from "../state/galleryGrouping";
import { GalleryScrollPositionHint, galleryScrollHintCopy } from "./GalleryScrollPositionHint";

const gallery = (id: number, artist: string, title: string): Gallery => ({
  id: galleryId(id),
  title,
  subtitle: "",
  artist,
  pages: 20,
  score: 0,
  publishedAt: "2026-08-31",
  coverIndex: 0,
  language: "korean",
  tags: [],
  series: [],
  characters: [],
});

const first = gallery(1, "하루", "첫 앨범");
const second = gallery(2, "나미", "가운데 앨범");
const third = gallery(3, "나미", "마지막 앨범");

describe("GalleryScrollPositionHint", () => {
  let scrollRoot: HTMLElement;
  let renderHost: HTMLDivElement;
  let reactRoot: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
    scrollRoot = document.createElement("section");
    renderHost = document.createElement("div");
    document.body.append(scrollRoot, renderHost);
    reactRoot = createRoot(renderHost);

    Object.defineProperties(scrollRoot, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 2_000 },
      clientWidth: { configurable: true, value: 800 },
      offsetWidth: { configurable: true, value: 812 },
      scrollTop: { configurable: true, writable: true, value: 800 },
    });
    scrollRoot.getBoundingClientRect = () => ({
      x: 0,
      y: 100,
      top: 100,
      right: 812,
      bottom: 500,
      left: 0,
      width: 812,
      height: 400,
      toJSON: () => undefined,
    });
    [first, second, third].forEach((item, index) => {
      const card = document.createElement("article");
      card.dataset.galleryId = String(item.id);
      const top = [110, 220, 410][index] ?? 110;
      const height = index === 0 ? 80 : 170;
      card.getBoundingClientRect = () => ({
        x: 10,
        y: top,
        top,
        right: 250,
        bottom: top + height,
        left: 10,
        width: 240,
        height,
        toJSON: () => undefined,
      });
      scrollRoot.append(card);
    });
  });

  afterEach(async () => {
    await act(async () => reactRoot.unmount());
    scrollRoot.remove();
    renderHost.remove();
    vi.unstubAllGlobals();
  });

  it("shows the nearby rendered album and position while scrolling", async () => {
    await act(async () => {
      reactRoot.render(
        <GalleryScrollPositionHint
          rootRef={{ current: scrollRoot }}
          view="downloads"
          grouping="all"
          items={[first, second, third]}
          groups={[]}
        />,
      );
    });
    expect(document.querySelector(".gallery-scroll-position-hint")).toBeNull();

    await act(async () => {
      scrollRoot.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 120 }));
      await new Promise((resolve) => window.setTimeout(resolve, 10));
    });

    const hint = document.querySelector<HTMLElement>(".gallery-scroll-position-hint");
    expect(hint).toHaveAttribute("role", "status");
    expect(hint).toHaveAttribute("aria-live", "polite");
    expect(hint).toHaveAttribute("data-view", "downloads");
    expect(hint).toHaveTextContent("전체 · 나미");
    expect(hint).toHaveTextContent("가운데 앨범");
    expect(hint).toHaveTextContent("2 / 3");
    expect(hint).toHaveTextContent("50%");
    expect(Number.parseFloat(hint?.style.top ?? "0")).toBeGreaterThanOrEqual(42);
    expect(Number.parseFloat(hint?.style.right ?? "0")).toBeGreaterThanOrEqual(8);
  });

  it("opens from the scrollbar gutter without a scroll and hides after leaving it", async () => {
    await act(async () => {
      reactRoot.render(
        <GalleryScrollPositionHint
          rootRef={{ current: scrollRoot }}
          view="auto-find"
          grouping="all"
          items={[first, second, third]}
          groups={[]}
        />,
      );
    });
    await act(async () => {
      scrollRoot.dispatchEvent(new MouseEvent("pointermove", { bubbles: true, clientX: 807, clientY: 200 }));
      await new Promise((resolve) => window.setTimeout(resolve, 10));
    });
    expect(document.querySelector(".gallery-scroll-position-hint")).toHaveAttribute("data-view", "auto-find");

    await act(async () => {
      scrollRoot.dispatchEvent(new MouseEvent("pointermove", { bubbles: true, clientX: 200, clientY: 200 }));
      await new Promise((resolve) => window.setTimeout(resolve, 280));
    });
    expect(document.querySelector(".gallery-scroll-position-hint")).toBeNull();
  });

  it("describes the active artist or period group as well as the overall position", () => {
    const artistGroups: GalleryGroup[] = [
      { key: "artist\u001f하루", label: "하루", items: [first] },
      { key: "artist\u001f나미", label: "나미", items: [second, third] },
    ];
    expect(galleryScrollHintCopy("auto-find", "artist", [first, second, third], artistGroups, 2)).toEqual({
      label: "즐겨찾기 작가 · 나미",
      title: "마지막 앨범",
      position: "2 / 2 · 전체 3 / 3",
    });
    expect(galleryScrollHintCopy("downloads", "day", [first, second], [
      { key: "day\u001f2026-08-31", label: "2026년 8월 31일", items: [first, second] },
    ], 0)).toEqual({
      label: "기간 · 2026년 8월 31일",
      title: "첫 앨범",
      position: "1 / 2 · 전체 1 / 2",
    });
  });
});
