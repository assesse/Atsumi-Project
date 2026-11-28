import { useEffect, useId, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { getQueueSelectionRange, queueRangeInWindow } from "../api/workConsole";
import { backend } from "../api/backend";
import { QueueAlbumPreview } from "./QueueAlbumPreview";
import type { QueueQuery, QueueRow, QueueSnapshot } from "../api/workConsole";
import { runningDownloadStates } from "../state/downloadCancellation";
import { downloadStatus } from "../state/downloadStatus";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
import { DEFAULT_GALLERY_PREVIEW_WIDTH } from "../layout/galleryPreviewPresets";
import { splitGalleryTitle } from "./galleryCardLayout";
import type { Gallery, GalleryId } from "../core/types";
import "./GalleryProcessingBadge.css";
import "./WorkQueuePanel.css";

const postprocessingLabels: Record<string,string> = { waiting: "후처리 대기", verifying_files: "파일 검증 중", hashing: "해시 계산 중", comparing: "중복 비교 중", finalizing: "완료 검증 중", unknown: "후처리 상태 확인 중" };
export function queueRowStatus(row: QueueRow) { return row.postprocessing ? postprocessingLabels[row.postprocessing] ?? postprocessingLabels.unknown : downloadStatus[row.state].label; }

export function queueProgress(snapshot: QueueSnapshot | null) {
  const counts = snapshot?.counts ?? {};
  const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
  const active = [...runningDownloadStates].reduce((sum, state) => sum + (counts[state] ?? 0), 0);
  const cancelled = (counts.cancelled ?? 0) + (counts.quarantined ?? 0);
  const completed = (counts.completed ?? 0) + cancelled;
  const review = counts.review_required ?? 0;
  const failed = (counts.failed ?? 0) + (counts.interrupted ?? 0);
  // Processing can settle without a successful download. Preserve outcome
  // colors, but don't leave the overall indicator unfinished after a stop.
  const settled = Math.max(0, total - active);
  return { total, active, completed, review, failed, cancelled, percent: total ? Math.floor(settled / total * 100) : 0 };
}

/** Stable segments animate real count changes; motion never estimates progress. */
export function QueueSummary({ snapshot, scoped = false, error }: { snapshot: QueueSnapshot | null; scoped?: boolean; error?: string | null }) {
  const counts = snapshot?.counts ?? {};
  const p = queueProgress(snapshot);
  const phases = snapshot?.postprocessingCounts;
  const waiting = (counts.queued ?? 0) + (counts.retry_wait ?? 0) + (phases?.waiting ?? 0) + (phases?.unknown ?? 0);
  const working = p.active - waiting;
  const stages = [
    ["정보 확인", counts.resolving_metadata ?? 0], ["다운로드", counts.downloading ?? 0],
    ... (phases ? Object.entries(phases).filter(([key]) => key !== "waiting" && key !== "unknown").map(([key,n]) => [postprocessingLabels[key] ?? key,n] as const)
      : [["후처리", (counts.hashing ?? 0) + (counts.verifying ?? 0)] as const]),
  ] as const;
  const stageDetail = stages.filter(([, n]) => n > 0).map(([label, n]) => `${label} ${n}개`).join(" · ");
  const waitingDetail = [counts.retry_wait ? `재시도 ${counts.retry_wait}개` : "", phases?.waiting ? `후처리 대기 ${phases.waiting}개` : "", phases?.unknown ? `상태 확인 ${phases.unknown}개` : ""].filter(Boolean).join(" · ");
  const completionDetail = p.cancelled ? ` (취소·격리 ${p.cancelled}개 포함)` : "";
  const segments = [
    ["complete", "완료", p.completed, completionDetail],
    ["active", "작업 중", working, stageDetail ? ` · ${stageDetail}` : ""],
    ["waiting", "대기", waiting, waitingDetail ? ` · ${waitingDetail}` : ""],
    ["review", "검토 필요", p.review, ""], ["error", "실패·중단", p.failed, ""],
  ] as const;
  const completionEdge = useRef<HTMLSpanElement>(null);
  const completionCount = useRef<HTMLElement>(null);
  const previous = useRef({ completed: p.completed, total: p.total, ready: Boolean(snapshot), stale: Boolean(error), scoped });
  useEffect(() => {
    const before = previous.current;
    previous.current = { completed: p.completed, total: p.total, ready: Boolean(snapshot), stale: Boolean(error), scoped };
    // Opening, changing scope, or recovering stale counts is not a new completion.
    if (!snapshot || error || !before.ready || before.stale || before.scoped !== scoped
      || before.total !== p.total || p.completed <= before.completed
      || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    const animations = [
      completionEdge.current?.animate?.([
        { opacity: 0, transform: "translateX(-14px)" },
        { opacity: .85, offset: .3 }, { opacity: 0, transform: "translateX(0)" },
      ], { duration: 850, easing: "ease-out" }),
      completionCount.current?.animate?.([
        { opacity: .5, transform: "translateY(3px)" }, { opacity: 1, transform: "translateY(0)" },
      ], { duration: 380, easing: "ease-out" }),
    ];
    return () => animations.forEach((animation) => animation?.cancel());
  }, [p.completed, p.total, scoped, Boolean(snapshot), Boolean(error)]);

  const running = working > 0 && !error;
  const finished = p.total > 0 && p.completed === p.total;
  return <section className={"queue-summary" + (error ? " is-stale" : "") + (running ? " is-running" : "") + (finished ? " is-finished" : "")} aria-label="다운로드 진행 요약">
    <div className="queue-summary-heading">
      <span>{scoped ? "선택한 작업" : "전체 작업"}</span>
      <span className="queue-summary-count" title={snapshot ? `완료 ${p.completed}개${completionDetail}` : undefined}>
        {snapshot ? p.total ? <><strong ref={completionCount}>{p.completed.toLocaleString()}</strong><span> / {p.total.toLocaleString()} 완료</span></> : "작업 없음" : "확인 중…"}
      </span>
    </div>
    <div className="queue-progress-track" role="group" aria-label={snapshot ? "자동 처리 종료 " + p.percent + "% (완료·검토 대기·중단 포함)" : "큐 확인 중"} aria-busy={!snapshot}>
      {segments.map(([tone, label, n, detail]) => <span key={tone} className={"queue-progress-segment tone-" + tone}
        style={{ width: `${p.total ? n / p.total * 100 : 0}%` }} title={`${label} ${n}개${detail}`}
        aria-label={`${label} ${n}개${detail}`} aria-hidden={n === 0} role="img">
        {tone === "complete" ? <span ref={completionEdge} className="queue-completion-edge" aria-hidden="true" /> : null}
      </span>)}
    </div>
    <div className="queue-summary-legend">
      {segments.filter(([tone, , n]) => n > 0 || ["complete", "active", "waiting"].includes(tone)).map(([tone, label, n, detail]) =>
        <span key={tone} className={"tone-" + tone} title={`${label} ${n}개${detail}`}>
          <i className="queue-legend-dot" aria-hidden="true" />{label} <b>{snapshot ? n.toLocaleString() : "—"}</b>
        </span>)}
    </div>
    {error ? <p className="queue-error" role="alert">큐 갱신 지연 · 마지막 확인값입니다.</p> : null}
  </section>;
}

const ROW_HEIGHT = 62;
type Props = { snapshot: QueueSnapshot | null; query: QueueQuery; error: string | null; onQuery: (query: QueueQuery) => void; onRefresh: () => void;
  previewWidth?: number;
  galleries?: ReadonlyMap<GalleryId, Gallery>;
  onCancelEntries: (ids: string[]) => Promise<string>; onOpen: (id: GalleryId, options?: { background?: boolean }) => void };

export function WorkQueuePanel({ snapshot, query, error, onQuery, onRefresh, onCancelEntries, onOpen, galleries, previewWidth = DEFAULT_GALLERY_PREVIEW_WIDTH }: Props) {
  const [selected, setSelected] = useState(new Set<string>());
  const anchor = useRef<string | null>(null);
  const selectionIntent = useRef(0);
  const [rangePending, setRangePending] = useState(false);
  const [preview, setPreview] = useState<{ id: string; element: HTMLElement } | null>(null);
  const previewId = useId();
  const hoverPointer = useRef({ x: -1, y: -1, scrolled: false });
  const [resolved, setResolved] = useState(new Map<GalleryId, { title: string; artist: string }>());
  const lookup = useRef(new Map<GalleryId, ReturnType<typeof backend.gallerySummaryGet>>());
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const offset = snapshot?.offset ?? ((snapshot?.page ?? 1) - 1) * (snapshot?.pageSize ?? 100);
  const pageSize = snapshot?.pageSize ?? 100;
  const total = snapshot?.totalRows ?? 0;
  const rows = snapshot?.items.filter((row) => runningDownloadStates.has(row.state)) ?? [];
  const titleOf = (row: QueueRow) => [row.title, galleries?.get(row.galleryId)?.title, resolved.get(row.galleryId)?.title]
    .find(title => title?.trim() && title !== "#" + row.galleryId && title !== "앨범 #" + row.galleryId) ?? "";
  const previewRow = preview && rows.find(row => row.entryId === preview.id);
  const missingTitleId = previewRow && !titleOf(previewRow) ? previewRow.galleryId : null;
  useEffect(() => {
    if (!missingTitleId) return;
    let current = true;
    const timer = window.setTimeout(() => {
      let request = lookup.current.get(missingTitleId);
      if (!request) {
        request = backend.gallerySummaryGet(missingTitleId);
        lookup.current.set(missingTitleId, request);
        void request.finally(() => lookup.current.delete(missingTitleId)).catch(() => {});
      }
      void request.then(result => {
        if (!current || !result.ok || result.data.id !== missingTitleId || !result.data.title.trim()) return;
        setResolved(previous => {
          const next = new Map(previous);
          if (next.size >= 200) next.delete(next.keys().next().value!);
          next.set(missingTitleId, { title: result.data.title, artist: result.data.artist });
          return next;
        });
      }).catch(() => {});
    }, 180);
    return () => { current = false; window.clearTimeout(timer); };
  }, [missingTitleId]);

  useEffect(() => {
    anchor.current = null;
    setSelected(new Set());
    setRangePending(false);
    setPreview(null);
    return () => { selectionIntent.current++; };
  }, [query.sequence, query.after]);

  useEffect(() => {
    if (!snapshot) return;
    const visible = new Map(snapshot.items.map((row) => [row.entryId, row]));
    setSelected((previous) => {
      const next = new Set(previous);
      for (const id of previous) {
        if (visible.has(id) && !runningDownloadStates.has(visible.get(id)!.state)) next.delete(id);
        else if (total === 0 || (offset === 0 && total <= pageSize && !visible.has(id))) next.delete(id);
      }
      return next.size === previous.size ? previous : next;
    });
    if ((query.offset ?? 0) >= total && (query.offset ?? 0) > 0) {
      const lastOffset = Math.max(0, total - pageSize);
      if (list.current) list.current.scrollTop = lastOffset * ROW_HEIGHT;
      onQuery({ offset: lastOffset });
    }
  }, [snapshot, total, pageSize, offset, query.offset, onQuery]);

  const selectRow = async (row: QueueRow, keys: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }, checkbox = false) => {
    if (busy) return;
    const intent = ++selectionIntent.current;
    const additive = keys.ctrlKey || keys.metaKey;
    setRangePending(false);
    setMessage("");
    if (keys.shiftKey && anchor.current) {
      let ids = queueRangeInWindow(rows, anchor.current, row.entryId);
      if (!ids.length) {
        setRangePending(true);
        try { ids = await getQueueSelectionRange(query, anchor.current, row.entryId); }
        catch { if (intent === selectionIntent.current) setMessage("선택 범위를 불러오지 못했습니다."); return; }
        finally { if (intent === selectionIntent.current) setRangePending(false); }
      }
      if (intent !== selectionIntent.current) return;
      if (ids.length) {
        setSelected(previous => additive ? new Set([...previous, ...ids]) : new Set(ids));
        return;
      }
      // The old anchor settled while scrolling. Start a new range at the clicked row.
    }
    anchor.current = row.entryId;
    setSelected(previous => {
      if (!checkbox && !additive) return new Set([row.entryId]);
      const next = new Set(previous);
      if (next.has(row.entryId)) next.delete(row.entryId); else next.add(row.entryId);
      return next;
    });
  };

  function previewAtPointer(id: string, event: ReactMouseEvent<HTMLButtonElement>) {
    const pointer = hoverPointer.current;
    // Chromium can send mouseenter when a new row scrolls beneath a stationary pointer.
    if (pointer.scrolled && pointer.x === event.clientX && pointer.y === event.clientY) return;
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    pointer.scrolled = false;
    if (preview?.id !== id || preview.element !== event.currentTarget) {
      setPreview({ id, element: event.currentTarget });
    }
  }

  const cancelSelected = async () => {
    if (busy || rangePending || error || !selected.size) return;
    const ids = [...selected]; // Never include later selections or arrivals.
    setBusy(true); setMessage("");
    try {
      await onCancelEntries(ids);
      setSelected((previous) => { const next = new Set(previous); ids.forEach((id) => next.delete(id)); return next; });
      onRefresh();
    } catch (e) { setMessage(String(e)); }
    finally { setBusy(false); }
  };

  return <section className="work-queue-panel" aria-label="대기 큐 관리">
    {message ? <p className="queue-error" role="alert">{message}</p> : null}
    <div ref={list} className="queue-items" role="list" aria-label="남은 대기 큐" aria-busy={!snapshot || busy}
      onScroll={event => {
        hoverPointer.current.scrolled = true;
        setPreview(null);
        const first = Math.floor(event.currentTarget.scrollTop / ROW_HEIGHT);
        const half = Math.max(1, Math.floor(pageSize / 2));
        const next = Math.min(Math.max(0, total - pageSize), Math.max(0, Math.floor(first / half) * half - Math.floor(half / 2)));
        if (next !== (query.offset ?? 0)) onQuery({ offset: next });
      }}>
      {offset > 0 && <div aria-hidden="true" style={{ height: offset * ROW_HEIGHT }} />}
      {rows.map((row, index) => {
        const fullTitle = titleOf(row);
        const title = fullTitle ? splitGalleryTitle(fullTitle).primary : "제목 미확인";
        const artist = row.artist || galleries?.get(row.galleryId)?.artist || resolved.get(row.galleryId)?.artist;
        const status = queueRowStatus(row);
        return <article className="queue-item" key={row.entryId} role="listitem"
          aria-posinset={offset + index + 1} aria-setsize={total} data-selected={selected.has(row.entryId)}
          style={{ height: ROW_HEIGHT }} onClick={event => void selectRow(row, event)}>
          <input type="checkbox" aria-label={(fullTitle || "#" + row.galleryId) + " 선택"} checked={selected.has(row.entryId)} disabled={busy}
            onClick={event => event.stopPropagation()} onChange={event => {
              const keys = event.nativeEvent instanceof MouseEvent ? event.nativeEvent : { ctrlKey: false, metaKey: false, shiftKey: false };
              void selectRow(row, keys, true);
            }} />
          <button type="button" className="queue-item-title" aria-describedby={preview?.id === row.entryId ? previewId : undefined}
            onMouseEnter={event => previewAtPointer(row.entryId, event)} onMouseMove={event => previewAtPointer(row.entryId, event)}
            onMouseLeave={() => setPreview(null)}
            onFocus={event => setPreview({ id: row.entryId, element: event.currentTarget })} onBlur={() => setPreview(null)}
            onClick={event => {
              event.stopPropagation();
              if (event.shiftKey || event.ctrlKey || event.metaKey) void selectRow(row, event);
              else { setPreview(null); onOpen(row.galleryId); }
            }}>
            <strong>{title}</strong>
            <small>{artist && <><span className="queue-item-artist">{artist}</span><span aria-hidden="true">·</span></>}
              <span className="queue-item-number">#{row.galleryId}</span></small>
          </button>
          <span className="queue-item-status" title={status + " · " + Math.floor(row.progress) + "%"}>
            <span role="img" aria-label={status} className={"gallery-processing-badge tone-" + downloadStatus[row.state].tone}>
              <GalleryStatusIcon kind={row.postprocessing === "waiting" ? "queued" : row.state} />
            </span>
            {row.postprocessing && <small>{status}</small>}
          </span>
        </article>;
      })}
      {total > offset + rows.length && <div aria-hidden="true" style={{ height: (total - offset - rows.length) * ROW_HEIGHT }} />}
      {snapshot && !total ? <p className="queue-empty">남은 작업이 없습니다.</p> : null}
    </div>
    {selected.size > 0 && <div className="queue-actions"><button type="button" disabled={rangePending || busy || Boolean(error)} onClick={() => void cancelSelected()}>
      {busy ? "취소 중…" : "선택 " + selected.size + "개 취소"}
    </button></div>}
    {preview && previewRow ? <QueueAlbumPreview key={preview.id} id={previewId} element={preview.element}
      width={previewWidth} row={previewRow} title={titleOf(previewRow)} gallery={galleries?.get(previewRow.galleryId)} /> : null}
  </section>;
}
