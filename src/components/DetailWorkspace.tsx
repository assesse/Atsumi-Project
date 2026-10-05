import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { Gallery, GalleryId } from "../core/types";
import { runningDownloadStates } from "../state/downloadCancellation";
import {
  galleryCoverThumbnailKey,
  sourcePageThumbnailKey,
  type ThumbnailClient,
} from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { CommunityReviewButton } from "../features/community/CommunityReviewButton";
import { BookmarkButton } from "../features/personalLibrary/BookmarkButton";
import { GalleryProcessingBadge, GalleryProcessingSurface } from "./GalleryProcessingBadge";
import type { BackgroundOpenOptions } from "../state/downloadStatus";
import { adjacentPreviewPages, readDetailPositions, saveDetailPosition } from "../state/detailPositions";
import { useThumbnailClient } from "../thumbnail/ThumbnailProvider";
import { GalleryThumbnail } from "./GalleryThumbnail";
import { orderGalleryArtists } from "./GalleryArtists";
import { ProgressiveDetailHero } from "./ProgressiveDetailHero";
import { MetadataChip } from "./MetadataChip";
import { detailPreviewLayout, type DetailPreviewLayout } from "./detailPreviewLayout";
import "./DetailWorkspace.css";
import { MovingTabs, WindowMotion, animateWindowClose } from "./WindowMotion";
import { CardContextMenu, isCardControl, useCardContextMenu } from "./CardContextMenu";
import { useAlbumMenuItems } from "./albumMenuItems";
import { attachDetailScrollSnap } from "./detailScrollSnap";
import "./PagePreviewOverlay.css";
import { nextPagePreviewAnchor, pagePreviewSlots } from "./pagePreviewNavigation";
import { sortGalleryTags, splitGalleryTitle } from "./galleryCardLayout";
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
  onExclude?: (id: GalleryId) => void;
  onCancelDownload?: (id: GalleryId) => void;
  pendingDownloadEntryIds?: ReadonlySet<string>;
  cancellingDownloadEntryIds?: ReadonlySet<string>;
  onOpenDownloadFolder?: (entryId: string) => void;
  onSetRepresentativePreview?: (galleryId: GalleryId, sourcePage: number | null) => Promise<boolean>;
  onMetadataSearch: (value: string, options?: BackgroundOpenOptions) => void;
  onMetadataFavorite: (value: string) => void;
  pageOpenRequest?: { galleryId: GalleryId; page: number; sequence: number } | null;
};

