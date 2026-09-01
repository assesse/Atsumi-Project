import { useCallback, useEffect, useRef, useState } from "react";
import packageMetadata from "../../package.json";
import type {
  ApiError,
  ApiResult,
  ExplorationExclusion,
  ExplorationExclusionRestoreResult,
  MaintenanceAction,
  MaintenanceResult,
  SettingsPatch,
  SettingsSnapshot,
  StorageUsageSnapshot,
  TagCatalogStatus,
} from "../api/contracts";
import type { GalleryId } from "../core/types";
import {
  DANBOORU_FILE_TYPES,
  DANBOORU_RATINGS,
  DANBOORU_SORTS,
  defaultDanbooruSearchFilters,
  loadDanbooruSearchPreferences,
  saveDanbooruSearchPreferences,
  type DanbooruFileType,
  type DanbooruRating,
  type DanbooruSearchFilters,
} from "../danbooru/searchPreferences";
import {
  GALLERY_PREVIEW_PRESETS,
  galleryPreviewPresetIndex,
} from "../layout/galleryPreviewPresets";
import { parseGlobalSearchTagInput } from "../search/globalSearchRules";
import type { AppUpdateCheckResult } from "../update/useAppUpdater";
import { FluentIcon } from "./FluentIcon";
import { DropdownSelect } from "./DropdownSelect";

type SettingsDialogProps = {
  open: boolean;
  settings: SettingsSnapshot;
  loading: boolean;
  error: ApiError | null;
  onClose: () => void;
  onSave: (patch: SettingsPatch) => Promise<boolean>;
  onLoadStorageUsage: () => Promise<ApiResult<StorageUsageSnapshot>>;
  onPreviewLayout: (layout: { maxColumns: number; previewWidth: number } | null) => void;
  onPreviewFolderName: (template: string) => Promise<ApiResult<string>>;
  onMaintenance: (action: MaintenanceAction) => Promise<ApiResult<MaintenanceResult>>;
  onCheckForUpdates: () => Promise<AppUpdateCheckResult>;
  onTagCatalogRefresh: () => Promise<void>;
  tagCatalogStatus?: TagCatalogStatus;
  tagCatalogRefreshing: boolean;
  onLoadExplorationExclusions: () => Promise<ApiResult<ExplorationExclusion[]>>;
  onRestoreExplorationExclusions: (galleryIds: GalleryId[]) => Promise<ApiResult<ExplorationExclusionRestoreResult>>;
};

const DEFAULT_FOLDER_NAME_TEMPLATE = "[{artist}] {title} [{group}] {id}";
const PROJECT_URL = "https://github.com/assesse/Atsumi-Project";
const FEEDBACK_URL = `${PROJECT_URL}/issues/new/choose`;
const exclusionReasonLabel = (kind: ExplorationExclusion["reasons"][number]["kind"]): string => ({
  manual: "직접 제외",
  duplicate_hidden: "중복 숨김",
  duplicate_resolved: "중복 판정",
  duplicate_pair: "중복 아님 쌍",
})[kind];

const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / (1024 ** unit);
  return `${value.toLocaleString("ko-KR", {
    maximumFractionDigits: value >= 100 || unit === 0 ? 0 : value >= 10 ? 1 : 2,
  })} ${units[unit]}`;
};

const storagePercent = (part: number, total: number): number => (
  total > 0 ? Math.min(100, Math.max(0, (part / total) * 100)) : 0
);

const copyText = async (value: string) => {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) throw new Error("clipboard unavailable");
};

