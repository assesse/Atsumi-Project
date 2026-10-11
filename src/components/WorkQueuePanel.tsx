import { useEffect, useRef, useState } from "react";
import type { QueueQuery, QueueRow, QueueSnapshot } from "../api/workConsole";
import { runningDownloadStates } from "../state/downloadCancellation";
import { downloadStatus } from "../state/downloadStatus";
import { GalleryStatusIcon } from "./GalleryStatusIcon";
import { splitGalleryTitle } from "./galleryCardLayout";
import type { GalleryId } from "../core/types";
import "./GalleryProcessingBadge.css";
import "./WorkQueuePanel.css";

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
  const waiting = (counts.queued ?? 0) + (counts.retry_wait ?? 0);
  const working = p.active - waiting;
  const stages = [
    ["정보 확인", counts.resolving_metadata ?? 0], ["다운로드", counts.downloading ?? 0],
    ["해시", counts.hashing ?? 0], ["검증", counts.verifying ?? 0],
  ] as const;
  const stageDetail = stages.filter(([, n]) => n > 0).map(([label, n]) => `${label} ${n}개`).join(" · ");
  const completionDetail = p.cancelled ? ` (취소·격리 ${p.cancelled}개 포함)` : "";
  const segments = [
    ["complete", "완료", p.completed, completionDetail],
    ["active", "작업 중", working, stageDetail ? ` · ${stageDetail}` : ""],
    ["waiting", "대기", waiting, counts.retry_wait ? ` (재시도 ${counts.retry_wait}개 포함)` : ""],
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
  onCancelEntries: (ids: string[]) => Promise<string>; onOpen: (id: GalleryId, options?: { background?: boolean }) => void };

export function WorkQueuePanel({ snapshot, query, error, onQuery, onRefresh, onCancelEntries, onOpen }: Props) {
  const [selected, setSelected] = useState(new Map<string, QueueRow>());
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const offset = snapshot?.offset ?? ((snapshot?.page ?? 1) - 1) * (snapshot?.pageSize ?? 100);
  const pageSize = snapshot?.pageSize ?? 100;
  const total = snapshot?.totalRows ?? 0;
  const rows = snapshot?.items.filter((row) => runningDownloadStates.has(row.state)) ?? [];
  useEffect(() => {
    if (!snapshot) return;
    const visible = new Map(snapshot.items.map((row) => [row.entryId, row]));
    setSelected((previous) => {
      const next = new Map(previous);
      for (const [id] of previous) {
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

  const cancelSelected = async () => {
    if (busy || error || !selected.size) return;
    const ids = [...selected.keys()]; // Never include later selections or arrivals.
    setBusy(true); setMessage("");
    try {
      await onCancelEntries(ids);
      setSelected((previous) => { const next = new Map(previous); ids.forEach((id) => next.delete(id)); return next; });
      onRefresh();
    } catch (e) { setMessage(String(e)); }
    finally { setBusy(false); }
  };

  return <section className="work-queue-panel" aria-label="대기 큐 관리">
    <p className="queue-empty">현재 대기·다운로드·검증·재시도 중인 작업입니다. 검토 대기·종료된 작업은 빠집니다.</p>
    <div className="queue-actions"><button type="button" disabled={!selected.size || busy || Boolean(error)} onClick={() => void cancelSelected()}>
      {busy ? "취소 중…" : "선택 " + selected.size + "개 취소"}
    </button></div>
    {message ? <p className="queue-error" role="alert">{message}</p> : null}
    <div ref={list} className="queue-items" role="list" aria-label="남은 대기 큐" aria-busy={!snapshot || busy}
      onScroll={(event) => {
        const topRow = Math.floor(event.currentTarget.scrollTop / ROW_HEIGHT);
        const next = Math.min(Math.max(0, total - pageSize), Math.max(0, Math.floor(topRow / 50) * 50 - 25));
        if (next !== (query.offset ?? 0)) onQuery({ offset: next });
      }}>
      {total > 0 ? <div aria-hidden="true" style={{ height: offset * ROW_HEIGHT }} /> : null}
      {rows.map((row) => <article className="queue-item" key={row.entryId} role="listitem" style={{ height: ROW_HEIGHT }}>
        <input type="checkbox" aria-label={row.title + " 선택"} disabled={busy} checked={selected.has(row.entryId)} onChange={(event) => {
          const checked = event.target.checked;
          setSelected((previous) => { const next = new Map(previous); if (checked) next.set(row.entryId, row); else next.delete(row.entryId); return next; });
        }} />
        <button type="button" className="queue-item-title" title={row.title} onClick={(event) => onOpen(row.galleryId, { background: event.ctrlKey || event.metaKey })}>
          <strong>{splitGalleryTitle(row.title).primary}</strong><small>{row.artist} · #{row.galleryId}</small>
        </button>
        <span className="queue-item-status" title={downloadStatus[row.state].label + " · " + Math.floor(row.progress) + "%"}>
          <span className={"gallery-processing-badge tone-" + downloadStatus[row.state].tone} role="img" aria-label={downloadStatus[row.state].label}><GalleryStatusIcon kind={row.state} /></span>
        </span>
      </article>)}
      {total > 0 ? <div aria-hidden="true" style={{ height: Math.max(0, total - offset - rows.length) * ROW_HEIGHT }} /> : null}
      {snapshot && !total ? <p className="queue-empty">남은 작업이 없습니다.</p> : null}
    </div>
  </section>;
}
