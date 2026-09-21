import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import type { BrowserDeleteReport, BrowserRecording } from "../../api/officialBrowser";
import { hasCompletedMerge, hasReplayableRanges, RecordingPlayback, recordingMergeLabel } from "./RecordingPlayback";
import "./RecordingLibrary.css";
import { RecordedChannel } from "./RecordedChannel";
import { emptyRecordingAttempt, recordingStatus } from "./recordingStatus";

const duration = (value: number) => {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${Math.floor(seconds / 60 % 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
};
const bytes = (value: number) => value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`;
const date = (value: number) => new Date(value).toLocaleString("ko-KR", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
const dateTime = (value: number) => { const instant = new Date(value); return Number.isFinite(instant.getTime()) ? instant.toISOString() : undefined; };
export function recordingChatNote(recording: BrowserRecording): { text: string; warning: boolean } {
  if (recording.captureChat === false) return { text: "이 기록은 채팅 저장을 사용하지 않았습니다.", warning: false };
  if (recording.captureChat !== true || !recording.chatStatus) return { text: "이 기록의 채팅 저장 상태는 확인되지 않았습니다.", warning: false };
  const warning = ["failed", "unavailable", "storage_failed", "disconnected", "observer_unavailable", "queue_overflow", "unsupported_frame", "frame_too_large", "invalid_frame", "message_too_large", "message_truncated", "decode_failed", "observer_overflow", "connection_gap", "partial"].includes(recording.chatStatus);
  const text = recording.chatStatus === "storage_failed" ? "채팅 저장 실패 · 일부 기록 누락 가능" : warning ? "채팅 일부 누락 가능" : recording.status === "recording" ? "채팅 기록 중" : "저장된 채팅";
  return { warning, text: text + (Number.isSafeInteger(recording.chatCount) && recording.chatCount! >= 0 ? ` · ${recording.chatCount!.toLocaleString("ko-KR")}개 기록` : "") };
}
function ChatWarning({ recording }: { recording: BrowserRecording }) {
  const note = recordingChatNote(recording);
  return note.warning ? <span className="official-browser-saved-chat-warning" role="img" aria-label={note.text} title={note.text}>⚠</span> : null;
}
const needsAttention = (recording: BrowserRecording) => recording.mediaRemovedAt == null && (recording.deletionPending || recording.status === "failed" || recording.status === "interrupted" && recording.ending?.reason !== "checking" || recording.progressive?.lastError || recording.merge?.status === "failed" || recording.merge?.status === "blocked" || recording.merge?.sourceCleanup?.status === "blocked" || recordingChatNote(recording).warning);
const canDelete = (item: BrowserRecording) => item.status !== "recording" && (item.deletionPending || item.merge?.status !== "merging" && item.merge?.sourceCleanup?.status !== "pending");
type Props = {
  recordings: BrowserRecording[]; selectedId: string | null; onSelect: (id: string) => void;
  disabled: boolean; privacy: boolean; retrying: boolean; opening: boolean;
  onFolder: (id: string) => void; onReplay: (id: string) => void; onOpenMerged: (id: string) => void;
  onRetryMerge: (id: string) => void; onOpenSegment: (id: string, index: number) => void;
  onDelete: (ids: string[]) => Promise<BrowserDeleteReport>;
  stopControl?: ReactNode;
};

/** Metadata-only browsing: entering the library never opens every video for a thumbnail. */
export function RecordingLibrary({ recordings, selectedId, onSelect, disabled, privacy, retrying, opening, onFolder, onReplay, onOpenMerged, onRetryMerge, onOpenSegment, onDelete, stopControl }: Props) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<"all" | "ready" | "attention">("all");
  const [limit, setLimit] = useState(24);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [diagnosticLimit, setDiagnosticLimit] = useState(24);
  const [selecting, setSelecting] = useState(false);
  const [checked, setChecked] = useState<string[]>([]);
  const [confirmIds, setConfirmIds] = useState<string[] | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteNotice, setDeleteNotice] = useState("");
  const [deleteFailures, setDeleteFailures] = useState<BrowserDeleteReport["failures"]>([]);
  const [deleteError, setDeleteError] = useState("");
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const deleteTrigger = useRef<HTMLButtonElement>(null);
  const selectionToggle = useRef<HTMLButtonElement>(null);
  const deleteInFlight = useRef(false);
  const dialogTitle = useId();
  const dialogDescription = useId();
  const active = recordings.filter(item => item.status === "recording");
  const attempts = recordings.filter(emptyRecordingAttempt);
  const diagnostics = recordings.filter(item => item.mediaRemovedAt != null);
  const saved = recordings.filter(item => item.mediaRemovedAt == null && item.status !== "recording" && !emptyRecordingAttempt(item));
  const filtered = useMemo(() => saved.filter(item => (privacy || !query.trim() || `${item.title} ${item.channelId}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()))
    && (filter === "all" || filter === "ready" && (hasCompletedMerge(item) || hasReplayableRanges(item)) || filter === "attention" && needsAttention(item))), [recordings, query, filter, privacy]);
  useEffect(() => setLimit(24), [query, filter]);
  // Filter changes never leave invisible recordings selected for deletion.
  useEffect(() => setChecked([]), [query, filter, privacy]);
  useEffect(() => setChecked(previous => {
    const next = previous.filter(id => recordings.some(item => item.id === id && canDelete(item)));
    return next.length === previous.length ? previous : next;
  }), [recordings]);
  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (confirmIds && !node.open) { node.showModal(); cancel.current?.focus(); }
    else if (!confirmIds && node.open) { node.close(); (deleteTrigger.current?.disabled ? selectionToggle.current : deleteTrigger.current)?.focus(); }
  }, [confirmIds]);
  useEffect(() => { if (privacy) setQuery(""); }, [privacy]);
  const selected = [...active, ...filtered, ...attempts, ...diagnostics].find(item => item.id === selectedId) ?? filtered[0] ?? active[0] ?? attempts[0];
  const title = (item: BrowserRecording) => privacy ? "녹화 영상" : item.title || item.channelId;
  const eligible = filtered.filter(canDelete).map(item => item.id);
  const allChecked = eligible.length > 0 && eligible.every(id => checked.includes(id));
  const toggle = (id: string) => { if (!disabled && !deleting) setChecked(previous => previous.includes(id) ? previous.filter(value => value !== id) : [...previous, id]); };
  const confirmDelete = async () => {
    if (!confirmIds?.length || disabled || deleteInFlight.current) return;
    deleteInFlight.current = true;
    setDeleting(true); setDeleteError(""); setDeleteNotice(""); setDeleteFailures([]);
    try {
      const report = await onDelete([...confirmIds]);
      setChecked(previous => previous.filter(id => !report.deletedIds.includes(id)));
      setDeleteNotice(report.deletedIds.length ? `${report.deletedIds.length}개 녹화를 삭제했습니다.` : "");
      setDeleteFailures(report.failures);
      setConfirmIds(null);
    } catch (error) {
      setDeleteError(error instanceof Error ? error.message : "삭제 결과를 확인하지 못했습니다. 목록을 확인한 뒤 다시 시도해 주세요.");
    } finally { deleteInFlight.current = false; setDeleting(false); }
  };
  const card = (item: BrowserRecording) => <div key={item.id} className={`recording-library-card-wrap${selecting && item.status !== "recording" ? " is-selecting" : ""}`}>
    <RecordedChannel id={item.id} privacy={privacy} />
    <button type="button" className={`recording-library-card official-browser-recording-select${item.status === "recording" ? " is-recording" : ""}`} aria-pressed={selecting && item.status !== "recording" ? checked.includes(item.id) : selected?.id === item.id} disabled={selecting && item.status !== "recording" && (!canDelete(item) || disabled || deleting)} title={selecting && !canDelete(item) ? "녹화·병합·파일 정리가 끝난 뒤 삭제할 수 있습니다." : recordingChatNote(item).text} aria-description={recordingChatNote(item).text} onClick={() => selecting && item.status !== "recording" ? toggle(item.id) : onSelect(item.id)}>
    <span className="recording-library-card-top"><span className={`recording-library-state${needsAttention(item) ? " needs-attention" : ""}`}><i aria-hidden />{item.deletionPending ? "삭제 미완료" : item.status === "recording" ? "녹화 중" : needsAttention(item) ? "확인 필요" : hasCompletedMerge(item) ? "재생 가능" : recordingMergeLabel(item)}</span><span className="recording-library-length">{duration(item.durationSeconds)}</span></span>
    <strong>{title(item)}</strong>
    <span className="recording-library-end-reason">{emptyRecordingAttempt(item) ? "시작 실패 · 영상 저장 없음" : recordingStatus(item)}</span>
    {item.broadcastKey && recordings.filter(other => other.channelId === item.channelId && other.broadcastKey === item.broadcastKey).length > 1 ? <small>같은 방송의 녹화 구간 · 원본별 보존</small> : null}
    <span className="recording-library-card-bottom"><time dateTime={dateTime(item.startedAt)}>{date(item.startedAt)}</time><span>{item.mediaRemovedAt != null ? "영상 없음" : bytes(hasCompletedMerge(item) ? item.merge!.bytes! : item.bytesWritten)} <ChatWarning recording={item} /></span></span>
    </button>
    {selecting && item.status !== "recording" ? <input className="recording-library-check" type="checkbox" aria-label={`${title(item)} 삭제 선택`} checked={checked.includes(item.id)} disabled={!canDelete(item) || disabled || deleting} onChange={() => toggle(item.id)} /> : null}
  </div>;
  return <section className="recording-library" aria-label="녹화 보관함">
    <header className="recording-library-heading"><div><h2>녹화 보관함</h2><p>저장 영상 {saved.length.toLocaleString("ko-KR")}개{active.length ? ` · 녹화 중 ${active.length}개` : ""}</p></div><div className="recording-library-heading-actions">{stopControl}<button ref={selectionToggle} type="button" disabled={disabled || deleting || !saved.length && !attempts.length && !selecting} aria-pressed={selecting} onClick={() => { setSelecting(value => !value); setChecked([]); }}>{selecting ? "선택 취소" : "선택"}</button></div></header>
    {active.length ? <section className="recording-library-active" aria-label="진행 중인 녹화"><h3>지금 녹화 중</h3><div>{active.map(card)}</div></section> : null}
    <div className="recording-library-toolbar"><div className="recording-library-filters" aria-label="녹화 필터">{([ ["all", "전체"], ["ready", "재생 가능"], ["attention", "확인 필요"] ] as const).map(([value, label]) => <button type="button" key={value} aria-pressed={filter === value} onClick={() => setFilter(value)}>{label}</button>)}</div><input type="search" aria-label="녹화 검색" placeholder="제목·채널 검색" value={query} disabled={privacy} onChange={event => setQuery(event.target.value)} /></div>
    {selecting ? <div className="recording-library-selection"><button type="button" disabled={disabled || deleting || !eligible.length} onClick={() => setChecked(allChecked ? [] : eligible)} title="현재 검색·필터에 맞는 삭제 가능한 녹화를 모두 선택합니다. 아직 펼치지 않은 항목도 포함됩니다.">{allChecked ? "전체 해제" : "전체 선택"}</button><span>{checked.length}개 선택</span><button ref={deleteTrigger} type="button" className="recording-library-delete" disabled={disabled || deleting || !checked.length} onClick={() => { setDeleteError(""); setConfirmIds([...checked]); }}>선택 삭제</button></div> : null}
    {deleteNotice ? <p role="status" className="recording-library-delete-notice">{deleteNotice}</p> : null}
    {deleteFailures.length ? <div role="alert" className="recording-library-delete-errors"><strong>{deleteFailures.length}개는 삭제하지 못했습니다.</strong><ul>{deleteFailures.map(failure => <li key={failure.id}><b>{recordings.find(item => item.id === failure.id) ? title(recordings.find(item => item.id === failure.id)!) : "녹화 영상"}</b> — {failure.error.message}</li>)}</ul></div> : null}
    <div className="recording-library-layout"><div className="recording-library-browse">
      <div className="recording-library-grid">{filtered.slice(0, limit).map(card)}</div>
      {attempts.length ? <details className="recording-library-attempts"><summary>시작 실패 기록 {attempts.length}개 · 방송 수에 포함하지 않음</summary><p>영상 저장 전 중단된 시도입니다. 재시도에 성공한 녹화와 구분하며, 남아 있는 채팅·진단 기록은 보존합니다.</p><div className="recording-library-grid">{attempts.map(card)}</div></details> : null}
      {diagnostics.length ? <details className="recording-library-attempts" onToggle={event => setShowDiagnostics(event.currentTarget.open)}><summary>영상 정리 기록 {diagnostics.length}개 · 로그 보존</summary><p>영상만 제거한 기록입니다. 당시 오류·채팅·시간표·프로필은 남아 있습니다.</p>{showDiagnostics ? <><div className="recording-library-grid">{diagnostics.slice(0, diagnosticLimit).map(card)}</div>{diagnostics.length > diagnosticLimit ? <button type="button" onClick={() => setDiagnosticLimit(value => value + 24)}>기록 더 보기</button> : null}</> : null}</details> : null}
      {!filtered.length ? <div className="recording-library-empty"><strong>{saved.length ? "조건에 맞는 녹화가 없습니다" : "아직 저장한 영상이 없습니다"}</strong><p>{saved.length ? "검색어나 필터를 바꿔 보세요." : "녹화를 마치면 이곳에서 영상과 채팅을 함께 볼 수 있습니다."}</p></div> : null}
      {filtered.length > limit ? <button type="button" className="recording-library-more" onClick={() => setLimit(value => value + 24)}>더 보기 · {filtered.length - limit}개</button> : null}
    </div>
    {selected ? <section className="recording-library-detail official-browser-files" aria-label="공식 녹화 파일">
      <div className="recording-library-detail-heading"><span>선택한 녹화</span><h3>{title(selected)}</h3><p>{date(selected.startedAt)}</p></div>
      <RecordedChannel id={selected.id} privacy={privacy} />
      <p className="official-browser-muted" title={recordingChatNote(selected).text}>{recordingStatus(selected)} · {duration(selected.durationSeconds)} · {selected.mediaRemovedAt != null ? "영상 없음" : bytes(hasCompletedMerge(selected) ? selected.merge!.bytes! : selected.bytesWritten)} <ChatWarning recording={selected} /></p>
      {selected.lastError && selected.ending?.reason !== "checking" ? <p className="official-browser-error" role="alert">{selected.lastError}</p> : null}
      {selected.partial && selected.mediaRemovedAt == null ? <p className="official-browser-note">마무리되지 않은 파일은 폴더에 보존되어 있습니다.</p> : null}
      {selected.deletionPending ? <p className="official-browser-note">삭제가 완료되지 않았습니다. 선택 삭제로 남은 파일 정리를 다시 시도해 주세요.</p> : <RecordingPlayback key={selected.id} recording={selected} disabled={disabled || deleting} privacyMode={privacy} retrying={retrying} opening={opening} onOpenMerged={onOpenMerged} onReplay={onReplay} onRetryMerge={onRetryMerge} onOpenSegment={onOpenSegment} />}
      <button type="button" className="recording-library-folder" disabled={disabled} onClick={() => onFolder(selected.id)}>녹화 폴더 열기</button>
    </section> : null}
    </div>
    <dialog ref={dialog} className="recording-library-delete-dialog" aria-labelledby={dialogTitle} aria-describedby={dialogDescription} aria-busy={deleting} onCancel={event => { event.preventDefault(); if (!deleteInFlight.current) setConfirmIds(null); }}>
      <h3 id={dialogTitle}>녹화 {confirmIds?.length ?? 0}개를 삭제할까요?</h3>
      <p id={dialogDescription}>선택한 녹화 폴더의 영상·원본 조각·채팅·부가정보와 채팅 검색 데이터를 삭제합니다. 휴지통으로 이동하지 않으며 복구할 수 없습니다.</p>
      <ul>{confirmIds?.map(id => { const item = recordings.find(item => item.id === id); return <li key={id}>{item ? title(item) : "녹화 영상"}</li>; })}</ul>
      {deleteError ? <p role="alert" className="official-browser-error">{deleteError}</p> : null}
      <footer><button ref={cancel} type="button" disabled={deleting} onClick={() => setConfirmIds(null)}>취소</button><button type="button" className="recording-library-delete" disabled={disabled || deleting || !confirmIds?.length} onClick={() => void confirmDelete()}>{deleting ? "삭제 중…" : "삭제"}</button></footer>
    </dialog>
  </section>;
}