export function SettingsDialog({
  open,
  settings,
  loading,
  error,
  onClose,
  onSave,
  onLoadStorageUsage,
  onPreviewLayout,
  onPreviewFolderName,
  onMaintenance,
  onCheckForUpdates,
  onTagCatalogRefresh,
  tagCatalogStatus,
  tagCatalogRefreshing,
  onLoadExplorationExclusions,
  onRestoreExplorationExclusions,
}: SettingsDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const closingInternally = useRef(false);
  const wasOpen = useRef(false);
  const [draft, setDraft] = useState<SettingsSnapshot>(settings);
  const [saving, setSaving] = useState(false);
  const [folderPreview, setFolderPreview] = useState("");
  const [folderPreviewError, setFolderPreviewError] = useState("");
  const folderPreviewRequest = useRef(0);
  const [maintenanceBusy, setMaintenanceBusy] = useState<MaintenanceAction["kind"] | null>(null);
  const [maintenanceMessage, setMaintenanceMessage] = useState("");
  const [informationMessage, setInformationMessage] = useState("");
  const [updateCheckBusy, setUpdateCheckBusy] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");
  const [rebuildOptions, setRebuildOptions] = useState({ thumbnail: true, duplicate: false, internal: false, autoFind: false });
  const [activeTab, setActiveTab] = useState<"general" | "hitomi" | "danbooru">("general");
  const [danbooruDraft, setDanbooruDraft] = useState<DanbooruSearchFilters>(defaultDanbooruSearchFilters);
  const [exclusions, setExclusions] = useState<ExplorationExclusion[]>([]);
  const [exclusionsLoading, setExclusionsLoading] = useState(false);
  const [exclusionsError, setExclusionsError] = useState("");
  const [selectedExclusionIds, setSelectedExclusionIds] = useState<Set<GalleryId>>(new Set());
  const [restoringExclusions, setRestoringExclusions] = useState(false);
  const [includeTagInput, setIncludeTagInput] = useState(settings.searchIncludeTags.join("\n"));
  const [excludeTagInput, setExcludeTagInput] = useState(settings.searchExcludeTags.join("\n"));
  const [storageUsage, setStorageUsage] = useState<StorageUsageSnapshot | null>(null);
  const [storageUsageLoading, setStorageUsageLoading] = useState(false);
  const [storageUsageError, setStorageUsageError] = useState("");
  const storageUsageRequest = useRef(0);
  const storageUsageLoadingRef = useRef(false);
  const storageUsagePath = useRef<string | null>(null);
  const storageUsageLoadedAt = useRef(0);

  useEffect(() => {
    if (open && !wasOpen.current) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setDraft(settings);
      setMaintenanceMessage("");
      setInformationMessage("");
      setUpdateMessage("");
      setActiveTab("general");
      setDanbooruDraft(loadDanbooruSearchPreferences());
      setExclusions([]);
      setExclusionsError("");
      setSelectedExclusionIds(new Set());
      setIncludeTagInput(settings.searchIncludeTags.join("\n"));
      setExcludeTagInput(settings.searchExcludeTags.join("\n"));
      onPreviewLayout({ maxColumns: settings.maxColumns, previewWidth: settings.previewWidth });
      if (!dialog.current?.open) dialog.current?.showModal();
      window.requestAnimationFrame(() => closeButton.current?.focus());
    } else if (!open && wasOpen.current && dialog.current?.open) {
      closingInternally.current = true;
      dialog.current.close();
      onPreviewLayout(null);
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else document.querySelector<HTMLElement>('[aria-label="설정"]')?.focus();
      });
    }
    wasOpen.current = open;
  }, [open, onPreviewLayout, settings]);

  const loadStorageUsage = useCallback((force = false) => {
    if (storageUsageLoadingRef.current) return;
    const requestedPath = settings.downloadRoot.trim();
    const fresh = storageUsagePath.current === requestedPath
      && Date.now() - storageUsageLoadedAt.current < 30_000;
    if (!force && fresh) return;
    const request = ++storageUsageRequest.current;
    storageUsageLoadingRef.current = true;
    setStorageUsageLoading(true);
    setStorageUsageError("");
    if (storageUsagePath.current !== requestedPath) setStorageUsage(null);
    void onLoadStorageUsage().then((result) => {
      if (storageUsageRequest.current !== request) return;
      if (result.ok) {
        setStorageUsage(result.data);
        storageUsagePath.current = requestedPath;
        storageUsageLoadedAt.current = Date.now();
      } else {
        setStorageUsageError(result.error.message);
      }
      storageUsageLoadingRef.current = false;
      setStorageUsageLoading(false);
    }).catch(() => {
      if (storageUsageRequest.current !== request) return;
      setStorageUsageError("저장공간 사용량을 불러오지 못했습니다.");
      storageUsageLoadingRef.current = false;
      setStorageUsageLoading(false);
    });
  }, [onLoadStorageUsage, settings.downloadRoot]);

  useEffect(() => {
    if (!open) return;
    loadStorageUsage();
  }, [loadStorageUsage, open]);

  useEffect(() => {
    if (!open) return undefined;
    const request = ++folderPreviewRequest.current;
    const timer = window.setTimeout(() => {
      void onPreviewFolderName(draft.folderNameTemplate).then((result) => {
        if (folderPreviewRequest.current !== request) return;
        if (result.ok) {
          setFolderPreview(result.data);
          setFolderPreviewError("");
        } else {
          setFolderPreview("");
          setFolderPreviewError(result.error.message);
        }
      }).catch(() => {
        if (folderPreviewRequest.current !== request) return;
        setFolderPreview("");
        setFolderPreviewError("미리보기를 만들 수 없습니다.");
      });
    }, 125);
    return () => window.clearTimeout(timer);
  }, [draft.folderNameTemplate, onPreviewFolderName, open]);

  useEffect(() => {
    if (!open || activeTab !== "hitomi") return undefined;
    let cancelled = false;
    setExclusionsLoading(true);
    setExclusionsError("");
    void onLoadExplorationExclusions().then((result) => {
      if (cancelled) return;
      if (result.ok) setExclusions(result.data);
      else setExclusionsError(result.error.message);
      setExclusionsLoading(false);
    }).catch(() => {
      if (cancelled) return;
      setExclusionsError("탐색 제외 앨범을 불러오지 못했습니다.");
      setExclusionsLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [activeTab, onLoadExplorationExclusions, open]);

  const patch = <K extends keyof SettingsSnapshot>(key: K, value: SettingsSnapshot[K]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const previewLayout = (maxColumns: number, previewWidth: number) => {
    onPreviewLayout({ maxColumns, previewWidth });
  };

  const close = () => {
    onPreviewLayout(null);
    onClose();
  };

  const restorePreferenceDefaults = () => {
    const maxColumns = 3;
    const previewWidth = 220;
    setDraft((current) => ({
      ...current,
      autoFindHistoryMode: "include_all_history",
      downloadOverlapAutoMode: "off",
      explorePageSize: 50,
      danbooruPageSize: 60,
      maxColumns,
      previewWidth,
      danbooruPreviewWidth: 190,
      relatedPreviewWidth: 240,
      privacyMode: false,
      concurrentImageRequests: 5,
      requestStartIntervalMs: 25,
    }));
    previewLayout(maxColumns, previewWidth);
    setMaintenanceMessage("화면·네트워크 설정을 기본값으로 되돌렸습니다. 저장을 눌러 적용하세요.");
  };

  const runMaintenance = async (action: MaintenanceAction) => {
    if (action.kind === "factoryReset" && !window.confirm("앱 데이터 전체를 초기화하고 앱을 다시 시작할까요? 외부 다운로드 원본 파일은 유지됩니다.")) return;
    setMaintenanceBusy(action.kind);
    const result = await onMaintenance(action);
    setMaintenanceBusy(null);
    setMaintenanceMessage(result.ok ? result.data.completedSteps.join(" · ") : result.error.message);
  };

  const copyInformation = async (kind: "feedback" | "diagnostics") => {
    const value = kind === "feedback"
      ? FEEDBACK_URL
      : [
          "Atsumi diagnostic summary",
          `version=${packageMetadata.version}`,
          `runtime=${"__TAURI_INTERNALS__" in window ? "desktop" : "browser-preview"}`,
          `project=${PROJECT_URL}`,
          "privateDataIncluded=false",
        ].join("\n");
    try {
      await copyText(value);
      setInformationMessage(kind === "feedback" ? "피드백 접수 주소를 복사했습니다." : "개인정보가 제외된 진단 정보를 복사했습니다.");
    } catch {
      setInformationMessage("클립보드에 복사하지 못했습니다.");
    }
  };

  const checkForUpdates = async () => {
    if (updateCheckBusy) return;
    setUpdateCheckBusy(true);
    setUpdateMessage("");
    try {
      const result = await onCheckForUpdates();
      if (result.status === "available") {
        setUpdateMessage(`v${result.info.version} 업데이트를 확인했습니다. 업데이트 창에서 설치할 수 있습니다.`);
      } else if (result.status === "current") {
        setUpdateMessage("현재 최신 버전입니다.");
      } else if (result.status === "unavailable") {
        setUpdateMessage("설치형 데스크톱 앱에서 업데이트를 확인할 수 있습니다.");
      } else {
        setUpdateMessage(result.message);
      }
    } catch {
      setUpdateMessage("업데이트 정보를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.");
    } finally {
      setUpdateCheckBusy(false);
    }
  };

  const restoreExclusions = async (galleryIds: GalleryId[]) => {
    if (!galleryIds.length) return;
    setRestoringExclusions(true);
    setExclusionsError("");
    try {
      const result = await onRestoreExplorationExclusions(galleryIds);
      if (result.ok) {
        const restored = new Set(result.data.restoredGalleryIds);
        setExclusions((current) => current.filter((item) => !restored.has(item.galleryId)));
        setSelectedExclusionIds((current) => new Set([...current].filter((id) => !restored.has(id))));
      } else {
        setExclusionsError(result.error.message);
      }
    } catch {
      setExclusionsError("선택한 앨범의 제외 또는 숨김을 해제하지 못했습니다.");
    } finally {
      setRestoringExclusions(false);
    }
  };

  const overlappingSearchTags = draft.searchIncludeTags.filter((tag) =>
    draft.searchExcludeTags.includes(tag));
  const tagCatalogIncomplete = !tagCatalogStatus?.entryCount
    || tagCatalogStatus.artistCount === 0
    || tagCatalogStatus.groupCount === 0;

  const save = async () => {
    setSaving(true);
    const success = await onSave({
      downloadRoot: draft.downloadRoot,
      folderNameTemplate: draft.folderNameTemplate,
      autoFindHistoryMode: draft.autoFindHistoryMode,
      downloadOverlapAutoMode: draft.downloadOverlapAutoMode,
      explorePageSize: draft.explorePageSize,
      danbooruPageSize: draft.danbooruPageSize,
      maxColumns: draft.maxColumns,
      previewWidth: draft.previewWidth,
      danbooruPreviewWidth: draft.danbooruPreviewWidth,
      relatedPreviewWidth: draft.relatedPreviewWidth,
      privacyMode: draft.privacyMode,
      cacheLimitGb: draft.cacheLimitGb,
      concurrentImageRequests: draft.concurrentImageRequests,
      requestStartIntervalMs: draft.requestStartIntervalMs,
      searchIncludeTags: draft.searchIncludeTags,
      searchExcludeTags: draft.searchExcludeTags,
    });
    setSaving(false);
    if (success) {
      saveDanbooruSearchPreferences(danbooruDraft);
      close();
    }
  };

  return (
    <dialog
      className="settings-dialog"
      ref={dialog}
      aria-labelledby="settings-dialog-title"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onClose={() => {
        if (closingInternally.current) {
          closingInternally.current = false;
          return;
        }
        onClose();
      }}
    >
      <div className="settings-form">
        <header className="dialog-header">
          <div>
            <span className="eyebrow">SETTINGS</span>
            <h2 id="settings-dialog-title">설정</h2>
          </div>
          <div className="dialog-header-actions">
            <button type="button" className="text-button primary" disabled={loading || saving || overlappingSearchTags.length > 0} onClick={() => void save()}>
              {saving ? "저장 중" : "저장"}
            </button>
            <button ref={closeButton} type="button" className="icon-button small" title="닫기" aria-label="닫기" onClick={close}>
              <FluentIcon glyph="\uE711" />
            </button>
          </div>
        </header>
        <nav className="settings-nav" role="tablist" aria-label="설정 분류">
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "general"}
            className={activeTab === "general" ? "is-active" : ""}
            onClick={() => setActiveTab("general")}
          >일반</button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "hitomi"}
            className={activeTab === "hitomi" ? "is-active" : ""}
            onClick={() => setActiveTab("hitomi")}
          >Hitomi</button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "danbooru"}
            className={activeTab === "danbooru" ? "is-active" : ""}
            onClick={() => setActiveTab("danbooru")}
          >Danbooru</button>
        </nav>
        <div className="settings-layout settings-layout-single">
          <section className="settings-content" data-settings-scroll-root="true">
              {error ? <div className="inline-error" role="alert">{error.message}</div> : null}
              {activeTab === "general" ? <div className="settings-scope-intro"><span className="eyebrow">ATSUMI COMMON</span><h3>일반 설정</h3><p>두 소스가 함께 사용하는 저장 위치·화면 크기·개인정보 보호·프로그램 관리 설정입니다.</p></div> : null}
              {activeTab === "hitomi" ? <div className="settings-scope-intro"><span className="eyebrow">HITOMI LIBRARY</span><h3>Hitomi 설정</h3><p>앨범·Auto Find·판본 중복·Related galleries·Hitomi 태그 검색에만 적용됩니다.</p></div> : null}
              {activeTab !== "danbooru" ? <>
                <div className="setting-row" hidden={activeTab !== "general"}>
                  <div><strong>다운로드 폴더</strong><span>Hitomi 앨범과 Danbooru 원본을 저장할 공통 루트</span></div>
                  <input value={draft.downloadRoot} placeholder="폴더를 선택하세요" aria-label="다운로드 폴더" onChange={(event) => patch("downloadRoot", event.target.value)} />
                </div>
                <section className="storage-usage-panel" aria-labelledby="storage-usage-title" hidden={activeTab !== "general"}>
                  <header className="storage-usage-header">
                    <div>
                      <span className="eyebrow">STORAGE</span>
                      <strong id="storage-usage-title">저장공간 사용량</strong>
                      <p>현재 저장된 다운로드 경로를 기준으로 계산합니다. 큰 폴더는 확인에 시간이 걸릴 수 있습니다.</p>
                    </div>
                    <button
                      type="button"
                      className="text-button"
                      disabled={storageUsageLoading}
                      onClick={() => loadStorageUsage(true)}
                    >{storageUsageLoading ? "계산 중" : "새로고침"}</button>
                  </header>
                  {storageUsageError ? <p className="storage-usage-error" role="alert">{storageUsageError}</p> : null}
                  {!storageUsage && storageUsageLoading ? <p className="storage-usage-loading" role="status">파일과 디스크 용량을 계산하고 있습니다.</p> : null}
                  {storageUsage ? <>
                    <div className="storage-summary-grid">
                      <article>
                        <span>메모리 미리보기 캐시</span>
                        <data value={storageUsage.memoryCacheBytes}>{formatBytes(storageUsage.memoryCacheBytes)}</data>
                        <small>현재 실행 중인 RAM 사용량</small>
                      </article>
                      <article>
                        <span>디스크 임시 캐시</span>
                        <data value={storageUsage.diskCache.bytes}>{formatBytes(storageUsage.diskCache.bytes)}</data>
                        <small>열려 있는 원본 페이지 미리보기</small>
                      </article>
                      <article>
                        <span>앱 데이터</span>
                        <data value={storageUsage.appData.bytes}>{formatBytes(storageUsage.appData.bytes)}</data>
                        <small>DB·분석 정보·마이그레이션 백업</small>
                      </article>
                      <article>
                        <span>다운로드 폴더</span>
                        <data value={storageUsage.downloads.bytes}>
                          {!settings.downloadRoot.trim() ? "폴더 미설정" : formatBytes(storageUsage.downloads.bytes)}
                        </data>
                        <small>{settings.downloadRoot.trim() && !storageUsage.downloads.exists ? "설정된 폴더가 존재하지 않음" : "다운로드 루트 아래의 실제 파일"}</small>
                      </article>
                    </div>
                    <div className="storage-volume-list">
                      {storageUsage.volumes.map((volume) => {
                        const total = volume.totalBytes;
                        const available = volume.availableBytes;
                        if (total === undefined || available === undefined || total <= 0) {
                          return <article className="storage-volume" key={volume.root}>
                            <div><strong>{volume.root}</strong><span>디스크 전체·남은 용량을 읽지 못했습니다.</span></div>
                            <small>Atsumi 관리 경로 {formatBytes(volume.atsumiBytes)}</small>
                          </article>;
                        }
                        const occupied = Math.min(total, Math.max(0, total - available));
                        const atsumiOnDisk = Math.min(occupied, volume.atsumiBytes);
                        const otherOccupied = Math.max(0, occupied - atsumiOnDisk);
                        return <article className="storage-volume" key={volume.root}>
                          <div className="storage-volume-heading">
                            <strong>{volume.root} 디스크</strong>
                            <span>{formatBytes(available)} 남음 / {formatBytes(total)}</span>
                          </div>
                          <div
                            className="storage-volume-meter"
                            role="img"
                            aria-label={`${volume.root} 전체 ${formatBytes(total)} 중 ${formatBytes(occupied)} 사용, Atsumi 관리 경로 ${formatBytes(volume.atsumiBytes)}`}
                          >
                            <span className="is-other" style={{ width: `${storagePercent(otherOccupied, total)}%` }} />
                            <span className="is-atsumi" style={{ width: `${storagePercent(atsumiOnDisk, total)}%` }} />
                          </div>
                          <div className="storage-volume-legend">
                            <span><i className="is-atsumi" />Atsumi 관리 경로 {formatBytes(volume.atsumiBytes)} ({storagePercent(volume.atsumiBytes, total).toLocaleString("ko-KR", { maximumFractionDigits: 2 })}%)</span>
                            <span><i className="is-other" />기타 사용 {formatBytes(otherOccupied)}</span>
                          </div>
                        </article>;
                      })}
                    </div>
                    {storageUsage.warnings.length ? <ul className="storage-usage-warnings">
                      {storageUsage.warnings.map((warning) => <li key={warning}>{warning}</li>)}
                    </ul> : null}
                  </> : null}
                </section>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div>
                    <strong>갤러리 폴더 이름</strong>
                    <span>{"사용가능 인자 : {artist}, {title}, {group}, {id}"}</span>
                    <span aria-live="polite">미리보기 : {folderPreview || "(확인 중)"}</span>
                    {folderPreviewError ? <span className="setting-validation-error" role="alert">{folderPreviewError}</span> : null}
                  </div>
                  <div>
                    <input
                      value={draft.folderNameTemplate}
                      aria-label="갤러리 폴더 이름 템플릿"
                      maxLength={512}
                      onChange={(event) => patch("folderNameTemplate", event.target.value)}
                    />
                    <button
                      type="button"
                      className="text-button"
                      onClick={() => patch("folderNameTemplate", DEFAULT_FOLDER_NAME_TEMPLATE)}
                    >
                      기본값 복원
                    </button>
                  </div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div>
                    <strong>Auto Find 기록 기준</strong>
                    <span>변경한 기준은 다음 Auto Find 실행부터 적용됩니다.</span>
                    <span>최신 기준은 검증 완료·격리된 소유 작품의 gallery ID 이후만 후보로 봅니다.</span>
                  </div>
                  <div className="settings-select-control">
                    <select
                      aria-label="Auto Find 기록 기준"
                      value={draft.autoFindHistoryMode}
                      onChange={(event) => patch("autoFindHistoryMode", event.target.value as SettingsSnapshot["autoFindHistoryMode"])}
                    >
                      <option value="include_all_history">전체 기록 포함</option>
                      <option value="newer_than_oldest_downloaded">가장 오래된 소유 작품 이후</option>
                    </select>
                    <FluentIcon glyph="\uE70D" />
                  </div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div>
                    <strong>다운로드 판본 자동 판정</strong>
                    <span>포함률 95% 이상이면서 판본 간 페이지 차이가 5장 이하인 포함·거의 동일 판본만 판단합니다.</span>
                    <span>제목에서 무검열 표식이 확인되면 그 판본을 우선합니다. 다만 포함 관계에서 더 큰 판본이 검열판이고 작은 판본이 무검열판인 충돌, 또는 근거가 부족한 경우에는 직접 검토합니다. 자동 제거도 영구 삭제가 아닌 복구 가능한 격리입니다.</span>
                  </div>
                  <div className="settings-select-control">
                    <select
                      aria-label="다운로드 판본 자동 판정"
                      value={draft.downloadOverlapAutoMode}
                      onChange={(event) => patch("downloadOverlapAutoMode", event.target.value as SettingsSnapshot["downloadOverlapAutoMode"])}
                    >
                      <option value="off">사용 안 함</option>
                      <option value="recommend">추천만 표시</option>
                      <option value="strict_quarantine">95% 기준 자동 정리</option>
                    </select>
                    <FluentIcon glyph="\uE70D" />
                  </div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div>
                    <strong>Hitomi 페이지당 앨범 수</strong>
                    <span>현재 열 수에 맞춰 마지막 행이 차도록 요청량을 가까운 열 배수로 자동 조정합니다.</span>
                  </div>
                  <div className="range-wrap">
                    <input
                      id="settings-explore-page-size"
                      aria-label="Hitomi 페이지당 앨범 수"
                      type="range"
                      min="10"
                      max="200"
                      step="10"
                      value={draft.explorePageSize}
                      onChange={(event) => patch("explorePageSize", Number(event.target.value))}
                    />
                    <output htmlFor="settings-explore-page-size">{draft.explorePageSize}개</output>
                  </div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div><strong>앨범 카드 최대 열 수</strong><span>창이 넓어도 설정한 열 수를 넘지 않습니다</span></div>
                  <div className="range-wrap"><input id="settings-max-columns" aria-label="앨범 카드 최대 열 수" type="range" min="1" max="4" step="1" value={draft.maxColumns} onChange={(event) => { const value = Number(event.target.value); patch("maxColumns", value); previewLayout(value, draft.previewWidth); }} /><output htmlFor="settings-max-columns">{draft.maxColumns}열</output></div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div><strong>Hitomi 카드 미리보기 크기</strong><span>Hitomi Explore·Auto Find·Downloads 카드에 적용</span></div>
                  <div className="range-wrap"><input id="settings-preview-width" aria-label="Hitomi 카드 미리보기 크기" type="range" min="0" max={GALLERY_PREVIEW_PRESETS.length - 1} step="1" value={galleryPreviewPresetIndex(draft.previewWidth)} onChange={(event) => { const preset = GALLERY_PREVIEW_PRESETS[Number(event.target.value)] ?? GALLERY_PREVIEW_PRESETS[2]!; patch("previewWidth", preset.width); previewLayout(draft.maxColumns, preset.width); }} /><output htmlFor="settings-preview-width">{draft.previewWidth}px</output></div>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div><strong>Related galleries 미리보기 크기</strong><span>Floating Detail 안의 Related galleries에만 적용</span></div>
                  <div className="range-wrap"><input id="settings-related-preview-width" aria-label="Related galleries 미리보기 크기" type="range" min="180" max="320" step="20" value={draft.relatedPreviewWidth} onChange={(event) => patch("relatedPreviewWidth", Number(event.target.value))} /><output htmlFor="settings-related-preview-width">{draft.relatedPreviewWidth}px</output></div>
                </div>
                <div className="setting-row" hidden={activeTab !== "general"}>
                  <div>
                    <strong>프라이버시 모드</strong>
                    <span>Hitomi와 Danbooru의 미리보기·상세 이미지를 가립니다.</span>
                  </div>
                  <label className="setting-checkbox">
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label="프라이버시 모드"
                      checked={draft.privacyMode}
                      onChange={(event) => patch("privacyMode", event.target.checked)}
                    />
                    <span>{draft.privacyMode ? "사용 중" : "사용 안 함"}</span>
                  </label>
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div><strong>동시 이미지 요청</strong><span>안정 기본값 5</span></div>
                  <input type="number" min="1" max="30" value={draft.concurrentImageRequests} aria-label="동시 이미지 요청" onChange={(event) => patch("concurrentImageRequests", Number(event.target.value))} />
                </div>
                <div className="setting-row" hidden={activeTab !== "hitomi"}>
                  <div><strong>요청 시작 간격</strong><span>안정 기본값 25ms</span></div>
                  <input type="number" min="0" max="5000" value={draft.requestStartIntervalMs} aria-label="요청 시작 간격" onChange={(event) => patch("requestStartIntervalMs", Number(event.target.value))} />
                </div>
                <div className="setting-row settings-reset-row" hidden={activeTab !== "general"}>
                  <div>
                    <strong>설정 초기화</strong>
                    <span>화면·미리보기·네트워크 설정을 기본값으로 되돌립니다. 저장을 눌러야 적용됩니다.</span>
                  </div>
                  <button type="button" className="text-button" disabled={maintenanceBusy !== null} onClick={restorePreferenceDefaults}>설정 기본값</button>
                </div>
                <section className="maintenance-panel" aria-labelledby="maintenance-panel-title" hidden={activeTab !== "hitomi"}>
                  <header className="maintenance-panel-header">
                    <strong id="maintenance-panel-title">저장 데이터 관리</strong>
                    <p>원본 파일과 사용자 판정을 보존하는 복구·검사 작업과, 외부 원본을 보존하는 앱 데이터 초기화를 제공합니다.</p>
                  </header>
                  {maintenanceMessage ? <p className="maintenance-message" role="status">{maintenanceMessage}</p> : null}
                  <div className="maintenance-list">
                    <article className="maintenance-item maintenance-item--quick-repair">
                      <div className="maintenance-copy">
                        <strong>빠른 복구</strong>
                        <p>다운로드, 검색 또는 미리보기가 멈출 때 캐시와 중단된 작업 상태를 정리합니다. 저장된 앨범과 원본 파일은 유지됩니다.</p>
                      </div>
                      <button type="button" className="text-button" disabled={maintenanceBusy !== null} onClick={() => void runMaintenance({ kind: "quickRepair" })}>{maintenanceBusy === "quickRepair" ? "복구 중" : "빠른 복구"}</button>
                    </article>
                    <article className="maintenance-item maintenance-item--rebuild">
                      <div className="maintenance-copy">
                        <strong>라이브러리 검사 및 재구축</strong>
                        <p>DB, manifest와 실제 파일을 검사하고 필요한 파생 데이터를 다시 만듭니다. 원본 파일과 사용자 판정은 유지됩니다.</p>
                      </div>
                      <fieldset className="maintenance-options">
                        <legend className="sr-only">재구축 항목</legend>
                        <label><input type="checkbox" checked={rebuildOptions.thumbnail} onChange={(event) => setRebuildOptions((current) => ({ ...current, thumbnail: event.target.checked }))} /> 미리보기 캐시 재생성</label>
                        <label><input type="checkbox" checked={rebuildOptions.duplicate} onChange={(event) => setRebuildOptions((current) => ({ ...current, duplicate: event.target.checked }))} /> 작품 중복 분석 재실행</label>
                        <label><input type="checkbox" checked={rebuildOptions.internal} onChange={(event) => setRebuildOptions((current) => ({ ...current, internal: event.target.checked }))} /> 내부 중복 분석 재실행</label>
                        <label><input type="checkbox" checked={rebuildOptions.autoFind} onChange={(event) => setRebuildOptions((current) => ({ ...current, autoFind: event.target.checked }))} /> Auto Find 결과 갱신</label>
                      </fieldset>
                      <button type="button" className="text-button" disabled={maintenanceBusy !== null} onClick={() => void runMaintenance({ kind: "rebuildLibrary", rebuildThumbnailData: rebuildOptions.thumbnail, rebuildDuplicateAnalysis: rebuildOptions.duplicate, rebuildInternalAnalysis: rebuildOptions.internal, rebuildAutoFindResults: rebuildOptions.autoFind })}>{maintenanceBusy === "rebuildLibrary" ? "검사 중" : "라이브러리 검사 및 재구축"}</button>
                    </article>
                    <article className="maintenance-item maintenance-item--factory-reset">
                      <div className="maintenance-copy">
                        <strong>앱 데이터 완전 초기화</strong>
                        <p>앱을 첫 실행 상태로 되돌립니다. 외부 다운로드 원본 파일과 quarantine/recovery 파일은 유지됩니다.</p>
                      </div>
                      <button type="button" className="text-button danger-button" disabled={maintenanceBusy !== null} onClick={() => void runMaintenance({ kind: "factoryReset", confirmation: "RESET_ALL_APP_DATA" })}>{maintenanceBusy === "factoryReset" ? "초기화 준비 중" : "앱 데이터 완전 초기화"}</button>
                    </article>
                  </div>
                </section>
                <section className="settings-about-panel" aria-labelledby="settings-about-title" hidden={activeTab !== "general"}>
                  <header>
                    <div>
                      <span className="eyebrow">ABOUT &amp; FEEDBACK</span>
                      <strong id="settings-about-title">프로그램 정보</strong>
                    </div>
                    <span className="settings-about-version">v{packageMetadata.version}</span>
                  </header>
                  <dl className="settings-about-details">
                    <div><dt>프로그램</dt><dd>Atsumi</dd></div>
                    <div><dt>제작</dt><dd>assesse · Atsumi contributors</dd></div>
                    <div><dt>프로젝트</dt><dd>github.com/assesse/Atsumi-Project</dd></div>
                  </dl>
                  <p>버그와 기능 제안은 GitHub Issues에서 받습니다. 복사되는 진단 정보에는 앨범 제목, 태그, 파일 경로, 데이터베이스 내용이 포함되지 않습니다.</p>
                  <div className="settings-about-actions">
                    <button type="button" className="text-button primary" disabled={updateCheckBusy} onClick={() => void checkForUpdates()}>{updateCheckBusy ? "확인 중" : "업데이트 확인"}</button>
                    <button type="button" className="text-button" onClick={() => void copyInformation("feedback")}>피드백 주소 복사</button>
                    <button type="button" className="text-button" onClick={() => void copyInformation("diagnostics")}>진단 정보 복사</button>
                  </div>
                  <p className="settings-about-message" role="status" aria-live="polite">{updateMessage || informationMessage}</p>
                </section>
              </> : null}
              {activeTab === "hitomi" ? <>
                <section className="search-catalog-panel" aria-labelledby="search-catalog-title">
                  <header>
                    <div>
                      <span className="eyebrow">SEARCH AUTOCOMPLETE</span>
                      <h3 id="search-catalog-title">검색어 자동완성 데이터</h3>
                      <p>Hitomi의 태그·작가·그룹 목록을 내려받아 Explore 검색 제안을 최신 상태로 유지합니다.</p>
                    </div>
                    <button
                      type="button"
                      className="text-button primary"
                      aria-busy={tagCatalogRefreshing || undefined}
                      disabled={tagCatalogRefreshing}
                      onClick={() => void onTagCatalogRefresh()}
                    >
                      {tagCatalogRefreshing ? <span className="spinner catalog-refresh-spinner" aria-hidden="true" /> : <FluentIcon glyph="\uE72C" />}
                      {tagCatalogRefreshing ? "최신화 중" : "지금 최신화"}
                    </button>
                  </header>
                  <div className={`search-catalog-status${tagCatalogIncomplete ? " is-incomplete" : ""}`} role="status" aria-live="polite">
                    <strong>{tagCatalogStatus?.entryCount
                      ? `${tagCatalogStatus.entryCount.toLocaleString()}개 항목 저장됨`
                      : "저장된 자동완성 데이터 없음"}</strong>
                    {tagCatalogStatus?.entryCount ? (
                      <span>
                        작가 {tagCatalogStatus.artistCount.toLocaleString()} · 그룹 {tagCatalogStatus.groupCount.toLocaleString()} · 일반 태그 {tagCatalogStatus.neutralCount.toLocaleString()} · F {tagCatalogStatus.femaleCount.toLocaleString()} · M {tagCatalogStatus.maleCount.toLocaleString()}
                      </span>
                    ) : <span>처음 최신화하기 전에도 검색 자체는 사용할 수 있습니다.</span>}
                    {tagCatalogStatus?.lastErrorMessage ? <small>최근 실패: {tagCatalogStatus.lastErrorMessage}</small> : null}
                  </div>
                </section>
                <section className="search-rules-panel" aria-labelledby="search-rules-title">
                  <header>
                    <span className="eyebrow">GLOBAL SEARCH RULES</span>
                    <h3 id="search-rules-title">모든 Explore 검색에 적용할 태그</h3>
                    <p>태그를 한 줄에 하나씩 입력하세요. 쉼표로도 구분할 수 있으며, 저장 후 새 검색·검색 기록 재실행·메타데이터 검색에 공통 적용됩니다.</p>
                  </header>
                  <div className="search-rule-fields">
                    <label>
                      <strong>필수 포함 태그</strong>
                      <span>모든 검색의 포함 조건에 자동으로 추가됩니다.</span>
                      <textarea
                        aria-label="모든 검색 필수 포함 태그"
                        value={includeTagInput}
                        placeholder={"female:glasses\nwebtoon"}
                        onChange={(event) => {
                          setIncludeTagInput(event.target.value);
                          patch("searchIncludeTags", parseGlobalSearchTagInput(event.target.value));
                        }}
                      />
                    </label>
                    <label>
                      <strong>항상 제외할 태그</strong>
                      <span>개별 검색이 같은 태그를 요구해도 제외 조건이 우선합니다.</span>
                      <textarea
                        aria-label="모든 검색 제외 태그"
                        value={excludeTagInput}
                        placeholder={"male:glasses\nfull_color"}
                        onChange={(event) => {
                          setExcludeTagInput(event.target.value);
                          patch("searchExcludeTags", parseGlobalSearchTagInput(event.target.value));
                        }}
                      />
                    </label>
                  </div>
                  {overlappingSearchTags.length ? (
                    <p className="inline-error" role="alert">포함과 제외에 동시에 지정된 태그를 정리하세요: {overlappingSearchTags.join(", ")}</p>
                  ) : null}
                </section>

                <section className="exclusion-manager" aria-labelledby="exclusion-manager-title">
                  <header className="exclusion-manager-header">
                    <div>
                      <span className="eyebrow">EXCLUDED ALBUMS</span>
                      <h3 id="exclusion-manager-title">탐색 제외·중복 숨김 앨범</h3>
                      <p>직접 제외했거나 중복 판정 때문에 Auto Find 또는 Downloads에서 숨겨진 앨범입니다. 해제해도 중복 판정 기록은 삭제되지 않습니다.</p>
                    </div>
                    <button
                      type="button"
                      className="text-button primary"
                      disabled={!selectedExclusionIds.size || restoringExclusions}
                      onClick={() => void restoreExclusions([...selectedExclusionIds])}
                    >{restoringExclusions ? "복원 중" : `선택 복원 (${selectedExclusionIds.size})`}</button>
                  </header>
                  {exclusionsError ? <div className="inline-error" role="alert">{exclusionsError}</div> : null}
                  {exclusionsLoading ? <p className="exclusion-empty" role="status">제외 앨범을 불러오는 중입니다.</p> : null}
                  {!exclusionsLoading && !exclusions.length ? <p className="exclusion-empty">현재 관리할 제외·숨김 앨범이 없습니다.</p> : null}
                  {exclusions.length ? (
                    <div className="exclusion-list">
                      <label className="exclusion-select-all">
                        <input
                          type="checkbox"
                          checked={selectedExclusionIds.size === exclusions.length}
                          onChange={(event) => setSelectedExclusionIds(event.target.checked
                            ? new Set(exclusions.map((item) => item.galleryId))
                            : new Set())}
                        />
                        전체 선택
                      </label>
                      {exclusions.map((item) => (
                        <article className="exclusion-item" key={item.galleryId}>
                          <input
                            type="checkbox"
                            aria-label={`${item.title} 선택`}
                            checked={selectedExclusionIds.has(item.galleryId)}
                            onChange={(event) => setSelectedExclusionIds((current) => {
                              const next = new Set(current);
                              if (event.target.checked) next.add(item.galleryId);
                              else next.delete(item.galleryId);
                              return next;
                            })}
                          />
                          <div className="exclusion-copy">
                            <strong>{item.title}</strong>
                            <span>{item.artist} · Gallery #{item.galleryId}</span>
                            <div className="exclusion-reasons">
                              {item.reasons.map((reason, index) => (
                                <span key={`${reason.kind}-${reason.excludedAt}-${index}`} title={`${reason.detail} · ${reason.excludedAt}`}>
                                  {exclusionReasonLabel(reason.kind)}
                                </span>
                              ))}
                            </div>
                          </div>
                          <button
                            type="button"
                            className="text-button"
                            disabled={restoringExclusions}
                            onClick={() => void restoreExclusions([item.galleryId])}
                          >제외/숨김 해제</button>
                        </article>
                      ))}
                    </div>
                  ) : null}
                </section>
              </> : null}
              {activeTab === "danbooru" ? (
                <DanbooruSettingsPanel
                  filters={danbooruDraft}
                  settings={draft}
                  onChange={setDanbooruDraft}
                  onSettingsChange={patch}
                  onReset={() => {
                    setDanbooruDraft(defaultDanbooruSearchFilters());
                    patch("danbooruPageSize", 60);
                    patch("danbooruPreviewWidth", 190);
                  }}
                />
              ) : null}
          </section>
        </div>
      </div>
    </dialog>
  );
}

