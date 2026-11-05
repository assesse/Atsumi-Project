import type { HTMLAttributes, MouseEventHandler } from "react";
import type { Gallery } from "../core/types";
import { useGalleryDownload } from "../state/downloadProgress";
import { galleryProcessingStatus, type GalleryProcessingOptions } from "../state/downloadStatus";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
import "./GalleryProcessingBadge.css";

export function processingSurfaceAttributes(download: Gallery["download"], soften = true, options: GalleryProcessingOptions = {}) {
  const status = galleryProcessingStatus(download, options);
  return {
    "data-processing-tone": status?.tone,
    "data-processing-muted": soften && status?.state === "completed" ? true : undefined,
    "data-processing-excluded": status?.state === "excluded" || status?.state === "quarantined" ? true : undefined,
  };
}

// Related cards subscribe to the same per-ID status source as Explore.
export function GalleryProcessingSurface({ gallery, duplicateCount, exclusionLabel, ...props }:
  HTMLAttributes<HTMLElement> & GalleryProcessingOptions & { gallery: Gallery }) {
  const download = useGalleryDownload(gallery.id, gallery.download);
  const options = { duplicateCount, exclusionLabel };
  const status = galleryProcessingStatus(download, options);
  return <article {...props} aria-label={props["aria-label"] ?? (status ? gallery.title + ", " + status.label : gallery.title)}
    {...processingSurfaceAttributes(download, true, options)} />;
}

export function GalleryProcessingBadge({ gallery, overlay = false, duplicateCount = 0, exclusionLabel, onClick, label }: {
  gallery: Gallery; overlay?: boolean;
  onClick?: MouseEventHandler<HTMLButtonElement>; label?: string;
} & GalleryProcessingOptions) {
  const download = useGalleryDownload(gallery.id, gallery.download);
  const status = galleryProcessingStatus(download, { duplicateCount, exclusionLabel });
  if (!status) return null;
  const state = status.state;
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
