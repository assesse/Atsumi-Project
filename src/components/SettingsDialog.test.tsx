import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type {
  ApiResult,
  ExplorationExclusion,
  ExplorationExclusionRestoreResult,
  MaintenanceAction,
  MaintenanceResult,
  SettingsSnapshot,
  StorageUsageSnapshot,
} from "../api/contracts";
import { galleryId, type GalleryId } from "../core/types";
import { SettingsDialog } from "./SettingsDialog";

const settings: SettingsSnapshot = {
  revision: 1,
  downloadRoot: "C:\\Atsumi",
  folderNameTemplate: "[{artist}] {title} [{group}] {id}",
  autoFindHistoryMode: "include_all_history",
  explorePageSize: 50,
  danbooruPageSize: 60,
  downloadOverlapAutoMode: "off",
  maxColumns: 3,
  previewWidth: 220,
  danbooruPreviewWidth: 190,
  relatedPreviewWidth: 240,
  privacyMode: false,
  cacheLimitGb: 5,
  concurrentImageRequests: 5,
  requestStartIntervalMs: 25,
  autoFindGrouping: "all",
  downloadsGrouping: "all",
  exploreDisplayMode: "detail",
  autoFindDisplayMode: "detail",
  downloadsDisplayMode: "detail",
  collapsedGroupKeys: [],
  searchIncludeTags: [],
  searchExcludeTags: [],
};

