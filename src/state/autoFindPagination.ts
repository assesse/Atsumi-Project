export type GalleryPageSlice<T> = {
  items: T[];
  page: number;
  pageSize: number;
  totalItems: number;
  totalPages: number;
  startIndex: number;
};

const positiveInteger = (value: number, fallback: number): number => (
  Number.isFinite(value) && value >= 1 ? Math.floor(value) : fallback
);

/** Client-only pagination for an already persisted list of gallery summaries. */
export function paginateGalleryItems<T>(
  items: readonly T[],
  requestedPage: number,
  requestedPageSize: number,
): GalleryPageSlice<T> {
  const pageSize = positiveInteger(requestedPageSize, 1);
  const totalItems = items.length;
  const totalPages = Math.max(1, Math.ceil(totalItems / pageSize));
  const page = Math.min(totalPages, positiveInteger(requestedPage, 1));
  const startIndex = (page - 1) * pageSize;

  return {
    items: items.slice(startIndex, startIndex + pageSize),
    page,
    pageSize,
    totalItems,
    totalPages,
    startIndex,
  };
}

/** Backwards-compatible name kept for the Auto Find call sites and tests. */
export const paginateAutoFindItems = paginateGalleryItems;

export type AutoFindPageSlice<T> = GalleryPageSlice<T>;
