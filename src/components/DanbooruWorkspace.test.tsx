import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createDanbooruApi, type DanbooruApi } from "../api/featureClients";
import type { DanbooruPost } from "../api/contracts";
import { DANBOORU_SORTS, defaultDanbooruSearchFilters } from "../danbooru/searchPreferences";
import { DanbooruWorkspace } from "./DanbooruWorkspace";

const settle = (delay = 20) => new Promise((resolve) => window.setTimeout(resolve, delay));

const post: DanbooruPost = {
  id: 12_345_678,
  createdAt: "2026-08-31T00:00:00Z",
  rating: "g",
  score: 321,
  favoriteCount: 45,
  imageWidth: 1200,
  imageHeight: 1800,
  fileExt: "jpg",
  fileSize: 2_048,
  previewUrl: "data:image/svg+xml,%3Csvg data-size='preview'/%3E",
  largeUrl: "data:image/svg+xml,%3Csvg data-size='large'/%3E",
  fileUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
  artists: ["sample_artist"],
  copyrights: ["sample_series"],
  characters: ["sample_character"],
  tags: ["blue_sky"],
  hasChildren: false,
};

const nextPost: DanbooruPost = {
  ...post,
  id: 12_345_679,
  imageWidth: 2400,
  imageHeight: 900,
  previewUrl: "data:image/svg+xml,%3Csvg data-size='next-preview'/%3E",
  largeUrl: "data:image/svg+xml,%3Csvg data-size='next-large'/%3E",
  artists: ["next_artist"],
};

const videoPost: DanbooruPost = {
  ...post,
  id: 12_345_680,
  fileExt: "mp4",
  fileUrl: "https://cdn.donmai.us/original/fixture-video.mp4",
  previewUrl: "data:image/svg+xml,%3Csvg data-size='video-preview'/%3E",
  largeUrl: "data:image/svg+xml,%3Csvg data-size='video-poster'/%3E",
};

const backend: DanbooruApi = {
  runtime: "browser-mock",
  danbooruSearch: vi.fn(async () => ({ ok: true as const, data: { items: [post], page: 1, hasMore: false } })),
  danbooruRandom: vi.fn(async () => ({ ok: true as const, data: post })),
  danbooruRelated: vi.fn(async () => ({
    ok: true as const,
    data: { parent: nextPost, siblings: [], children: [], pools: [] },
  })),
  danbooruAutocomplete: vi.fn(async () => ({ ok: true as const, data: [] })),
  danbooruDownload: vi.fn(async () => ({
    ok: true as const,
    data: { post, fileName: "12345678.jpg", downloadedAt: "1788200000000", bytes: post.fileSize },
  })),
  danbooruDownloadsList: vi.fn(async () => ({
    ok: true as const,
    data: { items: [], page: 1, total: 0, totalPages: 1 },
  })),
};

const danbooruApi = createDanbooruApi(backend);

