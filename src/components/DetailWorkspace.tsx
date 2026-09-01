import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { Gallery, GalleryId } from "../core/types";
import {
  galleryCoverThumbnailKey,
  sourcePageThumbnailKey,
  type ThumbnailClient,
} from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
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
  detailPreviewWindowStart,
} from "./detailPreviewWindow";
import { backend as defaultBackend, type BackendClient } from "../api/backend";
import { ProgressivePagePreview } from "./ProgressivePagePreview";
import {
  pagePreviewAspect,
  pagePreviewFrame,
  pagePreviewOrientation,
  pagePreviewSpreadDimension,
  validPagePreviewDimension,
} from "./pagePreviewFrame";
import {
  clampPagePreviewResizeBox,
  pagePreviewResizeBounds,
  resizePagePreviewBox,
  type PagePreviewResizeBox,
  type PagePreviewResizeEdge,
} from "./pagePreviewResize";

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
  onOpenRelated: (
    id: GalleryId,
    parentId: GalleryId,
    options?: { activate?: boolean },
  ) => void;
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

type PreviewWindowTransition = Readonly<{
  galleryId: GalleryId;
  start: number;
  direction: "previous" | "next";
}>;

type ResolvedPreviewDimension = Readonly<{
  galleryId: GalleryId;
  page: number;
  width: number;
  height: number;
}>;

type PreviewResizeSession = Readonly<{
  edge: PagePreviewResizeEdge;
  pointerX: number;
  pointerY: number;
  box: PagePreviewResizeBox;
}>;

const galleryPageCount = (pages: number): number =>
  Number.isFinite(pages) ? Math.max(0, Math.floor(pages)) : 0;

