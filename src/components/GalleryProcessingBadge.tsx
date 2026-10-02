import type { HTMLAttributes, MouseEventHandler } from "react";
import type { Gallery } from "../core/types";
import { useGalleryDownload } from "../state/downloadProgress";
import { downloadStatus } from "../state/downloadStatus";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
import "./GalleryProcessingBadge.css";

export function processingSurfaceAttributes(download: Gallery["download"], soften = true) {
  return {
    "data-processing-tone": download ? downloadStatus[download.state].tone : undefined,
    "data-processing-muted": soften && download?.state === "completed" ? true : undefined,
  };
}

// Related cards subscribe to the same per-ID status source as Explore.
export function GalleryProcessingSurface({ gallery, ...props }: HTMLAttributes<HTMLElement> & { gallery: Gallery }) {
  const download = useGalleryDownload(gallery.id, gallery.download);
  return <article {...props} {...processingSurfaceAttributes(download)} />;
}

export function GalleryProcessingBadge({ gallery, overlay = false, duplicateCount = 0, onClick, label }: {
  gallery: Gallery; overlay?: boolean; duplicateCount?: number;
  onClick?: MouseEventHandler<HTMLButtonElement>; label?: string;
}) {
  const download = useGalleryDownload(gallery.id, gallery.download);
  if (!download && !duplicateCount) return null;
  const duplicate = duplicateCount > 0 || (download?.state === "review_required" && download.reviewKind === "gallery_duplicate");
  const state = duplicate ? "duplicate" : download!.state;
  const status = duplicate ? { tone: "review", label: duplicateCount ? `중복 후보 ${duplicateCount}개` : "다운로드 판본 중복" } : downloadStatus[download!.state];
  const description = label ?? `${gallery.title}, ${status.label}`;
  const attributes = {
    className: `gallery-processing-badge tone-${status.tone}${overlay ? " is-overlay" : ""}`,
    "data-processing-state": state,
    title: description,
    "aria-label": description,
  };
  const icon = <GalleryStatusIcon kind={state} />;
  return onClick ? <button type="button" {...attributes} onClick={onClick}>{icon}</button>
    : <span {...attributes} role="img" tabIndex={0}>{icon}</span>;
}
