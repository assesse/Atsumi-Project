import type {
  ApiResult,
  DownloadOverlapAutomationHistoryItem,
  DownloadOverlapAutomationHistoryPage,
} from "../api/contracts";

type HistoryPageLoader = (request: {
  page: number;
  pageSize: number;
}) => Promise<ApiResult<DownloadOverlapAutomationHistoryPage>>;

const historyPageSize = 200;

export async function collectUnacknowledgedAutomationHistory(
  loadPage: HistoryPageLoader,
  isCurrent: () => boolean,
): Promise<DownloadOverlapAutomationHistoryItem[] | null> {
  const collected = new Map<string, DownloadOverlapAutomationHistoryItem>();

  for (let requestedPage = 1; ; requestedPage += 1) {
    if (!isCurrent()) return null;
    const result = await loadPage({ page: requestedPage, pageSize: historyPageSize });
    if (!isCurrent()) return null;
    if (!result.ok) throw new Error(result.error.message);

    const { page, pageSize, totalItems, items } = result.data;
    if (page !== requestedPage
      || pageSize !== historyPageSize
      || !Number.isSafeInteger(totalItems)
      || totalItems < 0
      || items.length > pageSize
      || (items.length === 0 && (page - 1) * pageSize < totalItems)) {
      throw new Error("자동 판본 분류 기록의 페이지 정보가 올바르지 않습니다. 다시 시도해 주세요.");
    }

    for (const item of items) collected.set(item.reviewId, item);
    if (page * pageSize >= totalItems) break;
  }

  return [...collected.values()]
    .filter((item) => !item.acknowledgedAt)
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt));
}