function DanbooruSettingsPanel({
  filters,
  settings,
  onChange,
  onSettingsChange,
  onReset,
}: {
  filters: DanbooruSearchFilters;
  settings: SettingsSnapshot;
  onChange: (filters: DanbooruSearchFilters) => void;
  onSettingsChange: <K extends keyof SettingsSnapshot>(key: K, value: SettingsSnapshot[K]) => void;
  onReset: () => void;
}) {
  const toggleRating = (rating: DanbooruRating, checked: boolean) => onChange({
    ...filters,
    ratings: checked
      ? DANBOORU_RATINGS.map(({ value }) => value).filter((value) => filters.ratings.includes(value) || value === rating)
      : filters.ratings.filter((value) => value !== rating),
  });
  const toggleFileType = (fileType: DanbooruFileType, checked: boolean) => onChange({
    ...filters,
    fileTypes: checked
      ? DANBOORU_FILE_TYPES.map(({ value }) => value).filter((value) => filters.fileTypes.includes(value) || value === fileType)
      : filters.fileTypes.filter((value) => value !== fileType),
  });
  return (
    <div className="danbooru-settings-panel">
      <div className="settings-scope-intro">
        <span className="eyebrow">DANBOORU POSTS</span>
        <h3>Danbooru 설정</h3>
        <p>개별 post 검색의 기본 메타 조건입니다. Hitomi 앨범 검색·Auto Find·중복 판정에는 영향을 주지 않습니다.</p>
      </div>
      <section className="danbooru-settings-section" aria-labelledby="danbooru-layout-title">
        <header><h3 id="danbooru-layout-title">카드와 페이지</h3><p>Danbooru에만 적용되며 Hitomi 카드 설정과 독립적으로 저장됩니다.</p></header>
        <div className="setting-row">
          <div><strong>페이지당 post 수</strong><span>현재 열 수에 맞춰 마지막 행이 차도록 100개 이내의 가까운 열 배수로 조정합니다.</span></div>
          <div className="range-wrap">
            <input id="settings-danbooru-page-size" aria-label="Danbooru 페이지당 post 수" type="range" min="10" max="100" step="10" value={settings.danbooruPageSize} onChange={(event) => onSettingsChange("danbooruPageSize", Number(event.target.value))} />
            <output htmlFor="settings-danbooru-page-size">{settings.danbooruPageSize}개</output>
          </div>
        </div>
        <div className="setting-row">
          <div><strong>카드 미리보기 크기</strong><span>Danbooru Explore·Downloads 카드에만 적용합니다.</span></div>
          <div className="range-wrap">
            <input id="settings-danbooru-preview-width" aria-label="Danbooru 카드 미리보기 크기" type="range" min="0" max={GALLERY_PREVIEW_PRESETS.length - 1} step="1" value={galleryPreviewPresetIndex(settings.danbooruPreviewWidth)} onChange={(event) => { const preset = GALLERY_PREVIEW_PRESETS[Number(event.target.value)] ?? GALLERY_PREVIEW_PRESETS[1]!; onSettingsChange("danbooruPreviewWidth", preset.width); }} />
            <output htmlFor="settings-danbooru-preview-width">{settings.danbooruPreviewWidth}px</output>
          </div>
        </div>
      </section>
      <section className="danbooru-settings-section" aria-labelledby="danbooru-default-search-title">
        <header><h3 id="danbooru-default-search-title">새 검색 기본값</h3><p>단부루 모드를 새로 열거나 기본 조건을 불러올 때 사용합니다.</p></header>
        <div className="setting-row">
          <div><strong>기본 등급</strong><span>4종: General(g), Sensitive(s), Questionable(q), Explicit(e)</span></div>
          <div className="danbooru-settings-checks">
            {DANBOORU_RATINGS.map((rating) => <label key={rating.value} title={rating.description}><input type="checkbox" checked={filters.ratings.includes(rating.value)} onChange={(event) => toggleRating(rating.value, event.target.checked)} /> {rating.label}</label>)}
          </div>
        </div>
        <div className="setting-row">
          <div><strong>기본 파일 형식</strong><span>미선택 또는 전체 선택은 형식을 제한하지 않습니다.</span></div>
          <div className="danbooru-settings-checks is-files">
            {DANBOORU_FILE_TYPES.map((fileType) => <label key={fileType.value}><input type="checkbox" checked={filters.fileTypes.includes(fileType.value)} onChange={(event) => toggleFileType(fileType.value, event.target.checked)} /> {fileType.label}</label>)}
          </div>
        </div>
        <div className="setting-row">
          <div><strong>기본 정렬</strong><span>최신순 외 정렬은 무료 검색의 일반 조건 1개를 사용합니다.</span></div>
          <DropdownSelect
            ariaLabel="Danbooru 기본 정렬"
            className="settings-dropdown"
            value={filters.sort}
            options={DANBOORU_SORTS}
            onChange={(sort) => onChange({ ...filters, sort })}
          />
        </div>
        <div className="setting-row">
          <div><strong>카드 이미지 품질</strong><span>카드는 최대 850px large/sample poster를 쓰고, MP4·WebM은 상세 화면에서 바로 재생합니다.</span></div>
          <span className="settings-fixed-value">고화질 고정</span>
        </div>
        <div className="setting-row settings-reset-row">
          <div><strong>Danbooru 기본값 초기화</strong><span>카드·페이지·검색 기본값을 되돌립니다.</span></div>
          <button type="button" className="text-button" onClick={onReset}>Danbooru 기본값</button>
        </div>
      </section>
      <section className="danbooru-metatag-guide" aria-labelledby="danbooru-metatag-guide-title">
        <header><span className="eyebrow">SEARCH METADATA</span><h3 id="danbooru-metatag-guide-title">검색 제한과 메타데이터</h3><p>익명·무료 Member는 제한 대상 조건을 최대 2개 사용할 수 있습니다.</p></header>
        <div className="danbooru-metatag-groups">
          <article><strong>고정 선택 값</strong><p>등급은 정확히 4종이며 관계는 존재 여부나 post ID를 받습니다.</p><code>rating:g|s|q|e · parent:any|none|ID · child:any|none|ID · filetype:jpg|png|gif|webp|avif|webm|mp4|zip</code></article>
          <article><strong>범위·비교 입력</strong><p>날짜 범위와 수치 비교를 같은 문법으로 조합할 수 있습니다.</p><code>date:2026-08-01..2026-08-31 · score:&gt;=20 · favcount:&gt;=5 · width:&gt;=1600 · ratio:&gt;1</code></article>
          <article><strong>제한에서 제외</strong><p>일반 태그 2개와 함께 추가해도 슬롯을 쓰지 않습니다.</p><code>status rating limit is id date age filesize filetype parent child md5 width height duration mpixels ratio score upvote downvotes favcount embedded tagcount pixiv_id pixiv</code></article>
          <article><strong>제한에 포함</strong><p>각 종류가 일반 태그와 같은 슬롯을 사용합니다.</p><code>order source pool user fav favgroup has ai note comment commentary search wildcards</code></article>
          <article><strong>정렬 값</strong><p>Atsumi가 제공하는 주요 정렬이며 한 번에 하나만 사용할 수 있습니다.</p><code>id_asc score favcount mpixels filesize tagcount portrait landscape</code></article>
        </div>
      </section>
    </div>
  );
}
