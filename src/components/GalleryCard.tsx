import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import type { Gallery, GalleryDisplayMode, GalleryId, ViewId } from "../core/types";
import type { InternalArtifactScanProgress } from "../api/contracts";
import { languagePresentation } from "../data/languages";
import {
  galleryCoverThumbnailKey,
  thumbnailConsumerForView,
  type ThumbnailClient,
  type ThumbnailPriority,
} from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
import { MetadataChip } from "./MetadataChip";
import { fitTagChips, sortGalleryTags, splitGalleryTitle, type TagFitResult } from "./galleryCardLayout";

type GalleryCardProps = {
  gallery: Gallery;
  thumbnailPriority?: ThumbnailPriority;
  thumbnailClient?: ThumbnailClient;
  view: ViewId;
  displayMode?: GalleryDisplayMode;
  explorationExcluded?: boolean;
  selected: boolean;
  /** True only for the derived two-or-more-card batch selection mode. */
  selectionContext: boolean;
  favoriteMetadata: ReadonlySet<string>;
  duplicateCandidateCount?: number;
  internalDuplicateResultCount?: number;
  internalDuplicateProgress?: InternalArtifactScanProgress;
  keyboardFocusable?: boolean;
  onKeyboardFocus?: (id: GalleryId) => void;
  onSelect: (id: GalleryId, modifiers: { ctrlKey: boolean; shiftKey: boolean }) => void;
  onOpenDetail: (id: GalleryId) => void;
  onOpenArtifact: (id: GalleryId) => void;
  onOpenDownloadFolder?: (entryId: string) => void;
  onOpenReview: (id: GalleryId) => void;
  onOpenInternalReview?: (entryId: string) => void;
  onStatusDetail: (id: GalleryId) => void;
  onMetadataSearch: (value: string) => void;
  onMetadataFavorite: (value: string) => void;
};

const workLabel: Partial<Record<NonNullable<Gallery["download"]>["state"], string>> = {
  queued: "대기",
  resolving_metadata: "정보 확인 중",
  downloading: "다운로드 중",
  hashing: "해시 중",
  verifying: "검사 중",
  retry_wait: "재시도 대기",
  review_required: "검토 필요",
  interrupted: "중단됨",
  failed: "실패",
  completed: "완료",
  quarantined: "격리됨",
  cancelled: "취소됨",
};

export function compactFavoriteTagValues(
  tags: readonly string[],
  favoriteMetadata: ReadonlySet<string>,
  limit = 3,
): string[] {
  if (limit <= 0) return [];
  return tags.filter((tag) => favoriteMetadata.has(tag)).slice(0, limit);
}