const previewDimensionKey = (galleryId: GalleryId, page: number): string =>
  `${galleryId}:${page}`;

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
  const previewResizeSession = useRef<PreviewResizeSession | null>(null);
  const [previewPage, setPreviewPage] = useState<number | null>(null);
  const [twoPageView, setTwoPageView] = useState(false);
  const [previewPageInput, setPreviewPageInput] = useState("1");
  const [previewViewport, setPreviewViewport] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  const [previewWindowTransition, setPreviewWindowTransition] = useState<PreviewWindowTransition | null>(null);
  const [resolvedPreviewDimensions, setResolvedPreviewDimensions] = useState<ReadonlyMap<string, ResolvedPreviewDimension>>(
    () => new Map(),
  );
  const [previewResizeBox, setPreviewResizeBox] = useState<PagePreviewResizeBox | null>(null);
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
    if (!tabs.length || event.defaultPrevented) return;
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
  const previewDimensionForPage = (page: number | null | undefined) => {
    if (!gallery || page === null || page === undefined) return undefined;
    return resolvedPreviewDimensions.get(previewDimensionKey(gallery.id, page))
      ?? gallery.pageDimensions?.find((item) => item.sourcePage === page);
  };
  const previewPageDimension = previewDimensionForPage(previewPage);
  const companionPreviewPage = previewPage !== null && previewPage < totalPageCount
    ? previewPage + 1
    : null;
  const companionPreviewDimension = previewDimensionForPage(companionPreviewPage);
  const twoPageEligible = previewPage !== null
    && companionPreviewPage !== null
    && validPagePreviewDimension(previewPageDimension)
    && validPagePreviewDimension(companionPreviewDimension)
    && pagePreviewOrientation(previewPageDimension) === "portrait"
    && pagePreviewOrientation(companionPreviewDimension) === "portrait";
  const isTwoPagePreview = twoPageView && twoPageEligible;
  const spreadDimension = isTwoPagePreview
    ? pagePreviewSpreadDimension(previewPageDimension, companionPreviewDimension)
    : undefined;
  const previewFrame = pagePreviewFrame(spreadDimension ?? previewPageDimension, previewViewport);
  const previewDisplayPages = previewPage === null
    ? []
    : isTwoPagePreview && companionPreviewPage !== null
      ? [previewPage, companionPreviewPage]
      : [previewPage];
  const previewNavigationStep = isTwoPagePreview ? 2 : 1;
  const previewResizable = gallery?.download?.state === "completed";
  const previewSourceOrientation = pagePreviewOrientation(previewPageDimension) ?? "pending";
  const previewResizeLimits = pagePreviewResizeBounds(previewViewport);
  const previewWindowSlideDirection = gallery
    && previewWindowTransition?.galleryId === gallery.id
    && previewWindowTransition.start === previewWindowStart
    ? previewWindowTransition.direction
    : "none";

  useEffect(() => {
    if (!gallery || !metadataLayout || previewLayouts.current.has(gallery.id)) return;
    previewLayouts.current.set(gallery.id, metadataLayout);
    setPreviewRevision((revision) => revision + 1);
  }, [gallery?.id, metadataLayout]);

  const setPreviewWindowStart = (start: number) => {
    if (!gallery || !previewPageCount) return;
    const currentStart = previewWindowStarts.current.get(gallery.id) ?? 1;
    const nextStart = detailPreviewWindowClampStart(start, totalPageCount, previewPageCount);
    if (currentStart === nextStart) return;
    setPreviewWindowTransition({
      galleryId: gallery.id,
      start: nextStart,
      direction: nextStart > currentStart ? "next" : "previous",
    });
    previewWindowStarts.current.set(gallery.id, nextStart);
    setPreviewRevision((revision) => revision + 1);
  };

  const shiftPreviewWindow = (direction: -1 | 1) => {
    if (!gallery || !previewPageCount) return;
    const currentStart = previewWindowStarts.current.get(gallery.id) ?? 1;
    setPreviewWindowStart(currentStart + direction * previewPageCount);
  };

  const handlePreviewDimensionResolved = useCallback((dimension: ResolvedPreviewDimension) => {
    setResolvedPreviewDimensions((current) => {
      const key = previewDimensionKey(dimension.galleryId, dimension.page);
      const existing = current.get(key);
      if (existing?.width === dimension.width && existing.height === dimension.height) return current;
      const next = new Map(current);
      next.set(key, dimension);
      while (next.size > 32) {
        const oldest = next.keys().next().value;
        if (oldest === undefined) break;
        next.delete(oldest);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    if (previewPage === null) return;
    const updateViewport = () => {
      const viewport = {
        width: window.innerWidth,
        height: window.visualViewport?.height ?? window.innerHeight,
      };
      setPreviewViewport(viewport);
      setPreviewResizeBox((current) => current
        ? clampPagePreviewResizeBox(current, viewport)
        : current);
    };
    updateViewport();
    window.addEventListener("resize", updateViewport);
    return () => window.removeEventListener("resize", updateViewport);
  }, [previewPage]);

  useEffect(() => {
    if (!twoPageView || twoPageEligible) return;
    setTwoPageView(false);
  }, [twoPageEligible, twoPageView]);

  useEffect(() => {
    if (previewPage !== null && previewResizable) return;
    previewResizeSession.current = null;
    previewDialog.current?.classList.remove("is-edge-resizing");
    setPreviewResizeBox((current) => current ? null : current);
    if (previewPage === null) {
      setTwoPageView((current) => current ? false : current);
      setResolvedPreviewDimensions((current) => current.size ? new Map() : current);
    }
  }, [previewPage, previewResizable]);

  useEffect(() => {
    const move = (event: globalThis.PointerEvent) => {
      const session = previewResizeSession.current;
      if (!session) return;
      setPreviewResizeBox(resizePagePreviewBox(
        session.box,
        session.edge,
        event.clientX - session.pointerX,
        event.clientY - session.pointerY,
        {
          width: window.innerWidth,
          height: window.visualViewport?.height ?? window.innerHeight,
        },
      ));
    };
    const finish = () => {
      if (!previewResizeSession.current) return;
      previewResizeSession.current = null;
      previewDialog.current?.classList.remove("is-edge-resizing");
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, []);

  useEffect(() => {
    if (!previewWindowTransition) return;
    const timeout = window.setTimeout(() => {
      setPreviewWindowTransition((current) => current === previewWindowTransition ? null : current);
    }, 240);
    return () => window.clearTimeout(timeout);
  }, [previewWindowTransition]);

  useEffect(() => {
    setPreviewPageInput(String(previewWindowStart));
  }, [gallery?.id, previewWindowStart]);

  const navigatePreviewPage = (direction: -1 | 1) => {
    if (previewPage === null) return;
    const requested = previewPage + direction * previewNavigationStep;
    if (direction > 0 && requested > totalPageCount) return;
    const next = direction < 0 ? Math.max(1, requested) : requested;
    if (next === previewPage) return;
    setPreviewPage(next);
  };

  const beginPagePreviewResize = (
    event: ReactPointerEvent<HTMLElement>,
    edge: PagePreviewResizeEdge,
  ) => {
    if (!previewResizable || event.button !== 0) return;
    const dialog = previewDialog.current;
    if (!dialog) return;
    const rect = dialog.getBoundingClientRect();
    const box = resizePagePreviewBox({
      left: rect.left,
      top: rect.top,
      width: rect.width,
      height: rect.height,
    }, edge, 0, 0, {
      width: window.innerWidth,
      height: window.visualViewport?.height ?? window.innerHeight,
    });
    event.preventDefault();
    event.stopPropagation();
    previewResizeSession.current = {
      edge,
      pointerX: event.clientX,
      pointerY: event.clientY,
      box,
    };
    setPreviewResizeBox(box);
    dialog.classList.add("is-edge-resizing");
  };

  const resizePagePreviewWithKeyboard = (
    event: KeyboardEvent<HTMLElement>,
    edge: Exclude<PagePreviewResizeEdge, "corner">,
  ) => {
    const changesWidth = edge === "right" && (event.key === "ArrowLeft" || event.key === "ArrowRight");
    const changesHeight = edge === "bottom" && (event.key === "ArrowUp" || event.key === "ArrowDown");
    if (!changesWidth && !changesHeight) return;
    const dialog = previewDialog.current;
    if (!dialog) return;
    event.preventDefault();
    event.stopPropagation();
    const rect = dialog.getBoundingClientRect();
    const step = event.shiftKey ? 48 : 12;
    const deltaX = event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0;
    const deltaY = event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0;
    setPreviewResizeBox(resizePagePreviewBox({
      left: rect.left,
      top: rect.top,
      width: rect.width,
      height: rect.height,
    }, edge, deltaX, deltaY, {
      width: window.innerWidth,
      height: window.visualViewport?.height ?? window.innerHeight,
    }));
  };

  const commitPreviewPageInput = () => {
    const page = Number(previewPageInput);
    if (!Number.isInteger(page) || page < 1 || page > totalPageCount) {
      setPreviewPageInput(String(previewWindowStart));
      return;
    }
    const nextStart = detailPreviewWindowStart(page, totalPageCount, previewPageCount);
    setPreviewPageInput(String(nextStart));
    setPreviewWindowStart(nextStart);
  };

  const navigateDetailWorkspace = (event: KeyboardEvent<HTMLElement>) => {
    if (
      event.defaultPrevented
      || event.nativeEvent.isComposing
      || event.ctrlKey
      || event.metaKey
      || event.altKey
      || event.shiftKey
      || (event.target instanceof Element
        && event.target.closest('input, textarea, select, [contenteditable="true"]'))
    ) return;
    const key = event.key.toLocaleLowerCase();
    const code = event.code;
    const activeIndex = activeId === null ? -1 : tabs.indexOf(activeId);
    const tabOffset = code === "KeyQ" || key === "q"
      ? -1
      : code === "KeyE" || key === "e"
        ? 1
        : 0;
    if (tabOffset && activeIndex >= 0 && tabs.length > 1) {
      const nextId = tabs[(activeIndex + tabOffset + tabs.length) % tabs.length];
      if (nextId === undefined) return;
      event.preventDefault();
      onActivate(nextId);
    }
  };

  useEffect(() => {
    if (minimized || !gallery || !previewPageCount || previewPage !== null) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (
        event.defaultPrevented
        || event.isComposing
        || event.ctrlKey
        || event.metaKey
        || event.altKey
        || event.shiftKey
        || document.querySelector("dialog[open]")
      ) return;
      const target = event.target instanceof Element
        ? event.target
        : document.activeElement instanceof Element
          ? document.activeElement
          : null;
      if (target?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      const key = event.key.toLocaleLowerCase();
      const previousWindow = event.key === "ArrowLeft" || event.code === "KeyA" || key === "a";
      const nextWindow = event.key === "ArrowRight" || event.code === "KeyD" || key === "d";
      const direction = previousWindow ? -1 : nextWindow ? 1 : 0;
      if (!direction) return;
      event.preventDefault();
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") event.stopPropagation();
      shiftPreviewWindow(direction);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [gallery, minimized, previewPage, previewPageCount, totalPageCount]);

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
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
      const key = event.key.toLocaleLowerCase();
      const previous = event.key === "ArrowLeft" || event.code === "KeyA" || key === "a";
      const nextPage = event.key === "ArrowRight" || event.code === "KeyD" || key === "d";
      if (!previous && !nextPage) return;
      const requested = previewPage + (previous ? -previewNavigationStep : previewNavigationStep);
      if (!previous && requested > totalPageCount) return;
      const next = previous ? Math.max(1, requested) : requested;
      if (next === previewPage) return;
      event.preventDefault();
      setPreviewPage(next);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [gallery, previewNavigationStep, previewPage, totalPageCount]);

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
        <section ref={workspace} className="detail-workspace" aria-label={`${gallery.title} 상세`} onKeyDown={navigateDetailWorkspace}>
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
                    <div className="preview-window">
                      <button
                        type="button"
                        className="preview-window-arrow is-previous"
                        aria-label="이전 미리보기 묶음"
                        title="이전 미리보기 묶음 (A)"
                        disabled={previewWindowStart === 1}
                        onClick={() => shiftPreviewWindow(-1)}
                      >
                        <svg aria-hidden="true" focusable="false" viewBox="0 0 20 32">
                          <path className="preview-chevron-outline" d="M15 3 5 16l10 13" />
                          <path className="preview-chevron-mark" d="M15 3 5 16l10 13" />
                        </svg>
                      </button>
                      <div className="preview-window-viewport">
                        <div
                          key={`${gallery.id}:${previewWindowStart}`}
                          className="preview-grid"
                          data-preview-columns={previewLayout.columns}
                          data-preview-orientation={previewLayout.orientation}
                          data-preview-direction={previewWindowSlideDirection}
                          onAnimationEnd={(event) => {
                            if (event.target !== event.currentTarget) return;
                            setPreviewWindowTransition((current) => (
                              current?.galleryId === gallery.id && current.start === previewWindowStart
                                ? null
                                : current
                            ));
                          }}
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
                      </div>
                      <button
                        type="button"
                        className="preview-window-arrow is-next"
                        aria-label="다음 미리보기 묶음"
                        title="다음 미리보기 묶음 (D)"
                        disabled={(previewPages.at(-1) ?? 0) >= totalPageCount}
                        onClick={() => shiftPreviewWindow(1)}
                      >
                        <svg aria-hidden="true" focusable="false" viewBox="0 0 20 32">
                          <path className="preview-chevron-outline" d="m5 3 10 13L5 29" />
                          <path className="preview-chevron-mark" d="m5 3 10 13L5 29" />
                        </svg>
                      </button>
                    </div>
                    {totalPageCount > 0 ? (
                      <nav className="preview-window-nav" aria-label="상세 페이지 탐색">
                        <label>
                          <span>페이지</span>
                          <input
                            type="number"
                            min={1}
                            max={totalPageCount}
                            inputMode="numeric"
                            aria-label="페이지 번호로 이동"
                            value={previewPageInput}
                            onChange={(event) => setPreviewPageInput(event.target.value)}
                            onBlur={commitPreviewPageInput}
                            onKeyDown={(event) => {
                              if (event.key !== "Enter") return;
                              event.preventDefault();
                              commitPreviewPageInput();
                            }}
                          />
                        </label>
                        <span aria-live="polite">{previewPages.at(0) ?? 0}–{previewPages.at(-1) ?? 0} / {totalPageCount}</span>
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
                    {gallery.download?.state === "completed" ? (
                      <span className="icon-button detail-download-complete" title="다운로드 완료" role="img" aria-label="다운로드 완료">
                        <FluentIcon glyph="\uE73E" />
                      </span>
                    ) : (
                      <button type="button" className="icon-button" title="다운로드" aria-label="다운로드" onClick={() => onQueue(gallery.id)}>
                        <FluentIcon glyph="\uE896" />
                      </button>
                    )}
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
                          title="더블클릭 또는 Enter로 상세 열기 · Ctrl/⌘+클릭으로 백그라운드 탭 열기"
                          onClick={(event) => {
                            if ((!event.ctrlKey && !event.metaKey) || event.button !== 0) return;
                            if ((event.target as Element).closest("button")) return;
                            event.preventDefault();
                            onOpenRelated(item.id, gallery.id, { activate: false });
                          }}
                          onDoubleClick={(event) => {
                            if ((event.target as Element).closest("button")) return;
                            if (event.ctrlKey || event.metaKey) return;
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
                          >
                            {item.download?.state === "completed" ? (
                              <span className="download-check" title="다운로드 완료" role="img" aria-label="다운로드 완료">
                                <GalleryStatusIcon kind="complete" />
                              </span>
                            ) : null}
                          </GalleryThumbnail>
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
        className={`page-preview-dialog${previewResizable ? " is-resizable" : ""}`}
        aria-labelledby="page-preview-title"
        data-page-preview-orientation={previewFrame.orientation}
        data-page-preview-source-orientation={previewSourceOrientation}
        data-page-preview-view={isTwoPagePreview ? "spread" : "single"}
        style={{
          "--page-preview-dialog-width": `${previewFrame.dialogWidth}px`,
          "--page-preview-dialog-height": `${previewFrame.dialogHeight}px`,
          "--page-preview-media-width": `${previewFrame.mediaWidth}px`,
          "--page-preview-media-height": `${previewFrame.mediaHeight}px`,
          "--page-preview-aspect-ratio": previewFrame.aspectRatio,
          ...(previewResizeBox ? {
            inset: "auto",
            left: `${previewResizeBox.left}px`,
            top: `${previewResizeBox.top}px`,
            width: `${previewResizeBox.width}px`,
            height: `${previewResizeBox.height}px`,
            margin: 0,
          } : {}),
        } as CSSProperties}
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
                <h2 id="page-preview-title">
                  {gallery.title} · {isTwoPagePreview ? `${previewPage}–${companionPreviewPage}페이지` : `${previewPage}페이지`}
                </h2>
              </div>
              <button ref={previewCloseButton} type="button" className="icon-button small" title="페이지 미리보기 닫기" aria-label="페이지 미리보기 닫기" onClick={() => setPreviewPage(null)}>
                <FluentIcon glyph="\uE711" />
              </button>
            </header>
            <div
              className="page-preview-media-stage"
              data-page-preview-count={previewDisplayPages.length}
              style={isTwoPagePreview ? {
                gridTemplateColumns: `${pagePreviewAspect(previewPageDimension)}fr ${pagePreviewAspect(companionPreviewDimension)}fr`,
              } : undefined}
            >
              {previewDisplayPages.map((page) => (
                <ProgressivePagePreview
                  key={`${gallery.id}:${page}`}
                  gallery={gallery}
                  page={page}
                  expectedDimension={previewDimensionForPage(page)}
                  client={thumbnailClient}
                  backend={backend}
                  onDimensionResolved={handlePreviewDimensionResolved}
                />
              ))}
            </div>
            <div className="page-preview-controls">
              <div className="page-preview-navigation">
                <button type="button" className="text-button" disabled={previewPage <= 1} onClick={() => navigatePreviewPage(-1)}>이전</button>
                <span>{isTwoPagePreview ? `${previewPage}–${companionPreviewPage}` : previewPage} / {totalPageCount}</span>
                <button
                  type="button"
                  className="text-button"
                  disabled={previewPage + previewNavigationStep > totalPageCount}
                  onClick={() => navigatePreviewPage(1)}
                >
                  다음
                </button>
              </div>
              {twoPageEligible ? (
                <button
                  type="button"
                  className="text-button page-preview-spread-toggle"
                  aria-label="두쪽 보기"
                  aria-pressed={isTwoPagePreview}
                  title={isTwoPagePreview ? "한쪽 보기로 전환" : "현재 페이지와 다음 페이지를 함께 보기"}
                  onClick={() => setTwoPageView((current) => !current)}
                >
                  <span className="page-preview-spread-icon" aria-hidden="true"><i /><i /></span>
                  <span>두쪽 보기</span>
                </button>
              ) : null}
            </div>
          </div>
        ) : null}
        {previewResizable && previewPage !== null ? (
          <>
            <div
              className="page-preview-resize-handle is-right"
              data-resize-edge="right"
              role="separator"
              tabIndex={0}
              aria-label="페이지 미리보기 너비 조절"
              aria-orientation="vertical"
              aria-valuemin={previewResizeLimits.minimumWidth}
              aria-valuemax={previewResizeLimits.maximumWidth}
              aria-valuenow={previewResizeBox?.width ?? previewFrame.dialogWidth}
              onPointerDown={(event) => beginPagePreviewResize(event, "right")}
              onKeyDown={(event) => resizePagePreviewWithKeyboard(event, "right")}
            />
            <div
              className="page-preview-resize-handle is-bottom"
              data-resize-edge="bottom"
              role="separator"
              tabIndex={0}
              aria-label="페이지 미리보기 높이 조절"
              aria-orientation="horizontal"
              aria-valuemin={previewResizeLimits.minimumHeight}
              aria-valuemax={previewResizeLimits.maximumHeight}
              aria-valuenow={previewResizeBox?.height ?? previewFrame.dialogHeight}
              onPointerDown={(event) => beginPagePreviewResize(event, "bottom")}
              onKeyDown={(event) => resizePagePreviewWithKeyboard(event, "bottom")}
            />
            <div
              className="page-preview-resize-handle is-corner"
              data-resize-edge="corner"
              aria-hidden="true"
              onPointerDown={(event) => beginPagePreviewResize(event, "corner")}
            />
          </>
        ) : null}
      </dialog>
    </>
  );
}
