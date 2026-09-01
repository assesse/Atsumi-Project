import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared } from "../api/contracts";
import type { Gallery } from "../core/types";
import { sourcePageThumbnailKey, type ThumbnailClient } from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";

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
  | { kind: "prepared"; requestId: string; entryId: string; media: DetailOriginalPrepared }
  | { kind: "displayed"; requestId: string; entryId: string; media: DetailOriginalPrepared }
  | { kind: "failed"; reason: string };

const newRequestId = (): string => crypto.randomUUID();

/**
 * Completed downloads progressively replace the bounded source thumbnail with
 * the verified local page. Incomplete galleries deliberately remain on the
 * network thumbnail path so PAGE PREVIEW never implies a local artifact exists.
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
  const [original, setOriginal] = useState<OriginalState>({ kind: "thumbnail" });
  const generation = useRef(0);

  useEffect(() => {
    if (!completedEntryId) {
      setOriginal({ kind: "thumbnail" });
      return;
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
  }, [backend, completedEntryId, gallery.id, page]);

  const media = (original.kind === "prepared" || original.kind === "displayed")
    && original.entryId === completedEntryId
    && original.media.galleryId === gallery.id
    && original.media.sourcePage === page
    ? original.media
    : undefined;
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
      snapshot.status !== "resolved"
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
  }, [gallery.id, onDimensionResolved, page]);

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
      data-original-source={completedEntryId ? "local-artifact" : "thumbnail"}
      data-page-orientation={orientation}
      style={{ aspectRatio } as CSSProperties}
      title={original.kind === "failed" ? "로컬 원본을 불러오지 못해 미리보기를 표시 중" : undefined}
    >
      <GalleryThumbnail
        className="page-preview-fallback"
        thumbnailKey={sourcePageThumbnailKey(gallery, page)}
        consumer="detail"
        priority="critical"
        client={client}
        sizing="container"
        expectedAspectRatio={hasExpectedDimension
          ? { width: expectedWidth, height: expectedHeight }
          : undefined}
        onTerminalSnapshot={handleThumbnailTerminal}
        alt={`${gallery.title} ${page}페이지 확대 미리보기`}
      />
      {media ? (
        <img
          className={`page-preview-original${original.kind === "displayed" ? " is-ready" : ""}`}
          src={media.mediaUrl}
          width={media.width}
          height={media.height}
          alt=""
          onLoad={() => setOriginal((current) => current.kind === "prepared" && current.requestId === media.requestId
            ? { kind: "displayed", requestId: current.requestId, entryId: current.entryId, media: current.media }
            : current)}
          onError={() => {
            setOriginal((current) => (current.kind === "prepared" || current.kind === "displayed") && current.requestId === media.requestId
              ? { kind: "failed", reason: "display-error" }
              : current);
            void backend.detailOriginalDispose(media.requestId);
          }}
        />
      ) : null}
    </div>
  );
}
