import { useLayoutEffect, useSyncExternalStore } from "react";
import { pagePreviewAspect } from "../../components/pagePreviewFrame";
import type { PixivDetail } from "./model";
import { originalPageWindow, pixivPageSource, type OriginalWindow } from "./originalWindow";

export function PixivOriginalPages({ detail, pages, pool, privacyMode }: {
  detail: PixivDetail; pages: (number | null)[]; pool: OriginalWindow; privacyMode: boolean;
}) {
  const signature = pages.join(",");
  useLayoutEffect(() => {
    if (privacyMode) { pool.clear(); return; }
    const window = originalPageWindow(pages, detail.pages.length);
    pool.setWindow(window.visible.map(index => pixivPageSource(detail, index)), window.prefetch.map(index => pixivPageSource(detail, index)));
  }, [detail, signature, pool, privacyMode]);
  useSyncExternalStore(pool.subscribe, pool.snapshot, pool.snapshot);
  const fallback = detail.pages[pages.find((page): page is number => page !== null) ?? 0];
  const aspects = pages.map(page => pagePreviewAspect(page === null ? fallback : detail.pages[page]));
  // Grow factors below one intentionally leave free space unused. Normalize
  // every slot to at least one without changing mixed-page or blank ratios.
  const minimumAspect = Math.min(...aspects);
  return <div className="pixiv-original-pages">{pages.map((page, slot) => {
    const key = page === null ? null : pixivPageSource(detail, page);
    const image = key ? pool.get(key) : undefined;
    return <div key={slot} className={`pixiv-original-slot${page === null ? " is-blank" : ""}`} style={{ flex: `${aspects[slot]! / minimumAspect} 1 0%` }} data-page={page === null ? "blank" : page + 1}>
      {page === null ? null : privacyMode ? <span>프라이버시 모드</span> : image?.state === "ready" ? <><img src={image.url} alt={`${detail.work.title} · ${page + 1}쪽`} draggable={false} onError={() => pool.failedImage(key!)} />{image.preview && <span className="pixiv-display-preview-label">대용량 원본 · 화면용 미리보기</span>}</>
        : image?.state === "error" ? <div role="alert"><span>{image.error}</span><button type="button" onClick={() => pool.retry(key!)}>다시 시도</button></div> : <span className="pixiv-spinner" role="status" aria-label="원본 불러오는 중" />}
    </div>;
  })}</div>;
}
