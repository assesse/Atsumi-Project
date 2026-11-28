import { useLayoutEffect, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { QueueRow } from "../api/workConsole";
import type { Gallery } from "../core/types";
import { GALLERY_PREVIEW_PRESETS, galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";
import { galleryCoverThumbnailKey } from "../thumbnail";
import { GalleryThumbnail } from "./GalleryThumbnail";

const EDGE = 8;
const COVER_RATIO = 2 / 3;

export function QueueAlbumPreview({ id, element, row, title, gallery, width }: {
  id: string; element: HTMLElement; row: QueueRow; title: string; gallery?: Gallery; width: number;
}) {
  const preset = galleryPreviewPreset(width);
  const [position, setPosition] = useState<CSSProperties>({ visibility: "hidden" });
  useLayoutEffect(() => {
    const panel = element.closest(".activity-panel");
    const place = () => {
      const trigger = element.getBoundingClientRect();
      const panelLeft = panel?.getBoundingClientRect().left ?? trigger.left;
      const desired = Math.max(0, Math.min(preset.width, innerWidth - 2 * EDGE, (innerHeight - 2 * EDGE) * COVER_RATIO));
      const leftRoom = Math.max(0, panelLeft - 2 * EDGE);
      const above = Math.max(0, trigger.top - 2 * EDGE);
      const below = Math.max(0, innerHeight - trigger.bottom - 2 * EDGE);
      const beside = leftRoom >= Math.min(desired, GALLERY_PREVIEW_PRESETS[0]!.width);
      const previewWidth = Math.min(desired, beside ? leftRoom : Math.max(above, below) * COVER_RATIO);
      const height = previewWidth / COVER_RATIO;
      const left = beside ? panelLeft - EDGE - previewWidth : trigger.left;
      const top = beside ? trigger.top : below >= height || below >= above ? trigger.bottom + EDGE : trigger.top - EDGE - height;
      setPosition({ width: previewWidth,
        left: Math.max(EDGE, Math.min(left, innerWidth - EDGE - previewWidth)),
        top: Math.max(EDGE, Math.min(top, innerHeight - EDGE - height)) });
    };
    place();
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(place) : null;
    observer?.observe(element);
    if (panel) observer?.observe(panel);
    window.addEventListener("resize", place);
    return () => { observer?.disconnect(); window.removeEventListener("resize", place); };
  }, [element, preset.width]);
  return createPortal(<aside id={id} role="tooltip" className="queue-album-preview" style={{ ...galleryPreviewPresetStyle(preset), ...position }}>
    <GalleryThumbnail className="queue-album-cover" fit="cover" thumbnailKey={galleryCoverThumbnailKey(gallery ?? { id: row.galleryId, coverIndex: row.galleryId % 6 })}
      consumer="detail" priority="visible" alt="" />
    <div className="queue-album-caption"><strong>{title || "#" + row.galleryId}</strong></div>
  </aside>, document.body);
}
