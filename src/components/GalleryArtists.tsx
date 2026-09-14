import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type MouseEventHandler } from "react";
import { createPortal } from "react-dom";
import { normalizeTokenValue } from "../search/searchTokens";
import "./GalleryArtists.css";

export type GalleryArtistsProps = {
  artist: string;
  artists?: readonly string[];
  favoriteMetadata: ReadonlySet<string>;
  compact?: boolean;
  disabled?: boolean;
  onSearch: (token: string) => void;
  onToggleFavorite: (token: string) => void;
  onClickCapture?: MouseEventHandler<HTMLButtonElement>;
};

/** Presentation only: never rewrite the primary artist used for grouping. */
const normalizedFavoriteCache = new WeakMap<ReadonlySet<string>, ReadonlySet<string>>();
export function orderGalleryArtists(artist: string, artists: readonly string[] | undefined, favoriteMetadata: ReadonlySet<string>) {
  let favorites = normalizedFavoriteCache.get(favoriteMetadata);
  if (!favorites) {
    favorites = new Set(Array.from(favoriteMetadata)
      .filter((token) => /^artist:/i.test(token.trim()))
      .map((token) => normalizeTokenValue(token.trim().slice(7))));
    normalizedFavoriteCache.set(favoriteMetadata, favorites);
  }
  const unique = new Map<string, { key: string; name: string; token: string; favorite: boolean; sourceIndex: number }>();
  const known = artists?.some((name) => name.trim()) ? artists : [artist];
  for (const value of known) {
    const name = value.trim();
    const key = normalizeTokenValue(name);
    if (key && !unique.has(key)) unique.set(key, { key, name: name.replaceAll("_", " "), token: `artist:${name}`, favorite: favorites.has(key), sourceIndex: unique.size });
  }
  return Array.from(unique.values()).sort((left, right) => Number(right.favorite) - Number(left.favorite));
}

export function fitGalleryArtistCount(widths: readonly number[], available: number, overflowWidths: readonly number[], total: number, gap = 5): number {
  const maximum = Math.min(3, widths.length, total);
  for (let count = maximum; count > 1; count -= 1) {
    const hidden = total - count;
    const required = widths.slice(0, count).reduce((sum, width) => sum + width, 0)
      + gap * (count - 1) + (hidden > 0 ? (overflowWidths[count - 1] ?? 0) + gap : 0);
    if (required <= available) return count;
  }
  return Math.min(1, total);
}

// One observer serves all mounted cards; only the open popover owns window listeners.
const widthSubscribers = new Map<Element, () => void>();
let widthObserver: ResizeObserver | undefined;
function observeArtistWidth(element: Element, update: () => void) {
  if (typeof ResizeObserver === "undefined") return () => {};
  widthObserver ??= new ResizeObserver((entries) => {
    for (const entry of entries) widthSubscribers.get(entry.target)?.();
  });
  widthSubscribers.set(element, update);
  widthObserver.observe(element);
  return () => {
    widthSubscribers.delete(element);
    widthObserver?.unobserve(element);
    if (!widthSubscribers.size) {
      widthObserver?.disconnect();
      widthObserver = undefined;
    }
  };
}

