import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type {
  InternalDuplicateReview,
  InternalRemovalPlan,
  InternalRemovalPlanRequest,
} from "../api/contracts";
import { artifactPageThumbnailKey, type ThumbnailClient } from "../thumbnail";
import { FluentIcon } from "./FluentIcon";
import { GalleryThumbnail } from "./GalleryThumbnail";
import {
  buildInternalRemovalSelections,
  buildInternalReviewBlocks,
  selectedInternalEditionTracks,
  selectionsMatchPlan,
} from "./internalDuplicateReviewModel";

type InternalDuplicateDialogProps = {
  open: boolean;
  review?: InternalDuplicateReview;
  plan?: InternalRemovalPlan;
  loading?: boolean;
  busy?: boolean;
  error?: string | null;
  thumbnailClient?: ThumbnailClient;
  onClose: () => void;
  onRetry: () => void;
  onRescan: () => void;
  onPlan: (request: InternalRemovalPlanRequest) => void;
  onApply: (plan: InternalRemovalPlan) => void;
  onUndo: (recordIds: string[]) => void;
};

const percent = (value: number): string => `${Math.round(Math.min(1, Math.max(0, value)) * 100)}%`;
const bytes = (value: number): string => new Intl.NumberFormat("ko-KR", {
  style: "unit",
  unit: value >= 1024 * 1024 ? "megabyte" : "kilobyte",
  maximumFractionDigits: 1,
}).format(value / (value >= 1024 * 1024 ? 1024 * 1024 : 1024));

const INTERNAL_REVIEW_DENSITY_STYLE = {
  "--internal-scene-column-width": "208px",
  "--internal-legacy-image-width": "200px",
} as CSSProperties;