describe("SettingsDialog operational boundaries", () => {
  it("exposes preset sizing and only safe, implemented reset operations", async () => {
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    const writeText = vi.fn<(value: string) => Promise<void>>(async (_value) => undefined);
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value: vi.fn(function (this: HTMLDialogElement) {
        this.setAttribute("open", "");
      }),
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onSave = vi.fn(async () => false);
    const onPreviewFolderName = vi.fn(async () => ({
      ok: true,
      data: "[작가] 작품 제목 [그룹] 4113714",
    } as const));
    const onMaintenance = vi.fn(async (action: MaintenanceAction): Promise<ApiResult<MaintenanceResult>> => ({
      ok: true,
      data: { action, completedSteps: ["done"], warnings: [], restartRequired: false },
    }));
    const onCheckForUpdates = vi.fn(async () => ({ status: "current" } as const));
    const onTagCatalogRefresh = vi.fn(async () => undefined);
    const onLoadStorageUsage = vi.fn(async (): Promise<ApiResult<StorageUsageSnapshot>> => ({
      ok: true,
      data: {
        memoryCacheBytes: 18 * 1024 * 1024,
        diskCache: { bytes: 6 * 1024 * 1024, exists: true, scanComplete: true, volumeRoot: "C:\\" },
        appData: { bytes: 164 * 1024 * 1024, exists: true, scanComplete: true, volumeRoot: "C:\\" },
        downloads: { bytes: 37 * 1024 * 1024 * 1024, exists: true, scanComplete: true, volumeRoot: "D:\\" },
        volumes: [{
          root: "D:\\",
          totalBytes: 2 * 1024 * 1024 * 1024 * 1024,
          availableBytes: 1_163 * 1024 * 1024 * 1024,
          atsumiBytes: 37 * 1024 * 1024 * 1024,
        }],
        warnings: [],
      },
    }));
    const excludedGalleryId = galleryId(4_113_714);
    const onLoadExplorationExclusions = vi.fn(async (): Promise<ApiResult<ExplorationExclusion[]>> => ({
      ok: true,
      data: [{
        galleryId: excludedGalleryId,
        title: "제외된 작품",
        artist: "작가",
        reasons: [{
          kind: "duplicate_resolved",
          detail: "중복 판정 완료: fixture",
          excludedAt: "2026-08-27T00:00:00Z",
        }],
      }],
    }));
    const onRestoreExplorationExclusions = vi.fn(async (galleryIds: GalleryId[]): Promise<ApiResult<ExplorationExclusionRestoreResult>> => ({
      ok: true,
      data: {
        restoredGalleryIds: galleryIds,
        snapshot: { candidates: [], cutoffEvidence: [], truncations: [] },
      },
    }));

    try {
      await act(async () => root.render(
        <SettingsDialog
          open
          settings={settings}
          loading={false}
          error={null}
          onClose={vi.fn()}
          onSave={onSave}
          onLoadStorageUsage={onLoadStorageUsage}
          onPreviewLayout={vi.fn()}
          onPreviewFolderName={onPreviewFolderName}
          onMaintenance={onMaintenance}
          onCheckForUpdates={onCheckForUpdates}
          onTagCatalogRefresh={onTagCatalogRefresh}
          tagCatalogStatus={{ revision: 3, entryCount: 10_000, neutralCount: 4_000, femaleCount: 3_000, maleCount: 1_000, artistCount: 1_500, groupCount: 500 }}
          tagCatalogRefreshing={false}
          onLoadExplorationExclusions={onLoadExplorationExclusions}
          onRestoreExplorationExclusions={onRestoreExplorationExclusions}
        />,
      ));
      await act(async () => {
        await new Promise((resolve) => window.setTimeout(resolve, 140));
      });

      expect(container.querySelector(".settings-nav")).not.toBeNull();
      expect([...container.querySelectorAll('[role="tab"]')].map((tab) => tab.textContent)).toEqual(["일반", "Hitomi", "Danbooru"]);
      expect(container.textContent).not.toContain("다음 단계");
      expect(container.querySelectorAll('[data-settings-scroll-root="true"]')).toHaveLength(1);
      expect(container.querySelector(".settings-dialog > .settings-form")).not.toBeNull();
      const storage = container.querySelector<HTMLElement>(".storage-usage-panel");
      expect(onLoadStorageUsage).toHaveBeenCalledTimes(1);
      expect(storage).toHaveTextContent("메모리 미리보기 캐시");
      expect(storage).toHaveTextContent("18 MB");
      expect(storage).toHaveTextContent("다운로드 폴더");
      expect(storage).toHaveTextContent("37 GB");
      expect(storage).toHaveTextContent("D:\\ 디스크");
      expect(storage?.querySelector(".storage-volume-meter")).toHaveAttribute(
        "aria-label",
        expect.stringContaining("Atsumi 관리 경로 37 GB"),
      );
      await act(async () => {
        [...storage?.querySelectorAll<HTMLButtonElement>("button") ?? []]
          .find((button) => button.textContent === "새로고침")
          ?.click();
        await Promise.resolve();
      });
      expect(onLoadStorageUsage).toHaveBeenCalledTimes(2);
      const about = container.querySelector<HTMLElement>(".settings-about-panel");
      expect(about).toHaveTextContent("Atsumi");
      expect(about).toHaveTextContent("assesse · Atsumi contributors");
      expect(about).toHaveTextContent("앨범 제목, 태그, 파일 경로, 데이터베이스 내용이 포함되지 않습니다");
      const aboutButtons = [...about?.querySelectorAll<HTMLButtonElement>("button") ?? []];
      await act(async () => aboutButtons.find((button) => button.textContent === "업데이트 확인")?.click());
      expect(onCheckForUpdates).toHaveBeenCalledOnce();
      expect(about).toHaveTextContent("현재 최신 버전입니다.");
      await act(async () => aboutButtons.find((button) => button.textContent === "피드백 주소 복사")?.click());
      expect(writeText).toHaveBeenLastCalledWith("https://github.com/assesse/Atsumi-Project/issues/new/choose");
      await act(async () => aboutButtons.find((button) => button.textContent === "진단 정보 복사")?.click());
      expect(writeText).toHaveBeenLastCalledWith(expect.stringContaining("privateDataIncluded=false"));
      expect(writeText.mock.calls.at(-1)?.[0]).not.toMatch(/C:\\|앨범|tag=/i);

      expect(container.querySelector(".settings-reset-row")).not.toBeNull();
      expect(container.querySelector(".maintenance-panel .settings-reset-row")).toBeNull();
      const maintenanceItems = [...container.querySelectorAll<HTMLElement>(".maintenance-item")];
      expect(maintenanceItems).toHaveLength(3);
      const [quickRepair, rebuild, factoryReset] = maintenanceItems;
      expect(quickRepair).toHaveTextContent("빠른 복구");
      expect(quickRepair?.querySelectorAll("button")).toHaveLength(1);
      expect(rebuild).toHaveTextContent("라이브러리 검사 및 재구축");
      expect(rebuild?.querySelectorAll('input[type="checkbox"]')).toHaveLength(4);
      expect([...rebuild?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? []].map((input) => input.checked)).toEqual([true, false, false, false]);
      expect(factoryReset).toHaveTextContent("앱 데이터 완전 초기화");
      expect(factoryReset).toHaveTextContent("외부 다운로드 원본 파일과 quarantine/recovery 파일은 유지됩니다.");
      expect(factoryReset).toHaveClass("maintenance-item--factory-reset");
      const maintenance = [...container.querySelectorAll<HTMLButtonElement>(".maintenance-item > button")];
      expect(maintenance.map((button) => button.textContent)).toEqual(["빠른 복구", "라이브러리 검사 및 재구축", "앱 데이터 완전 초기화"]);
      await act(async () => maintenance[0]?.click());
      expect(onMaintenance).toHaveBeenCalledWith({ kind: "quickRepair" });
      const duplicateOption = rebuild?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')[1];
      await act(async () => duplicateOption?.click());
      await act(async () => maintenance[1]?.click());
      expect(onMaintenance).toHaveBeenLastCalledWith({
        kind: "rebuildLibrary",
        rebuildThumbnailData: true,
        rebuildDuplicateAnalysis: true,
        rebuildInternalAnalysis: false,
        rebuildAutoFindResults: false,
      });
      await act(async () => maintenance[2]?.click());
      expect(confirm).toHaveBeenCalledWith(expect.stringContaining("외부 다운로드 원본 파일은 유지"));

      const template = container.querySelector<HTMLInputElement>('[aria-label="갤러리 폴더 이름 템플릿"]');
      expect(template?.value).toBe("[{artist}] {title} [{group}] {id}");
      const historyMode = container.querySelector<HTMLSelectElement>('[aria-label="Auto Find 기록 기준"]');
      expect(historyMode?.value).toBe("include_all_history");
      expect(historyMode?.closest(".settings-select-control")?.querySelector(".fluent")).not.toBeNull();
      const explorePageSize = container.querySelector<HTMLInputElement>('[aria-label="Hitomi 페이지당 앨범 수"]');
      expect(explorePageSize?.min).toBe("10");
      expect(explorePageSize?.max).toBe("200");
      expect(explorePageSize?.step).toBe("10");
      expect(explorePageSize?.value).toBe("50");
      await act(async () => {
        if (!explorePageSize) throw new Error("Explore page size input missing");
        const setRangeValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setRangeValue?.call(explorePageSize, "80");
        explorePageSize.dispatchEvent(new Event("input", { bubbles: true }));
        explorePageSize.dispatchEvent(new Event("change", { bubbles: true }));
      });
      const previewRange = container.querySelector<HTMLInputElement>('[aria-label="Hitomi 카드 미리보기 크기"]');
      expect(previewRange?.min).toBe("0");
      expect(previewRange?.max).toBe("6");
      expect(previewRange?.value).toBe("2");
      const relatedPreviewRange = container.querySelector<HTMLInputElement>('[aria-label="Related galleries 미리보기 크기"]');
      expect(relatedPreviewRange?.min).toBe("180");
      expect(relatedPreviewRange?.max).toBe("320");
      expect(relatedPreviewRange?.value).toBe("240");
      const privacyMode = container.querySelector<HTMLInputElement>('[aria-label="프라이버시 모드"]');
      expect(privacyMode).not.toBeChecked();
      await act(async () => privacyMode?.click());
      expect(privacyMode).toBeChecked();
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent === "설정 기본값")
          ?.click();
      });
      expect(privacyMode).not.toBeChecked();
      expect(container.textContent).toContain("사용가능 인자 : {artist}, {title}, {group}, {id}");
      expect(container.textContent).toContain("미리보기 : [작가] 작품 제목 [그룹] 4113714");
      expect(container.textContent).not.toContain("{id}는 필수입니다");
      expect(container.textContent).not.toContain("사용할 수 있으며");
      expect(onPreviewFolderName).toHaveBeenCalledTimes(1);
      await act(async () => {
        if (!template) throw new Error("template input missing");
        template.value = "{title} {id}";
        template.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent === "기본값 복원")
          ?.click();
      });
      await act(async () => {
        privacyMode?.click();
      });
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
          .find((button) => button.textContent === "Hitomi")
          ?.click();
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(onLoadExplorationExclusions).toHaveBeenCalledTimes(1);
      expect(container.textContent).toContain("무검열 표식이 확인되면 그 판본을 우선");
      expect(container.textContent).toContain("더 큰 판본이 검열판이고 작은 판본이 무검열판인 충돌");
      expect(container.textContent).not.toContain("무검열 표식을 우선하며, 표식이 충돌");
      const searchCatalog = container.querySelector<HTMLElement>(".search-catalog-panel");
      expect(searchCatalog).toHaveTextContent("검색어 자동완성 데이터");
      expect(searchCatalog).toHaveTextContent("10,000개 항목 저장됨");
      expect(searchCatalog).toHaveTextContent("작가 1,500 · 그룹 500");
      await act(async () => {
        [...searchCatalog?.querySelectorAll<HTMLButtonElement>("button") ?? []]
          .find((button) => button.textContent?.includes("지금 최신화"))
          ?.click();
        await Promise.resolve();
      });
      expect(onTagCatalogRefresh).toHaveBeenCalledOnce();
      const includeTags = container.querySelector<HTMLTextAreaElement>('[aria-label="모든 검색 필수 포함 태그"]');
      const excludeTags = container.querySelector<HTMLTextAreaElement>('[aria-label="모든 검색 제외 태그"]');
      await act(async () => {
        if (!includeTags || !excludeTags) throw new Error("global search tag inputs missing");
        const setTextareaValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
        setTextareaValue?.call(includeTags, "Female:Glasses\nwebtoon");
        includeTags.dispatchEvent(new Event("input", { bubbles: true }));
        setTextareaValue?.call(excludeTags, "male:glasses");
        excludeTags.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(container.textContent).toContain("제외된 작품");
      expect(container.textContent).toContain("중복 판정");
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent === "제외/숨김 해제")
          ?.click();
        await Promise.resolve();
      });
      expect(onRestoreExplorationExclusions).toHaveBeenCalledWith([excludedGalleryId]);
      expect(container.textContent).not.toContain("제외된 작품");
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
          .find((button) => button.textContent === "Danbooru")
          ?.click();
      });
      const danbooruSettings = container.querySelector<HTMLElement>(".danbooru-settings-panel");
      expect(danbooruSettings).toBeVisible();
      expect(danbooruSettings).toHaveTextContent("4종: General(g), Sensitive(s), Questionable(q), Explicit(e)");
      expect(danbooruSettings).toHaveTextContent("status rating limit is id date age filesize filetype");
      expect(danbooruSettings?.querySelectorAll('.danbooru-settings-checks:not(.is-files) input[type="checkbox"]')).toHaveLength(4);
      const danbooruSort = danbooruSettings?.querySelector<HTMLButtonElement>('[aria-label="Danbooru 기본 정렬"]');
      expect(danbooruSort).toHaveTextContent("최신 등록순");
      expect(danbooruSort).toHaveAttribute("aria-expanded", "false");
      const danbooruPageSize = danbooruSettings?.querySelector<HTMLInputElement>('[aria-label="Danbooru 페이지당 post 수"]');
      const danbooruPreviewWidth = danbooruSettings?.querySelector<HTMLInputElement>('[aria-label="Danbooru 카드 미리보기 크기"]');
      expect(danbooruPageSize).toHaveValue("60");
      expect(danbooruPreviewWidth).toHaveValue("1");
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent === "저장")
          ?.click();
      });
      expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
        folderNameTemplate: "[{artist}] {title} [{group}] {id}",
        autoFindHistoryMode: "include_all_history",
        explorePageSize: 50,
        danbooruPageSize: 60,
        danbooruPreviewWidth: 190,
        relatedPreviewWidth: 240,
        privacyMode: true,
        searchIncludeTags: ["female:glasses", "webtoon"],
        searchExcludeTags: ["male:glasses"],
      }));
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) {
        Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      } else {
        Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
      }
      if (previousClipboard) {
        Object.defineProperty(navigator, "clipboard", previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, "clipboard");
      }
      confirm.mockRestore();
    }
  });

  it("keeps autocomplete refresh status and busy state inside Search management", async () => {
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value() { this.setAttribute("open", ""); },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <SettingsDialog
          open
          settings={settings}
          loading={false}
          error={null}
          onClose={vi.fn()}
          onSave={vi.fn(async () => false)}
          onLoadStorageUsage={vi.fn(async () => ({
            ok: true as const,
            data: {
              memoryCacheBytes: 0,
              diskCache: { bytes: 0, exists: false, scanComplete: true },
              appData: { bytes: 0, exists: false, scanComplete: true },
              downloads: { bytes: 0, exists: false, scanComplete: true },
              volumes: [],
              warnings: [],
            },
          }))}
          onPreviewLayout={vi.fn()}
          onPreviewFolderName={vi.fn(async () => ({ ok: true as const, data: "preview" }))}
          onMaintenance={vi.fn()}
          onCheckForUpdates={vi.fn()}
          onTagCatalogRefresh={vi.fn()}
          tagCatalogStatus={{ revision: 2, entryCount: 0, neutralCount: 0, femaleCount: 0, maleCount: 0, artistCount: 0, groupCount: 0 }}
          tagCatalogRefreshing
          onLoadExplorationExclusions={vi.fn(async () => ({ ok: true as const, data: [] }))}
          onRestoreExplorationExclusions={vi.fn()}
        />,
      ));
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
          .find((button) => button.textContent === "Hitomi")
          ?.click();
        await Promise.resolve();
      });
      const refresh = [...container.querySelectorAll<HTMLButtonElement>(".search-catalog-panel button")]
        .find((button) => button.textContent?.includes("최신화 중"));
      expect(refresh).toBeDisabled();
      expect(refresh).toHaveAttribute("aria-busy", "true");
      expect(refresh?.querySelector(".catalog-refresh-spinner")).not.toBeNull();
      expect(container.querySelector(".search-catalog-status")).toHaveTextContent("저장된 자동완성 데이터 없음");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
    }
  });
});