function GalleryCardComponent({
  gallery,
  thumbnailPriority = "prefetch",
  thumbnailClient,
  view,
  displayMode = "detail",
  explorationExcluded = false,
  selected,
  selectionContext,
  favoriteMetadata,
  duplicateCandidateCount = 0,
  internalDuplicateResultCount = 0,
  internalDuplicateProgress,
  keyboardFocusable = true,
  onKeyboardFocus,
  onSelect,
  onOpenDetail,
  onOpenArtifact,
  onOpenDownloadFolder,
  onOpenReview,
  onOpenInternalReview,
  onStatusDetail,
  onMetadataSearch,
  onMetadataFavorite,
}: GalleryCardProps) {
  const download = gallery.download;
  const isExplorationBlind = view === "explore"
    && (download?.state === "quarantined" || explorationExcluded);
  const explorationBlindLabel = download?.state === "quarantined"
    ? "격리된 앨범"
    : "중복 판정으로 제외";
  const gestureSelectionContext = useRef(selectionContext);
  useEffect(() => {
    gestureSelectionContext.current = selectionContext;
  }, [selectionContext]);
  const progress = Math.min(
    100,
    Math.max(0, download?.state === "completed" ? 100 : download?.progress ?? 0),
  );
  const statusClass = ["failed", "interrupted"].includes(download?.state ?? "") ? " failed" : "";
  const language = gallery.languageKnown === false
    ? { label: "언어 확인 중", icon: null, fallback: "?" }
    : languagePresentation[gallery.language];
  const { primary: displayTitle, secondary: subtitle } = splitGalleryTitle(gallery.title, gallery.subtitle);
  const thumbnailKey = galleryCoverThumbnailKey(gallery);
  const thumbnailConsumer = thumbnailConsumerForView(view);
  const sortedTags = displayMode === "detail" ? sortGalleryTags(gallery.tags, favoriteMetadata) : [];
  const compactFavoriteTags = displayMode === "compact"
    ? compactFavoriteTagValues(gallery.tags, favoriteMetadata)
    : [];
  const tagLayoutKey = `${gallery.title}\u0000${gallery.subtitle ?? ""}\u0000${sortedTags
    .map((tag) => `${tag.namespace}:${Number(tag.favorite)}:${tag.value}`)
    .join("\u0001")}`;
  const [tagLayout, setTagLayout] = useState<{ key: string; result: TagFitResult } | null>(null);
  const currentTagLayout = tagLayout?.key === tagLayoutKey
    ? tagLayout.result
    : { visibleCount: sortedTags.length, hiddenCount: 0, showOverflow: false };
  const visibleTags = sortedTags.slice(0, currentTagLayout.visibleCount);
  const hiddenTags = sortedTags.slice(currentTagLayout.visibleCount);
  const overflowDigitCount = String(Math.max(1, sortedTags.length)).length;
  const cardRef = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const tagListRef = useRef<HTMLDivElement>(null);
  const tagChipRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const overflowMeasureRefs = useRef<Array<HTMLSpanElement | null>>([]);
  const lastContentSize = useRef({ width: 0, height: 0 });
  const hasDuplicateCandidates = duplicateCandidateCount > 0;
  const hasInternalDuplicateResult = view === "downloads"
    && download?.state === "completed"
    && internalDuplicateResultCount > 0
    && Boolean(download.entryId)
    && Boolean(onOpenInternalReview);
  const visibleInternalDuplicateProgress = view === "downloads" ? internalDuplicateProgress : undefined;
  const internalScanPercent = Math.min(100, Math.max(0, visibleInternalDuplicateProgress?.progressPercent ?? 0));
  const internalScanStage = visibleInternalDuplicateProgress?.stage === "hashing"
    ? visibleInternalDuplicateProgress.totalPages > 0
      ? `페이지 ${visibleInternalDuplicateProgress.processedPages}/${visibleInternalDuplicateProgress.totalPages}`
      : "페이지 해시 계산"
    : visibleInternalDuplicateProgress?.stage === "comparing"
      ? visibleInternalDuplicateProgress.totalPairs > 0
        ? `비교 ${visibleInternalDuplicateProgress.comparedPairs}/${visibleInternalDuplicateProgress.totalPairs}`
        : "페이지 비교"
      : visibleInternalDuplicateProgress ? "결과 정리" : "";
  const isDownloadOverlapReview = download?.state === "review_required" && download.reviewKind === "gallery_duplicate";
  const showsGlobalDuplicate = hasDuplicateCandidates && !isDownloadOverlapReview;
  const iconOnlyStatus = showsGlobalDuplicate || download?.state === "downloading" || download?.state === "review_required";
  const cardStatusClass = download?.state === "completed"
    ? " is-complete"
    : download?.state === "downloading"
      ? " is-downloading"
      : showsGlobalDuplicate || ["review_required", "interrupted", "failed", "quarantined", "cancelled"].includes(download?.state ?? "")
        ? " has-problem"
        : "";
  const statusLabel = selectionContext
    ? `${gallery.title}만 선택`
    : isDownloadOverlapReview
      ? `${gallery.title}, 다운로드 판본 중복, 검토 열기`
    : showsGlobalDuplicate
      ? `${gallery.title}, 중복 후보 ${duplicateCandidateCount}개, 검토 열기`
    : download?.state === "downloading"
    ? `${gallery.title}, 다운로드 중 ${progress}%, 작업 상태 열기`
    : download?.state === "review_required"
      ? `${gallery.title}, ${isDownloadOverlapReview ? "다운로드 판본 중복" : "중복 의심"}, 검토 열기`
      : download ? `${gallery.title}, ${workLabel[download.state]}, 작업 상태 열기` : "";
  const compactStatusLabel = visibleInternalDuplicateProgress
    ? `내부 검사 ${internalScanPercent}%`
    : showsGlobalDuplicate
      ? `중복 ${duplicateCandidateCount}`
      : download
        ? workLabel[download.state] ?? download.state
        : view === "auto-find" ? "후보" : "탐색";

  const invalidateTagLayout = useCallback(() => {
    setTagLayout((current) => current ? null : current);
  }, []);

  useLayoutEffect(() => {
    if (cardRef.current) cardRef.current.inert = isExplorationBlind;
  }, [isExplorationBlind]);

  useLayoutEffect(() => {
    const content = contentRef.current;
    const tagList = tagListRef.current;
    if (!content || !tagList || !sortedTags.length || tagLayout?.key === tagLayoutKey) return;

    const style = getComputedStyle(tagList);
    const gapX = Number.parseFloat(style.columnGap) || 0;
    const gapY = Number.parseFloat(style.rowGap) || 0;
    if (tagList.clientWidth <= 0 || tagList.clientHeight <= 0) return;
    const measurements = tagChipRefs.current.slice(0, sortedTags.length).map((chip) => {
      const rect = chip?.getBoundingClientRect();
      return { width: rect?.width ?? 0, height: rect?.height ?? 0 };
    });
    const overflowMeasurements = overflowMeasureRefs.current.slice(0, overflowDigitCount).map((chip) => {
      const rect = chip?.getBoundingClientRect();
      return { width: rect?.width ?? 0, height: rect?.height ?? 0 };
    });
    if (measurements.some(({ width, height }) => width <= 0 || height <= 0)
      || overflowMeasurements.some(({ width, height }) => width <= 0 || height <= 0)) return;
    const result = fitTagChips(
      measurements,
      overflowMeasurements,
      tagList.clientWidth,
      tagList.clientHeight,
      gapX,
      gapY,
    );
    setTagLayout({ key: tagLayoutKey, result });
  }, [overflowDigitCount, sortedTags.length, tagLayout, tagLayoutKey]);

  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const next = { width: content.clientWidth, height: content.clientHeight };
      if (next.width === lastContentSize.current.width && next.height === lastContentSize.current.height) return;
      lastContentSize.current = next;
      invalidateTagLayout();
    });
    observer.observe(content);
    let disposed = false;
    document.fonts?.ready.then(() => {
      if (!disposed) invalidateTagLayout();
    });
    return () => {
      disposed = true;
      observer.disconnect();
    };
  }, [invalidateTagLayout]);

  const selectsInsteadOfActivating = (event: Pick<MouseEvent<HTMLElement>, "ctrlKey" | "shiftKey">) =>
    selectionContext || event.ctrlKey || event.shiftKey;

  const selectFromInteractiveTarget = (event: MouseEvent<HTMLElement>) => {
    if (isExplorationBlind) {
      event.preventDefault();
      event.stopPropagation();
      return true;
    }
    // A selected-card context still permits metadata navigation. Only an
    // explicit range/toggle gesture turns an interactive chip into selection.
    if (!event.ctrlKey && !event.shiftKey) return false;
    event.preventDefault();
    event.stopPropagation();
    if (event.detail <= 1) onSelect(gallery.id, event);
    return true;
  };

  const openStatus = (event: MouseEvent<HTMLButtonElement>) => {
    if (isExplorationBlind) return;
    if (selectFromInteractiveTarget(event)) return;
    event.stopPropagation();
    if (showsGlobalDuplicate || download?.state === "review_required") onOpenReview(gallery.id);
    else onStatusDetail(gallery.id);
  };

  const selectFromKeyboard = (event: KeyboardEvent<HTMLElement>) => {
    if (isExplorationBlind) return;
    if (event.target !== event.currentTarget) return;
    if (event.key === " ") {
      event.preventDefault();
      onSelect(gallery.id, { ctrlKey: true, shiftKey: false });
      return;
    }
    if (event.key === "Enter" && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      if (view === "downloads" && download?.state === "completed" && onOpenDownloadFolder) {
        onOpenDownloadFolder(download.entryId);
      }
      else if (view === "downloads") onOpenArtifact(gallery.id);
      else onOpenDetail(gallery.id);
    }
  };

  return (
    <article
      className={`gallery-card${displayMode === "compact" ? " is-compact" : ""}${selected ? " is-selected" : ""}${gallery.favorite ? " is-favorite" : ""}${cardStatusClass}${visibleInternalDuplicateProgress ? " is-internal-scanning" : ""}${isExplorationBlind ? " is-quarantined-blind is-exploration-blind" : ""}`}
      ref={cardRef}
      data-gallery-id={gallery.id}
      data-display-mode={displayMode}
      style={{ "--download-progress": `${progress}%` } as CSSProperties}
      role="listitem"
      tabIndex={keyboardFocusable && !isExplorationBlind ? 0 : -1}
      aria-disabled={isExplorationBlind || undefined}
      aria-label={[
        gallery.title,
        subtitle || null,
        download?.state === "completed" ? "다운로드 완료" : null,
        visibleInternalDuplicateProgress ? `내부 중복 검사 ${internalScanPercent}%` : null,
        isExplorationBlind ? `${explorationBlindLabel}, 내용 가림` : null,
        selected ? "선택됨" : "선택 안 됨",
      ].filter(Boolean).join(", ")}
      onKeyDown={selectFromKeyboard}
      onFocus={() => onKeyboardFocus?.(gallery.id)}
      onClick={(event) => {
        if (isExplorationBlind) return;
        if ((event.target as Element).closest("button")) return;
        if (event.detail > 1) return;
        gestureSelectionContext.current = selectsInsteadOfActivating(event);
        onSelect(gallery.id, event);
      }}
      onDoubleClick={(event) => {
        if (isExplorationBlind) return;
        if ((event.target as Element).closest("button")) return;
        if (gestureSelectionContext.current || event.ctrlKey || event.shiftKey) {
          gestureSelectionContext.current = false;
          return;
        }
        gestureSelectionContext.current = false;
        event.currentTarget.focus();
        if (view === "downloads") onOpenArtifact(gallery.id);
        else onOpenDetail(gallery.id);
      }}
      onContextMenu={(event) => {
        if (isExplorationBlind) {
          event.preventDefault();
          return;
        }
        if ((event.target as Element).closest("button")) return;
        event.preventDefault();
        event.currentTarget.focus();
        if (view === "downloads" && (hasDuplicateCandidates || isDownloadOverlapReview)) onOpenReview(gallery.id);
        else onOpenDetail(gallery.id);
      }}
    >
      {selectionContext ? (
        <span className="selection-indicator" aria-hidden="true">
          <svg viewBox="0 0 16 16" focusable="false">
            <path d="m3.5 8.1 2.8 2.8 6.2-6.2" />
          </svg>
        </span>
      ) : null}
      <GalleryThumbnail
        className="cover"
        thumbnailKey={thumbnailKey}
        consumer={thumbnailConsumer}
        priority={thumbnailPriority}
        client={thumbnailClient}
        sizing="intrinsic"
        expectedAspectRatio={gallery.thumbnailWidth !== undefined && gallery.thumbnailHeight !== undefined
          ? { width: gallery.thumbnailWidth, height: gallery.thumbnailHeight }
          : undefined}
        alt={`${gallery.title} 표지`}
      >
        {download ? <span className="status-wash" aria-hidden="true" /> : null}
        {language.icon || language.fallback ? (
          <span className="language-flag">
            {language.icon ? <img src={language.icon} alt={language.label} /> : <span>{language.fallback}</span>}
          </span>
        ) : null}
        {view === "explore" && download?.state === "completed" ? (
          <span className="download-check" title="다운로드 완료">
            <GalleryStatusIcon kind="complete" />
          </span>
        ) : null}
        {showsGlobalDuplicate || (download && !["completed", "quarantined"].includes(download.state)) ? (
          <button
            type="button"
            className={`status-pill${statusClass}${iconOnlyStatus ? ` icon-only is-${showsGlobalDuplicate ? "review_required" : download?.state}` : ""}${showsGlobalDuplicate ? " has-duplicate-count" : ""}`}
            title={selectionContext ? `${gallery.title}만 선택` : showsGlobalDuplicate ? `중복 후보 ${duplicateCandidateCount}개 · 클릭하여 검토` : download?.state === "downloading" ? `다운로드 중 · ${progress}%` : download?.state === "review_required" ? `${isDownloadOverlapReview ? "다운로드 판본 중복" : "중복 의심"} · 클릭하여 검토` : download ? workLabel[download.state] : "작업 상태"}
            aria-label={statusLabel}
            onClick={openStatus}
          >
            {showsGlobalDuplicate ? (
              <><GalleryStatusIcon kind="warning" /><span className="duplicate-count">{duplicateCandidateCount}</span></>
            ) : download?.state === "downloading" ? (
              <GalleryStatusIcon kind="downloading" />
            ) : download?.state === "review_required" ? (
              <GalleryStatusIcon kind="warning" />
            ) : download ? workLabel[download.state] : null}
          </button>
        ) : null}
        {view === "downloads" ? (
          <div
            className="progress-track"
            role="progressbar"
            aria-label={`${download ? workLabel[download.state] : "다운로드"} 진행률`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progress}
          >
            <span style={{ width: `${progress}%` }} />
          </div>
        ) : null}
        {displayMode === "compact" && compactFavoriteTags.length ? (
          <div className="compact-favorite-tags" aria-label={`즐겨찾기 태그: ${compactFavoriteTags.join(", ")}`}>
            {compactFavoriteTags.map((tag) => (
              <MetadataChip
                key={tag}
                value={tag}
                favorite
                kind="tag"
                onClickCapture={selectFromInteractiveTarget}
                onSearch={onMetadataSearch}
                onToggleFavorite={onMetadataFavorite}
              />
            ))}
          </div>
        ) : null}
        {displayMode === "compact" ? (
          <div className="compact-card-summary">
            <strong title={gallery.title}>{displayTitle}</strong>
            <span title={gallery.artist}>{gallery.artist || "작가 정보 없음"}</span>
            <small>
              <span>{gallery.pages}p · #{gallery.id}</span>
              <b className={`is-${download?.state ?? (view === "auto-find" ? "candidate" : "explore")}`}>{compactStatusLabel}</b>
            </small>
          </div>
        ) : null}
        {displayMode === "compact" && hasInternalDuplicateResult ? (
          <button
            type="button"
            className="compact-internal-result-badge"
            aria-label={`${gallery.title}, 내부 중복 검토 결과 ${internalDuplicateResultCount}개 열기`}
            title={`내부 중복 검토 결과 ${internalDuplicateResultCount}개 · 클릭하여 검토`}
            onClick={(event) => {
              if (selectFromInteractiveTarget(event)) return;
              event.stopPropagation();
              onOpenInternalReview?.(download.entryId!);
            }}
          >
            <GalleryStatusIcon kind="warning" />
            <span>{internalDuplicateResultCount}</span>
          </button>
        ) : null}
      </GalleryThumbnail>
      {displayMode === "detail" ? <div ref={contentRef} className={`card-content${visibleInternalDuplicateProgress ? " has-internal-scan" : ""}`}>
        <div className="card-title" title={gallery.title}>
          <strong title={gallery.title}>{displayTitle}</strong>
          {subtitle ? <span className="title-sub">{subtitle}</span> : null}
        </div>
        <div className="card-byline" aria-label="작가 및 그룹">
          <MetadataChip
            value={`artist:${gallery.artist}`}
            label={gallery.artist}
            kind="byline"
            favorite={gallery.favorite}
            onClickCapture={selectFromInteractiveTarget}
            onSearch={onMetadataSearch}
            onToggleFavorite={onMetadataFavorite}
          />
          {gallery.group ? (
            <>
              <span className="byline-separator" aria-hidden="true">·</span>
              <MetadataChip
                value={`group:${gallery.group}`}
                label={gallery.group}
                kind="byline"
                favorite={favoriteMetadata.has(`group:${gallery.group}`)}
                onClickCapture={selectFromInteractiveTarget}
                onSearch={onMetadataSearch}
                onToggleFavorite={onMetadataFavorite}
              />
            </>
          ) : null}
        </div>
        {visibleInternalDuplicateProgress ? (
          <div
            className="internal-duplicate-card-progress"
            role="progressbar"
            aria-label={`${gallery.title} 내부 중복 검사 · ${internalScanStage}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={internalScanPercent}
          >
            <span>내부 검사 {visibleInternalDuplicateProgress.artifactIndex}/{visibleInternalDuplicateProgress.totalArtifacts}</span>
            <i aria-hidden="true"><b style={{ width: `${internalScanPercent}%` }} /></i>
            <strong>{internalScanPercent}%</strong>
            <small>{internalScanStage}</small>
          </div>
        ) : null}
        <div ref={tagListRef} className="tag-list" aria-label={`태그: ${sortedTags.map((tag) => tag.value).join(", ")}`}>
          {visibleTags.map((tag, index) => (
            <MetadataChip
              key={`${tag.value}\u0000${index}`}
              ref={(chip) => { tagChipRefs.current[index] = chip; }}
              value={tag.value}
              favorite={tag.favorite}
              kind="tag"
              onClickCapture={selectFromInteractiveTarget}
              onSearch={onMetadataSearch}
              onToggleFavorite={onMetadataFavorite}
            />
          ))}
          {currentTagLayout.showOverflow ? (
            <span
              className="tag-overflow"
              role="note"
              aria-label={`추가 태그 ${currentTagLayout.hiddenCount}개`}
              title={hiddenTags.map((tag) => tag.value).join(", ")}
            >
              +{currentTagLayout.hiddenCount}
            </span>
          ) : null}
          {Array.from({ length: overflowDigitCount }, (_, index) => (
            <span
              key={`overflow-measure-${index + 1}`}
              ref={(chip) => { overflowMeasureRefs.current[index] = chip; }}
              className="tag-overflow tag-overflow-measure"
              aria-hidden="true"
            >
              +{"8".repeat(index + 1)}
            </span>
          ))}
        </div>
        <div className="meta-bottom">
          {hasInternalDuplicateResult ? (
            <button
              type="button"
              className="internal-result-badge"
              aria-label={`${gallery.title}, 내부 중복 검토 결과 ${internalDuplicateResultCount}개 열기`}
              title={`내부 중복 검토 결과 ${internalDuplicateResultCount}개 · 클릭하여 검토`}
              onClick={(event) => {
                if (selectFromInteractiveTarget(event)) return;
                event.stopPropagation();
                onOpenInternalReview?.(download.entryId!);
              }}
            >
              <GalleryStatusIcon kind="warning" />
              <span>내부 검토 {internalDuplicateResultCount}</span>
            </button>
          ) : null}
          <span>{gallery.pages}p</span>
          <span>#{gallery.id}</span>
        </div>
      </div> : null}
      {isExplorationBlind ? (
        <div className="quarantined-blind-overlay" aria-hidden="true">
          <GalleryStatusIcon kind="warning" />
          <strong>{explorationBlindLabel}</strong>
          <span>검색 결과 위치만 유지됩니다</span>
        </div>
      ) : null}
    </article>
  );
}

export const GalleryCard = memo(GalleryCardComponent);
