import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BackendClient } from "../api/backend";
import type { DanbooruPost } from "../api/contracts";
import { defaultDanbooruSearchFilters } from "../danbooru/searchPreferences";
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

const backend = {
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
} as unknown as BackendClient;

describe("DanbooruWorkspace", () => {
  beforeEach(() => {
    window.localStorage.removeItem("atsumi.danbooru-state.v1");
    window.localStorage.removeItem("atsumi.danbooru-search-preferences.v1");
    vi.clearAllMocks();
  });

  it("loads real-mode post projections and records an original download", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={backend}
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

  it("measures the post grid before requesting a complete final row", async () => {
    const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get() {
        return this instanceof HTMLElement && this.classList.contains("danbooru-content") ? 620 : 0;
      },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          <DanbooruWorkspace
            backend={backend}
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
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (originalClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", originalClientWidth);
      else delete (HTMLElement.prototype as unknown as { clientWidth?: number }).clientWidth;
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
            backend={backend}
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
          .find((button) => button.textContent === "조건 적용")
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
          .find((option) => option.textContent?.includes("점수 높은순"))
          ?.click();
        await settle();
      });
      expect(backend.danbooruSearch).toHaveBeenLastCalledWith(expect.objectContaining({
        tags: expect.stringContaining("order:score"),
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
      sort: "score",
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
            backend={backend}
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
        tags: "sample_artist rating:g order:score",
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
            backend={backend}
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
            backend={backend}
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
            backend={backend}
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
            backend={backend}
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
            backend={backend}
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
            backend={backend}
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
