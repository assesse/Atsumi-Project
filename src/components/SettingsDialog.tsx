import { useEffect, useRef, useState } from "react";
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
} from "../api/contracts";
import type { GalleryId } from "../core/types";
import {
  GALLERY_PREVIEW_PRESETS,
  galleryPreviewPresetIndex,
} from "../layout/galleryPreviewPresets";
import { parseGlobalSearchTagInput } from "../search/globalSearchRules";
import type { AppUpdateCheckResult } from "../update/useAppUpdater";
import { FluentIcon } from "./FluentIcon";

type SettingsDialogProps = {
  open: boolean;
  settings: SettingsSnapshot;
  loading: boolean;
  error: ApiError | null;
  onClose: () => void;
  onSave: (patch: SettingsPatch) => Promise<boolean>;
  onPreviewLayout: (layout: { maxColumns: number; previewWidth: number } | null) => void;
  onPreviewFolderName: (template: string) => Promise<ApiResult<string>>;
  onMaintenance: (action: MaintenanceAction) => Promise<ApiResult<MaintenanceResult>>;
  onCheckForUpdates: () => Promise<AppUpdateCheckResult>;
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
  onPreviewLayout,
  onPreviewFolderName,
  onMaintenance,
  onCheckForUpdates,
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
  const [activeTab, setActiveTab] = useState<"general" | "search">("general");
  const [exclusions, setExclusions] = useState<ExplorationExclusion[]>([]);
  const [exclusionsLoading, setExclusionsLoading] = useState(false);
  const [exclusionsError, setExclusionsError] = useState("");
  const [selectedExclusionIds, setSelectedExclusionIds] = useState<Set<GalleryId>>(new Set());
  const [restoringExclusions, setRestoringExclusions] = useState(false);
  const [includeTagInput, setIncludeTagInput] = useState(settings.searchIncludeTags.join("\n"));
  const [excludeTagInput, setExcludeTagInput] = useState(settings.searchExcludeTags.join("\n"));

  useEffect(() => {
    if (open && !wasOpen.current) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setDraft(settings);
      setMaintenanceMessage("");
      setInformationMessage("");
      setUpdateMessage("");
      setActiveTab("general");
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
    if (!open || activeTab !== "search") return undefined;
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
      maxColumns,
      previewWidth,
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

  const save = async () => {
    setSaving(true);
    const success = await onSave({
      downloadRoot: draft.downloadRoot,
      folderNameTemplate: draft.folderNameTemplate,
      autoFindHistoryMode: draft.autoFindHistoryMode,
      maxColumns: draft.maxColumns,
      previewWidth: draft.previewWidth,
      relatedPreviewWidth: draft.relatedPreviewWidth,
      privacyMode: draft.privacyMode,
      cacheLimitGb: draft.cacheLimitGb,
      concurrentImageRequests: draft.concurrentImageRequests,
      requestStartIntervalMs: draft.requestStartIntervalMs,
      searchIncludeTags: draft.searchIncludeTags,
      searchExcludeTags: draft.searchExcludeTags,
    });
    setSaving(false);
    if (success) close();
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
            aria-selected={activeTab === "search"}
            className={activeTab === "search" ? "is-active" : ""}
            onClick={() => setActiveTab("search")}
          >검색 관리</button>
        </nav>
        <div className="settings-layout settings-layout-single">
          <section className="settings-content" data-settings-scroll-root="true">
              {error ? <div className="inline-error" role="alert">{error.message}</div> : null}
              {activeTab === "general" ? <>
                <div className="setting-row">
                  <div><strong>다운로드 폴더</strong><span>완료된 갤러리를 저장할 위치</span></div>
                  <input value={draft.downloadRoot} placeholder="폴더를 선택하세요" aria-label="다운로드 폴더" onChange={(event) => patch("downloadRoot", event.target.value)} />
                </div>
                <div className="setting-row">
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
                <div className="setting-row">
                  <div>
                    <strong>Auto Find 기록 기준</strong>
                    <span>변경한 기준은 다음 Auto Find 실행부터 적용됩니다.</span>
                    <span>최신 기준은 검증 완료·격리된 소유 작품의 gallery ID 이후만 후보로 봅니다.</span>
                  </div>
                  <select
                    aria-label="Auto Find 기록 기준"
                    value={draft.autoFindHistoryMode}
                    onChange={(event) => patch("autoFindHistoryMode", event.target.value as SettingsSnapshot["autoFindHistoryMode"])}
                  >
                    <option value="include_all_history">전체 기록 포함</option>
                    <option value="newer_than_oldest_downloaded">가장 오래된 소유 작품 이후</option>
                  </select>
                </div>
                <div className="setting-row">
                  <div><strong>앨범 카드 최대 열 수</strong><span>창이 넓어도 설정한 열 수를 넘지 않습니다</span></div>
                  <div className="range-wrap"><input id="settings-max-columns" aria-label="앨범 카드 최대 열 수" type="range" min="1" max="4" step="1" value={draft.maxColumns} onChange={(event) => { const value = Number(event.target.value); patch("maxColumns", value); previewLayout(value, draft.previewWidth); }} /><output htmlFor="settings-max-columns">{draft.maxColumns}열</output></div>
                </div>
                <div className="setting-row">
                  <div><strong>앨범 미리보기 크기</strong><span>Explore와 Downloads에 함께 적용</span></div>
                  <div className="range-wrap"><input id="settings-preview-width" aria-label="앨범 미리보기 크기" type="range" min="0" max={GALLERY_PREVIEW_PRESETS.length - 1} step="1" value={galleryPreviewPresetIndex(draft.previewWidth)} onChange={(event) => { const preset = GALLERY_PREVIEW_PRESETS[Number(event.target.value)] ?? GALLERY_PREVIEW_PRESETS[2]!; patch("previewWidth", preset.width); previewLayout(draft.maxColumns, preset.width); }} /><output htmlFor="settings-preview-width">{draft.previewWidth}px</output></div>
                </div>
                <div className="setting-row">
                  <div><strong>Related galleries 미리보기 크기</strong><span>Floating Detail 안의 Related galleries에만 적용</span></div>
                  <div className="range-wrap"><input id="settings-related-preview-width" aria-label="Related galleries 미리보기 크기" type="range" min="180" max="320" step="20" value={draft.relatedPreviewWidth} onChange={(event) => patch("relatedPreviewWidth", Number(event.target.value))} /><output htmlFor="settings-related-preview-width">{draft.relatedPreviewWidth}px</output></div>
                </div>
                <div className="setting-row">
                  <div>
                    <strong>개인정보 보호 모드</strong>
                    <span>앨범·페이지 미리보기만 화면에서 가립니다. 이미지 요청과 캐시는 계속 사용됩니다.</span>
                  </div>
                  <label className="setting-checkbox">
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label="개인정보 보호 모드"
                      checked={draft.privacyMode}
                      onChange={(event) => patch("privacyMode", event.target.checked)}
                    />
                    <span>{draft.privacyMode ? "사용 중" : "사용 안 함"}</span>
                  </label>
                </div>
                <div className="setting-row">
                  <div><strong>동시 이미지 요청</strong><span>안정 기본값 5</span></div>
                  <input type="number" min="1" max="30" value={draft.concurrentImageRequests} aria-label="동시 이미지 요청" onChange={(event) => patch("concurrentImageRequests", Number(event.target.value))} />
                </div>
                <div className="setting-row">
                  <div><strong>요청 시작 간격</strong><span>안정 기본값 25ms</span></div>
                  <input type="number" min="0" max="5000" value={draft.requestStartIntervalMs} aria-label="요청 시작 간격" onChange={(event) => patch("requestStartIntervalMs", Number(event.target.value))} />
                </div>
                <div className="setting-row settings-reset-row">
                  <div>
                    <strong>설정 초기화</strong>
                    <span>화면·미리보기·네트워크 설정을 기본값으로 되돌립니다. 저장을 눌러야 적용됩니다.</span>
                  </div>
                  <button type="button" className="text-button" disabled={maintenanceBusy !== null} onClick={restorePreferenceDefaults}>설정 기본값</button>
                </div>
                <section className="maintenance-panel" aria-labelledby="maintenance-panel-title">
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
                <section className="settings-about-panel" aria-labelledby="settings-about-title">
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
              </> : <>
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
              </>}
          </section>
        </div>
      </div>
    </dialog>
  );
}
