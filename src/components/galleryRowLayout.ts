export type GalleryCardRowMetric = Readonly<{
  index: number;
  top: number;
  intrinsicThumbnailHeight: number;
}>;

export type GalleryCardRow = Readonly<{
  indices: readonly number[];
  height: number;
}>;

export function groupGalleryCardRows(
  metrics: readonly GalleryCardRowMetric[],
  topTolerance = 1,
): GalleryCardRow[] {
  const rows: Array<{ top: number; indices: number[]; height: number }> = [];
  let lastTop = -Infinity;
  let ordered = true;
  for (const metric of metrics) {
    if (!Number.isFinite(metric.top) || !Number.isFinite(metric.intrinsicThumbnailHeight)
      || metric.intrinsicThumbnailHeight <= 0) continue;
    const last = rows.at(-1);
    ordered &&= metric.top >= lastTop;
    lastTop = metric.top;
    // Grid hosts arrive in row order. Avoid scanning all previous rows for
    // every card in a large artist folder; retain the unordered-input fallback.
    const row = ordered
      ? last && metric.top - last.top <= topTolerance ? last : undefined
      : rows.find((candidate) => Math.abs(candidate.top - metric.top) <= topTolerance);
    if (row) {
      row.indices.push(metric.index);
      row.height = Math.max(row.height, metric.intrinsicThumbnailHeight);
    } else {
      rows.push({
        top: metric.top,
        indices: [metric.index],
        height: metric.intrinsicThumbnailHeight,
      });
    }
  }
  return rows.map(({ indices, height }) => ({ indices, height: Math.round(height * 100) / 100 }));
}
