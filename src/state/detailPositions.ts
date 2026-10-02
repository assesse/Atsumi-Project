export type DetailPosition = { scrollTop: number; previewStart: number };
const key = "atsumi.detail-positions.v1";
const limit = 96;

export function readDetailPositions(): Map<number, DetailPosition> {
  try {
    const raw = JSON.parse(sessionStorage.getItem(key) ?? "[]") as unknown;
    if (!Array.isArray(raw)) return new Map();
    return new Map(raw.slice(-limit).filter((entry): entry is [number, DetailPosition] =>
      Array.isArray(entry) && Number.isSafeInteger(entry[0]) && entry[0] > 0
      && entry[1] && Number.isFinite(entry[1].scrollTop) && entry[1].scrollTop >= 0
      && Number.isSafeInteger(entry[1].previewStart) && entry[1].previewStart >= 1));
  } catch { return new Map(); }
}

export function saveDetailPosition(id: number, value: Partial<DetailPosition>): void {
  const positions = readDetailPositions();
  const next = { scrollTop: 0, previewStart: 1, ...positions.get(id), ...value };
  positions.delete(id);
  positions.set(id, next);
  try { sessionStorage.setItem(key, JSON.stringify(Array.from(positions).slice(-limit))); } catch { /* Session storage may be unavailable. */ }
}

/** Exactly two neighbours on each side; the displayed spread is never duplicated. */
export function adjacentPreviewPages(first: number, last: number, total: number): number[] {
  return [first - 2, first - 1, last + 1, last + 2].filter((page) => page >= 1 && page <= total);
}
