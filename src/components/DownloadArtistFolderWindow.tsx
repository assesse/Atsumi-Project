import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { Gallery, GalleryDisplayMode } from "../core/types";
import { resolveCompactGalleryColumns, resolveGalleryColumns } from "../layout/galleryColumns";
import type { GalleryGroup } from "../state/galleryGrouping";
import { FluentIcon } from "./FluentIcon";
import "./DownloadArtistFolderWindow.css";

/** A DOM-layer window: album detail and page preview must remain above it. */
export function DownloadArtistFolderWindow({ group, folderType, id, contentsRef, previewWidth, displayMode, maximumColumns,
  suspended, onColumnsChange, renderGrid, onClose }: {
  group: GalleryGroup; folderType: string; id: string;
  contentsRef?: RefObject<HTMLElement | null>;
  previewWidth: number; displayMode: GalleryDisplayMode; maximumColumns: number;
  suspended: boolean; onColumnsChange?: (columns: number) => void;
  renderGrid: (items: Gallery[], label: string, columns?: number) => ReactNode;
  onClose: () => void;
}) {
  const panel = useRef<HTMLElement>(null);
  const localContents = useRef<HTMLElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const [columns, setColumns] = useState(1);
  useLayoutEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton.current?.focus({ preventScroll: true });
    return () => {
      if (opener?.isConnected && (document.activeElement === document.body || panel.current?.contains(document.activeElement))) {
        opener.focus({ preventScroll: true });
      }
    };
  }, []);
  useLayoutEffect(() => {
    const viewport = localContents.current;
    if (!viewport) return;
    const update = () => {
      const style = getComputedStyle(viewport);
      const width = viewport.clientWidth - (Number.parseFloat(style.paddingLeft) || 0) - (Number.parseFloat(style.paddingRight) || 0);
      const next = displayMode === "compact"
        ? resolveCompactGalleryColumns(width, previewWidth)
        : resolveGalleryColumns(width, maximumColumns, previewWidth);
      setColumns(next); onColumnsChange?.(next);
    };
    update();
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(update) : null;
    observer?.observe(viewport);
    return () => observer?.disconnect();
  }, [displayMode, maximumColumns, onColumnsChange, previewWidth]);
  useLayoutEffect(() => {
    if (suspended) return;
    const keyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || document.querySelector('dialog[open], [role="menu"]')) return;
      if (event.key === "Escape") {
        event.preventDefault(); event.stopPropagation(); close.current();
      } else if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const controls = [...(panel.current?.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex="0"]') ?? [])]
          .filter((node) => !node.closest('[aria-hidden="true"]'));
        const first = controls[0]; const last = controls.at(-1);
        if (!first || !last) return;
        if (event.shiftKey && (document.activeElement === first || !panel.current?.contains(document.activeElement))) {
          event.preventDefault(); last.focus();
        } else if (!event.shiftKey && (document.activeElement === last || !panel.current?.contains(document.activeElement))) {
          event.preventDefault(); first.focus();
        }
      }
    };
    document.addEventListener("keydown", keyDown);
    return () => document.removeEventListener("keydown", keyDown);
  }, [suspended]);
  return createPortal(<div className="download-artist-folder-backdrop" onPointerDown={(event) => {
    if (!suspended && event.target === event.currentTarget) close.current();
  }}>
    <section ref={panel} id={id} className="download-artist-folder-window" role="dialog" aria-modal={!suspended || undefined}
      style={{ "--folder-preferred-width": `${Math.max(1600, (Math.max(460, previewWidth + 260) + 12) * 3 + 44)}px` } as CSSProperties}
      aria-labelledby={`${id}-title`}>
      <header className="download-artist-folder-window-heading">
        <FluentIcon glyph="\uE8B7" />
        <h2 id={`${id}-title`}>{group.label}</h2>
        <span>{folderType} · {group.items.length.toLocaleString()}개 작품</span>
        <button ref={closeButton} type="button" className="icon-button" aria-label="폴더 닫기" onClick={onClose}><FluentIcon glyph="\uE711" /></button>
      </header>
      <section className="download-artist-folder-window-scroll" ref={(node) => {
        localContents.current = node;
        if (contentsRef) contentsRef.current = node;
      }}>
        {renderGrid(group.items, `${group.label} 다운로드 작품`, columns)}
      </section>
    </section>
  </div>, document.body);
}
