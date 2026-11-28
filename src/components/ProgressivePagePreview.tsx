import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared } from "../api/contracts";
import type { Gallery } from "../core/types";
import {
  artifactPageThumbnailKey,
  galleryCoverThumbnailKey,
  sourcePageThumbnailKey,
  useThumbnailClient,
  type ThumbnailClient,
  type ThumbnailSnapshot,
} from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";
import { useOriginalPagePool } from "./OriginalPageWindow";

type ProgressivePagePreviewProps = {
  gallery: Gallery;
  page: number;
  expectedDimension?: Readonly<{ width?: number; height?: number }>;
  client?: ThumbnailClient;
  backend: BackendClient;
  onDimensionResolved?: (dimension: Readonly<{
    galleryId: Gallery["id"];
    page: number;
    width: number;
    height: number;
  }>) => void;
};

type OriginalState =
  | { kind: "thumbnail" }
  | { kind: "preparing"; requestId: string }
  | { kind: "prepared"; requestId: string; entryId?: string; media: DetailOriginalPrepared }
  | { kind: "displayed"; requestId: string; entryId?: string; media: DetailOriginalPrepared }
  | { kind: "failed"; reason: string };

const newRequestId = (): string => crypto.randomUUID();
const hasPreview = (snapshot: ThumbnailSnapshot): boolean => snapshot.status === "loading"
  || (snapshot.status === "resolved" && snapshot.asset.kind !== "missing");

/**
 * Keep the matching preview visible until the full original is decoded.
 * Originals still prefer verified local pages and use the bounded window pool.
 */
