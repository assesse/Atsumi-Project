import { useEffect, useRef, useState } from "react";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared } from "../api/contracts";
import type { Gallery } from "../core/types";
import { galleryCoverThumbnailKey, type ThumbnailClient } from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";

type ProgressiveDetailHeroProps = {
  gallery: Gallery;
  pageDimension?: { readonly sourcePage: number; readonly width?: number; readonly height?: number };
  client?: ThumbnailClient;
  backend: BackendClient;
};

type OriginalState =
  | { kind: "idle" }
  | { kind: "preparing"; requestId: string }
  | { kind: "prepared"; requestId: string; media: DetailOriginalPrepared }
  | { kind: "displayed"; requestId: string; media: DetailOriginalPrepared }
  | { kind: "failed"; reason: string };

const newRequestId = (): string => crypto.randomUUID();

/**
 * A retained cover is always the first visual layer. The full page-one image
 * is prepared through one terminal backend command and disposed on lifecycle
 * boundaries; no readiness event or automatic retry path is involved.
 */
export function ProgressiveDetailHero({ gallery, pageDimension, client, backend }: ProgressiveDetailHeroProps) {
  const [original, setOriginal] = useState<OriginalState>({ kind: "idle" });
  const generation = useRef(0);
  const originalImage = useRef<HTMLImageElement | null>(null);
  const ratio = pageDimension ?? gallery.pageDimensions?.find((page) => page.sourcePage === 1);
  const expectedAspectRatio = ratio?.width !== undefined && ratio?.height !== undefined
    ? { width: ratio.width, height: ratio.height }
    : gallery.thumbnailWidth !== undefined && gallery.thumbnailHeight !== undefined
      ? { width: gallery.thumbnailWidth, height: gallery.thumbnailHeight }
      : { width: 1, height: 1 };

  useEffect(() => {
    const requestId = newRequestId();
    const currentGeneration = ++generation.current;
    let active = true;
    let terminal = false;
    const fail = (reason: string) => {
      if (!active || generation.current !== currentGeneration) return;
      setOriginal({ kind: "failed", reason });
    };
    setOriginal({ kind: "preparing", requestId });
    const timeout = window.setTimeout(() => {
      if (!active || generation.current !== currentGeneration) return;
      terminal = true;
      void backend.detailOriginalDispose(requestId);
      fail("timeout");
    }, 60_000);

    void backend.detailOriginalPrepare({ requestId, galleryId: gallery.id, sourcePage: 1 }).then((result) => {
      if (!active || terminal || generation.current !== currentGeneration) {
        if (result.ok) void backend.detailOriginalDispose(requestId);
        return;
      }
      window.clearTimeout(timeout);
      if (!result.ok) {
        fail(result.error.code);
        return;
      }
      if (result.data.requestId !== requestId || result.data.galleryId !== gallery.id || result.data.sourcePage !== 1) {
        void backend.detailOriginalDispose(requestId);
        fail("invalid-response");
        return;
      }
      setOriginal({ kind: "prepared", requestId, media: result.data });
    }).catch(() => {
      window.clearTimeout(timeout);
      fail("unavailable");
    });

    return () => {
      active = false;
      terminal = true;
      window.clearTimeout(timeout);
      generation.current = Math.max(generation.current, currentGeneration + 1);
      void backend.detailOriginalDispose(requestId);
    };
  }, [backend, gallery.id]);

  const abandon = (requestId: string, reason: string) => {
    setOriginal((current) => {
      if ((current.kind === "prepared" || current.kind === "displayed") && current.requestId === requestId) {
        return { kind: "failed", reason };
      }
      return current;
    });
    void backend.detailOriginalDispose(requestId);
  };
  const media = (original.kind === "prepared" || original.kind === "displayed") && original.media.galleryId === gallery.id
    ? original.media
    : undefined;

  useEffect(() => {
    if (original.kind !== "prepared" || original.media.galleryId !== gallery.id) return;
    const image = originalImage.current;
    if (image?.complete && image.naturalWidth > 0) {
      setOriginal((current) => current.kind === "prepared" && current.requestId === original.requestId
        ? { kind: "displayed", requestId: current.requestId, media: current.media }
        : current);
    }
  }, [gallery.id, original]);

  return (
    <div
      className="detail-hero"
      data-original-state={original.kind}
      title={original.kind === "failed" ? "원본을 불러오지 못해 썸네일을 표시 중" : undefined}
      aria-live="polite"
      style={{ aspectRatio: `${expectedAspectRatio.width} / ${expectedAspectRatio.height}` }}
    >
      <GalleryThumbnail
        className="detail-cover"
        thumbnailKey={galleryCoverThumbnailKey(gallery)}
        consumer="detail"
        priority="critical"
        client={client}
        sizing="container"
        expectedAspectRatio={expectedAspectRatio}
        alt={`${gallery.title} 표지`}
      />
      {media ? (
        <img
          ref={originalImage}
          className={`detail-hero-original${original.kind === "displayed" ? " is-ready" : ""}`}
          src={media.mediaUrl}
          width={media.width}
          height={media.height}
          alt=""
          onLoad={() => setOriginal((current) => current.kind === "prepared" && current.requestId === media.requestId && current.media.galleryId === gallery.id
            ? { kind: "displayed", requestId: current.requestId, media: current.media }
            : current)}
          onError={() => abandon(media.requestId, "display-error")}
        />
      ) : null}
    </div>
  );
}