export function GalleryArtists({ artist, artists, favoriteMetadata, compact = false, disabled = false, onSearch, onToggleFavorite, onClickCapture }: GalleryArtistsProps) {
  const ordered = useMemo(() => orderGalleryArtists(artist, artists, favoriteMetadata), [artist, artists, favoriteMetadata]);
  const sourceKey = JSON.stringify([artist, artists]);
  const layoutKey = JSON.stringify(ordered.map(({ key, favorite }) => [key, favorite]));
  const [fit, setFit] = useState<{ key: string; count: number } | null>(null);
  const visibleCount = compact ? Math.min(1, ordered.length) : fit?.key === layoutKey ? fit.count : Math.min(2, ordered.length);
  const hiddenCount = ordered.length - visibleCount;
  const lineRef = useRef<HTMLDivElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const measureRef = useRef<HTMLSpanElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pinned = useRef(false);
  const focusOnOpen = useRef(false);
  const [openSource, setOpenSource] = useState<string | null>(null);
  const open = openSource === sourceKey && hiddenCount > 0 && !disabled;
  const [position, setPosition] = useState({ left: -10_000, top: -10_000 });
  const popoverId = useId();

  const cancelClose = useCallback(() => {
    if (closeTimer.current !== undefined) clearTimeout(closeTimer.current);
    closeTimer.current = undefined;
  }, []);
  const close = useCallback((restoreFocus = false) => {
    cancelClose();
    pinned.current = false;
    focusOnOpen.current = false;
    setOpenSource(null);
    if (restoreFocus) moreRef.current?.focus();
  }, [cancelClose]);
  const scheduleClose = () => {
    cancelClose();
    if (pinned.current) return;
    closeTimer.current = setTimeout(() => {
      if (!lineRef.current?.contains(document.activeElement) && !popoverRef.current?.contains(document.activeElement)) close();
    }, 180);
  };

  useEffect(() => () => cancelClose(), [cancelClose]);
  useEffect(() => {
    close();
  }, [sourceKey, close]);
  useEffect(() => { if (disabled || hiddenCount === 0) close(); }, [disabled, hiddenCount, close]);

  useLayoutEffect(() => {
    const line = lineRef.current;
    if (!line || compact || ordered.length < 2) return;
    const update = () => {
      const width = line.getBoundingClientRect().width;
      const names = Array.from(measureRef.current?.querySelectorAll<HTMLElement>("[data-measure-artist]") ?? []).map((element) => element.getBoundingClientRect().width);
      if (width <= 0 || names.some((value) => value <= 0)) return;
      const overflow = Array.from(measureRef.current?.querySelectorAll<HTMLElement>("[data-measure-overflow]") ?? []).map((element) => element.getBoundingClientRect().width);
      const count = fitGalleryArtistCount(names, width, overflow, ordered.length);
      setFit((current) => current?.key === layoutKey && current.count === count ? current : { key: layoutKey, count });
    };
    update();
    return observeArtistWidth(line, update);
  }, [compact, layoutKey, ordered.length]);

  useLayoutEffect(() => {
    if (!open) return;
    const update = () => {
      const anchor = moreRef.current?.getBoundingClientRect();
      const panel = popoverRef.current?.getBoundingClientRect();
      if (!anchor || !panel) return;
      const below = anchor.bottom + 7;
      setPosition({
        left: Math.max(8, Math.min(anchor.right - panel.width, window.innerWidth - panel.width - 8)),
        top: Math.max(8, Math.min(below + panel.height <= window.innerHeight - 8 ? below : anchor.top - panel.height - 7, window.innerHeight - panel.height - 8)),
      });
    };
    update();
    if (focusOnOpen.current) {
      popoverRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
      focusOnOpen.current = false;
    }
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !lineRef.current?.contains(event.target) && !popoverRef.current?.contains(event.target)) close();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close(Boolean(popoverRef.current?.contains(document.activeElement)));
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape, true);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", escape, true);
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open, close, layoutKey]);

  if (!ordered.length) return (
    <div className={`gallery-artists${compact ? " is-compact" : ""}`}>
      <span className="gallery-artists-label">작가 정보 없음</span>
    </div>
  );
  const search = (token: string) => { close(); onSearch(token); };
  const portalTarget = lineRef.current?.closest("dialog[open]") ?? document.body;

  return (
    <div
      ref={lineRef}
      className={`gallery-artists${compact ? " is-compact" : ""}`}
      onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
      onMouseLeave={scheduleClose}
      onMouseEnter={cancelClose}
      onBlur={(event) => {
        if (event.relatedTarget instanceof Node && (event.currentTarget.contains(event.relatedTarget) || popoverRef.current?.contains(event.relatedTarget))) return;
        if (open) close();
      }}
    >
      {ordered.slice(0, visibleCount).map((item) => (
        <button
          type="button"
          disabled={disabled}
          key={item.key}
          className={`byline artist gallery-artists-name${item.favorite ? " favorite" : ""}`}
          aria-label={`${item.name}${item.favorite ? ", 즐겨찾기" : ""}, 좌클릭 검색, 우클릭 즐겨찾기 변경`}
          title={`${item.name} · 좌클릭 검색 / 우클릭 즐겨찾기`}
          onClickCapture={onClickCapture}
          onClick={(event) => { if (!event.defaultPrevented) search(item.token); }}
          onContextMenu={(event) => { event.preventDefault(); if (!disabled) onToggleFavorite(item.token); }}
        >
          {item.favorite ? <span className="gallery-artists-star" aria-hidden="true">★</span> : null}
          <span className="gallery-artists-label">{item.name}</span>
        </button>
      ))}
      {hiddenCount > 0 ? (
        <button
          type="button"
          disabled={disabled}
          className="gallery-artists-more"
          ref={moreRef}
          aria-label={`참여 작가 ${ordered.length}명 모두 보기, ${hiddenCount}명 더 있음`}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-controls={open ? popoverId : undefined}
          onClickCapture={onClickCapture}
          onMouseEnter={() => { if (!disabled) { cancelClose(); setOpenSource(sourceKey); } }}
          onKeyDown={(event) => {
            if (event.key !== "ArrowDown") return;
            event.preventDefault();
            pinned.current = true;
            if (open) popoverRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
            else { focusOnOpen.current = true; setOpenSource(sourceKey); }
          }}
          onClick={(event) => {
            if (event.defaultPrevented) return;
            cancelClose();
            if (open && pinned.current) { close(); return; }
            pinned.current = true;
            if (event.detail === 0 && open) popoverRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
            else focusOnOpen.current = event.detail === 0;
            setOpenSource(sourceKey);
          }}
        >{compact ? `외 ${hiddenCount}명` : `+${hiddenCount}명`}</button>
      ) : null}
      {!compact && ordered.length > 1 ? (
        <span className="gallery-artists-measure" ref={measureRef} aria-hidden="true">
          {ordered.slice(0, 3).map((item) => (
            <span className="byline artist gallery-artists-name" data-measure-artist key={item.key}>
              {item.favorite ? <span className="gallery-artists-star">★</span> : null}<span>{item.name}</span>
            </span>
          ))}
          {ordered.slice(0, 3).map((item, index) => <span className="gallery-artists-more" data-measure-overflow key={item.key}>+{ordered.length - index - 1}명</span>)}
        </span>
      ) : null}
      {open && hiddenCount > 0 ? createPortal(
        <div
          ref={popoverRef}
          id={popoverId}
          role="dialog"
          aria-label={`참여 작가 ${ordered.length}명`}
          className="gallery-artists-popover"
          style={position}
          onMouseEnter={cancelClose}
          onMouseLeave={scheduleClose}
          onPointerDown={(event) => event.stopPropagation()}
          onBlur={(event) => {
            if (event.relatedTarget instanceof Node && (event.currentTarget.contains(event.relatedTarget) || lineRef.current?.contains(event.relatedTarget))) return;
            close();
          }}
        >
          <div className="gallery-artists-popover-heading">참여 작가 <span>{ordered.length}명</span></div>
          <ul>
            {ordered.slice().sort((left, right) => left.sourceIndex - right.sourceIndex).map((item) => (
              <li key={item.key}>
                <button
                  type="button"
                  className="gallery-artists-popover-search"
                  aria-label={`${item.name} 작가 검색`}
                  onClickCapture={onClickCapture}
                  onClick={(event) => { if (!event.defaultPrevented) search(item.token); }}
                  onContextMenu={(event) => { event.preventDefault(); onToggleFavorite(item.token); }}
                >{item.name}</button>
                <button
                  type="button"
                  className="gallery-artists-popover-favorite"
                  aria-label={`${item.name} 즐겨찾기 ${item.favorite ? "해제" : "등록"}`}
                  aria-pressed={item.favorite}
                  title={`즐겨찾기 ${item.favorite ? "해제" : "등록"}`}
                  onClickCapture={onClickCapture}
                  onClick={(event) => { if (!event.defaultPrevented) onToggleFavorite(item.token); }}
                  onContextMenu={(event) => { event.preventDefault(); onToggleFavorite(item.token); }}
                ><span aria-hidden="true">{item.favorite ? "★" : "☆"}</span></button>
              </li>
            ))}
          </ul>
          <div className="gallery-artists-popover-hint">이름으로 검색 · 별표로 즐겨찾기 변경</div>
        </div>, portalTarget,
      ) : null}
    </div>
  );
}
