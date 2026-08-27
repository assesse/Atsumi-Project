import { galleryPreviewPreset, galleryPreviewPresetStyle } from "../layout/galleryPreviewPresets";

type GalleryGridSkeletonProps = {
  columns: number;
  previewWidth: number;
  rows?: number;
};

/** A stable placeholder with the same horizontal-card rhythm as GalleryGrid. */
export function GalleryGridSkeleton({ columns, previewWidth, rows = 3 }: GalleryGridSkeletonProps) {
  const safeColumns = Math.max(1, Math.floor(columns));
  const count = safeColumns * Math.max(1, Math.floor(rows));
  const preset = galleryPreviewPreset(previewWidth);

  return (
    <div
      className="gallery-grid gallery-grid-skeleton"
      role="status"
      aria-live="polite"
      aria-busy="true"
      aria-label="갤러리 결과를 불러오는 중"
      data-preview-size={preset.key}
      style={{
        gridTemplateColumns: `repeat(${safeColumns}, minmax(0, 1fr))`,
        ...galleryPreviewPresetStyle(preset),
      }}
    >
      <span className="sr-only">갤러리 결과를 불러오는 중</span>
      {Array.from({ length: count }, (_, index) => (
        <article className="gallery-card-skeleton" key={index} aria-hidden="true">
          <i className="skeleton-media" />
          <div className="skeleton-copy">
            <i className="skeleton-line skeleton-title" />
            <i className="skeleton-line skeleton-title skeleton-title-short" />
            <i className="skeleton-line skeleton-byline" />
            <div className="skeleton-tags"><i /><i /><i /></div>
            <i className="skeleton-line skeleton-footer" />
          </div>
        </article>
      ))}
    </div>
  );
}
