import { useEffect, useRef, useState } from "react";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared } from "../api/contracts";
import type { Gallery } from "../core/types";
import { sourcePageThumbnailKey, type ThumbnailClient } from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";

type ProgressivePagePreviewProps = {
  gallery: Gallery;
  page: number;
  client?: ThumbnailClient;
  backend: BackendClient;
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
export function ProgressivePagePreview({ gallery, page, client, backend }: ProgressivePagePreviewProps) {
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

  return (
    <div
      className="page-preview-media"
      data-original-state={original.kind}
      data-original-source={completedEntryId ? "local-artifact" : "thumbnail"}
      title={original.kind === "failed" ? "로컬 원본을 불러오지 못해 미리보기를 표시 중" : undefined}
    >
      <GalleryThumbnail
        className="page-preview-fallback"
        thumbnailKey={sourcePageThumbnailKey(gallery, page)}
        consumer="detail"
        priority="critical"
        client={client}
        sizing="container"
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