describe("DanbooruWorkspace", () => {
  beforeEach(() => {
    window.localStorage.removeItem("atsumi.danbooru-state.v1");
    window.localStorage.removeItem("atsumi.danbooru-search-preferences.v1");
    vi.clearAllMocks();
  });

  it.each(DANBOORU_SORTS)("requests the server sort $value from its actual dropdown option", async ({ label, metatag, description }) => {
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Danbooru 정렬 기준"]')!.click());
      await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(option => option.textContent?.includes(label))!.click());
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith({ tags: metatag ?? "", page: 1, pageSize: 50 });
      expect(container.querySelector('[data-post-id="12345678"]')).toBeInTheDocument();
      if (description) expect(container).toHaveTextContent(`인기순: ${description}`);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("preserves loaded cards and the exact sort after an upstream query timeout without retry loops", async () => {
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      vi.mocked(backend.danbooruSearch).mockResolvedValueOnce({ ok: false, error: { code: "DANBOORU_QUERY_TIMEOUT", message: "Danbooru 서버 검색 시간 초과 · 태그나 기간을 좁혀 주세요.", retryable: true } });
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Danbooru 정렬 기준"]')!.click());
      await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(option => option.textContent?.includes('파일 큰순'))!.click());
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith({ tags: "order:filesize", page: 1, pageSize: 50 });
      expect(container.querySelector('[data-post-id="12345678"]')).toBeInTheDocument();
      expect(container.querySelector('[role="alert"]')).toHaveTextContent('태그나 기간');
      expect(container.querySelector('.danbooru-empty')).toBeNull();
      const calls = vi.mocked(backend.danbooruSearch).mock.calls.length;
      await act(async () => settle(100));
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(calls);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("keeps the popularity period on later pages and replaces it when switching sorts", async () => {
    const search = vi.fn<DanbooruApi["danbooruSearch"]>(async request => ({ ok: true, data: { items: [post], page: request.page, hasMore: true } }));
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    const select = async (label: string) => {
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Danbooru 정렬 기준"]')!.click());
      await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(option => option.textContent?.includes(label))!.click());
    };
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={{ ...danbooruApi, danbooruSearch: search }} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      await select("인기순 · 일주일");
      expect(search).toHaveBeenLastCalledWith({ tags: "age:<7days order:score", page: 1, pageSize: 50 });
      await act(async () => [...container.querySelectorAll<HTMLButtonElement>(".danbooru-pager button")].find(button => button.textContent === "다음")!.click());
      expect(search).toHaveBeenLastCalledWith({ tags: "age:<7days order:score", page: 2, pageSize: 50 });
      await select("인기순 · 한 달");
      expect(search).toHaveBeenLastCalledWith({ tags: "age:<30days order:score", page: 1, pageSize: 50 });
      await select("최신 등록순");
      expect(search).toHaveBeenLastCalledWith({ tags: "", page: 1, pageSize: 50 });
      expect(container).not.toHaveTextContent("누적 점수순");
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("opens compact filters as a dismissible overlay without retired relationship options", async () => {
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      const trigger = container.querySelector<HTMLButtonElement>(".danbooru-filter-button")!;
      await act(async () => trigger.click());
      expect(container.querySelector('.danbooru-search-tools > [role="dialog"]')).toHaveAttribute("id", "danbooru-search-filters");
      expect(container).not.toHaveTextContent("게시물 관계");
      expect(container).not.toHaveTextContent("YYYYMMDD 붙여넣기 가능");
      const reset = container.querySelector<HTMLButtonElement>('[aria-label="조건 초기화"]')!;
      expect(reset).toHaveAttribute("title", "조건 초기화");
      expect(reset.querySelector(".fluent")).not.toBeNull();
      expect(container.querySelector(".danbooru-filter-popover footer .primary")).toHaveTextContent(/^적용$/);
      expect(container.querySelectorAll('input[type="date"]')).toHaveLength(2);
      await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(container.querySelector(".danbooru-filter-popover")).toBeNull();
      expect(document.activeElement).toBe(trigger);
      await act(async () => trigger.click());
      await act(async () => container.querySelector(".danbooru-content")!.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(container.querySelector(".danbooru-filter-popover")).toBeNull();
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("removes a retired relationship condition from a restored committed query", async () => {
    window.localStorage.setItem("atsumi.danbooru-state.v1", JSON.stringify({ exploreDraft: "solo", exploreCommitted: "solo child:any", filters: { ...defaultDanbooruSearchFilters(), relationship: "has_children" } }));
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith(expect.objectContaining({ tags: "solo" }));
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("rejects a reversed date range without requesting it or erasing loaded cards", async () => {
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    const props = { backend: danbooruApi, railCollapsed: false, pageSize: 50, previewWidth: 220, onToggleRail: vi.fn(), onSourceChange: vi.fn(), onOpenSettings: vi.fn() };
    const type = async (label: string, value: string) => {
      const input = container.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
      await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
    };
    try {
      await act(async () => root.render(<DanbooruWorkspace {...props} />));
      await act(async () => container.querySelector<HTMLButtonElement>(".danbooru-filter-button")!.click());
      for (const [label, value] of [["시작 연도", "2018"], ["시작 월", "10"], ["시작 일", "08"], ["종료 연도", "2012"], ["종료 월", "10"], ["종료 일", "08"]] as const) await type(label, value);
      const calls = vi.mocked(backend.danbooruSearch).mock.calls.length;
      await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "적용")!.click());
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(calls);
      expect(container.querySelector('[role="alert"]')).toHaveTextContent("종료일은 시작일 이후");
      expect(container.querySelector('[data-post-id="12345678"]')).toBeInTheDocument();
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("keeps a sorted result after idle and ignores an older empty response", async () => {
    let resolveOld: ((result: Awaited<ReturnType<DanbooruApi["danbooruSearch"]>>) => void) | undefined;
    const search = vi.fn<DanbooruApi["danbooruSearch"]>()
      .mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }))
      .mockResolvedValue({ ok: true, data: { items: [post], page: 1, hasMore: false } });
    const api = { ...danbooruApi, danbooruSearch: search };
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    const props = { backend: api, railCollapsed: false, pageSize: 50, previewWidth: 220, onToggleRail: vi.fn(), onSourceChange: vi.fn(), onOpenSettings: vi.fn() };
    try {
      await act(async () => root.render(<DanbooruWorkspace {...props} />));
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Danbooru 정렬 기준"]')!.click());
      await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(option => option.textContent?.includes("파일 큰순"))!.click());
      expect(search).toHaveBeenLastCalledWith({ tags: "order:filesize", page: 1, pageSize: 50 });
      await act(async () => resolveOld!({ ok: true, data: { items: [], page: 1, hasMore: false } }));
      vi.useFakeTimers();
      await act(async () => { vi.advanceTimersByTime(60_000); });
      expect(container.querySelector('[data-post-id="12345678"]')).toBeInTheDocument();
      expect(container).not.toHaveTextContent("검색 결과가 없습니다");
      expect(search).toHaveBeenCalledTimes(2);
    } finally { vi.useRealTimers(); await act(async () => root.unmount()); container.remove(); }
  });

  it("retains results, scroll and detail selection while inactive without loading or handling detail keys", async () => {
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    const props = { backend: danbooruApi, railCollapsed: false, pageSize: 50, previewWidth: 220, onToggleRail: vi.fn(), onSourceChange: vi.fn(), onOpenSettings: vi.fn() };
    try {
      await act(async () => { root.render(<DanbooruWorkspace {...props} />); await settle(); });
      const viewport = container.querySelector<HTMLElement>(".danbooru-content")!;
      await act(async () => { viewport.scrollTop = 480; viewport.dispatchEvent(new Event("scroll")); });
      await act(async () => container.querySelector<HTMLButtonElement>(".danbooru-card-preview")!.click());
      const calls = vi.mocked(backend.danbooruSearch).mock.calls.length;
      await act(async () => root.render(<DanbooruWorkspace {...props} active={false} />));
      expect(container.querySelector(".danbooru-shell")).toHaveAttribute("hidden");
      expect(container.querySelector(".danbooru-detail")).toBeNull();
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      await act(async () => root.render(<DanbooruWorkspace {...props} />));
      expect(viewport.scrollTop).toBe(480);
      expect(container.querySelector(".danbooru-detail")).not.toBeNull();
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(calls);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("returns from an artist search to its loaded result and scroll without refetching", async () => {
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />); await settle(); });
      const viewport = container.querySelector<HTMLElement>(".danbooru-content")!;
      await act(async () => { viewport.scrollTop = 350; viewport.dispatchEvent(new Event("scroll")); });
      await act(async () => container.querySelector<HTMLButtonElement>(".danbooru-card-preview")!.click());
      vi.mocked(backend.danbooruSearch).mockResolvedValueOnce({ ok: true, data: { items: [nextPost], page: 1, hasMore: false } });
      await act(async () => { container.querySelector<HTMLButtonElement>(".danbooru-tag-section button")!.click(); await settle(); });
      expect(container.querySelector(`[data-post-id="${nextPost.id}"]`)).not.toBeNull();
      const calls = vi.mocked(backend.danbooruSearch).mock.calls.length;
      await act(async () => container.querySelector<HTMLButtonElement>(".danbooru-search-history button")!.click());
      expect(container.querySelector(`[data-post-id="${post.id}"]`)).not.toBeNull();
      expect(viewport.scrollTop).toBe(350);
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(calls);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("dismisses download feedback on a timer without relying on CSS animation", async () => {
    vi.useFakeTimers();
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<DanbooruWorkspace backend={danbooruApi} railCollapsed={false} pageSize={50} previewWidth={220} onToggleRail={vi.fn()} onSourceChange={vi.fn()} onOpenSettings={vi.fn()} />));
      await act(async () => container.querySelector<HTMLButtonElement>(".danbooru-save-button")!.click());
      expect(container.querySelector(".toast")).toHaveTextContent("원본을 저장했습니다");
      await act(async () => vi.advanceTimersByTimeAsync(4000));
      expect(container.querySelector(".toast")).toBeNull();
    } finally { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); }
  });

  it("honors community navigation after a source round trip reuses a sequence number", async () => {
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    const props = { backend: danbooruApi, railCollapsed: false, pageSize: 50, previewWidth: 220, onToggleRail: vi.fn(), onSourceChange: vi.fn(), onOpenSettings: vi.fn() };
    try {
      await act(async () => { root.render(<DanbooruWorkspace {...props} />); await settle(); });
      await act(async () => root.render(<DanbooruWorkspace {...props} navigationRequest={{ source: "danbooru", view: "downloads", sequence: 1 }} />));
      expect(container.querySelector("h1")).toHaveTextContent("저장한 Danbooru 원본");
      await act(async () => root.render(<DanbooruWorkspace {...props} active={false} navigationRequest={null} />));
      await act(async () => root.render(<DanbooruWorkspace {...props} navigationRequest={{ source: "danbooru", view: "explore", sequence: 1 }} />));
      expect(container.querySelector("h1")).toHaveTextContent("Danbooru post 탐색");
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(1);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("loads real-mode post projections and records an original download", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      expect(container).toHaveTextContent("Danbooru post 탐색");
      expect(container.querySelector('[data-post-id="12345678"]')).toBeInTheDocument();
      const cardImage = container.querySelector<HTMLImageElement>('[data-post-id="12345678"] img');
      expect(cardImage).toHaveAttribute("src", post.largeUrl);
      expect(cardImage).not.toHaveAttribute("srcset");
      const save = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("원본 저장"));
      await act(async () => {
        save?.click();
        await settle();
      });
      expect(backend.danbooruDownload).toHaveBeenCalledWith(post.id);
      expect(container).toHaveTextContent("저장 완료");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("measures complete rows initially and updates column boundaries without refetching for resize pixels", async () => {
    let contentWidth = 620;
    let resizeCallback: ResizeObserverCallback | undefined;
    const originalResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class implements ResizeObserver {
      constructor(callback: ResizeObserverCallback) { resizeCallback = callback; }
      observe() {}
      unobserve() {}
      disconnect() {}
    };
    const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get() {
        return this instanceof HTMLElement && this.classList.contains("danbooru-content") ? contentWidth : 0;
      },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={190}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      expect(backend.danbooruSearch).toHaveBeenCalledWith({ tags: "", page: 1, pageSize: 51 });
      for (let step = 0; step < 25; step += 1) {
        contentWidth += 1;
        await act(async () => resizeCallback?.([], {} as ResizeObserver));
      }
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(1);
      contentWidth = 1_250;
      await act(async () => resizeCallback?.([], {} as ResizeObserver));
      expect(backend.danbooruSearch).toHaveBeenCalledTimes(1);
      // Crossing to six columns takes effect immediately for the next search;
      // layout is not subject to the expensive tag-fit settling delay.
      await act(async () => container.querySelector<HTMLButtonElement>('button[type="submit"]')?.click());
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith({ tags: "", page: 1, pageSize: 54 });
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (originalClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", originalClientWidth);
      else delete (HTMLElement.prototype as unknown as { clientWidth?: number }).clientWidth;
      globalThis.ResizeObserver = originalResizeObserver;
    }
  });

  it("composes metadata filters and sorting separately from regular tags", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(search, "sample_artist");
        search.dispatchEvent(new Event("input", { bubbles: true }));
        container.querySelector<HTMLButtonElement>(".danbooru-filter-button")?.click();
      });
      const overview = container.querySelector(".danbooru-overview");
      expect(overview).toContainElement(container.querySelector(".danbooru-search-tools"));
      expect(overview).toContainElement(container.querySelector(".danbooru-filter-panel"));
      expect(overview).toContainElement(container.querySelector(".danbooru-heading"));
      expect(overview?.nextElementSibling).toHaveClass("danbooru-content");
      const general = container.querySelector<HTMLInputElement>('.danbooru-filter-panel input[type="checkbox"]');
      await act(async () => general?.click());
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>(".danbooru-filter-panel button")]
          .find((button) => button.textContent === "적용")
          ?.click();
        await settle();
      });
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith(expect.objectContaining({
        tags: expect.stringContaining("sample_artist rating:s,q,e"),
      }));
      const sort = container.querySelector<HTMLButtonElement>('[aria-label="Danbooru 정렬 기준"]')!;
      await act(async () => {
        sort.click();
        await Promise.resolve();
        [...document.body.querySelectorAll<HTMLButtonElement>('[role="option"]')]
          .find((option) => option.textContent?.includes("파일 큰순"))
          ?.click();
        await settle();
      });
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith(expect.objectContaining({
        tags: expect.stringContaining("order:filesize"),
      }));
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("uses Settings defaults for the next search without replacing the restored result query", async () => {
    const previousFilters = {
      ...defaultDanbooruSearchFilters(),
      ratings: ["e"],
    };
    const nextSearchDefaults = {
      ...defaultDanbooruSearchFilters(),
      ratings: ["g"],
      sort: "file_size",
    };
    window.localStorage.setItem("atsumi.danbooru-state.v1", JSON.stringify({
      view: "explore",
      exploreDraft: "sample_artist",
      exploreCommitted: "sample_artist rating:e",
      downloadsDraft: "",
      downloadsCommitted: "",
      explorePage: 1,
      downloadsPage: 1,
      filters: previousFilters,
    }));
    window.localStorage.setItem("atsumi.danbooru-search-preferences.v1", JSON.stringify(nextSearchDefaults));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });

      expect(backend.danbooruSearch).toHaveBeenLastCalledWith({
        tags: "sample_artist rating:e",
        page: 1,
        pageSize: 50,
      });

      await act(async () => {
        container.querySelector<HTMLFormElement>(".danbooru-header form")?.requestSubmit();
        await settle();
      });
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith({
        tags: "sample_artist rating:g order:filesize",
        page: 1,
        pageSize: 50,
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("opens a random post in the detail dialog", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      const random = container.querySelector<HTMLButtonElement>('button[aria-label="랜덤 post 열기"]');
      await act(async () => {
        random?.click();
        await settle();
      });
      const detailDialog = container.querySelector('[role="dialog"]');
      expect(detailDialog).toHaveTextContent("FLOATING DETAIL");
      expect(detailDialog).toHaveTextContent("DANBOORU POST #12345678");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("plays MP4 posts only in Floating Detail and exposes global header controls", async () => {
    vi.mocked(backend.danbooruSearch).mockResolvedValueOnce({
      ok: true,
      data: { items: [videoPost], page: 1, hasMore: false },
    });
    const onActivity = vi.fn();
    const onPrivacyModeToggle = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={60}
            previewWidth={190}
            activityCount={2}
            activityOpen={false}
            privacyMode
            privacyModePending={false}
            onActivity={onActivity}
            onPrivacyModeToggle={onPrivacyModeToggle}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      expect(container.querySelector(".danbooru-card video")).toBeNull();
      expect(container.querySelector('[aria-label="활동 기록"] .activity-count')).toHaveTextContent("2");
      expect(container.querySelector('[aria-label="프라이버시 모드"]')).toHaveAttribute("aria-pressed", "true");
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[aria-label="활동 기록"]')?.click();
        container.querySelector<HTMLButtonElement>('[aria-label="프라이버시 모드"]')?.click();
        container.querySelector<HTMLButtonElement>('[data-post-id="12345680"] .danbooru-card-preview')?.click();
        await settle();
      });
      expect(onActivity).toHaveBeenCalledOnce();
      expect(onPrivacyModeToggle).toHaveBeenCalledOnce();
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveAttribute("src", videoPost.fileUrl);
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveAttribute("poster", videoPost.largeUrl);
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveAttribute("controls");
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveAttribute("autoplay");
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveProperty("muted", true);
      expect(container.querySelector<HTMLVideoElement>(".danbooru-detail-media video")).toHaveAttribute("preload", "auto");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("toggles shared favorites from detail tags with the context menu", async () => {
    const onMetadataFavorite = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            favoriteMetadata={new Set(["artist:sample_artist"])}
            onMetadataFavorite={onMetadataFavorite}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[data-post-id="12345678"] .danbooru-card-preview')?.click();
        await settle();
      });
      const artist = container.querySelector<HTMLButtonElement>('[data-favorite-token="artist:sample_artist"]');
      const tag = container.querySelector<HTMLButtonElement>('[data-favorite-token="blue_sky"]');
      expect(artist).toHaveClass("is-favorite");
      expect(artist).toHaveTextContent("★");
      const contextMenu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      await act(async () => tag?.dispatchEvent(contextMenu));
      expect(contextMenu.defaultPrevented).toBe(true);
      expect(onMetadataFavorite).toHaveBeenCalledWith("blue_sky");

      await act(async () => artist?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
      expect(onMetadataFavorite).toHaveBeenCalledWith("artist:sample_artist");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("shows related posts below tags and keeps navigation inside Floating Detail", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[data-post-id="12345678"] .danbooru-card-preview')?.click();
        await settle();
      });
      expect(backend.danbooruRelated).toHaveBeenCalledWith({ postId: post.id, hasChildren: false });
      expect(container.querySelector(".danbooru-relations")).toHaveTextContent("부모");
      await act(async () => {
        container.querySelector<HTMLButtonElement>('.danbooru-related-card[aria-label*="12345679"]')?.click();
        await settle();
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345679");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("navigates loaded posts from Floating Detail without hijacking text inputs", async () => {
    vi.mocked(backend.danbooruSearch).mockResolvedValueOnce({
      ok: true,
      data: { items: [post, nextPost], page: 1, hasMore: false },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[data-post-id="12345678"] .danbooru-card-preview')?.click();
        await settle();
      });
      const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      search.focus();
      await act(async () => {
        search.dispatchEvent(new KeyboardEvent("keydown", { key: "d", code: "KeyD", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345678");

      container.querySelector<HTMLElement>('[role="dialog"]')?.focus();
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", code: "KeyD", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345679");
      expect(container.querySelector<HTMLImageElement>(".danbooru-detail-media img")).toHaveAttribute("width", "2400");
      expect(container.querySelector<HTMLImageElement>(".danbooru-detail-media img")).toHaveAttribute("height", "900");

      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345678");
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345679");
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "a", code: "KeyA", bubbles: true, cancelable: true }));
      });
      expect(container.querySelector('[role="dialog"]')).toHaveTextContent("DANBOORU POST #12345678");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps Explore and Downloads search text independent", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={danbooruApi}
            railCollapsed={false}
            pageSize={50}
            previewWidth={220}
            onToggleRail={vi.fn()}
            onSourceChange={vi.fn()}
            onOpenSettings={vi.fn()}
          />,
        );
        await settle();
      });
      const exploreSearch = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => {
        exploreSearch.value = "sample_artist blue_sky";
        exploreSearch.dispatchEvent(new Event("input", { bubbles: true }));
        exploreSearch.form?.requestSubmit();
        await settle();
      });
      const downloads = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Downloads"));
      await act(async () => {
        downloads?.click();
        await settle();
      });
      expect(container.querySelector<HTMLInputElement>('input[type="search"]')).toHaveValue("");
      expect(backend.danbooruDownloadsList).toHaveBeenLastCalledWith({ page: 1, pageSize: 50, query: "" });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
