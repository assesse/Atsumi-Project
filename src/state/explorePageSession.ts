import type { ApiError, ApiResult, GalleryPage } from "../api/contracts";

export type ExplorePagePriority = "foreground" | "prefetch-next" | "prefetch-previous";

export type ExplorePageLoadResult =
  | { status: "ready"; page: GalleryPage; source: "cache" | "network"; scrollTop: number }
  | { status: "failed"; error: ApiError }
  | { status: "stale" };

type SettledPage = {
  page: GalleryPage;
  lastAccessed: number;
  releaseWarmup?: () => void;
};

type FailedPrefetch = {
  error: ApiError;
  lastAccessed: number;
};

type InFlightPage = {
  requestId: string;
  priority: ExplorePagePriority;
  foregroundIntent?: number;
  promise: Promise<ExplorePageLoadResult>;
};

export type ExplorePageSessionOptions = {
  fetchPage: (queryId: string, page: number, requestId: string) => Promise<ApiResult<GalleryPage>>;
  cancelPage?: (requestId: string) => void | Promise<unknown>;
  warmPage?: (page: GalleryPage, priority: Exclude<ExplorePagePriority, "foreground">) => void | (() => void);
  promotePage?: (queryId: string, page: number) => void;
  maxSettledPages?: number;
  now?: () => number;
};

/** Query-scoped page cache and warmup lease owner for Explore. */
export class ExplorePageSession {
  private readonly fetchPage: ExplorePageSessionOptions["fetchPage"];
  private readonly warmPage?: ExplorePageSessionOptions["warmPage"];
  private readonly cancelPage?: ExplorePageSessionOptions["cancelPage"];
  private readonly promotePage?: ExplorePageSessionOptions["promotePage"];
  private readonly maxSettledPages: number;
  private readonly now: () => number;
  private generation = 0;
  private foregroundIntent = 0;
  private requestSequence = 0;
  private queryId: string | null = null;
  private currentPage = 0;
  private totalPages = 0;
  private pages = new Map<number, SettledPage>();
  private failures = new Map<number, FailedPrefetch>();
  private inFlight = new Map<number, InFlightPage>();
  private scrollPositions = new Map<number, number>();

  constructor(options: ExplorePageSessionOptions) {
    this.fetchPage = options.fetchPage;
    this.warmPage = options.warmPage;
    this.cancelPage = options.cancelPage;
    this.promotePage = options.promotePage;
    this.maxSettledPages = options.maxSettledPages ?? 5;
    this.now = options.now ?? Date.now;
    if (!Number.isInteger(this.maxSettledPages) || this.maxSettledPages < 1) {
      throw new RangeError("Explore page cache capacity must be a positive integer");
    }
  }

  start(queryId: string, firstPage: GalleryPage): void {
    this.reset();
    this.generation += 1;
    this.queryId = queryId;
    this.currentPage = firstPage.page;
    this.totalPages = firstPage.totalPages;
    this.pages.set(firstPage.page, { page: firstPage, lastAccessed: this.now() });
  }

  clear(): void {
    this.reset();
    this.generation += 1;
  }

  recordScroll(page: number, scrollTop: number): void {
    if (!this.queryId || !this.pages.has(page) || !Number.isInteger(page) || page < 1) return;
    this.scrollPositions.set(page, Math.max(0, Number.isFinite(scrollTop) ? scrollTop : 0));
  }

  scrollFor(page: number): number {
    return this.scrollPositions.get(page) ?? 0;
  }

  cachedPageNumbers(): number[] {
    return [...this.pages.keys()].sort((left, right) => left - right);
  }

  inFlightPageNumbers(): number[] {
    return [...this.inFlight.keys()].sort((left, right) => left - right);
  }

  open(pageNumber: number): Promise<ExplorePageLoadResult> {
    const intent = ++this.foregroundIntent;
    return this.load(pageNumber, "foreground", intent);
  }

  prefetchAdjacent(): void {
    if (!this.queryId || !this.pages.has(this.currentPage)) return;
    const next = this.currentPage + 1;
    const previous = this.currentPage - 1;
    // Forward navigation is intentionally submitted before back navigation.
    if (next <= this.totalPages) void this.load(next, "prefetch-next");
    if (previous >= 1) void this.load(previous, "prefetch-previous");
  }

  private load(
    pageNumber: number,
    priority: ExplorePagePriority,
    foregroundIntent?: number,
  ): Promise<ExplorePageLoadResult> {
    const queryId = this.queryId;
    const generation = this.generation;
    if (!queryId || !Number.isInteger(pageNumber) || pageNumber < 1 || pageNumber > this.totalPages) {
      return Promise.resolve({
        status: "failed",
        error: { code: "PAGE_OUT_OF_RANGE", message: "요청한 검색 페이지가 범위를 벗어났습니다.", retryable: false, action: "none" },
      });
    }

    const cached = this.pages.get(pageNumber);
    if (cached) {
      cached.lastAccessed = this.now();
      if (priority === "foreground" && foregroundIntent === this.foregroundIntent) {
        this.activateForeground(pageNumber);
      }
      return Promise.resolve({ status: "ready", page: cached.page, source: "cache", scrollTop: this.scrollFor(pageNumber) });
    }

    if (priority === "foreground") {
      // A direct request can retry a failed prefetch exactly once per user
      // action; prefetch itself remains suppressed by the failure record.
      this.failures.delete(pageNumber);
    } else {
      const failure = this.failures.get(pageNumber);
      if (failure) {
        failure.lastAccessed = this.now();
        return Promise.resolve({ status: "failed", error: failure.error });
      }
    }

    const existing = this.inFlight.get(pageNumber);
    if (existing) {
      if (priority === "foreground" && existing.priority !== "foreground") {
        existing.priority = "foreground";
        existing.foregroundIntent = foregroundIntent;
        this.promotePage?.(queryId, pageNumber);
      } else if (priority === "foreground") {
        existing.foregroundIntent = foregroundIntent;
      }
      return existing.promise;
    }

    const request: InFlightPage = {
      requestId: `explore-page-${this.generation}-${++this.requestSequence}`,
      priority,
      ...(priority === "foreground" ? { foregroundIntent } : {}),
      promise: Promise.resolve({ status: "stale" }),
    };
    request.promise = this.resolvePage(queryId, generation, pageNumber, request);
    this.inFlight.set(pageNumber, request);
    return request.promise;
  }

