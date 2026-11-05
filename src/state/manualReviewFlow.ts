import type { ManualOverlapItem } from "./manualOverlapSequence";

export type ManualReviewSession = { items: ManualOverlapItem[]; index: number };

/** Submitted work is unavailable for another action, but is not completed yet. */
export function nextManualReviewIndex(session: ManualReviewSession, unavailable: ReadonlySet<string>): number {
  for (let step = 1; step < session.items.length; step++) {
    const index = (session.index + step) % session.items.length;
    if (!unavailable.has(session.items[index]!.reviewId)) return index;
  }
  return -1;
}

export function settleManualReviewSession(session: ManualReviewSession, completed: ReadonlySet<string>): ManualReviewSession | null {
  const current = session.items[session.index]?.reviewId;
  const items = session.items.filter(item => !completed.has(item.reviewId));
  if (!items.length) return null;
  const retained = items.findIndex(item => item.reviewId === current);
  return { items, index: retained >= 0 ? retained : Math.min(session.index, items.length - 1) };
}

/** User choices queue immediately, while filesystem mutations stay strictly serial. */
export class SerializedReviewWork {
  private tail: Promise<unknown> = Promise.resolve();
  private pending = new Set<string>();
  private jobs = 0;
  onChange?: (ids: ReadonlySet<string>, jobs: number) => void;

  get size() { return this.jobs; }
  get ids(): ReadonlySet<string> { return new Set(this.pending); }

  enqueue<T>(ids: readonly string[], run: () => Promise<T>): Promise<T> {
    const unique = [...new Set(ids)];
    if (unique.some(id => this.pending.has(id))) return Promise.reject(Error("이미 적용 중인 판본입니다."));
    for (const id of unique) this.pending.add(id);
    this.jobs++;
    this.onChange?.(this.ids, this.jobs);

    const result = this.tail.then(run).finally(() => {
      for (const id of unique) this.pending.delete(id);
      this.jobs--;
      this.onChange?.(this.ids, this.jobs);
    });
    this.tail = result.catch(() => undefined);
    return result;
  }
}