type MetadataBoxProps = {
  label: string;
  values: string[];
  type: string;
  favorite?: boolean;
  favoriteMetadata?: ReadonlySet<string>;
  onSearch: (value: string, options?: BackgroundOpenOptions) => void;
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

const selectedDetailTab = (workspace: HTMLElement | null) =>
  [...(workspace?.querySelectorAll<HTMLElement>("[role='tab'][aria-selected='true']") ?? [])]
    .find((tab) => !tab.closest("[inert]"));

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
    <div className="metadata-box" data-tour-follow-kind={type === "group" ? "group" : undefined}>
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
    onCancelDownload,
    pendingDownloadEntryIds,
    cancellingDownloadEntryIds,
    onOpenDownloadFolder,
    onSetRepresentativePreview,
    onMetadataSearch,
    onMetadataFavorite,
  } = props;
  const sharedThumbnailClient = useThumbnailClient(thumbnailClient);
  const relatedMenu = useCardContextMenu();
  const [relatedMenuId, setRelatedMenuId] = useState<GalleryId | null>(null);
  const relatedMenuGallery = relatedMenuId === null ? undefined : galleries.get(relatedMenuId);
  const relatedMenuItems = useAlbumMenuItems(relatedMenuGallery, {
    open: () => { if (relatedMenuId !== null && activeId !== null) onOpenRelated(relatedMenuId, activeId); },
    background: () => { if (relatedMenuId !== null && activeId !== null) onOpenRelated(relatedMenuId, activeId, { activate: false }); },
    queue: () => { if (relatedMenuId !== null) onQueue(relatedMenuId); },
    exclude: props.onExclude ? () => { if (relatedMenuId !== null) props.onExclude?.(relatedMenuId); } : undefined,
    pending: relatedMenuGallery?.download ? pendingDownloadEntryIds?.has(relatedMenuGallery.download.entryId) : false,
  });
  const [closeRequest, setCloseRequest] = useState<GalleryId | "all" | null>(null);
  const closeDialog = useRef<HTMLDialogElement>(null);
  const closeCancelButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (closeRequest !== null && !closeDialog.current?.open) {
      closeDialog.current?.showModal();
      closeCancelButton.current?.focus({ preventScroll: true });
    }
    else if (closeRequest === null && closeDialog.current?.open) closeDialog.current.close();
  }, [closeRequest]);
  const requestClose = (id: GalleryId | "all") => {
    if (id === "all" && tabs.length > 1) setCloseRequest(id);
    else if (id === "all") onCloseAll();
    else onClose(id);
  };
  const workspace = useRef<HTMLElement>(null);
  const scrollSaveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const restoreButton = useRef<HTMLButtonElement>(null);
  const previousVisible = useRef(false);
  const previousTabCount = useRef(0);
  const opener = useRef<HTMLElement | null>(null);
  const previewDialog = useRef<HTMLDialogElement>(null);
  const previewCloseButton = useRef<HTMLButtonElement>(null);
  const previewOpener = useRef<HTMLButtonElement | null>(null);
  const previewClosingInternally = useRef(false);
  const previewResizeSession = useRef<PreviewResizeSession | null>(null);
  const representativeSaveInFlight = useRef(false);
  const [representativeSave, setRepresentativeSave] = useState<{ galleryId: GalleryId; status: "busy" | "error" } | null>(null);
  const [previewPage, setPreviewPage] = useState<number | null>(null);
  const cancelPreviewExit = useRef<(() => void) | null>(null);
  const requestPreviewClose = useCallback(() => {
    if (previewDialog.current?.dataset.windowClosing === "true") return;
    if (previewDialog.current) previewDialog.current.dataset.windowClosing = "true";
    cancelPreviewExit.current = animateWindowClose(previewDialog.current, () => setPreviewPage(null));
  }, []);
  useLayoutEffect(() => {
    cancelPreviewExit.current?.();
    cancelPreviewExit.current = null;
    if (previewDialog.current) delete previewDialog.current.dataset.windowClosing;
    return () => { cancelPreviewExit.current?.(); };
  }, [activeId, previewPage]);
  const [twoPageView, setTwoPageView] = useState(false);
  const [readingDirection, setReadingDirection] = useState<"ltr" | "rtl">(() => {
    try { return localStorage.getItem("atsumi.pagePreview.readingDirection") === "rtl" ? "rtl" : "ltr"; }
    catch { return "ltr"; }
  });
  const [previewControlsVisible, setPreviewControlsVisible] = useState(false);
  const [previewControlsPinned, setPreviewControlsPinned] = useState(() => {
    try { return localStorage.getItem("atsumi.pagePreview.controlsPinned") === "true"; }
    catch { return false; }
  });
  const [previewArrowPulse, setPreviewArrowPulse] = useState<{ side: "left" | "right"; sequence: number } | null>(null);
  useEffect(() => {
    if (!previewArrowPulse) return;
    const timeout = window.setTimeout(() => setPreviewArrowPulse(null), 300);
    return () => window.clearTimeout(timeout);
  }, [previewArrowPulse]);
  useEffect(() => {
    try { localStorage.setItem("atsumi.pagePreview.controlsPinned", String(previewControlsPinned)); } catch { /* optional preference */ }
  }, [previewControlsPinned]);
  const previewControlsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const revealPreviewControls = () => {
    setPreviewControlsVisible(true);
    if (previewControlsTimer.current) clearTimeout(previewControlsTimer.current);
    const hide = () => {
      const node = previewDialog.current;
      if (node?.querySelector('[aria-expanded="true"], input:focus, textarea:focus, button:focus-visible')) {
        previewControlsTimer.current = setTimeout(hide, 500);
      } else setPreviewControlsVisible(false);
    };
    previewControlsTimer.current = setTimeout(hide, 1800);
  };
  useEffect(() => {
    setPreviewControlsVisible(false);
    return () => { if (previewControlsTimer.current) clearTimeout(previewControlsTimer.current); };
  }, [previewPage === null, activeId]);
  useEffect(() => {
    try { localStorage.setItem("atsumi.pagePreview.readingDirection", readingDirection); } catch { /* optional preference */ }
  }, [readingDirection]);
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
  const initialPreviewPositions = useMemo(() => new Map(Array.from(readDetailPositions(), ([id, value]) => [id as GalleryId, value.previewStart])), []);
  const previewWindowStarts = useRef(initialPreviewPositions);
  const [, setPreviewRevision] = useState(0);

  useEffect(() => {
    const visible = tabs.length > 0 && !minimized;
    if (previousTabCount.current === 0 && tabs.length > 0) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    if (visible && !previousVisible.current) {
      window.requestAnimationFrame(() => {
        selectedDetailTab(workspace.current)?.focus();
      });
    } else if (!visible && minimized && previousVisible.current) {
      window.requestAnimationFrame(() => restoreButton.current?.focus());
    } else if (previousTabCount.current > 0 && tabs.length === 0) {
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else document.querySelector<HTMLElement>(".gallery-viewport")?.focus();
      });
    }
    previousVisible.current = visible;
    previousTabCount.current = tabs.length;
  }, [minimized, tabs.length]);

  useEffect(() => {
    const activeTabs = new Set([...tabs, ...Array.from(previewWindowStarts.current.keys()).slice(-32)]);
    for (const id of previewLayouts.current.keys()) {
      if (!activeTabs.has(id)) previewLayouts.current.delete(id);
    }
    for (const id of previewWindowStarts.current.keys()) {
      if (!activeTabs.has(id)) previewWindowStarts.current.delete(id);
    }
  }, [tabs]);

  useLayoutEffect(() => {
    const body = workspace.current?.querySelector<HTMLElement>(".detail-body");
    const panel = body?.querySelector<HTMLElement>(".detail-layout");
    const motion = !minimized && panel?.animate && !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
      ? panel.animate([{ opacity: .65, translate: "0 4px" }, { opacity: 1, translate: "0 0" }], { duration: 130, easing: "ease-out" }) : null;
    if (body && activeId !== null) body.scrollTop = readDetailPositions().get(activeId)?.scrollTop ?? 0;
    if (!minimized && activeId !== null) {
      window.requestAnimationFrame(() => {
        selectedDetailTab(workspace.current)?.focus();
      });
    }
    return () => {
      motion?.cancel();
      clearTimeout(scrollSaveTimer.current);
      if (body && activeId !== null && !minimized) saveDetailPosition(activeId, { scrollTop: body.scrollTop });
    };
  }, [activeId, minimized]);

  useEffect(() => {
    if (minimized || activeId === null) return;
    const focusForeground = () => {
      if (document.visibilityState === "hidden") return;
      if (document.activeElement instanceof Element
        && document.activeElement.closest('[role="dialog"][data-gallery-shortcuts-suspended]')) return;
      const preview = previewDialog.current;
      if (preview?.open) {
        if ([...document.querySelectorAll("dialog[open]")].some((dialog) => dialog !== preview)) return;
        // Preserve comment/collection editors. On return from another app the
        // native WebView may instead leave focus on body or a background tab.
        if (!preview.contains(document.activeElement)) preview.focus({ preventScroll: true });
        return;
      }
      if (document.querySelector('dialog[open], .activity-panel, [role="dialog"][data-gallery-shortcuts-suspended]')) return;
      if (!workspace.current?.contains(document.activeElement)) {
        selectedDetailTab(workspace.current)?.focus({ preventScroll: true });
      }
    };
    window.addEventListener("focus", focusForeground);
    document.addEventListener("visibilitychange", focusForeground);
    return () => {
      window.removeEventListener("focus", focusForeground);
      document.removeEventListener("visibilitychange", focusForeground);
    };
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
  useEffect(() => {
    if (minimized || activeId === null) return;
    const body = workspace.current?.querySelector<HTMLElement>(".detail-body");
    if (body) return attachDetailScrollSnap(body);
  }, [activeId, minimized, Boolean(gallery)]);
  const participatingArtists = useMemo(
    () => gallery ? orderGalleryArtists(gallery.artist, gallery.artists, favoriteMetadata) : [],
    [gallery?.artist, gallery?.artists, favoriteMetadata],
  );
  const totalPageCount = gallery ? galleryPageCount(gallery.pages) : 0;
  const consumedPageRequest = useRef<number | null>(null);
  useEffect(() => {
    const request = props.pageOpenRequest;
    if (!request || consumedPageRequest.current === request.sequence || request.galleryId !== activeId || minimized || !gallery) return;
    consumedPageRequest.current = request.sequence;
    setTwoPageView(false);
    setPreviewPage(request.page > 0 && request.page <= totalPageCount ? request.page : null);
  }, [props.pageOpenRequest, activeId, minimized, gallery, totalPageCount]);
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
  const primaryPreviewPage = Math.max(1, previewPage ?? 1);
  const previewPageDimension = previewDimensionForPage(primaryPreviewPage);
  const isTwoPagePreview = twoPageView;
  const previewDisplayPages = pagePreviewSlots(previewPage, totalPageCount, isTwoPagePreview, readingDirection);
  const actualPreviewPages = previewDisplayPages.filter((page): page is number => page !== null);
  const previewPageLabel = actualPreviewPages.length > 1
    ? `${Math.min(...actualPreviewPages)}–${Math.max(...actualPreviewPages)}` : String(actualPreviewPages[0] ?? primaryPreviewPage);
  const previewSlotDimensions = previewDisplayPages.map((page) => previewDimensionForPage(page ?? primaryPreviewPage) ?? { width: 2, height: 3 });
  const spreadDimension = isTwoPagePreview
    ? pagePreviewSpreadDimension(previewSlotDimensions[0], previewSlotDimensions[1])
    : undefined;
  const previewFrame = pagePreviewFrame(spreadDimension ?? previewPageDimension, previewViewport);
  useEffect(() => {
    if (!gallery || minimized || previewPage === null || gallery.download?.state === "completed") return;
    const last = Math.min(totalPageCount, previewPage + (isTwoPagePreview ? 1 : 0));
    const releases = adjacentPreviewPages(primaryPreviewPage, last, totalPageCount).map((page) =>
      sharedThumbnailClient.subscribe({ key: sourcePageThumbnailKey(gallery, page), consumer: "detail", priority: "prefetch" }, () => {}));
    return () => releases.forEach((release) => release());
  }, [gallery?.id, gallery?.thumbnailKey, gallery?.download?.state, minimized, previewPage, primaryPreviewPage, isTwoPagePreview, totalPageCount, sharedThumbnailClient]);
  const previewResizable = gallery?.download?.state === "completed";
  useEffect(() => {
    if (!gallery || !previewResizable) return;
    setResolvedPreviewDimensions((previous) => {
      const next = new Map(previous);
      for (const [key,value] of next) if (value.galleryId === gallery.id) next.delete(key);
      return next;
    });
    previewLayouts.current.delete(gallery.id);
  }, [gallery?.id, previewResizable, gallery?.download?.revision]);
  const representativeBusy = representativeSave?.status === "busy";
  const representativeError = representativeSave?.galleryId === gallery?.id && representativeSave?.status === "error";
  const currentManualRepresentative = gallery?.representativePreview?.mode === "manual"
    && gallery.representativePreview.entryId === gallery.download?.entryId
    && gallery.representativePreview.sourcePage === primaryPreviewPage
    && gallery.representativePreview.manualSourcePage === primaryPreviewPage;
  const previewSourceOrientation = pagePreviewOrientation(previewPageDimension) ?? "pending";
  const previewResizeLimits = pagePreviewResizeBounds(previewViewport);
  const previewWindowSlideDirection = gallery
    && previewWindowTransition?.galleryId === gallery.id
    && previewWindowTransition.start === previewWindowStart
    ? previewWindowTransition.direction
    : "none";

  useEffect(() => {
    setRepresentativeSave((current) => current?.status === "error" ? null : current);
  }, [activeId, previewPage]);

  const setRepresentativePreview = async (sourcePage: number | null) => {
    if (!gallery || !previewResizable || !onSetRepresentativePreview || representativeSaveInFlight.current) return;
    representativeSaveInFlight.current = true;
    setRepresentativeSave({ galleryId: gallery.id, status: "busy" });
    try {
      const saved = await onSetRepresentativePreview(gallery.id, sourcePage);
      setRepresentativeSave(saved ? null : { galleryId: gallery.id, status: "error" });
    } catch {
      setRepresentativeSave({ galleryId: gallery.id, status: "error" });
    } finally {
      representativeSaveInFlight.current = false;
    }
  };

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
    saveDetailPosition(gallery.id, { previewStart: nextStart });
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

  const navigatePreviewPage = useCallback((direction: -1 | 1) => {
    if (previewPage === null) return;
    const next = nextPagePreviewAnchor(previewPage, totalPageCount, isTwoPagePreview, direction);
    if (next === null) return;
    setPreviewPage(next);
    const side = (direction < 0) === (readingDirection === "ltr") ? "left" : "right";
    setPreviewArrowPulse((previous) => ({ side, sequence: (previous?.sequence ?? 0) + 1 }));
  }, [previewPage, totalPageCount, isTwoPagePreview, readingDirection]);

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
    if (document.documentElement.dataset.tutorialOpen === "true") return;
    if (
      event.defaultPrevented
      || event.nativeEvent.isComposing
      || event.ctrlKey
      || event.metaKey
      || event.altKey
      || event.shiftKey
      || (event.target instanceof Element
        && event.target.closest('input, textarea, select, [contenteditable="true"], [data-gallery-shortcuts-suspended]'))
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
      if (document.documentElement.dataset.tutorialOpen === "true" || document.querySelector("[data-card-context-menu]")) return;
      if (
        event.defaultPrevented
        || event.isComposing
        || event.ctrlKey
        || event.metaKey
        || event.altKey
        || event.shiftKey
        || document.querySelector("dialog[open]")
        || document.querySelector(".activity-panel")
      ) return;
      const target = event.target instanceof Element
        ? event.target
        : document.activeElement instanceof Element
          ? document.activeElement
          : null;
      if (target?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [data-gallery-shortcuts-suspended]')) return;
      const key = event.key.toLocaleLowerCase();
      if ((event.code === "KeyQ" || key === "q" || event.code === "KeyE" || key === "e") && tabs.length > 1 && activeId !== null) {
        event.preventDefault();
        event.stopPropagation();
        onActivate(tabs[(tabs.indexOf(activeId) + (event.code === "KeyQ" || key === "q" ? -1 : 1) + tabs.length) % tabs.length]!);
        return;
      }
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
  }, [gallery, minimized, previewPage, previewPageCount, totalPageCount, tabs, activeId, onActivate]);

  useEffect(() => {
    const node = previewDialog.current;
    if (!node) return;
    if (previewPage !== null && (totalPageCount === 0 || previewPage > totalPageCount)) {
      setPreviewPage(null);
      return;
    }
    if (previewPage !== null && gallery && !node.open) {
      node.showModal();
      window.requestAnimationFrame(() => node.focus());
    } else if ((previewPage === null || !gallery) && node.open) {
      previewClosingInternally.current = true;
      node.close();
      const target = previewOpener.current;
      previewOpener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else selectedDetailTab(workspace.current)?.focus();
      });
    }
  }, [gallery, previewPage, totalPageCount]);

  useEffect(() => {
    if (previewPage === null || !gallery) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (document.documentElement.dataset.tutorialOpen === "true" || document.querySelector("[data-card-context-menu], [data-window-closing='true']")) return;
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
      const preview = previewDialog.current;
      if (!preview?.open || [...document.querySelectorAll("dialog[open]")].some((dialog) => dialog !== preview)) return;
      const target = event.target instanceof Element ? event.target : document.activeElement;
      if (target instanceof Element && target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [data-gallery-shortcuts-suspended], [data-resize-edge]')) return;
      const key = event.key.toLocaleLowerCase();
      const left = event.key === "ArrowLeft" || event.code === "KeyA" || key === "a";
      const right = event.key === "ArrowRight" || event.code === "KeyD" || key === "d";
      if (!left && !right) return;
      const previous = readingDirection === "rtl" ? right : left;
      event.preventDefault();
      event.stopPropagation();
      navigatePreviewPage(previous ? -1 : 1);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [gallery, previewPage, readingDirection, navigatePreviewPage]);

  return (
    <>
      {tabs.length ? (
        <button ref={restoreButton} type="button" className="detail-restore detail-restore-dock" data-minimized={minimized}
          inert={!minimized} aria-hidden={!minimized || undefined} tabIndex={minimized ? 0 : -1} title="최소화한 상세 탭 다시 열기" onClick={onRestore}>
          <FluentIcon glyph="\uE8A7" />
          <span>상세 탭 {tabs.length}</span>
          <small>다시 열기</small>
        </button>
      ) : null}
      <WindowMotion show={!minimized && Boolean(gallery) && tabs.length > 0} anchor={restoreButton} travel={minimized}>
      {!minimized && gallery && tabs.length > 0 ? (
        <section ref={workspace} className="detail-workspace" aria-label={`${gallery.title} 상세`} onKeyDown={navigateDetailWorkspace}>
          <div className="detail-tabbar">
            <div className="detail-tabs" role="tablist">
              <MovingTabs>{tabs.map((id, index) => {
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
                        requestClose(id);
                        window.requestAnimationFrame(() => {
                          selectedDetailTab(workspace.current)?.focus();
                        });
                      }}
                    >
                      ×
                    </button>
                  </div>
                );
              })}</MovingTabs>
            </div>
            <button type="button" className="icon-button small" title="상세 최소화" aria-label="상세 최소화" onClick={onMinimize}>
              <FluentIcon glyph="\uE921" />
            </button>
            <button type="button" className="icon-button small" title="상세 전체 닫기" aria-label="상세 전체 닫기" onClick={() => requestClose("all")}>
              <FluentIcon glyph="\uE711" />
            </button>
          </div>
          <div
            className="detail-body"
            data-thumbnail-scroll-root
            id={`detail-panel-${gallery.id}`}
            role="tabpanel"
            aria-labelledby={`detail-tab-${gallery.id}`}
            onScroll={(event) => {
              const scrollTop = event.currentTarget.scrollTop;
              clearTimeout(scrollSaveTimer.current);
              scrollSaveTimer.current = setTimeout(() => saveDetailPosition(gallery.id, { scrollTop }), 250);
            }}
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
                    {gallery.download?.state !== "completed" ? <GalleryProcessingBadge gallery={gallery} /> : null}
                  </div>
                  <div className="detail-title-actions" data-tour="hitomi-detail-actions">
                    <BookmarkButton gallery={gallery} />
                    <CommunityReviewButton work={{ source: "hitomi", workId: String(gallery.id) }} />
                    {gallery.download && onCancelDownload && (runningDownloadStates.has(gallery.download.state) || cancellingDownloadEntryIds?.has(gallery.download.entryId)) ? (
                      <button type="button" className="icon-button danger-button"
                        title={cancellingDownloadEntryIds?.has(gallery.download.entryId) ? "취소 중…" : "다운로드 취소 · 이미 받은 파일은 유지합니다"}
                        aria-label={cancellingDownloadEntryIds?.has(gallery.download.entryId) ? "다운로드 취소 중" : "다운로드 취소"}
                        disabled={pendingDownloadEntryIds?.has(gallery.download.entryId)} onClick={() => onCancelDownload(gallery.id)}>
                        {cancellingDownloadEntryIds?.has(gallery.download.entryId) ? <span className="spinner" /> : <FluentIcon glyph="\uE71A" />}
                      </button>
                    ) : gallery.download?.state === "completed" ? (
                      <span className="icon-button detail-download-complete" data-processing-state="completed" role="img" title="다운로드 완료" aria-label="다운로드 완료">
                        <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="m5 12 4 4L19 6" /></svg>
                      </span>
                    ) : (
                      <button type="button" className="icon-button" title="다운로드" aria-label="다운로드"
                        disabled={gallery.download && pendingDownloadEntryIds?.has(gallery.download.entryId)} onClick={() => onQueue(gallery.id)}>
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
                <div className="detail-metadata-layout" data-tour="hitomi-detail-follow">
                  <div className="detail-metadata-primary">
                    <div className="metadata-box detail-participating-artists" data-tour-follow-kind="artist" aria-label="참여 작가">
                      <span>작가{participatingArtists.length > 1 ? ` · ${participatingArtists.length}명` : ""}</span>
                      <div className="metadata-value">
                        {participatingArtists.map((artist) => (
                          <MetadataChip
                            key={artist.key}
                            value={artist.token}
                            label={`${artist.favorite ? "★ " : ""}${artist.name}`}
                            favorite={artist.favorite}
                            onSearch={onMetadataSearch}
                            onToggleFavorite={onMetadataFavorite}
                          />
                        ))}
                      </div>
                    </div>
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
                      .map((item) => { const title = splitGalleryTitle(item.title, item.subtitle); return (
                        <GalleryProcessingSurface
                          key={item.id}
                          gallery={item}
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
                            if (!isCardControl(event.target) && (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) {
                              setRelatedMenuId(item.id); relatedMenu.onKeyDown(event); return;
                            }
                            if (event.key !== "Enter" || event.target !== event.currentTarget) return;
                            event.preventDefault();
                            onOpenRelated(item.id, gallery.id);
                          }}
                          onContextMenu={(event) => { if (!isCardControl(event.target)) { setRelatedMenuId(item.id); relatedMenu.open(event); } }}
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
                            <span className="processing-preview-wash" aria-hidden="true" />
                            <GalleryProcessingBadge gallery={item} overlay />
                          </GalleryThumbnail>
                          <div className="related-copy card-content">
                            <div className="card-title" title={item.title}><strong>{title.primary}</strong>{title.secondary ? <span className="title-sub">{title.secondary}</span> : null}</div>
                            <div className="card-byline">
                              <MetadataChip value={`artist:${item.artist}`} label={item.artist} kind="byline" favorite={favoriteMetadata.has(`artist:${item.artist}`)} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} />
                              {item.group ? <MetadataChip value={`group:${item.group}`} label={item.group} kind="byline" favorite={favoriteMetadata.has(`group:${item.group}`)} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} /> : null}
                            </div>
                            <div className="tag-list">
                              {sortGalleryTags(item.tags, favoriteMetadata).slice(0, 4).map((tag) => (
                                <MetadataChip key={tag.value} value={tag.value} kind="tag" favorite={tag.favorite} onSearch={onMetadataSearch} onToggleFavorite={onMetadataFavorite} />
                              ))}
                            </div>
                            <div className="meta-bottom"><span>{item.pages}p</span><span>#{item.id}</span></div>
                          </div>
                        </GalleryProcessingSurface>
                      ); })}
                  </div>
                </section>
              </section>
            </div>
          </div>
        </section>
      ) : null}
      </WindowMotion>
      <CardContextMenu anchor={minimized || !gallery ? null : relatedMenu.anchor} close={relatedMenu.close} label={relatedMenuGallery?.title ?? "연관 앨범"} items={relatedMenuItems} />
      <dialog ref={closeDialog} className="detail-close-dialog" aria-labelledby="detail-close-title" aria-describedby="detail-close-description" onCancel={() => setCloseRequest(null)}>
        <div className="detail-close-copy">
          <h2 id="detail-close-title">{closeRequest === "all" ? `탭 ${tabs.length}개를 닫을까요?` : "탭을 닫을까요?"}</h2>
          <p id="detail-close-description"><kbd>Ctrl+Shift+T</kbd>로 다시 열 수 있어요.</p>
        </div>
        <div className="detail-close-actions">
          <button type="button" className="detail-close-confirm" onClick={() => {
            if (closeRequest === "all") onCloseAll();
            else if (closeRequest !== null) onClose(closeRequest);
            setCloseRequest(null);
          }}>{closeRequest === "all" ? "모두 닫기" : "닫기"}</button>
          <button ref={closeCancelButton} type="button" onClick={() => setCloseRequest(null)}>취소</button>
        </div>
      </dialog>
      <dialog
        ref={previewDialog}
        tabIndex={-1}
        className={`page-preview-dialog${previewResizable ? " is-resizable" : ""}`}
        aria-labelledby="page-preview-title"
        data-page-preview-orientation={previewFrame.orientation}
        data-page-preview-source-orientation={previewSourceOrientation}
        data-page-preview-view={isTwoPagePreview ? "spread" : "single"}
        data-reading-direction={readingDirection}
        data-controls-visible={previewControlsPinned || previewControlsVisible}
        data-controls-pinned={previewControlsPinned}
        onPointerMove={revealPreviewControls}
        onPointerDown={revealPreviewControls}
        onKeyDown={(event) => { if (event.key === "Tab") revealPreviewControls(); }}
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
          requestPreviewClose();
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
                  {gallery.title} · {previewPageLabel}페이지
                </h2>
              </div>
              <div className="page-preview-header-actions">
                {actualPreviewPages.map((page) => <BookmarkButton key={page} gallery={gallery} page={page} pageLabelInside />)}
                <CommunityReviewButton work={{ source: "hitomi", workId: String(gallery.id) }} small />
                {previewResizable && onSetRepresentativePreview ? (
                  <>
                    {representativeBusy ? <span className="page-preview-save-status" role="status">저장 중</span> : null}
                    {representativeError ? <span className="page-preview-save-status is-error" role="alert">저장 실패</span> : null}
                    <button
                      type="button"
                      className="icon-button small page-preview-representative"
                      aria-label="앨범커버로 지정"
                      aria-pressed={currentManualRepresentative}
                      title={currentManualRepresentative
                        ? `${primaryPreviewPage}페이지가 앨범커버로 지정됨`
                        : isTwoPagePreview ? `${primaryPreviewPage}페이지를 앨범커버로 지정` : "앨범커버로 지정"}
                      disabled={representativeBusy || currentManualRepresentative}
                      onClick={() => { void setRepresentativePreview(primaryPreviewPage); }}
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                        <rect x="3" y="4" width="18" height="16" rx="2" />
                        <circle cx="8" cy="9" r="1.5" />
                        <path d="m3 16 5-4 4 3 4-5 5 7" />
                      </svg>
                      {currentManualRepresentative ? <span className="page-preview-representative-check" aria-hidden="true">✓</span> : null}
                    </button>
                    {gallery.representativePreview?.mode === "manual" ? (
                      <button
                        type="button"
                        className="icon-button small"
                        aria-label="자동 선택으로 되돌리기"
                        title="자동 선택으로 되돌리기"
                        disabled={representativeBusy}
                        onClick={() => { void setRepresentativePreview(null); }}
                      >
                        <FluentIcon glyph="\uE72C" />
                      </button>
                    ) : null}
                  </>
                ) : null}
                <button ref={previewCloseButton} type="button" className="icon-button small" title="페이지 미리보기 닫기" aria-label="페이지 미리보기 닫기" onClick={requestPreviewClose}>
                  <FluentIcon glyph="\uE711" />
                </button>
              </div>
            </header>
            <div
              className="page-preview-media-stage"
              data-page-preview-count={previewDisplayPages.length}
              style={isTwoPagePreview ? {
                gridTemplateColumns: previewSlotDimensions.map((dimension) => `${pagePreviewAspect(dimension)}fr`).join(" "),
              } : undefined}
            >
              {previewDisplayPages.map((page, index) => page === null
                ? <div key={`blank-${index}`} className="page-preview-empty-slot" aria-hidden="true" /> : (
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
            {(["left", "right"] as const).map((side) => {
              const direction = (side === "left") === (readingDirection === "ltr") ? -1 : 1;
              const turning = previewArrowPulse?.side === side;
              return <button key={side} type="button" className={`page-preview-arrow is-${side}${turning ? " is-turning" : ""}`}
                aria-label={direction < 0 ? "이전 페이지" : "다음 페이지"}
                disabled={nextPagePreviewAnchor(previewPage, totalPageCount, isTwoPagePreview, direction) === null}
                onClick={() => navigatePreviewPage(direction)}>
                <svg viewBox="0 0 32 48" aria-hidden="true"><g key={previewArrowPulse?.sequence ?? 0} className={turning ? "page-preview-chevron-pulse" : undefined}><path className="chevron-outline" d={side === "left" ? "M22 6 9 24 22 42" : "M10 6 23 24 10 42"} /><path d={side === "left" ? "M22 6 9 24 22 42" : "M10 6 23 24 10 42"} /></g></svg>
              </button>;
            })}
            <div className="page-preview-controls">
              <button type="button" className="icon-button small page-preview-pin" aria-label={previewControlsPinned ? "UI 고정 해제" : "UI 고정"}
                title={previewControlsPinned ? "UI 고정 해제" : "UI 항상 표시"} aria-pressed={previewControlsPinned}
                onClick={() => { setPreviewControlsPinned((current) => !current); revealPreviewControls(); }}>
                <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M8 3h8l-1 6 3 4v2H6v-2l3-4-1-6Zm4 12v6" /></svg>
              </button>
              <div className="page-preview-navigation" aria-live="polite" aria-label="현재 페이지">
                <span>{previewPageLabel} / {totalPageCount}</span>
              </div>
              <div className="page-preview-toolbar">
                <button type="button" className="text-button" aria-label="읽기 방향" title="읽기 방향과 좌우 이동 방향을 바꿉니다"
                  onClick={() => setReadingDirection((value) => value === "ltr" ? "rtl" : "ltr")}>
                  {readingDirection === "ltr" ? "좌 → 우" : "우 → 좌"}
                </button>
                <button
                  type="button"
                  className="text-button page-preview-spread-toggle"
                  aria-label="두쪽 보기"
                  aria-pressed={isTwoPagePreview}
                  title={isTwoPagePreview ? "한쪽 보기로 전환" : "현재 페이지와 다음 페이지를 함께 보기"}
                  onClick={() => { setTwoPageView((current) => !current); if (previewPage === 0) setPreviewPage(1); }}
                >
                  <span className="page-preview-spread-icon" aria-hidden="true"><i /><i /></span>
                  <span>두쪽 보기</span>
                </button>
              </div>
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
