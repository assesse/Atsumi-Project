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
  const completed = counts.completed ?? 0;
  const review = counts.review_required ?? 0;
  const failed = (counts.failed ?? 0) + (counts.interrupted ?? 0);
  const cancelled = (counts.cancelled ?? 0) + (counts.quarantined ?? 0);
  // Processing can settle without a successful download. Preserve outcome
  // colors, but don't leave the overall indicator unfinished after a stop.
  const settled = Math.max(0, total - active);
  return { total, active, completed, review, failed, cancelled, percent: total ? Math.floor(settled / total * 100) : 0 };
}

/** The bar is the entire summary; each segment explains itself on hover/focus. */
export function QueueSummary({ snapshot, error }: { snapshot: QueueSnapshot | null; scoped?: boolean; error?: string | null }) {
  const counts = snapshot?.counts ?? {};
  const p = queueProgress(snapshot);
  const segments = [
    ["complete", "완료", p.completed], ["waiting", "대기", counts.queued ?? 0],
    ["metadata", "정보 확인", counts.resolving_metadata ?? 0],
    ["active", "다운로드", counts.downloading ?? 0], ["hashing", "해시", counts.hashing ?? 0],
    ["verifying", "검증", counts.verifying ?? 0], ["retry", "재시도 대기", counts.retry_wait ?? 0],
    ["review", "검토 필요", p.review], ["error", "실패·중단", p.failed], ["muted", "취소·격리", p.cancelled],
  ] as const;
  return <section className={"queue-summary" + (error ? " is-stale" : "")} aria-label="다운로드 진행 요약">
    <div className="queue-progress-track" role="group" aria-label={snapshot ? "자동 처리 종료 " + p.percent + "% (완료·검토 대기·중단 포함)" : "큐 확인 중"} aria-busy={!snapshot}>
      {segments.filter(([, , n]) => n > 0).map(([tone, label, n]) => <span key={tone}
        className={"tone-" + tone + (p.active > 0 && ["metadata", "active", "hashing", "verifying"].includes(tone) ? " is-running" : "")}
        style={{ width: (p.total ? n / p.total * 100 : 0) + "%" }} title={label + " " + n + "개"} aria-label={label + " " + n + "개"} role="img" tabIndex={0} />)}
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

  return <section className="work-queue-panel" aria-label="다운로드 큐 관리">
    <QueueSummary snapshot={snapshot} error={error} />
    <p className="queue-empty">현재 대기·다운로드·검증·재시도 중인 작업입니다. 검토 대기·종료된 작업은 빠집니다.</p>
    <div className="queue-actions"><button type="button" disabled={!selected.size || busy || Boolean(error)} onClick={() => void cancelSelected()}>
      {busy ? "취소 중…" : "선택 " + selected.size + "개 취소"}
    </button></div>
    {message ? <p className="queue-error" role="alert">{message}</p> : null}
    <div ref={list} className="queue-items" role="list" aria-label="남은 다운로드 큐" aria-busy={!snapshot || busy}
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
