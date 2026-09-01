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

const backend = {
  runtime: "browser-mock",
  danbooruSearch: vi.fn(async () => ({ ok: true as const, data: { items: [post], page: 1, hasMore: false } })),
  danbooruRandom: vi.fn(async () => ({ ok: true as const, data: post })),
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
      const sort = container.querySelector<HTMLSelectElement>('[aria-label="Danbooru 정렬 기준"]')!;
      await act(async () => {
        sort.value = "score";
        sort.dispatchEvent(new Event("change", { bubbles: true }));
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
