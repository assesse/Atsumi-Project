import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { type Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient } from "../thumbnail";
import { DetailWorkspace } from "./DetailWorkspace";
import { detailPreviewWindowSize } from "./detailPreviewWindow";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("DetailWorkspace page previews", () => {
  it("uses the fixed window regardless of Related height", () => {
    expect(detailPreviewWindowSize(18, 3)).toBe(9);
  });

  it("shows the storage-folder action only after a download entry exists", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const source: Gallery = { ...mockGalleries[0]!, pageDimensions: [] };
    delete source.download;
    const started: Gallery = {
      ...source,
      download: {
        entryId: "detail-folder-entry",
        state: "downloading",
        progress: 25,
      },
    };
    const onQueue = vi.fn();
    const onOpenDownloadFolder = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (gallery: Gallery) => root.render(
      <DetailWorkspace
        tabs={[gallery.id]}
        activeId={gallery.id}
        minimized={false}
        galleries={new Map([[gallery.id, gallery]])}
        favoriteMetadata={new Set()}
        thumbnailClient={client}
        onActivate={vi.fn()}
        onClose={vi.fn()}
        onCloseAll={vi.fn()}
        onMinimize={vi.fn()}
        onRestore={vi.fn()}
        onOpenRelated={vi.fn()}
        onQueue={onQueue}
        onOpenDownloadFolder={onOpenDownloadFolder}
        onMetadataSearch={vi.fn()}
        onMetadataFavorite={vi.fn()}
      />
    );

    try {
      await act(async () => render(source));
      expect(container.querySelector('[aria-label="저장 폴더 열기"]')).toBeNull();
      expect(container.querySelector('[aria-label="다운로드"]')).not.toBeNull();

      await act(async () => render(started));
      const folderButton = container.querySelector<HTMLButtonElement>('[aria-label="저장 폴더 열기"]');
      expect(folderButton).not.toBeNull();
      expect(folderButton?.closest(".detail-title-actions")).not.toBeNull();
      await act(async () => {
        folderButton?.click();
        container.querySelector<HTMLButtonElement>('[aria-label="다운로드"]')?.click();
      });
      expect(onOpenDownloadFolder).toHaveBeenCalledWith("detail-folder-entry");
      expect(onQueue).toHaveBeenCalledWith(started.id);

      await act(async () => render({
        ...started,
        download: { ...started.download!, state: "quarantined" },
      }));
      expect(container.querySelector('[aria-label="저장 폴더 열기"]')).toBeNull();

      await act(async () => render({
        ...started,
        download: { ...started.download!, state: "completed", progress: 100 },
      }));
      expect(container.querySelector('[aria-label="다운로드"]')).toBeNull();
      expect(container.querySelector('[aria-label="다운로드 완료"]')).toHaveClass("detail-download-complete");
      expect(container.querySelector('[aria-label="저장 폴더 열기"]')).not.toBeNull();
      expect(container.querySelector(".page-preview-dialog")).toHaveClass("is-resizable");
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("shows only an accessible centered spinner until preview columns are known", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const source = mockGalleries[0]!;
    const pendingGallery: Gallery = { ...source, pages: 18, pageDimensions: undefined };
    const readyGallery: Gallery = {
      ...pendingGallery,
      pageDimensions: Array.from({ length: 8 }, (_, index) => ({
        sourcePage: index + 1,
        width: 720,
        height: 1080,
      })),
    };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (gallery: Gallery) => root.render(
      <DetailWorkspace tabs={[gallery.id]} activeId={gallery.id} minimized={false} galleries={new Map([[gallery.id, gallery]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
    );

    try {
      await act(async () => render(pendingGallery));

      const loading = container.querySelector(".detail-preview-loading");
      expect(loading).toHaveAttribute("role", "status");
      expect(loading).toHaveAttribute("aria-label", "추가 페이지 미리보기 준비 중");
      expect(loading?.querySelector(".spinner")).not.toBeNull();
      expect(container.querySelector(".preview-grid")).toBeNull();
      expect(container.querySelector(".preview-window-nav")).toBeNull();
      expect(container.querySelectorAll(".preview-thumb")).toHaveLength(0);
      expect(container.querySelectorAll('[data-thumbnail-kind="source-page"]')).toHaveLength(0);
      expect(container).not.toHaveTextContent("페이지 정보를 불러오는 중");

      await act(async () => render(readyGallery));

      expect(container.querySelector(".detail-preview-loading")).toBeNull();
      expect(container.querySelector(".preview-grid")).toHaveAttribute("data-preview-columns", "3");
      expect(container.querySelectorAll(".preview-thumb")).toHaveLength(9);
      expect(container.querySelectorAll('[data-thumbnail-kind="source-page"]')).toHaveLength(9);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("renders only the current page window and keeps a zero-page gallery safe", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    const previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value: vi.fn(function (this: HTMLDialogElement) {
        this.setAttribute("open", "");
      }),
    });
    Object.defineProperty(HTMLDialogElement.prototype, "close", {
      configurable: true,
      value: vi.fn(function (this: HTMLDialogElement) {
        this.removeAttribute("open");
      }),
    });
    const previousScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollTo");
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      value: vi.fn(),
    });
    const source = mockGalleries[0]!;
    const gallery: Gallery = { ...source, pages: 99, pageDimensions: Array.from({ length: 99 }, (_, index) => ({ sourcePage: index + 1, width: 720, height: 1080 })) };
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "missing", reason: "test fixture" }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (item: Gallery) => root.render(
      <DetailWorkspace
        tabs={[item.id]}
        activeId={item.id}
        minimized={false}
        galleries={new Map([[item.id, item]])}
        favoriteMetadata={new Set()}
        thumbnailClient={client}
        onActivate={vi.fn()}
        onClose={vi.fn()}
        onCloseAll={vi.fn()}
        onMinimize={vi.fn()}
        onRestore={vi.fn()}
        onOpenRelated={vi.fn()}
        onQueue={vi.fn()}
        onMetadataSearch={vi.fn()}
        onMetadataFavorite={vi.fn()}
      />
    );

    try {
      await act(async () => render(gallery));

      expect(container.querySelectorAll(".preview-thumb").length).toBeGreaterThan(0);
      expect(container.querySelectorAll(".preview-thumb").length).toBeLessThan(gallery.pages);
      expect(container.querySelectorAll('[data-thumbnail-kind="source-page"]')).toHaveLength(container.querySelectorAll(".preview-thumb").length);
      expect(container.querySelector(".preview-grid")).toHaveAttribute("data-preview-columns", "3");
      expect(container.querySelector(".preview-grid")).toHaveAttribute("data-preview-orientation", "portrait");
      expect(container.querySelector(".detail-cover")).toHaveAttribute("data-thumbnail-kind", "gallery-cover");

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".preview-thumb")?.click();
      });
      expect(container.querySelector(".page-preview-dialog")).toHaveAttribute("open");
      expect(container.querySelector("#page-preview-title")).toHaveTextContent("1페이지");
      expect(container.querySelectorAll('[data-thumbnail-kind="source-page"]')).toHaveLength(container.querySelectorAll(".preview-thumb").length + 1);

      await act(async () => render({ ...gallery, pages: 0 }));

      expect(container.querySelectorAll(".preview-thumb")).toHaveLength(0);
      expect(container.querySelectorAll('[data-thumbnail-kind="source-page"]')).toHaveLength(0);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
      if (previousScrollTo) {
        Object.defineProperty(HTMLElement.prototype, "scrollTo", previousScrollTo);
      } else {
        Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
      }
      if (previousShowModal) {
        Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      } else {
        Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
      }
      if (previousClose) {
        Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
      } else {
        Reflect.deleteProperty(HTMLDialogElement.prototype, "close");
      }
    }
  });

  it("moves only the bounded page window with side arrows, A/D, and direct page input", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const resizeCallbacks: Array<() => void> = [];
    class TestResizeObserver {
      constructor(callback: () => void) { resizeCallbacks.push(callback); }
      observe() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    const gallery: Gallery = { ...mockGalleries[0]!, pages: 5000, pageDimensions: Array.from({ length: 8 }, (_, index) => ({ sourcePage: index + 1, width: 720, height: 1080 })) };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    const backgroundControl = document.createElement("button");
    backgroundControl.textContent = "background card";
    document.body.append(container);
    document.body.append(backgroundControl);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <DetailWorkspace tabs={[gallery.id]} activeId={gallery.id} minimized={false} galleries={new Map([[gallery.id, gallery]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
      ));
      expect(container.querySelectorAll(".preview-thumb").length).toBeLessThan(20);
      const nav = container.querySelector(".preview-window-nav");
      expect(nav?.querySelector("button")).toBeNull();
      const input = container.querySelector<HTMLInputElement>('input[aria-label="페이지 번호로 이동"]')!;
      const next = container.querySelector<HTMLButtonElement>('[aria-label="다음 미리보기 묶음"]');
      const previous = container.querySelector<HTMLButtonElement>('[aria-label="이전 미리보기 묶음"]');
      expect(next).toHaveClass("is-next");
      expect(previous).toHaveClass("is-previous");
      expect(previous).toBeDisabled();
      const initialStart = container.querySelector<HTMLButtonElement>(".preview-thumb")?.textContent;
      await act(async () => {
        next?.click();
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")?.textContent).not.toBe(initialStart);
      await act(async () => {
        previous?.click();
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")?.textContent).toBe(initialStart);

      const workspace = container.querySelector<HTMLElement>(".detail-workspace")!;
      await act(async () => {
        workspace.dispatchEvent(new KeyboardEvent("keydown", { key: "d", code: "KeyD", bubbles: true }));
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "10페이지 확대");

      backgroundControl.focus();
      await act(async () => {
        backgroundControl.dispatchEvent(new KeyboardEvent("keydown", { key: "d", code: "KeyD", bubbles: true }));
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "19페이지 확대");

      input.focus();
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "a", code: "KeyA", bubbles: true }));
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "19페이지 확대");

      const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      await act(async () => {
        valueSetter?.call(input, "4999");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "4996페이지 확대");
      expect(input.value).toBe("4996");

      await act(async () => {
        resizeCallbacks.forEach((callback) => callback());
      });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "4996페이지 확대");
      expect(container.querySelectorAll(".preview-thumb").length).toBeLessThan(20);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      backgroundControl.remove();
      container.remove();
    }
  });

  it("locks a landscape preview grid for the active detail tab", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const gallery: Gallery = { ...mockGalleries[0]!, pages: 8, pageDimensions: Array.from({ length: 8 }, (_, index) => ({ sourcePage: index + 1, width: 1600, height: 900 })) };
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "image" as const, url: "https://images.example.test/landscape.jpg", width: 1600, height: 900 }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <DetailWorkspace
          tabs={[gallery.id]}
          activeId={gallery.id}
          minimized={false}
          galleries={new Map([[gallery.id, gallery]])}
          favoriteMetadata={new Set()}
          thumbnailClient={client}
          onActivate={vi.fn()}
          onClose={vi.fn()}
          onCloseAll={vi.fn()}
          onMinimize={vi.fn()}
          onRestore={vi.fn()}
          onOpenRelated={vi.fn()}
          onQueue={vi.fn()}
          onMetadataSearch={vi.fn()}
          onMetadataFavorite={vi.fn()}
        />,
      ));
      expect(container.querySelector(".preview-grid")).toHaveAttribute("data-preview-columns", "2");
      expect(container.querySelector(".preview-grid")).toHaveAttribute("data-preview-orientation", "landscape");
      expect(container.querySelector(".preview-thumb .gallery-thumbnail")).toHaveStyle({ aspectRatio: "1600 / 900" });
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("changes only the dialog page with A/D and leaves the background bundle in place", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value: vi.fn(function (this: HTMLDialogElement) { this.setAttribute("open", ""); }),
    });
    const gallery: Gallery = { ...mockGalleries[0]!, pages: 25, pageDimensions: Array.from({ length: 8 }, (_, index) => ({ sourcePage: index + 1, width: 800, height: 1000 })) };
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "image" as const, url: "https://images.example.test/page.jpg", width: 800, height: 1000 }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <DetailWorkspace tabs={[gallery.id]} activeId={gallery.id} minimized={false} galleries={new Map([[gallery.id, gallery]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
      ));
      await act(async () => container.querySelector<HTMLButtonElement>(".preview-thumb")?.click());
      expect(container.querySelector("#page-preview-title")).toHaveTextContent("1페이지");
      expect(container.querySelector<HTMLImageElement>(".page-preview-media img")?.alt).toContain("1페이지");
      const backgroundStart = container.querySelector<HTMLButtonElement>(".preview-thumb")?.textContent;
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", code: "KeyD", bubbles: true }));
      });
      expect(container.querySelector("#page-preview-title")).toHaveTextContent("2페이지");
      expect(container.querySelector<HTMLImageElement>(".page-preview-media img")?.alt).toContain("2페이지");
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")?.textContent).toBe(backgroundStart);
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "a", code: "KeyA", bubbles: true }));
      });
      expect(container.querySelector("#page-preview-title")).toHaveTextContent("1페이지");
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
    }
  });

  it("cycles Floating Detail tabs with Q/E without hijacking text input", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const galleries = mockGalleries.slice(0, 3).map((gallery) => ({ ...gallery, pageDimensions: [] }));
    const tabs = galleries.map((gallery) => gallery.id);
    const onActivate = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <DetailWorkspace tabs={tabs} activeId={tabs[0]!} minimized={false} galleries={new Map(galleries.map((gallery) => [gallery.id, gallery]))} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={onActivate} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
      ));
      const activeTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')!;
      await act(async () => {
        activeTab.dispatchEvent(new KeyboardEvent("keydown", { key: "e", code: "KeyE", bubbles: true }));
        activeTab.dispatchEvent(new KeyboardEvent("keydown", { key: "q", code: "KeyQ", bubbles: true }));
      });
      expect(onActivate).toHaveBeenNthCalledWith(1, tabs[1]);
      expect(onActivate).toHaveBeenNthCalledWith(2, tabs[2]);

      const input = container.querySelector<HTMLInputElement>('[aria-label="페이지 번호로 이동"]')!;
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "e", code: "KeyE", bubbles: true }));
      });
      expect(onActivate).toHaveBeenCalledTimes(2);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("ignores global A/D while editing, composing, modified, modal, minimized, or closed", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const gallery: Gallery = {
      ...mockGalleries[0]!,
      pages: 30,
      pageDimensions: Array.from({ length: 8 }, (_, index) => ({
        sourcePage: index + 1,
        width: 720,
        height: 1080,
      })),
    };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    const backgroundControl = document.createElement("button");
    const input = document.createElement("input");
    const textarea = document.createElement("textarea");
    const select = document.createElement("select");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    document.body.append(container, backgroundControl, input, textarea, select, editable);
    const root = createRoot(container);
    const render = (minimized: boolean, tabs = [gallery.id]) => root.render(
      <DetailWorkspace tabs={tabs} activeId={tabs[0] ?? null} minimized={minimized} galleries={new Map([[gallery.id, gallery]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
    );
    const press = async (target: HTMLElement, init: KeyboardEventInit) => {
      const event = new KeyboardEvent("keydown", {
        key: "a",
        code: "KeyA",
        bubbles: true,
        cancelable: true,
        ...init,
      });
      await act(async () => {
        target.focus();
        target.dispatchEvent(event);
      });
      return event;
    };

    try {
      await act(async () => render(false));
      await press(backgroundControl, { key: "d", code: "KeyD" });
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "10페이지 확대");

      for (const modifiers of [
        { ctrlKey: true },
        { metaKey: true },
        { altKey: true },
        { shiftKey: true },
        { isComposing: true },
      ]) {
        await press(backgroundControl, modifiers);
      }
      for (const editor of [input, textarea, select, editable]) await press(editor, {});
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "10페이지 확대");

      const modal = document.createElement("dialog");
      modal.setAttribute("open", "");
      document.body.append(modal);
      await press(backgroundControl, {});
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "10페이지 확대");
      modal.remove();

      await act(async () => render(true));
      const minimizedEvent = await press(backgroundControl, {});
      expect(minimizedEvent.defaultPrevented).toBe(false);
      await act(async () => render(false));
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "10페이지 확대");

      await act(async () => render(false, []));
      const closedEvent = await press(backgroundControl, {});
      expect(closedEvent.defaultPrevented).toBe(false);
      await act(async () => render(false));
      expect(container.querySelector<HTMLButtonElement>(".preview-thumb")).toHaveAttribute("title", "1페이지 확대");
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
      backgroundControl.remove();
      input.remove();
      textarea.remove();
      select.remove();
      editable.remove();
      document.querySelector("dialog[open]")?.remove();
    }
  });

  it("uses the card tag order in detail and keeps series and characters out of related galleries", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const parent: Gallery = { ...mockGalleries[0]!, relatedIds: [mockGalleries[6]!.id] };
    const related: Gallery = {
      ...mockGalleries[6]!,
      download: { entryId: "related-complete", state: "completed", progress: 100 },
    };
    const onMetadataSearch = vi.fn();
    const onMetadataFavorite = vi.fn();
    const onOpenRelated = vi.fn();
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "missing", reason: "test fixture" }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DetailWorkspace
          tabs={[parent.id]}
          activeId={parent.id}
          minimized={false}
          galleries={new Map([[parent.id, parent], [related.id, related]])}
          favoriteMetadata={new Set(["series:rain archives", "character:mira lane"])}
          thumbnailClient={client}
          onActivate={vi.fn()}
          onClose={vi.fn()}
          onCloseAll={vi.fn()}
          onMinimize={vi.fn()}
          onRestore={vi.fn()}
          onOpenRelated={onOpenRelated}
          onQueue={vi.fn()}
          onMetadataSearch={onMetadataSearch}
          onMetadataFavorite={onMetadataFavorite}
        />,
      ));

      const mainSeries = container.querySelector<HTMLButtonElement>('[title^="rain archives"]');
      const mainCharacter = container.querySelector<HTMLButtonElement>('[title^="mira lane ·"]');
      expect(mainSeries).toHaveClass("favorite");
      expect(mainCharacter).toHaveClass("favorite");
      expect(container.querySelector(".related-card")?.textContent).not.toContain("rain archives");
      expect(container.querySelector(".related-card")?.textContent).not.toContain("mira lane");

      const relatedTags = [...container.querySelectorAll<HTMLButtonElement>(".related-card .tag")]
        .map((chip) => chip.textContent?.replace(/[★FM]/g, "").trim());
      expect(relatedTags).toEqual(["coat", "suit", "rain", "drama"]);
      expect(container.querySelector(".related-open-command")).toBeNull();
      expect(container.querySelector(".related-card .meta-bottom")).toHaveTextContent(`${related.pages}p`);
      expect(container.querySelector(".related-card .meta-bottom")).toHaveTextContent(`#${related.id}`);
      expect(container.querySelector(".related-card")).toHaveAttribute("tabindex", "0");
      expect(container.querySelector('.related-card [aria-label="다운로드 완료"]')).toHaveClass("download-check");
      expect(container.querySelector('.related-card [data-status-icon="complete"]')).not.toBeNull();

      await act(async () => {
        mainSeries?.click();
        mainCharacter?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
        container.querySelector<HTMLButtonElement>(".related-card .tag")?.click();
        container.querySelector<HTMLButtonElement>(".related-card .byline")?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
      });
      expect(onMetadataSearch).toHaveBeenCalledWith("series:rain_archives");
      expect(onMetadataFavorite).toHaveBeenCalledWith("character:mira lane");
      expect(onOpenRelated).not.toHaveBeenCalled();
      const relatedCard = container.querySelector<HTMLElement>(".related-card");
      await act(async () => {
        relatedCard?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
        relatedCard?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, ctrlKey: true }));
        relatedCard?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, metaKey: true }));
      });
      expect(onOpenRelated).toHaveBeenNthCalledWith(1, related.id, parent.id, { activate: false });
      expect(onOpenRelated).toHaveBeenNthCalledWith(2, related.id, parent.id, { activate: false });
      await act(async () => {
        relatedCard?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
        relatedCard?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      });
      expect(onOpenRelated).toHaveBeenNthCalledWith(3, related.id, parent.id);
      expect(onOpenRelated).toHaveBeenNthCalledWith(4, related.id, parent.id);
      expect(onOpenRelated).toHaveBeenCalledTimes(4);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("uses primary metadata and tags columns, with portrait-only intrinsic related frames", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const parent: Gallery = { ...mockGalleries[0]!, relatedIds: [mockGalleries[6]!.id] };
    const portrait: Gallery = { ...mockGalleries[6]!, thumbnailWidth: 600, thumbnailHeight: 900 };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (related: Gallery) => root.render(
      <DetailWorkspace tabs={[parent.id]} activeId={parent.id} minimized={false} galleries={new Map([[parent.id, parent], [related.id, related]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
    );
    try {
      await act(async () => render(portrait));
      expect(container.querySelector(".detail-metadata-layout")).not.toBeNull();
      expect(container.querySelectorAll(".detail-metadata-primary > .metadata-box")).toHaveLength(5);
      expect(container.querySelector(".detail-metadata-tags")).not.toBeNull();
      expect(container.querySelector(".section-heading > span")).toBeNull();
      expect(container.querySelector<HTMLElement>(".related-card")).toHaveStyle({ "--related-cover-aspect-ratio": "600 / 900" });

      await act(async () => render({ ...portrait, thumbnailWidth: 1200, thumbnailHeight: 600 }));
      expect(container.querySelector<HTMLElement>(".related-card")).toHaveStyle({ "--related-cover-aspect-ratio": "2 / 3" });
      expect(container.querySelector(".related-cover")).toHaveAttribute("data-thumbnail-priority", "prefetch");
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("uses the same translated tag tooltip for Detail and Related cards", async () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
    const parent: Gallery = { ...mockGalleries[0]!, tags: ["female:mind_control"], relatedIds: [mockGalleries[6]!.id] };
    const related: Gallery = { ...mockGalleries[6]!, tags: ["female:mind_control"] };
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <DetailWorkspace tabs={[parent.id]} activeId={parent.id} minimized={false} galleries={new Map([[parent.id, parent], [related.id, related]])} favoriteMetadata={new Set()} thumbnailClient={client} onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()} onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />,
      ));
      const detailTag = container.querySelector<HTMLButtonElement>(".detail-metadata-tags .tag")!;
      const relatedTag = container.querySelector<HTMLButtonElement>(".related-card .tag")!;
      expect(detailTag).toHaveAttribute("data-tag-tooltip-language", "ko");
      expect(relatedTag).toHaveAttribute("data-tag-tooltip-language", "ko");
      await act(async () => detailTag.focus());
      expect(document.body.querySelector("[role='tooltip']")).toHaveTextContent("정신조종");
      await act(async () => detailTag.blur());
      await act(async () => relatedTag.focus());
      expect(document.body.querySelector("[role='tooltip']")).toHaveTextContent("정신조종");
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });
});
