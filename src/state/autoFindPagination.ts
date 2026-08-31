export type AutoFindPageSlice<T> = {
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

/**
 * Client-only pagination for the already persisted Auto Find snapshot.
 * It deliberately has no knowledge of Explore queries or Downloads pages.
 */
export function paginateAutoFindItems<T>(
  items: readonly T[],
  requestedPage: number,
  requestedPageSize: number,
): AutoFindPageSlice<T> {
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