  private async resolvePage(
    queryId: string,
    generation: number,
    pageNumber: number,
    request: InFlightPage,
  ): Promise<ExplorePageLoadResult> {
    let result: ApiResult<GalleryPage>;
    try {
      // Record the in-flight entry before an immediately resolving fetch can
      // settle it, so concurrent callers always coalesce.
      result = await Promise.resolve().then(() => this.fetchPage(queryId, pageNumber, request.requestId));
    } catch {
      result = {
        ok: false,
        error: { code: "BACKEND_UNAVAILABLE", message: "검색 페이지를 불러오지 못했습니다.", retryable: true, action: "retry" },
      };
    }

    if (generation !== this.generation || queryId !== this.queryId || this.inFlight.get(pageNumber) !== request) {
      return { status: "stale" };
    }
    this.inFlight.delete(pageNumber);

    if (!result.ok) {
      if (request.priority !== "foreground") {
        this.failures.set(pageNumber, { error: result.error, lastAccessed: this.now() });
        this.evictFailures();
      }
      return { status: "failed", error: result.error };
    }

    this.failures.delete(pageNumber);
    this.pages.get(pageNumber)?.releaseWarmup?.();
    this.pages.set(pageNumber, { page: result.data, lastAccessed: this.now() });
    this.totalPages = result.data.totalPages;

    if (request.priority === "foreground" && request.foregroundIntent === this.foregroundIntent) {
      this.activateForeground(pageNumber);
    } else {
      this.evictSettledPages();
      this.syncWarmups();
    }
    return { status: "ready", page: result.data, source: "network", scrollTop: this.scrollFor(pageNumber) };
  }

  private activateForeground(pageNumber: number): void {
    const entry = this.pages.get(pageNumber);
    if (!entry) return;
    entry.lastAccessed = this.now();
    this.currentPage = pageNumber;
    this.evictSettledPages();
    this.syncWarmups();
    this.prefetchAdjacent();
  }

  private syncWarmups(): void {
    if (!this.warmPage) return;
    for (const [pageNumber, entry] of this.pages) {
      const distance = pageNumber - this.currentPage;
      if (distance === 0 || Math.abs(distance) > 1) {
        if (entry.releaseWarmup) {
          entry.releaseWarmup();
          delete entry.releaseWarmup;
        }
        continue;
      }
      if (!entry.releaseWarmup) {
        const priority: Exclude<ExplorePagePriority, "foreground"> = distance > 0
          ? "prefetch-next"
          : "prefetch-previous";
        const release = this.warmPage(entry.page, priority);
        if (release) entry.releaseWarmup = release;
      }
    }
  }

  private evictSettledPages(): void {
    if (!this.currentPage) return;
    const candidates = [...this.pages.entries()]
      .filter(([page]) => page !== this.currentPage)
      .sort(([leftPage, left], [rightPage, right]) => {
        const leftDistance = Math.abs(leftPage - this.currentPage);
        const rightDistance = Math.abs(rightPage - this.currentPage);
        const leftOutside = leftDistance > 2 ? 1 : 0;
        const rightOutside = rightDistance > 2 ? 1 : 0;
        if (leftOutside !== rightOutside) return rightOutside - leftOutside;
        if (leftDistance !== rightDistance) return rightDistance - leftDistance;
        return left.lastAccessed - right.lastAccessed;
      });

    for (const [page, entry] of candidates) {
      const outsideWindow = Math.abs(page - this.currentPage) > 2;
      if (!outsideWindow && this.pages.size <= this.maxSettledPages) break;
      this.pages.delete(page);
      this.failures.delete(page);
      this.scrollPositions.delete(page);
      entry.releaseWarmup?.();
    }
  }

  private evictFailures(): void {
    for (const [page] of this.failures) {
      if (Math.abs(page - this.currentPage) > 2) this.failures.delete(page);
    }
    while (this.failures.size > this.maxSettledPages) {
      const oldest = [...this.failures.entries()]
        .sort(([, left], [, right]) => left.lastAccessed - right.lastAccessed)[0];
      if (!oldest) return;
      this.failures.delete(oldest[0]);
    }
  }

  private reset(): void {
    for (const entry of this.pages.values()) entry.releaseWarmup?.();
    if (this.cancelPage) {
      for (const request of this.inFlight.values()) {
        void Promise.resolve(this.cancelPage(request.requestId)).catch(() => undefined);
      }
    }
    this.queryId = null;
    this.currentPage = 0;
    this.totalPages = 0;
    this.pages.clear();
    this.failures.clear();
    this.inFlight.clear();
    this.scrollPositions.clear();
  }
}
