import type { ApiResult, DownloadOverlapReview } from "../api/contracts";

/** Revision-scoped single flight shared by inventory and automatic classification.
 * The revision index is advisory: mutations still load fresh evidence and use
 * expectedRevision at the native boundary. Failed loads never become cache hits. */
export class DownloadOverlapReviewCache {
  private values = new Map<string, DownloadOverlapReview>();
  private pending = new Map<string, { revision: number; fresh: boolean; promise: Promise<ApiResult<DownloadOverlapReview>> }>();
  constructor(private load: (id: string, reconcile: boolean) => Promise<ApiResult<DownloadOverlapReview>>) {}
  peek(id: string, revision = 0) {
    const value = this.values.get(id);
    return value && value.revision >= revision ? value : undefined;
  }
  store(review: DownloadOverlapReview) {
    if ((this.values.get(review.reviewId)?.revision ?? -1) <= review.revision) this.values.set(review.reviewId, review);
  }
  invalidate(ids: readonly string[]) {
    for (const id of ids) { this.values.delete(id); this.pending.delete(id); }
  }
  get(id: string, revision = 0, fresh = false): Promise<ApiResult<DownloadOverlapReview>> {
    const known = this.peek(id, revision);
    if (!fresh && known) return Promise.resolve({ ok: true, data: known });
    const pending = this.pending.get(id);
    if (pending && pending.revision >= revision && (!fresh || pending.fresh)) return pending.promise;
    const entry = { revision, fresh, promise: null as unknown as Promise<ApiResult<DownloadOverlapReview>> };
    entry.promise = this.load(id, fresh).then(result => {
      if (result.ok && this.pending.get(id) === entry) this.store(result.data);
      return result;
    }).finally(() => { if (this.pending.get(id) === entry) this.pending.delete(id); });
    this.pending.set(id, entry);
    return entry.promise;
  }
}
