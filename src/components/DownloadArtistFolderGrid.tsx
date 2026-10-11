import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type { Gallery, GalleryDisplayMode, GalleryId } from "../core/types";
import { galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";
import {
  galleryGroupStorageKey,
  recentDownloadedGalleries,
  summarizeArtistFolderTags,
  type GalleryGroup,
} from "../state/galleryGrouping";
import {
  galleryCoverThumbnailKey,
  useThumbnailClient,
  type GalleryCoverSessionRetainer,
  type ThumbnailClient,
  type ThumbnailPriority,
} from "../thumbnail";
import { GalleryCoverBackgroundPreloader } from "../thumbnail/backgroundCoverPreloader";
import { GalleryThumbnail } from "./GalleryThumbnail";
import { DownloadArtistFolderWindow } from "./DownloadArtistFolderWindow";

type DownloadArtistFolderGridProps = {
  groups: readonly GalleryGroup[];
  previewGalleryIdsByArtist?: ReadonlyMap<string, readonly GalleryId[]>;
  columns: number;
  previewWidth: number;
  favoriteMetadata: ReadonlySet<string>;
  collapsedGroupKeys: ReadonlySet<string>;
  onToggle: (key: string) => void;
  onPreviewItems?: (ids: readonly GalleryId[]) => void;
  onStopPreviewItems?: (ids: readonly GalleryId[]) => void;
  renderGrid: (items: Gallery[], ariaLabel: string, columns?: number) => ReactNode;
  contentsRef?: RefObject<HTMLElement | null>;
  displayMode?: GalleryDisplayMode;
  maximumColumns?: number;
  onColumnsChange?: (columns: number) => void;
  suspended?: boolean;
  thumbnailClient?: ThumbnailClient;
  coverRetainer?: GalleryCoverSessionRetainer;
};

type PreviewOverlayPosition = {
  left: number;
  top: number;
  width: number;
  height: number;
  tileWidth: number;
  count: number;
  originX: number;
  originY: number;
};

const PREVIEW_GAP = 12;
const folderType = (group: GalleryGroup) => group.key.startsWith("artist\u001fgroup\u001f") ? "그룹 폴더" : "작가 폴더";

const expectedRatio = (gallery: Gallery): { width: number; height: number } | undefined => (
  gallery.thumbnailWidth && gallery.thumbnailHeight
    ? { width: gallery.thumbnailWidth, height: gallery.thumbnailHeight }
    : undefined
);

const previewCountForColumns = (columns: number): number => (
  columns >= 4 ? 5 : columns >= 3 ? 4 : 3
);

export function savedArtistPreviewGalleries(items: readonly Gallery[], savedIds: readonly GalleryId[] | undefined, limit: number): Gallery[] {
  if (limit <= 0) return [];
  const available = new Map(items.map((gallery) => [gallery.id, gallery]));
  const selected: Gallery[] = [];
  for (const id of savedIds ?? []) {
    const gallery = available.get(id);
    if (gallery) {
      selected.push(gallery);
      available.delete(id);
      if (selected.length >= limit) return selected;
    }
  }
  // Keep hidden/filtered works out, even if an older saved index still contains them.
  return [...selected, ...recentDownloadedGalleries([...available.values()], limit - selected.length)];
}

const visibleTagLabel = (tag: string): string => {
  const separator = tag.indexOf(":");
  return (separator >= 0 ? tag.slice(separator + 1) : tag).replaceAll("_", " ");
};

/**
 * A visual index of virtual artist folders. It deliberately does not reuse a
 * GalleryCard: the cover stack opens a bucket and never selects or downloads a
 * gallery itself.
 */
export function DownloadArtistFolderGrid({
  groups,
  previewGalleryIdsByArtist,
  columns,
  previewWidth,
  favoriteMetadata,
  collapsedGroupKeys,
  onToggle,
  onPreviewItems,
  onStopPreviewItems,
  renderGrid,
  contentsRef,
  displayMode = "detail",
  maximumColumns = 4,
  onColumnsChange,
  suspended = false,
  thumbnailClient,
  coverRetainer,
}: DownloadArtistFolderGridProps) {
  const client = useThumbnailClient(thumbnailClient);
  const backgroundPreloader = useRef<GalleryCoverBackgroundPreloader | null>(null);
  const [previewedGroupKey, setPreviewedGroupKey] = useState<string | null>(null);
  const [overlayPosition, setOverlayPosition] = useState<PreviewOverlayPosition | null>(null);
  const previewAnchor = useRef<HTMLElement | null>(null);
  const previewInteraction = useRef<"pointer" | "keyboard" | null>(null);
  const safeColumns = Math.max(1, Math.trunc(columns));
  const hoverPreviewLimit = previewCountForColumns(safeColumns);
  const preset = galleryPreviewPreset(previewWidth);
  // Older callers may still carry several expanded keys. Render only one
  // window even in that case, never a row-expanding grid of folder contents.
  const openedGroup = groups.find((group) => !collapsedGroupKeys.has(galleryGroupStorageKey("downloads", group)));
  const rows = Array.from(
    { length: Math.ceil(groups.length / safeColumns) },
    (_, rowIndex) => groups.slice(rowIndex * safeColumns, (rowIndex + 1) * safeColumns),
  );
  const style = {
    gridTemplateColumns: `repeat(${safeColumns}, minmax(0, 1fr))`,
    ...galleryPreviewPresetStyle(preset),
  } as CSSProperties;
  const previewedGroup = useMemo(
    () => groups.find((group) => group.key === previewedGroupKey),
    [groups, previewedGroupKey],
  );
  const groupPreviews = useMemo(() => new Map(groups.map((group) => [
    group.key,
    savedArtistPreviewGalleries(group.items, previewGalleryIdsByArtist?.get(group.label.trim().toLocaleLowerCase()), Math.max(12, hoverPreviewLimit * 2)),
  ])), [groups, previewGalleryIdsByArtist, hoverPreviewLimit]);
  // Hover/open/selection should not recount every tag in the downloaded library.
  const groupTagSummaries = useMemo(() => new Map(groups.map((group) => [
    group.key, summarizeArtistFolderTags(group.items, favoriteMetadata, 6),
  ])), [groups, favoriteMetadata]);
  const overlayPreviews = useMemo(
    () => (previewedGroup ? groupPreviews.get(previewedGroup.key) ?? [] : []).slice(0, hoverPreviewLimit),
    [hoverPreviewLimit, previewedGroup, groupPreviews],
  );
  const preloadItems = useMemo(
    () => [...groupPreviews.values()].flatMap((items) => items.slice(0, hoverPreviewLimit)),
    [groupPreviews, hoverPreviewLimit],
  );

  useEffect(() => {
    const preloader = new GalleryCoverBackgroundPreloader(client, coverRetainer);
    backgroundPreloader.current = preloader;
    return () => {
      preloader.dispose();
      backgroundPreloader.current = null;
    };
  }, [client, coverRetainer]);

  useEffect(() => {
    backgroundPreloader.current?.update(preloadItems);
  }, [client, coverRetainer, preloadItems]);

  const updateOverlayPosition = useCallback(() => {
    const anchor = previewAnchor.current;
    const cover = anchor?.querySelector<HTMLElement>(".download-artist-folder-preview");
    if (!anchor || !cover || overlayPreviews.length < 2) {
      setOverlayPosition(null);
      return;
    }
    const coverRect = cover.getBoundingClientRect();
    const viewportRect = anchor.closest(".gallery-viewport")?.getBoundingClientRect();
    const viewportLeft = Math.max(8, viewportRect?.left ?? 8);
    const viewportRight = Math.min(window.innerWidth - 8, viewportRect?.right ?? window.innerWidth - 8);
    const viewportTop = Math.max(8, viewportRect?.top ?? 8);
    const viewportBottom = Math.min(window.innerHeight - 8, viewportRect?.bottom ?? window.innerHeight - 8);
    const availableWidth = viewportRight - viewportLeft;
    const availableHeight = viewportBottom - viewportTop;
    const tileWidth = coverRect.width;
    const height = coverRect.height;
    if (tileWidth <= 0 || height <= 0 || height > availableHeight
      || coverRect.bottom <= viewportTop || coverRect.top >= viewportBottom) {
      setOverlayPosition(null);
      return;
    }
    // Keep every cover at its on-card size; show fewer covers on narrow screens.
    const count = Math.min(overlayPreviews.length, Math.floor((availableWidth + PREVIEW_GAP) / (tileWidth + PREVIEW_GAP)));
    if (count < 2) {
      setOverlayPosition(null);
      return;
    }
    const width = count * tileWidth + (count - 1) * PREVIEW_GAP;
    const left = Math.min(Math.max(coverRect.left, viewportLeft), viewportRight - width);
    const top = Math.min(Math.max(coverRect.top, viewportTop), viewportBottom - height);
    setOverlayPosition({ left, top, width, height, tileWidth, count, originX: coverRect.left - left, originY: coverRect.top - top });
  }, [overlayPreviews.length]);

  useLayoutEffect(() => {
    if (!previewedGroupKey) return;
    updateOverlayPosition();
    const scrollContainer = previewAnchor.current?.closest(".gallery-viewport");
    window.addEventListener("resize", updateOverlayPosition);
    scrollContainer?.addEventListener("scroll", updateOverlayPosition, { passive: true });
    const resizeObserver = typeof ResizeObserver === "function" ? new ResizeObserver(updateOverlayPosition) : null;
    if (previewAnchor.current) resizeObserver?.observe(previewAnchor.current);
    if (scrollContainer) resizeObserver?.observe(scrollContainer);
    return () => {
      window.removeEventListener("resize", updateOverlayPosition);
      scrollContainer?.removeEventListener("scroll", updateOverlayPosition);
      resizeObserver?.disconnect();
    };
  }, [previewedGroupKey, updateOverlayPosition]);

  const showFolderPreview = useCallback((group: GalleryGroup, anchor: HTMLElement, interaction: "pointer" | "keyboard" = "pointer") => {
    previewInteraction.current = interaction;
    if (previewAnchor.current === anchor) return;
    const sample = groupPreviews.get(group.key) ?? [];
    previewAnchor.current = anchor;
    setPreviewedGroupKey(group.key);
    onPreviewItems?.(sample.map((gallery) => gallery.id));
  }, [groupPreviews, onPreviewItems]);

  const hideFolderPreview = useCallback((group: GalleryGroup) => {
    const sample = groupPreviews.get(group.key) ?? [];
    previewAnchor.current = null;
    previewInteraction.current = null;
    setPreviewedGroupKey((current) => current === group.key ? null : current);
    setOverlayPosition(null);
    onStopPreviewItems?.(sample.map((gallery) => gallery.id));
  }, [groupPreviews, onStopPreviewItems]);

  return (
    <>
      <div
        className="download-artist-folder-grid"
        data-group-view="downloads"
        data-preview-size={preset.key}
        style={style}
        role="list"
        aria-label="다운로드 작가 폴더"
      >
        {rows.map((row, rowIndex) => (
          <Fragment key={row.map((group) => group.key).join("\u0000")}>
            {row.map((group, columnIndex) => {
              const groupIndex = rowIndex * safeColumns + columnIndex;
              const storageKey = galleryGroupStorageKey("downloads", group);
              const collapsed = openedGroup?.key !== group.key;
              const latest = groupPreviews.get(group.key)?.[0];
              const tagSummary = groupTagSummaries.get(group.key) ?? [];
              const emptyTagLabel = group.items.some((gallery) => gallery.tagsKnown === false)
                ? "태그 정보 미확인"
                : "표시할 태그가 없습니다";
              const priority: ThumbnailPriority = groupIndex < safeColumns ? "visible" : "prefetch";
              const headingId = `download-artist-folder-${groupIndex}`;
              return (
                <article
                  className={`download-artist-folder-card${collapsed ? " is-collapsed" : " is-expanded"}`}
                  key={group.key}
                  role="listitem"
                >
                  <h2>
                    <button
                      type="button"
                      id={headingId}
                      className="download-artist-folder-button"
                      aria-expanded={!collapsed}
                      aria-controls={collapsed ? undefined : `${headingId}-contents`}
                      aria-label={`${group.label} ${folderType(group)}, ${group.items.length}개 작품, ${collapsed ? "열기" : "닫기"}`}
                      onFocus={(event) => {
                        const anchor = event.currentTarget.querySelector<HTMLElement>(".download-artist-folder-preview-stack");
                        if (collapsed && anchor && event.currentTarget.matches(":focus-visible")) {
                          showFolderPreview(group, anchor, "keyboard");
                        }
                      }}
                      onBlur={(event) => {
                        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) hideFolderPreview(group);
                      }}
                      onClick={() => {
                        hideFolderPreview(group);
                        onToggle(storageKey);
                      }}
                    >
                      <span
                        className="download-artist-folder-preview-stack"
                        data-preview-count={latest ? 1 : 0}
                        data-work-count={group.items.length}
                        aria-hidden="true"
                        onMouseEnter={(event) => {
                          if (collapsed) showFolderPreview(group, event.currentTarget);
                        }}
                        onMouseLeave={() => {
                          if (previewInteraction.current === "pointer") hideFolderPreview(group);
                        }}
                      >
                        {latest ? (
                          <GalleryThumbnail
                            as="span"
                            className="download-artist-folder-preview is-preview-1"
                            key={latest.id}
                            thumbnailKey={galleryCoverThumbnailKey(latest)}
                            consumer="downloads"
                            priority={priority}
                            alt=""
                            expectedAspectRatio={expectedRatio(latest)}
                            style={{ "--folder-cover-aspect": latest.thumbnailWidth && latest.thumbnailHeight ? latest.thumbnailWidth / latest.thumbnailHeight : 2 / 3 } as CSSProperties}
                            data-gallery-id={Number(latest.id)}
                            client={thumbnailClient}
                          />
                        ) : null}
                        <i className="download-artist-folder-tab" aria-hidden="true" />
                        <span className="download-artist-folder-type">{folderType(group)}</span>
                      </span>
                      <span className="download-artist-folder-copy">
                        <strong className="download-artist-folder-name">{group.label}</strong>
                        <span className="download-artist-folder-count">{group.items.length.toLocaleString()}개 작품</span>
                        <span
                          className={`download-artist-folder-tags${tagSummary.length ? "" : " is-empty"}`}
                          aria-label={tagSummary.length
                            ? `자주 사용하거나 즐겨찾기한 태그: ${tagSummary.map((tag) => tag.value).join(", ")}`
                            : emptyTagLabel}
                        >
                          {tagSummary.length ? tagSummary.map((tag) => (
                            <span
                              className={`download-artist-folder-tag${tag.favorite ? " is-favorite" : ""}`}
                              title={`${tag.value} · 현재 불러온 앨범 ${tag.count}개에서 확인${tag.favorite ? " · 즐겨찾기" : ""}`}
                              key={tag.value}
                            >
                              {tag.favorite ? <i aria-hidden="true">★</i> : null}
                              <b>{visibleTagLabel(tag.value)}</b>
                              <small>{tag.count}</small>
                            </span>
                          )) : <span>{emptyTagLabel}</span>}
                        </span>
                        <span className="download-artist-folder-action" aria-hidden="true">
                          {collapsed ? "작품 보기" : "열린 폴더"}<i>›</i>
                        </span>
                      </span>
                    </button>
                  </h2>
                </article>
              );
            })}
          </Fragment>
        ))}
      </div>
      {openedGroup ? <DownloadArtistFolderWindow
        key={openedGroup.key}
        group={openedGroup}
        folderType={folderType(openedGroup)}
        id={`download-artist-folder-${groups.indexOf(openedGroup)}-contents`}
        contentsRef={contentsRef}
        previewWidth={previewWidth}
        displayMode={displayMode}
        maximumColumns={maximumColumns}
        onColumnsChange={onColumnsChange}
        suspended={suspended}
        renderGrid={renderGrid}
        onClose={() => onToggle(galleryGroupStorageKey("downloads", openedGroup))}
      /> : null}
      {previewedGroup && overlayPosition && overlayPreviews.length ? createPortal(
        <aside
          className="download-artist-folder-preview-overlay"
          key={previewedGroup.key}
          style={{
            left: overlayPosition.left,
            top: overlayPosition.top,
            width: overlayPosition.width,
            height: overlayPosition.height,
            gridTemplateColumns: `repeat(${overlayPosition.count}, var(--folder-preview-width))`,
            "--folder-preview-width": `${overlayPosition.tileWidth}px`,
            "--folder-preview-height": `${overlayPosition.height}px`,
            "--folder-preview-gap": `${PREVIEW_GAP}px`,
          } as CSSProperties}
          data-preview-count={overlayPosition.count}
          data-artist={previewedGroup.label}
          aria-hidden="true"
        >
          {overlayPreviews.slice(0, overlayPosition.count).map((gallery, index) => (
            <GalleryThumbnail
              as="span"
              className="download-artist-folder-overlay-preview"
              style={{
                "--folder-preview-origin-x": `${overlayPosition.originX - index * (overlayPosition.tileWidth + PREVIEW_GAP)}px`,
                "--folder-preview-origin-y": `${overlayPosition.originY}px`,
                "--folder-preview-tilt": `${index === 0 ? 0 : (index % 2 ? -1 : 1) * (3 + index)}deg`,
                "--folder-preview-delay": `${index * 28}ms`,
                "--folder-cover-aspect": gallery.thumbnailWidth && gallery.thumbnailHeight ? gallery.thumbnailWidth / gallery.thumbnailHeight : 2 / 3,
                zIndex: overlayPosition.count - index,
              } as CSSProperties}
              key={gallery.id}
              thumbnailKey={galleryCoverThumbnailKey(gallery)}
              consumer="downloads"
              priority="visible"
              alt=""
              expectedAspectRatio={expectedRatio(gallery)}
              data-gallery-id={Number(gallery.id)}
              client={thumbnailClient}
            />
          ))}
        </aside>,
        document.body,
      ) : null}
    </>
  );
}