export function InternalDuplicateDialog({
  open,
  review,
  plan,
  loading = false,
  busy = false,
  error = null,
  thumbnailClient,
  onClose,
  onRetry,
  onRescan,
  onPlan,
  onApply,
  onUndo,
}: InternalDuplicateDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const [keepPages, setKeepPages] = useState<Record<string, number>>({});
  const [selectedTrackIdsByBlock, setSelectedTrackIdsByBlock] = useState<Record<string, string[]>>({});

  useEffect(() => {
    if (!open) return;
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!dialog.current?.open) {
      if (typeof dialog.current?.showModal === "function") dialog.current.showModal();
      else dialog.current?.setAttribute("open", "");
    }
    window.requestAnimationFrame(() => closeButton.current?.focus());
    return () => {
      if (dialog.current?.open) {
        if (typeof dialog.current.close === "function") dialog.current.close();
        else dialog.current.removeAttribute("open");
      }
      const target = opener.current;
      opener.current = null;
      if (target?.isConnected) target.focus();
    };
  }, [open]);

  const activeRecords = useMemo(
    () => review?.quarantineRecords.filter((record) => record.state !== "restored") ?? [],
    [review?.quarantineRecords],
  );
  const blocks = useMemo(() => buildInternalReviewBlocks(review?.groups ?? []), [review?.groups]);
  useEffect(() => {
    if (!review) return;
    setKeepPages(Object.fromEntries(review.groups.map((group) => [group.groupId, group.recommendedKeepSourcePage])));
    setSelectedTrackIdsByBlock(Object.fromEntries(
      buildInternalReviewBlocks(review.groups)
        .filter((block) => block.edition)
        .map((block) => [block.blockId, [block.tracks[0]!.id]]),
    ));
  }, [review]);

  const selections = useMemo(
    () => buildInternalRemovalSelections(blocks, selectedTrackIdsByBlock, keepPages),
    [blocks, keepPages, selectedTrackIdsByBlock],
  );
  const activePlan = selectionsMatchPlan(selections, plan) ? plan ?? undefined : undefined;

  const toggleEditionTrack = (blockId: string, fallbackTrackId: string, trackId: string) => {
    setSelectedTrackIdsByBlock((current) => {
      const selected = current[blockId] ?? [fallbackTrackId];
      if (!selected.includes(trackId)) {
        return { ...current, [blockId]: [...selected, trackId] };
      }
      // At least one edition must remain selected so a row can never become an
      // accidental "remove everything" request.
      if (selected.length === 1) return current;
      return { ...current, [blockId]: selected.filter((id) => id !== trackId) };
    });
  };

  const preview = () => {
    if (!review) return;
    onPlan({
      entryId: review.entryId,
      selections,
    });
  };

  if (!open && !review) return null;
  return (
    <dialog
      ref={dialog}
      className="review-dialog internal-review-dialog"
      data-image-density="fixed-200"
      style={INTERNAL_REVIEW_DENSITY_STYLE}
      aria-labelledby="internal-review-title"
      aria-busy={loading || busy}
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onClose={onClose}
    >
      <div className="review-form">
        <header className="dialog-header">
          <div>
            <span className="eyebrow">INTERNAL PAGE REVIEW</span>
            <h2 id="internal-review-title">앨범 내부 중복 페이지 검토</h2>
          </div>
          <button ref={closeButton} type="button" className="icon-button small" aria-label="닫기" onClick={onClose}>
            <FluentIcon glyph="\uE711" />
          </button>
        </header>

        {loading && !review ? (
          <div className="review-loading" role="status"><span className="spinner" /> 저장된 내부 중복 근거를 불러오는 중</div>
        ) : error && !review ? (
          <div className="review-loading" role="alert">
            <strong>내부 중복 검토를 불러오지 못했습니다.</strong>
            <span>{error}</span>
            <button type="button" className="text-button" onClick={onRetry}>다시 불러오기</button>
          </div>
        ) : review ? (
          <div className="review-scroll internal-review-scroll" data-scroll-axis="vertical">
            {error ? <div className="inline-error" role="alert"><span>{error}</span><button type="button" className="text-button" onClick={onRetry}>최신 내용 다시 불러오기</button></div> : null}
            <div className="review-summary">
              <span className="review-signal">{review.groups.length}개 동기화 행</span>
              <strong>{review.title}</strong>
              <span>원본 페이지 번호는 바뀌지 않습니다. 선택한 파일은 앨범 폴더 안의 격리 영역으로 이동하며 자동 영구 삭제되지 않습니다.</span>
            </div>

            {blocks.length ? blocks.map((block, blockIndex) => (
              <section className="internal-scene-block" key={block.blockId} aria-labelledby={`internal-block-${blockIndex}`}>
                <header>
                  <div><h3 id={`internal-block-${blockIndex}`}>장면 묶음 {blockIndex + 1}</h3><span>{block.rows[0]?.relation === "exact" ? "SHA-256 정확 일치" : "연속 장면 시각 일치"}</span></div>
                  <span>{block.rows.length}행</span>
                </header>
                {block.edition ? (
                  <>
                    <fieldset className="internal-edition-tracks">
                      <legend>남길 판본 세트 선택 · 복수 선택 가능</legend>
                      <div
                        className="internal-scene-matrix"
                        role="region"
                        aria-label={`장면 묶음 ${blockIndex + 1} 판본 행렬`}
                        data-scroll-axis="horizontal"
                        style={{ "--internal-scene-count": block.rows.length } as CSSProperties}
                      >
                        <div className="internal-scene-matrix-row internal-scene-matrix-header">
                          <span>판본 세트</span>
                          {block.rows.map((group) => <strong key={group.groupId}>장면 {group.sequenceIndex + 1}</strong>)}
                        </div>
                        {block.tracks.map((track) => {
                          const selectedTracks = selectedInternalEditionTracks(block, selectedTrackIdsByBlock);
                          const selectedTrackIds = new Set(selectedTracks.map((item) => item.id));
                          const selected = selectedTrackIds.has(track.id);
                          return (
                            <div
                              className={`internal-scene-matrix-row internal-edition-track-row${selected ? " is-kept" : " is-quarantine"}`}
                              key={track.id}
                            >
                              <label className="internal-edition-track-control">
                                <input
                                  type="checkbox"
                                  name={`track-${block.blockId}`}
                                  checked={selected}
                                  aria-label={`${track.label} 유지`}
                                  onChange={() => toggleEditionTrack(block.blockId, block.tracks[0]!.id, track.id)}
                                />
                                <span>
                                  <strong>{track.label}</strong>
                                  <small>{track.pages[0]}–{track.pages.at(-1)}p · {track.coveredRows}/{block.rows.length}장</small>
                                  {track.missingRows ? <small>{track.missingRows}개 장면 누락</small> : null}
                                </span>
                              </label>
                              {block.rows.map((group) => {
                                const page = group.pages.find((candidate) => candidate.editionTrackId === track.id);
                                const rowHasSelectedPage = group.pages.some((candidate) => (
                                  candidate.editionTrackId !== undefined && selectedTrackIds.has(candidate.editionTrackId)
                                ));
                                if (!page) return <span
                                  className={`internal-scene-cell is-missing${selected ? rowHasSelectedPage ? " is-kept" : " is-preserved" : ""}`}
                                  key={group.groupId}
                                  aria-label={selected
                                    ? rowHasSelectedPage ? `${track.label} 장면 누락` : "선택 세트 전체 누락 · 이 행 보존"
                                    : `${track.label} 장면 누락`}
                                  title={selected && !rowHasSelectedPage ? "선택 세트 전체 누락 · 이 행 보존" : undefined}
                                >{selected ? rowHasSelectedPage ? "선택 세트 누락" : "누락 · 행 보존" : "—"}</span>;
                                const preserved = !selected && !rowHasSelectedPage;
                                return <div className={`internal-scene-cell${selected ? " is-kept" : preserved ? " is-preserved" : " is-quarantine"}`} key={group.groupId}>
                                  <GalleryThumbnail
                                    className="internal-page-image"
                                    thumbnailKey={artifactPageThumbnailKey(review.entryId, page.sourcePage, page.sourcePage - 1)}
                                    consumer="review"
                                    priority={blockIndex === 0 && group.sequenceIndex < 2 ? "visible" : "prefetch"}
                                    client={thumbnailClient}
                                    alt={`${track.label} 원본 ${page.sourcePage}페이지`}
                                  ><span>{page.sourcePage}p</span></GalleryThumbnail>
                                  <small>{selected ? "유지" : preserved ? "행 보존" : "격리 예정"}</small>
                                </div>;
                              })}
                            </div>
                          );
                        })}
                      </div>
                    </fieldset>
                    {(() => {
                      const selectedTracks = selectedInternalEditionTracks(block, selectedTrackIdsByBlock);
                      const selectedTrackIds = new Set(selectedTracks.map((track) => track.id));
                      const blockSelections = buildInternalRemovalSelections([block], selectedTrackIdsByBlock, keepPages);
                      const removals = blockSelections.reduce((count, selection) => count + selection.removeSourcePages.length, 0);
                      const keptPages = selectedTracks.reduce((count, track) => count + track.coveredRows, 0);
                      const preservedRows = block.rows.filter((group) => !group.pages.some((page) => (
                        page.editionTrackId !== undefined && selectedTrackIds.has(page.editionTrackId)
                      ))).length;
                      return selectedTracks.length ? <p className="internal-selection-summary">
                        선택 판본: <strong>{selectedTracks.map((track) => track.label).join(", ")}</strong> · 유지 페이지: {keptPages}개 · 격리 예정: {removals}개
                        {preservedRows ? ` · 선택한 세트들에 모두 없는 장면 ${preservedRows}행은 이번 작업에서 건드리지 않습니다.` : null}
                      </p> : null;
                    })()}
                  </>
                ) : <div className="internal-scene-rows">
                  {block.rows.map((group) => (
                    <fieldset key={group.groupId} className="internal-scene-row">
                      <legend>행 {group.sequenceIndex + 1} · 신뢰도 {percent(group.confidence)}</legend>
                      <div className="internal-page-options">
                        {group.pages.map((page, pageIndex) => {
                          const selected = (keepPages[group.groupId] ?? group.recommendedKeepSourcePage) === page.sourcePage;
                          return (
                            <label key={page.sourcePage} className={`internal-page-option${selected ? " is-kept" : ""}`}>
                              <input
                                type="radio"
                                name={`keep-${group.groupId}`}
                                checked={selected}
                                onChange={() => setKeepPages((current) => ({ ...current, [group.groupId]: page.sourcePage }))}
                              />
                              <GalleryThumbnail
                                className="internal-page-image"
                                thumbnailKey={artifactPageThumbnailKey(review.entryId, page.sourcePage, page.sourcePage - 1)}
                                consumer="review"
                                priority={blockIndex === 0 && pageIndex < 4 ? "visible" : "prefetch"}
                                client={thumbnailClient}
                                alt={`${review.title} 원본 ${page.sourcePage}페이지`}
                              ><span>{page.sourcePage}p</span></GalleryThumbnail>
                              <strong>{selected ? "유지" : "격리 예정"} · 원본 {page.sourcePage}p</strong>
                              <small>{page.exactSha256 ? "파일 일치" : `시각 ${percent(page.visualSimilarity)} · detail ${page.detailHashDistance}`}</small>
                            </label>
                          );
                        })}
                      </div>
                    </fieldset>
                  ))}
                </div>}
              </section>
            )) : <div className="review-empty">현재 검토할 내부 중복 페이지가 없습니다.</div>}

            {activePlan ? (
              <section className="internal-plan-summary" aria-live="polite">
                <div><strong>격리 계획 확인</strong><span>{activePlan.filesToQuarantine}개 파일 · {bytes(activePlan.bytesToQuarantine)}</span></div>
                <p>이 작업은 파일을 영구 삭제하지 않습니다. 적용 후 아래 격리 이력에서 되돌릴 수 있습니다.</p>
                <button type="button" className="text-button danger-button" disabled={busy} onClick={() => onApply(activePlan)}>계획대로 격리 적용</button>
              </section>
            ) : null}

            {activeRecords.length ? (
              <section className="internal-quarantine-history">
                <div><h3>격리 이력</h3><span>{activeRecords.length}개 파일</span></div>
                <ul>{activeRecords.map((record) => <li key={record.recordId}><strong>원본 {record.sourcePage}p</strong><span>{record.state === "quarantined" ? "격리됨" : "처리 중"}</span></li>)}</ul>
                <button type="button" className="text-button" disabled={busy || activeRecords.some((record) => record.state !== "quarantined")} onClick={() => onUndo(activeRecords.map((record) => record.recordId))}>선택 앨범의 격리 페이지 모두 되돌리기</button>
              </section>
            ) : null}
          </div>
        ) : null}

        <div className="review-actions">
          <button type="button" className="text-button" disabled={busy} onClick={onRescan}><FluentIcon glyph="\uE9D9" /> 이 앨범 다시 검사</button>
          <span />
          <button type="button" className="text-button" disabled={!selections.length || busy} onClick={preview}>격리 계획 미리보기</button>
          <button type="button" className="text-button" onClick={onClose}>닫기</button>
        </div>
      </div>
    </dialog>
  );
}
