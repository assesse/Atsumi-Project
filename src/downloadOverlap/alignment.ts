import type { DownloadOverlapCandidate, DownloadOverlapPagePair } from "../api/contracts";

export type DownloadOverlapAlignmentColumn = {
  key: string;
  existingPage?: number;
  incomingPage?: number;
  pair?: DownloadOverlapPagePair;
};

const appendUnmatched = (
  columns: DownloadOverlapAlignmentColumn[],
  existingStart: number,
  existingCount: number,
  incomingStart: number,
  incomingCount: number,
): void => {
  const columnCount = Math.max(existingCount, incomingCount);
  for (let index = 0; index < columnCount; index += 1) {
    const existingPage = index < existingCount ? existingStart + index : undefined;
    const incomingPage = index < incomingCount ? incomingStart + index : undefined;
    columns.push({
      key: `unique:${existingPage ?? "gap"}:${incomingPage ?? "gap"}:${columns.length}`,
      ...(existingPage ? { existingPage } : {}),
      ...(incomingPage ? { incomingPage } : {}),
    });
  }
};

/**
 * Builds a shared page axis. A matched pair occupies one column; pages present
 * on only one side leave a visible gap in the other row.
 */
export const buildDownloadOverlapAlignment = (
  candidate: DownloadOverlapCandidate,
  incomingPageCount: number,
): DownloadOverlapAlignmentColumn[] => {
  const existingPageCount = Math.max(0, Math.floor(candidate.existing.pageCount));
  const normalizedIncomingCount = Math.max(0, Math.floor(incomingPageCount));
  const pairs = [...candidate.pagePairs]
    .filter((pair) => pair.existingSourcePage >= 1
      && pair.existingSourcePage <= existingPageCount
      && pair.incomingSourcePage >= 1
      && pair.incomingSourcePage <= normalizedIncomingCount)
    .sort((left, right) => left.existingSourcePage - right.existingSourcePage
      || left.incomingSourcePage - right.incomingSourcePage);

  const columns: DownloadOverlapAlignmentColumn[] = [];
  let nextExisting = 1;
  let nextIncoming = 1;
  for (const pair of pairs) {
    // The persisted alignment is monotonic. Ignore corrupt/repeated pairs here
    // so the comparison remains usable while backend validation reports them.
    if (pair.existingSourcePage < nextExisting || pair.incomingSourcePage < nextIncoming) continue;
    appendUnmatched(
      columns,
      nextExisting,
      pair.existingSourcePage - nextExisting,
      nextIncoming,
      pair.incomingSourcePage - nextIncoming,
    );
    columns.push({
      key: `match:${pair.existingSourcePage}:${pair.incomingSourcePage}`,
      existingPage: pair.existingSourcePage,
      incomingPage: pair.incomingSourcePage,
      pair,
    });
    nextExisting = pair.existingSourcePage + 1;
    nextIncoming = pair.incomingSourcePage + 1;
  }
  appendUnmatched(
    columns,
    nextExisting,
    Math.max(0, existingPageCount - nextExisting + 1),
    nextIncoming,
    Math.max(0, normalizedIncomingCount - nextIncoming + 1),
  );
  return columns;
};

export const uniquePagesForSide = (
  columns: DownloadOverlapAlignmentColumn[],
  side: "existing" | "incoming",
): number[] => columns.flatMap((column) => {
  if (column.pair) return [];
  const page = side === "existing" ? column.existingPage : column.incomingPage;
  return page ? [page] : [];
});

export const formatPageRanges = (pages: number[]): string => {
  const sorted = [...new Set(pages)].sort((left, right) => left - right);
  if (!sorted.length) return "없음";
  const ranges: string[] = [];
  let start = sorted[0]!;
  let end = start;
  for (const page of sorted.slice(1)) {
    if (page === end + 1) {
      end = page;
      continue;
    }
    ranges.push(start === end ? `${start}p` : `${start}~${end}p`);
    start = page;
    end = page;
  }
  ranges.push(start === end ? `${start}p` : `${start}~${end}p`);
  return ranges.join(", ");
};
