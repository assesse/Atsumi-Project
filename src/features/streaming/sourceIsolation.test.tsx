import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../../App";
import { backend, type BackendEventMap } from "../../api/backend";
import type { ApiResult } from "../../api/contracts";
import { emptyOfficialBrowserSnapshot, type BrowserRecording } from "../../api/officialBrowser";
import { galleryId } from "../../core/types";
import { browserFixtureThumbnailAdapter, ThumbnailClient, ThumbnailProvider } from "../../thumbnail";
import type { OfficialBrowserPanelProps } from "./OfficialBrowserPanel";

const official = vi.hoisted(() => ({ open: vi.fn(), start: vi.fn(), stop: vi.fn(), snapshot: vi.fn(), setViewport: vi.fn(), openFolder: vi.fn(), openSegment: vi.fn(), openMerged: vi.fn(), retryMerge: vi.fn(), connectExtension: vi.fn(), login: vi.fn(), logout: vi.fn(), openInstaller: vi.fn(), confirmControl: vi.fn(), requestControl: vi.fn(), ackUiAction: vi.fn() }));
vi.mock("../../api/officialBrowser", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../api/officialBrowser")>(),
  createOfficialBrowserApi: () => ({ runtime: "tauri", ...official }),
}));
// Keep the gallery backend in its local fixture runtime, but mount the actual
// official panel in desktop mode against the isolated native API mock above.
vi.mock("./OfficialBrowserPanel", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./OfficialBrowserPanel")>();
  return { ...actual, OfficialBrowserPanel: (props: OfficialBrowserPanelProps) => <actual.OfficialBrowserPanel {...props} runtime="tauri" /> };
});
const success = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const recording: BrowserRecording = { id: "official-fixture", channelId: "b".repeat(32), title: "유지되는 공식 녹화", startedAt: 1_800_000_000_000, updatedAt: 1_800_000_015_000, status: "recording", mimeType: "video/webm", outputDir: "C:\\SyntheticRecordings\\official-fixture", segmentCount: 1, bytesWritten: 1024, durationSeconds: 15, lastError: null, segments: [{ index: 0, file: "segment-000000.webm", bytes: 1024, durationSeconds: 15 }] };
const settle = () => new Promise((resolve) => setTimeout(resolve, 30));

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0));
  vi.stubGlobal("cancelAnimationFrame", (timer: ReturnType<typeof setTimeout>) => clearTimeout(timer));
  official.snapshot.mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: true, ready: true, status: "recording", channelId: recording.channelId, recordingId: recording.id, recordings: [recording] }));
  official.setViewport.mockResolvedValue(success(undefined));
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("CHZZK App composition", () => {
  it("keeps recording and gallery work independent across all three modes", async () => {
    const sourceKey = "atsumi.content-source.v1";
    const tutorialKey = "atsumi.tutorial.dismissed.v1";
    const previousSource = localStorage.getItem(sourceKey);
    const previousTutorial = localStorage.getItem(tutorialKey);
    localStorage.setItem(sourceKey, "hitomi");
    localStorage.setItem(tutorialKey, "true");
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue(success({ queryId: "retained-hitomi", firstPage: { page: 1, totalPages: 1, items: [{ id: galleryId(9_123_000), title: "유지되는 갤러리", artist: "fixture artist", pages: 1, language: "korean", tags: [], series: [], characters: [], publishedRank: 20260910, popularity: 1, thumbnailWidth: 512, thumbnailHeight: 768 }] } }));
    const subscriptions = vi.spyOn(backend, "on");
    const cancel = vi.spyOn(backend, "downloadCancel");
    const thumbnails = new ThumbnailClient(browserFixtureThumbnailAdapter);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const switchMode = async (label: string) => {
      await act(async () => container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]')!.click());
      const option = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find((item) => item.querySelector("strong")?.textContent === label)!;
      await act(async () => { option.click(); await settle(); });
    };
    try {
      await act(async () => { root.render(<ThumbnailProvider client={thumbnails}><App /></ThumbnailProvider>); await settle(); });
      await act(async () => { container.querySelector<HTMLButtonElement>('button[type="submit"][aria-label="검색"]')!.click(); await settle(); });
      expect(container.querySelector('[data-gallery-id="9123000"]')).toHaveTextContent("유지되는 갤러리");
      expect(official.snapshot).not.toHaveBeenCalled();
      const initialSubscriptions = subscriptions.mock.calls.length;
      await switchMode("CHZZK");
      expect(container.querySelectorAll(".official-browser-stage")).toHaveLength(1);
      expect(official.snapshot).toHaveBeenCalled();
      expect(container.querySelector(".streaming-connect")).toBeNull();
      expect(container.querySelector(".streaming-current")).toBeNull();
      expect(container.querySelector("video")).toBeNull();
      expect(container.querySelector(".streaming-chat")).toBeNull();
      expect(container.querySelector(".official-browser-recordings")).toBeNull();
      expect(container.querySelector(".gallery-viewport")).toBeNull();
      expect(container.querySelector(".danbooru-workspace")).toBeNull();
      expect(container.querySelector(".main-nav")).not.toHaveTextContent("Auto Find");
      const subscription = subscriptions.mock.calls.find(([name]) => name === "download:changed")!;
      const notifyDownload = subscription[1] as (event: BackendEventMap["download:changed"]) => void;
      await act(async () => { notifyDownload({ entryId: "background-gallery", galleryId: 9_123_000, revision: 10, state: "failed", errorMessage: "background update" }); await settle(); });
      await switchMode("Danbooru");
      expect(container.querySelector(".danbooru-workspace")).not.toBeNull();
      expect(container.querySelector(".streaming-workspace")).toBeNull();
      await switchMode("CHZZK");
      expect(container.querySelectorAll(".official-browser-stage")).toHaveLength(1);
      expect(container.querySelector(".streaming-current")).toBeNull();
      expect(container.querySelector("video")).toBeNull();
      await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="녹화 목록"]')!.click());
      expect(container.querySelector(".recording-library")).toHaveTextContent("유지되는 공식 녹화");
      expect(container.querySelector(".recording-library")).toHaveTextContent("녹화 중");
      expect(container.querySelector(".recording-library")).toHaveTextContent("segment-000000.webm");
      expect(container.querySelector(".official-browser-stage")).toBeNull();
      expect(container.querySelector("video,.streaming-chat")).toBeNull();
      await switchMode("Hitomi");
      expect(container.querySelector('[data-gallery-id="9123000"]')).toHaveTextContent("유지되는 갤러리");
      expect(container.querySelector('[data-gallery-id="9123000"]')).toHaveTextContent("실패");
      expect(search).toHaveBeenCalledTimes(1);
      expect(subscriptions).toHaveBeenCalledTimes(initialSubscriptions);
      expect(cancel).not.toHaveBeenCalled();
      expect(official.open).not.toHaveBeenCalled();
      expect(official.start).not.toHaveBeenCalled();
      expect(official.stop).not.toHaveBeenCalled();
      expect(official.openSegment).not.toHaveBeenCalled();
      expect(official.openFolder).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      thumbnails.dispose();
      container.remove();
      if (previousSource === null) localStorage.removeItem(sourceKey); else localStorage.setItem(sourceKey, previousSource);
      if (previousTutorial === null) localStorage.removeItem(tutorialKey); else localStorage.setItem(tutorialKey, previousTutorial);
    }
  });
});