export function ProgressivePagePreview({
  gallery,
  page,
  expectedDimension,
  client,
  backend,
  onDimensionResolved,
}: ProgressivePagePreviewProps) {
  const completedEntryId = gallery.download?.state === "completed" ? gallery.download.entryId : undefined;
  const thumbnailClient = useThumbnailClient(client);
  const fallbackKey = useMemo(() => {
    if (completedEntryId) return artifactPageThumbnailKey(completedEntryId, page, gallery.coverIndex + page - 1);
    const source = { id: gallery.id, thumbnailKey: gallery.thumbnailKey, coverIndex: gallery.coverIndex };
    const pageKey = sourcePageThumbnailKey(source, page);
    // A card's cover is the first source page, never a placeholder for other pages.
    if (page === 1 && !hasPreview(thumbnailClient.getSnapshot(pageKey))) {
      const coverKey = galleryCoverThumbnailKey(source);
      if (hasPreview(thumbnailClient.getSnapshot(coverKey))) return coverKey;
    }
    return pageKey;
  }, [completedEntryId, gallery.coverIndex, gallery.id, gallery.thumbnailKey, page, thumbnailClient]);
  const pool = useOriginalPagePool();
  const [original, setOriginal] = useState<OriginalState>({ kind: "thumbnail" });
  const generation = useRef(0);
  const originalImage = useRef<HTMLImageElement>(null);
  const decodingImage = useRef<HTMLImageElement | null>(null);

  useEffect(() => {
    if (pool) {
      let active = true;
      setOriginal({ kind: "preparing", requestId: "pool" });
      void Promise.resolve().then(() => {
        const promise = pool.get(page);
        if (!promise) throw new Error("original unavailable");
        return promise;
      }).then(media => { if (active) setOriginal({ kind: "prepared", requestId: media.requestId, entryId: completedEntryId, media }); })
        .catch(() => { if (active) setOriginal({ kind: "failed", reason: "unavailable" }); });
      return () => { active = false; };
    }
    const requestId = newRequestId();
    const currentGeneration = ++generation.current;
    let active = true;
    let terminal = false;
    setOriginal({ kind: "preparing", requestId });
    const timeout = window.setTimeout(() => {
      if (!active || generation.current !== currentGeneration) return;
      terminal = true;
      void backend.detailOriginalDispose(requestId);
      setOriginal({ kind: "failed", reason: "timeout" });
    }, 60_000);

    void backend.detailOriginalPrepare({
      requestId,
      galleryId: gallery.id,
      sourcePage: page,
      entryId: completedEntryId,
    }).then((result) => {
      if (!active || terminal || generation.current !== currentGeneration) {
        if (result.ok) void backend.detailOriginalDispose(requestId);
        return;
      }
      window.clearTimeout(timeout);
      if (!result.ok) {
        setOriginal({ kind: "failed", reason: result.error.code });
        return;
      }
      const media = result.data;
      if (
        media.requestId !== requestId
        || media.galleryId !== gallery.id
        || media.sourcePage !== page
      ) {
        void backend.detailOriginalDispose(requestId);
        setOriginal({ kind: "failed", reason: "invalid-response" });
        return;
      }
      setOriginal({ kind: "prepared", requestId, entryId: completedEntryId, media });
    }).catch(() => {
      window.clearTimeout(timeout);
      void backend.detailOriginalDispose(requestId).catch(() => undefined);
      if (active && !terminal && generation.current === currentGeneration) {
        setOriginal({ kind: "failed", reason: "unavailable" });
      }
    });

    return () => {
      active = false;
      terminal = true;
      window.clearTimeout(timeout);
      generation.current = Math.max(generation.current, currentGeneration + 1);
      void backend.detailOriginalDispose(requestId);
    };
  }, [backend, completedEntryId, gallery.id, gallery.download?.revision, page, pool]);

  const media = (original.kind === "prepared" || original.kind === "displayed")
    && original.entryId === completedEntryId
    && original.media.galleryId === gallery.id
    && original.media.sourcePage === page
    ? original.media
    : undefined;
  const displayed = !!media && original.kind === "displayed";
  const failDisplay = useCallback((image: HTMLImageElement, requestId: string) => {
    if (originalImage.current !== image) return;
    setOriginal(current => (current.kind === "prepared" || current.kind === "displayed") && current.requestId === requestId
      ? { kind: "failed", reason: "display-error" }
      : current);
    void backend.detailOriginalDispose(requestId).catch(() => undefined);
  }, [backend]);
  const displayOriginal = useCallback((image: HTMLImageElement, requestId: string) => {
    if (originalImage.current !== image || decodingImage.current === image) return;
    decodingImage.current = image;
    const show = () => {
      if (originalImage.current !== image) return;
      setOriginal(current => current.kind === "prepared" && current.requestId === requestId
        ? { ...current, kind: "displayed" }
        : current);
    };
    if (typeof image.decode !== "function") {
      show();
      return;
    }
    void Promise.resolve().then(() => image.decode()).then(show, () => failDisplay(image, requestId));
  }, [failDisplay]);

  useEffect(() => {
    const image = originalImage.current;
    if (media && image?.complete && image.naturalWidth > 0) displayOriginal(image, media.requestId);
  }, [displayOriginal, media]);

  const expectedWidth = expectedDimension?.width;
  const expectedHeight = expectedDimension?.height;
  const hasExpectedDimension = typeof expectedWidth === "number"
    && Number.isFinite(expectedWidth)
    && expectedWidth > 0
    && typeof expectedHeight === "number"
    && Number.isFinite(expectedHeight)
    && expectedHeight > 0;
  const aspectRatio = hasExpectedDimension ? `${expectedWidth} / ${expectedHeight}` : "2 / 3";
  const aspect = hasExpectedDimension ? expectedWidth / expectedHeight : 2 / 3;
  const orientation = aspect > 1.05 ? "landscape" : aspect < 0.95 ? "portrait" : "square";
  const handleThumbnailTerminal = useCallback((snapshot: {
    status: "resolved" | "error";
    width?: number;
    height?: number;
    kind?: "image" | "sprite" | "missing";
  }) => {
    if (
      media !== undefined
      || snapshot.status !== "resolved"
      || (snapshot.kind !== undefined && snapshot.kind !== "image")
      || typeof snapshot.width !== "number"
      || !Number.isFinite(snapshot.width)
      || snapshot.width <= 0
      || typeof snapshot.height !== "number"
      || !Number.isFinite(snapshot.height)
      || snapshot.height <= 0
    ) return;
    onDimensionResolved?.({
      galleryId: gallery.id,
      page,
      width: snapshot.width,
      height: snapshot.height,
    });
  }, [gallery.id, media, onDimensionResolved, page]);

  useEffect(() => {
    if (!media || !Number.isFinite(media.width) || media.width <= 0 || !Number.isFinite(media.height) || media.height <= 0) return;
    onDimensionResolved?.({
      galleryId: gallery.id,
      page,
      width: media.width,
      height: media.height,
    });
  }, [gallery.id, media, onDimensionResolved, page]);

  return (
    <div
      className="page-preview-media"
      data-original-state={original.kind}
      data-original-source={completedEntryId ? "local-artifact" : "source-original"}
      data-page-orientation={orientation}
      style={{ aspectRatio } as CSSProperties}
      title={original.kind === "failed" ? "원본을 불러오지 못해 미리보기를 표시 중" : undefined}
    >
      <GalleryThumbnail
        className="page-preview-fallback"
        thumbnailKey={fallbackKey}
        consumer="detail"
        priority="critical"
        client={thumbnailClient}
        aria-hidden={displayed || undefined}
        style={{ position: "absolute", inset: 0 }}
        sizing="container"
        expectedAspectRatio={hasExpectedDimension
          ? { width: expectedWidth, height: expectedHeight }
          : undefined}
        onTerminalSnapshot={handleThumbnailTerminal}
        alt={`${gallery.title} ${page}페이지 확대 미리보기`}
      />
      {media ? (
        <img
          key={media.requestId}
          ref={originalImage}
          aria-hidden={!displayed || undefined}
          style={{ backgroundColor: "var(--thumbnail-frame)" }}
          className={`page-preview-original${original.kind === "displayed" ? " is-ready" : ""}`}
          src={media.mediaUrl}
          width={media.width}
          height={media.height}
          alt={`${gallery.title} ${page}페이지 원본`}
          onLoad={event => displayOriginal(event.currentTarget, media.requestId)}
          onError={event => failDisplay(event.currentTarget, media.requestId)}
        />
      ) : null}
    </div>
  );
}
