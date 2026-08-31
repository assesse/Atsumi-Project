import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BackendClient } from "../api/backend";
import type { DanbooruPost } from "../api/contracts";
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
  previewUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
  largeUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
  fileUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E",
  artists: ["sample_artist"],
  copyrights: ["sample_series"],
  characters: ["sample_character"],
  tags: ["blue_sky"],
  hasChildren: false,
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
