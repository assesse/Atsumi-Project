import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import { useProgressiveGalleryWindow } from "./useProgressiveGalleryWindow";

describe("useProgressiveGalleryWindow", () => {
  it("reports entries and exits so stale detail hydration can be discarded", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const originalObserver = globalThis.IntersectionObserver;
    let notify: IntersectionObserverCallback | undefined;
    const observed: Element[] = [];
    globalThis.IntersectionObserver = class {
      readonly root = null;
      readonly rootMargin = "";
      readonly thresholds = [0];
      constructor(callback: IntersectionObserverCallback) { notify = callback; }
      observe(element: Element) { observed.push(element); }
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    } as unknown as typeof IntersectionObserver;
    const onEnter = vi.fn();
    const onLeave = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    function Harness() {
      const viewport = useRef<HTMLDivElement>(null);
      const active = useProgressiveGalleryWindow({
        rootRef: viewport,
        enabled: true,
        observeKey: "fixture",
        onEnter,
        onLeave,
      });
      return <div ref={viewport}>
        <div data-progressive-gallery-id="101" />
        <div data-progressive-gallery-id="102" />
        <output>{[...active].join(",")}</output>
      </div>;
    }

    try {
      await act(async () => root.render(<Harness />));
      expect(observed).toHaveLength(2);
      if (!notify) throw new Error("Intersection observer was not installed");
      await act(async () => notify?.([
        { target: observed[0]!, isIntersecting: true } as IntersectionObserverEntry,
        { target: observed[1]!, isIntersecting: true } as IntersectionObserverEntry,
      ], {} as IntersectionObserver));
      expect(container.querySelector("output")).toHaveTextContent("101,102");
      expect(onEnter).toHaveBeenCalledWith([galleryId(101), galleryId(102)]);

      await act(async () => notify?.([
        { target: observed[0]!, isIntersecting: false } as IntersectionObserverEntry,
      ], {} as IntersectionObserver));
      expect(container.querySelector("output")).toHaveTextContent("102");
      expect(onLeave).toHaveBeenCalledWith([galleryId(101)]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      globalThis.IntersectionObserver = originalObserver;
      vi.unstubAllGlobals();
    }
  });

  it("retains every entered slot across viewport exits and disabled periods", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const originalObserver = globalThis.IntersectionObserver;
    let notify: IntersectionObserverCallback | undefined;
    const observed: Element[] = [];
    globalThis.IntersectionObserver = class {
      readonly root = null;
      readonly rootMargin = "";
      readonly thresholds = [0];
      constructor(callback: IntersectionObserverCallback) { notify = callback; }
      observe(element: Element) { observed.push(element); }
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    } as unknown as typeof IntersectionObserver;
    const onEnter = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    function Harness({ enabled }: { enabled: boolean }) {
      const viewport = useRef<HTMLDivElement>(null);
      const active = useProgressiveGalleryWindow({
        rootRef: viewport,
        enabled,
        observeKey: "retained-fixture",
        onEnter,
        retainEntered: true,
      });
      return <div ref={viewport}>
        <div data-progressive-gallery-id="201" />
        <output>{[...active].join(",")}</output>
      </div>;
    }

    try {
      await act(async () => root.render(<Harness enabled />));
      if (!notify) throw new Error("Intersection observer was not installed");
      await act(async () => notify?.([
        { target: observed[0]!, isIntersecting: true } as IntersectionObserverEntry,
      ], {} as IntersectionObserver));
      expect(container.querySelector("output")).toHaveTextContent("201");

      await act(async () => notify?.([
        { target: observed[0]!, isIntersecting: false } as IntersectionObserverEntry,
      ], {} as IntersectionObserver));
      expect(container.querySelector("output")).toHaveTextContent("201");

      await act(async () => notify?.([
        { target: observed[0]!, isIntersecting: true } as IntersectionObserverEntry,
      ], {} as IntersectionObserver));
      expect(onEnter).toHaveBeenCalledTimes(2);

      await act(async () => root.render(<Harness enabled={false} />));
      expect(container.querySelector("output")).toHaveTextContent("201");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      globalThis.IntersectionObserver = originalObserver;
      vi.unstubAllGlobals();
    }
  });
});
