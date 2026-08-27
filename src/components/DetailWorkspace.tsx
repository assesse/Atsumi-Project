import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import type { Gallery, GalleryId } from "../core/types";
import {
  galleryCoverThumbnailKey,
  sourcePageThumbnailKey,
  type ThumbnailClient,
} from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";
import { ProgressiveDetailHero } from "./ProgressiveDetailHero";
import { MetadataChip } from "./MetadataChip";
import { detailPreviewLayout, type DetailPreviewLayout } from "./detailPreviewLayout";
import { sortGalleryTags } from "./galleryCardLayout";
import { galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";
import {
  detailPreviewWindowClampStart,
  detailPreviewWindowRange,
  detailPreviewWindowSize,
} from "./detailPreviewWindow";
import { backend as defaultBackend, type BackendClient } from "../api/backend";

type DetailWorkspaceProps = {
  tabs: GalleryId[];
  activeId: GalleryId | null;
  minimized: boolean;
  galleries: ReadonlyMap<GalleryId, Gallery>;
  favoriteMetadata: ReadonlySet<string>;
  previewWidth?: number;
  relatedPreviewWidth?: number;
  thumbnailClient?: ThumbnailClient;
  backend?: BackendClient;
  onActivate: (id: GalleryId) => void;
  onClose: (id: GalleryId) => void;
  onCloseAll: () => void;
  onMinimize: () => void;
  onRestore: () => void;
  onOpenRelated: (id: GalleryId, parentId: GalleryId) => void;
  onQueue: (id: GalleryId) => void;
  onOpenDownloadFolder?: (entryId: string) => void;
  onMetadataSearch: (value: string) => void;
  onMetadataFavorite: (value: string) => void;
};

type MetadataBoxProps = {
  label: string;
  values: string[];
  type: string;
  favorite?: boolean;
  favoriteMetadata?: ReadonlySet<string>;
  onSearch: (value: string) => void;
  onFavorite: (value: string) => void;
};

const galleryPageCount = (pages: number): number =>
  Number.isFinite(pages) ? Math.max(0, Math.floor(pages)) : 0;

const metadataSearchToken = (namespace: string, value: string): string =>
  `${namespace}:${value.trim().replace(/\s+/g, "_")}`;

const relatedCoverAspectRatio = (gallery: Gallery): string => {
  const width = gallery.thumbnailWidth;
  const height = gallery.thumbnailHeight;
  if (
    typeof width === "number" && Number.isFinite(width) && width > 0
    && typeof height === "number" && Number.isFinite(height) && height > width
  ) return `${width} / ${height}`;
  return "2 / 3";
};

function MetadataBox({ label, values, type, favorite, favoriteMetadata, onSearch, onFavorite }: MetadataBoxProps) {
  return (
    <div className="metadata-box">
      <span>{label}</span>
      <div className="metadata-value">
        {values.map((value) => (
          <MetadataChip
            key={`${type}:${value}`}
            value={`${type}:${value}`}
            searchValue={["series", "character"].includes(type) ? metadataSearchToken(type, value) : undefined}
            label={["series", "character"].includes(type) ? value.replaceAll("_", " ") : value}
            favorite={favorite ?? favoriteMetadata?.has(`${type}:${value}`)}
            onSearch={onSearch}
            onToggleFavorite={onFavorite}
          />
        ))}
      </div>
    </div>
  );
}

export function DetailWorkspace(props: DetailWorkspaceProps) {
  const {
    tabs,
    activeId,
    minimized,
    galleries,
    favoriteMetadata,
    previewWidth = 220,
    relatedPreviewWidth = 240,
    thumbnailClient,
    backend = defaultBackend,
    onActivate,
    onClose,
    onCloseAll,
    onMinimize,
    onRestore,
    onOpenRelated,
    onQueue,
    onOpenDownloadFolder,
    onMetadataSearch,
    onMetadataFavorite,
  } = props;
  const workspace = useRef<HTMLElement>(null);
  const restoreButton = useRef<HTMLButtonElement>(null);
  const previousVisible = useRef(false);
  const previousTabCount = useRef(0);
  const opener = useRef<HTMLElement | null>(null);
  const previewDialog = useRef<HTMLDialogElement>(null);
  const previewCloseButton = useRef<HTMLButtonElement>(null);
  const previewOpener = useRef<HTMLButtonElement | null>(null);
  const previewClosingInternally = useRef(false);
  const [previewPage, setPreviewPage] = useState<number | null>(null);
  const previewLayouts = useRef(new Map<GalleryId, DetailPreviewLayout>());
  const previewWindowStarts = useRef(new Map<GalleryId, number>());
  const [, setPreviewRevision] = useState(0);

  useEffect(() => {
    const visible = tabs.length > 0 && !minimized;
    if (previousTabCount.current === 0 && tabs.length > 0) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    if (visible && !previousVisible.current) {
      window.requestAnimationFrame(() => {
        workspace.current?.querySelector<HTMLElement>("[role='tab'][aria-selected='true']")?.focus();
      });
    } else if (!visible && minimized) {
      window.requestAnimationFrame(() => restoreButton.current?.focus());
    } else if (previousTabCount.current > 0 && tabs.length === 0) {
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else document.querySelector<HTMLElement>(".view-header input")?.focus();
      });
    }
    previousVisible.current = visible;
    previousTabCount.current = tabs.length;
  }, [minimized, tabs.length]);

  useEffect(() => {
    const activeTabs = new Set(tabs);
    for (const id of previewLayouts.current.keys()) {
      if (!activeTabs.has(id)) previewLayouts.current.delete(id);
    }
    for (const id of previewWindowStarts.current.keys()) {
      if (!activeTabs.has(id)) previewWindowStarts.current.delete(id);
    }
  }, [tabs]);

  useEffect(() => {
    workspace.current?.querySelector<HTMLElement>(".detail-body")?.scrollTo?.({ top: 0, left: 0 });
    if (!minimized && activeId !== null) {
      window.requestAnimationFrame(() => {
        workspace.current?.querySelector<HTMLElement>("[role='tab'][aria-selected='true']")?.focus();
      });
    }
  }, [activeId, minimized]);

  const navigateTabs = (event: KeyboardEvent<HTMLElement>, index: number) => {
    if (!tabs.length) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight") nextIndex = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") nextIndex = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = tabs.length - 1;
    if (nextIndex === null) return;
    event.preventDefault();
    const nextId = tabs[nextIndex];
    if (nextId === undefined) return;
    onActivate(nextId);
    const tabsHost = event.currentTarget.closest(".detail-tabs");
    window.requestAnimationFrame(() => {
      if (nextIndex !== null) tabsHost?.querySelectorAll<HTMLElement>("[role='tab']")[nextIndex]?.focus();
    });
  };

  const gallery = activeId === null ? undefined : galleries.get(activeId);
  const totalPageCount = gallery ? galleryPageCount(gallery.pages) : 0;
  const pageOneDimension = gallery?.pageDimensions?.find((page) => page.sourcePage === 1);
  const metadataReady = gallery?.pageDimensions !== undefined;
  const metadataLayout = gallery && metadataReady
    ? detailPreviewLayout((gallery.pageDimensions ?? []).slice(0, 8))
    : undefined;
  const lockedPreviewLayout = gallery ? previewLayouts.current.get(gallery.id) : undefined;
  const previewLayout = lockedPreviewLayout ?? metadataLayout ?? { columns: 3 as const, orientation: "pending" as const };
  const previewPageCount = gallery && metadataReady
    ? detailPreviewWindowSize(totalPageCount, previewLayout.columns)
    : 0;
  const previewWindowStart = gallery ? (previewWindowStarts.current.get(gallery.id) ?? 1) : 1;
  const previewPages = detailPreviewWindowRange(previewWindowStart, totalPageCount, previewPageCount);

  useEffect(() => {
    if (!gallery || !metadataLayout || previewLayouts.current.has(gallery.id)) return;
    previewLayouts.current.set(gallery.id, metadataLayout);
    setPreviewRevision((revision) => revision + 1);
  }, [gallery?.id, metadataLayout]);

  const setPreviewWindowStart = (start: number) => {
    if (!gallery || !previewPageCount) return;
    const nextStart = detailPreviewWindowClampStart(start, totalPageCount, previewPageCount);
    if (previewWindowStarts.current.get(gallery.id) === nextStart) return;
    previewWindowStarts.current.set(gallery.id, nextStart);
    setPreviewRevision((revision) => revision + 1);
  };

  useEffect(() => {
    const node = previewDialog.current;
    if (!node) return;
    if (previewPage !== null && (totalPageCount === 0 || previewPage > totalPageCount)) {
      setPreviewPage(null);
      return;
    }
    if (previewPage !== null && gallery && !node.open) {
      node.showModal();
      window.requestAnimationFrame(() => previewCloseButton.current?.focus());
    } else if ((previewPage === null || !gallery) && node.open) {
      previewClosingInternally.current = true;
      node.close();
      const target = previewOpener.current;
      previewOpener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else workspace.current?.querySelector<HTMLElement>("[role='tab'][aria-selected='true']")?.focus();
      });
    }
  }, [gallery, previewPage, totalPageCount]);

  useEffect(() => {
    if (previewPage === null || !gallery) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      const next = event.key === "ArrowLeft" ? previewPage - 1 : previewPage + 1;
      if (next < 1 || next > totalPageCount) return;
      event.preventDefault();
      setPreviewPage(next);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [gallery, previewPage, previewPageCount, totalPageCount]);

  if (!tabs.length) return null;

  return (
    <>
      {minimized ? (
        <button ref={restoreButton} type="button" className="detail-restore" onClick={onRestore}>
          <FluentIcon glyph="\uE8A7" />
          <span>상세 탭 {tabs.length}</span>
        </button>
      ) : null}
      {!minimized && gallery ? (
        <section ref={workspace} className="detail-workspace" aria-label={`${gallery.title} 상세`}>
          <div className="detail-tabbar">
            <div className="detail-tabs" role="tablist">
              {tabs.map((id, index) => {
                const tab = galleries.get(id);
                if (!tab) return null;
                return (
                  <div key={id} role="presentation" className={`detail-tab${id === activeId ? " is-active" : ""}`}>
                    <button
                      type="button"
                      role="tab"
                      id={`detail-tab-${id}`}
                      aria-controls={`detail-panel-${id}`}
                      tabIndex={id === activeId ? 0 : -1}
                      aria-selected={id === activeId}
                      className="tab-activate"
                      onClick={() => onActivate(id)}
                      onKeyDown={(event) => navigateTabs(event, index)}
                    >
                      {tab.title}
                    </button>
                    <button
                      type="button"
                      className="tab-close"
                      aria-label={`${tab.title} 탭 닫기`}
                      onClick={(event) => {
                        event.stopPropagation();
                        onClose(id);
                        window.requestAnimationFrame(() => {
                          workspace.current?.querySelector<HTMLElement>("[role='tab'][aria-selected='true']")?.focus();
                        });
                      }}
                    >
                      ×
                    </button>
                  </div>
                );
              })}
            </div>
            <button type="button" className="icon-button small" title="상세 최소화" aria-label="상세 최소화" onClick={onMinimize}>
              <FluentIcon glyph="\uE921" />
            </button>
            <button type="button" className="icon-button small" title="상세 전체 닫기" aria-label="상세 전체 닫기" onClick={onCloseAll}>
              <FluentIcon glyph="\uE711" />
            </button>
          </div>
          <div
            className="detail-body"
            data-thumbnail-scroll-root
            id={`detail-panel-${gallery.id}`}
            role="tabpanel"
            aria-labelledby={`detail-tab-${gallery.id}`}
          >
            <div className="detail-layout">
              <section className="detail-media">
                <ProgressiveDetailHero gallery={gallery} pageDimension={pageOneDimension} client={thumbnailClient} backend={backend} />
                {!metadataReady ? (
                  <div className="detail-preview-loading" role="status" aria-label="추가 페이지 미리보기 준비 중">
                    <span className="spinner" aria-hidden="true" />
                  </div>
                ) : (
                  <>
                    <div
                      className="preview-grid"
                      data-preview-columns={previewLayout.columns}
                      data-preview-orientation={previewLayout.orientation}
                    >
                      {previewPages.map((page, index) => {
                        const dimension = gallery?.pageDimensions?.find((item) => item.sourcePage === page);
                        const fallback = previewLayout.columns === 2
                          ? { width: 16, height: 9 }
                          : { width: 2, height: 3 };
                        return (
                          <button
                            key={page}
                            type="button"
                            className="preview-thumb"
                            title={`${page}페이지 확대`}
                            onClick={(event) => {
                              previewOpener.current = event.currentTarget;
                              setPreviewPage(page);
                            }}
                          >
                            <GalleryThumbnail
                              as="span"
                              thumbnailKey={sourcePageThumbnailKey(gallery, page)}
                              consumer="detail"
                              priority={index < previewLayout.columns ? "visible" : "prefetch"}
                              client={thumbnailClient}
                              sizing="intrinsic"
                              expectedAspectRatio={dimension?.width !== undefined && dimension?.height !== undefined
                                ? { width: dimension.width, height: dimension.height }
                                : fallback}
                              alt={`${gallery.title} ${page}페이지 미리보기`}
                            />
                            <span>{page}</span>
                          </button>
                        );
                      })}
                    </div>
                    {totalPageCount > 0 ? (
                      <nav className="preview-window-nav" aria-label="상세 페이지 탐색">
                        <button type="button" className="text-button" onClick={() => setPreviewWindowStart(Math.max(1, previewWindowStart - previewPageCount))} disabled={previewWindowStart === 1}>이전 묶음</button>
                        <span>{previewPages.at(0) ?? 0}–{previewPages.at(-1) ?? 0} / {totalPageCount}</span>
                        <button type="button" className="text-button" onClick={() => setPreviewWindowStart(previewWindowStart + previewPageCount)} disabled={(previewPages.at(-1) ?? 0) >= totalPageCount}>다음 묶음</button>
                      </nav>
                    ) : null}
                  </>
                )}
              </section>
              <section className="detail-info">
                <div className="detail-title-row">
                  <div>
                    <span className="eyebrow">FLOATING DETAIL</span>
                    <h2>
                      {gallery.title}
                      <br />
                      {gallery.subtitle}
                    </h2>
                    <p>#{gallery.id} · {gallery.pages} pages</p>
                  </div>
                  <div className="detail-title-actions">
                    <button type="button" className="icon-button" title="다운로드" aria-label="다운로드" onClick={() => onQueue(gallery.id)}>
                      <FluentIcon glyph="\uE896" />
                    </button>
                    {gallery.download && gallery.download.state !== "quarantined" && onOpenDownloadFolder ? (
                      <button
                        type="button"
                        className="icon-button"
                        title="저장 폴더 열기"
                        aria-label="저장 폴더 열기"
                        onClick={() => onOpenDownloadFolder(gallery.download!.entryId)}
                      >
                        <FluentIcon glyph="\uE8B7" />
                      </button>
                    ) : null}
                  </div>
                </div>
                <div className="detail-metadata-layout">
                  <div className="detail-metadata-primary">
                    <MetadataBox label="작가" values={[gallery.artist]} type="artist" favorite={gallery.favorite} onSearch={onMetadataSearch} onFavorite={onMetadataFavorite} />
                    <MetadataBox label="그룹" values={gallery.group ? [gallery.group] : []} type="group" favoriteMetadata={favoriteMetadata} onSearch={onMetadataSearch} onFavorite={onMetadataFavorite} />
                    <MetadataBox label="언어" values={[gallery.language]} type="language" onSearch={onMetadataSearch} onFavorite={onMetadataFavorite} />
                    <MetadataBox label="시리즈" values={gallery.series ?? []} type="series" favoriteMetadata={favoriteMetadata} onSearch={onMetadataSearch} onFavorite={onMetadataFavorite} />
                    <MetadataBox label="캐릭터" values={gallery.characters ?? []} type="character" favoriteMetadata={favoriteMetadata} onSearch={onMetadataSearch} onFavorite={onMetadataFavorite} />
                  </div>
                  <div className="metadata-box tags-box detail-metadata-tags">
                    <span>태그</span>
                    <div className="metadata-value">
                      {sortGalleryTags(gallery.tags, favoriteMetadata).map((tag) => (
                        <MetadataChip key={tag.value} value={tag.value} kind="tag" favorite={tag.favorite} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} />
                      ))}
                    </div>
                  </div>
                </div>
                <section className="related-section">
                  <div className="section-heading">
                    <h3>Related galleries</h3>
                  </div>
                  <div className="related-list">
                    {(gallery.relatedIds ?? [])
                      .flatMap((id) => {
                        const item = galleries.get(id);
                        return item ? [item] : [];
                      })
                      .slice(0, 5)
                      .map((item) => (
                        <article
                          key={item.id}
                          className="related-card"
                          tabIndex={0}
                          style={{
                            ...galleryPreviewPresetStyle(galleryPreviewPreset(previewWidth)),
                            "--related-preview-width": `${relatedPreviewWidth}px`,
                            "--related-cover-aspect-ratio": relatedCoverAspectRatio(item),
                          } as CSSProperties}
                          title="더블클릭 또는 Enter로 상세 열기"
                          onDoubleClick={(event) => {
                            if ((event.target as Element).closest("button")) return;
                            onOpenRelated(item.id, gallery.id);
                          }}
                          onKeyDown={(event) => {
                            if (event.key !== "Enter" || event.target !== event.currentTarget) return;
                            event.preventDefault();
                            onOpenRelated(item.id, gallery.id);
                          }}
                        >
                          <GalleryThumbnail
                            className="related-cover"
                            thumbnailKey={galleryCoverThumbnailKey(item)}
                            consumer="detail"
                            priority="prefetch"
                            client={thumbnailClient}
                            sizing="container"
                            alt={`${item.title} 표지`}
                          />
                          <div className="related-copy card-content">
                            <div className="card-title"><strong>{item.title}</strong>{item.subtitle ? <span className="title-sub">{item.subtitle}</span> : null}</div>
                            <div className="card-byline">
                              <MetadataChip value={`artist:${item.artist}`} label={item.artist} kind="byline" favorite={item.favorite} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} />
                              {item.group ? <MetadataChip value={`group:${item.group}`} label={item.group} kind="byline" favorite={favoriteMetadata.has(`group:${item.group}`)} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} /> : null}
                            </div>
                            <div className="tag-list">
                              {sortGalleryTags(item.tags, favoriteMetadata).slice(0, 4).map((tag) => (
                                <MetadataChip key={tag.value} value={tag.value} kind="tag" favorite={tag.favorite} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} />
                              ))}
                            </div>
                            <div className="meta-bottom"><span>{item.pages}p</span><span>#{item.id}</span></div>
                          </div>
                        </article>
                      ))}
                  </div>
                </section>
              </section>
            </div>
          </div>
        </section>
      ) : null}
      <dialog
        ref={previewDialog}
        className="page-preview-dialog"
        aria-labelledby="page-preview-title"
        onCancel={(event) => {
          event.preventDefault();
          setPreviewPage(null);
        }}
        onClose={() => {
          if (previewClosingInternally.current) {
            previewClosingInternally.current = false;
            return;
          }
          setPreviewPage(null);
          const target = previewOpener.current;
          previewOpener.current = null;
          window.requestAnimationFrame(() => target?.isConnected && target.focus());
        }}
      >
        {gallery && previewPage !== null ? (
          <div className="page-preview-dialog-body">
            <header className="dialog-header">
              <div>
                <span className="eyebrow">PAGE PREVIEW</span>
                <h2 id="page-preview-title">{gallery.title} · {previewPage}페이지</h2>
              </div>
              <button ref={previewCloseButton} type="button" className="icon-button small" title="페이지 미리보기 닫기" aria-label="페이지 미리보기 닫기" onClick={() => setPreviewPage(null)}>
                <FluentIcon glyph="\uE711" />
              </button>
            </header>
            <GalleryThumbnail
              className="page-preview-media"
              thumbnailKey={sourcePageThumbnailKey(gallery, previewPage)}
              consumer="detail"
              priority="critical"
              client={thumbnailClient}
              alt={`${gallery.title} ${previewPage}페이지 확대 미리보기`}
            />
            <div className="page-preview-controls">
              <button type="button" className="text-button" disabled={previewPage <= 1} onClick={() => setPreviewPage(previewPage - 1)}>이전</button>
              <span>{previewPage} / {totalPageCount}</span>
              <button type="button" className="text-button" disabled={previewPage >= totalPageCount} onClick={() => setPreviewPage(previewPage + 1)}>다음</button>
            </div>
          </div>
        ) : null}
      </dialog>
    </>
  );
}
