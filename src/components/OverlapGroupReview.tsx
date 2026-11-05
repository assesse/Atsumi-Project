import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { DownloadOverlapDecisionRequest, DownloadOverlapMergeRequest, DownloadOverlapPagePair } from "../api/contracts";
import { buildGroupAlignment, groupEditionStatus, relationDisplayPairs, visibleGroupRelations, type GroupEdition, type OverlapReviewGroup } from "../downloadOverlap/groupReview";
import { emptyMergeSelection, mergeOutputPages, selectMergePages, type MergeSelection } from "../downloadOverlap/mergeSelection";
import { keepEditions, planEditionDisposition, type EditionDisposition, type EditionDispositions } from "../downloadOverlap/editionDisposition";
import { orderEditionBatch, type EditionBatchHandler, type EditionBatchOperation, type EditionBatchProgress } from '../downloadOverlap/editionBatch';
import { FluentIcon } from "./FluentIcon";
import "./OverlapGroupReview.css";
import { routeOverlapWheel } from "./overlapReviewViewport";
import { useGroupPageEvidence, type PageEvidenceLoader } from "../downloadOverlap/useGroupPageEvidence";

type Side = DownloadOverlapMergeRequest["sourceSide"];
export type GroupPageSelection = {
  evidence: string; reviewId: string; reviewRevision: number; candidateId: string;
  pages: MergeSelection; anchor: { side: Side; page: number };
};
export type GroupPageRender = {
  edition: GroupEdition; side: Side; page?: number; pair?: DownloadOverlapPagePair; index: number;
  label: string; visible: boolean; displayMatch: { matched: boolean; reordered?: boolean; counterparts?: string[]; pending?: boolean; unverified?: boolean }; selection?: GroupPageSelection; canSelect: boolean;
  onToggle: (side: Side, page: number, pair?: DownloadOverlapPagePair, range?: boolean) => void;
};
export type GroupReviewDraft = {
  shown: string[]; activeKey: string | null;
  selections: Record<string, GroupPageSelection>;
  dispositions: Record<string, { evidence: string; modes: EditionDispositions }>;
  excludeOverrides: Record<string, boolean>;
  visibility: { matched: boolean; unique: boolean; gap: boolean };
};
type Props = {
  initialDraft?: GroupReviewDraft;
  onDraftChange?: (draft: GroupReviewDraft) => void;
  loadPageEvidence?: PageEvidenceLoader;
  suspended?: boolean;
  group: OverlapReviewGroup; disabled: boolean; loading: boolean;
  progress?: { completed: number; total: number; failure?: string; completedKeys?: string[]; failedKey?: string };
  renderPage: (props: GroupPageRender) => ReactNode;
  renderCover: (edition: GroupEdition, keeper?: boolean) => ReactNode;
  onClearPreview: () => void;
  onApply?: (keys: string[]) => void;
  onApplyEditions?: EditionBatchHandler;
  onDecision: (request: DownloadOverlapDecisionRequest) => void;
  onMerge?: (request: DownloadOverlapMergeRequest) => void;
};
const count = (selection?: GroupPageSelection) => selection
  ? selection.pages.incoming.length + selection.pages.existing.length : 0;
const isDone = (item: GroupEdition) => ["excluded", "kept", "different"].includes(item.status);

export function OverlapGroupReview({ initialDraft, onDraftChange, group, disabled, loading, progress, renderPage, renderCover,
  onClearPreview, onApply, onApplyEditions, onDecision, onMerge, loadPageEvidence, suspended = false }: Props) {
  const [shown, setShown] = useState<Set<string>>(() => new Set(initialDraft?.shown ?? (group.editions.length === 1 ? group.editions : group.editions.filter((item) => item.status === "uncertain").slice(0, 2)).map((item) => item.key)));
  const [activeKey, setActiveKey] = useState<string | null>(initialDraft?.activeKey ?? null);
  const [selections, setSelections] = useState<Record<string, GroupPageSelection>>(initialDraft?.selections ?? {});
  const [dispositions, setDispositions] = useState<Record<string, { evidence: string; modes: EditionDispositions }>>(initialDraft?.dispositions ?? {});
  const [excludeOverrides, setExcludeOverrides] = useState<Record<string, boolean>>(initialDraft?.excludeOverrides ?? {});
  const [submitted, setSubmitted] = useState<string[]>([]);
  const [visibility, setVisibility] = useState(initialDraft?.visibility ?? { matched: true, unique: true, gap: false });
  const draftSink = useRef(onDraftChange); draftSink.current = onDraftChange;
  useEffect(() => { draftSink.current?.({ shown:[...shown], activeKey, selections, dispositions, excludeOverrides, visibility }); },
    [shown, activeKey, selections, dispositions, excludeOverrides, visibility]);
  const [editionProgress, setEditionProgress] = useState<EditionBatchProgress | null>(null);
  const [batchBusy, setBatchBusy] = useState(false);
  disabled ||= batchBusy;
  const [hint, setHint] = useState<string | null>(null);
  const [scrollLeft, setScrollLeft] = useState(0);
  const [viewportWidth, setViewportWidth] = useState(1024);
  const scroller = useRef<HTMLDivElement>(null);
  const clearPreview = useRef(onClearPreview);
  clearPreview.current = onClearPreview;
  const compared = useMemo(() => group.editions.filter((item) => shown.has(item.key) && item.keeperCompatible), [group.editions, shown]);
  const relations = useMemo(() => visibleGroupRelations(group, compared), [group, compared]);
  const evidence = useGroupPageEvidence(relations, loadPageEvidence);
  const allPairsKnown = relations.length === compared.length * (compared.length + 1) / 2;
  const pairsFor = (item: GroupEdition) => relationDisplayPairs({key:item.key,review:item.review,candidate:item.candidate,evidence:item.evidence}, evidence.supplements[item.key]);
  const active = compared.find((item) => item.key === activeKey) ?? compared[0];
  const validSelection = (item: GroupEdition) => selections[item.key]?.evidence === item.evidence ? selections[item.key] : undefined;
  const modesFor = (item: GroupEdition) => dispositions[item.key]?.evidence === item.evidence ? dispositions[item.key]!.modes : keepEditions();
  const setMode = (item: GroupEdition, side: Side, mode: EditionDisposition) => {
    setActiveKey(item.key); setHint(null);
    setDispositions(current => ({...current,[item.key]: {evidence:item.evidence,modes:{...(current[item.key]?.evidence===item.evidence?current[item.key]!.modes:keepEditions()),[side]:mode}}}));
  };
  const activeSelection = active ? validSelection(active) : undefined;
  const appliedKeys = new Set(editionProgress?.completedKeys ?? []);
  // Closing a comparison does not discard a deliberately changed draft.
  const applyScope = group.editions.filter(item => item.canDecide && !appliedKeys.has(item.key)
    && (shown.has(item.key) || dispositions[item.key] !== undefined || selections[item.key] !== undefined));
  const processed = new Set(progress?.completedKeys ?? (progress?.total === submitted.length ? submitted.slice(0, progress.completed) : []));
  const selectedExclusions = group.editions.filter((item) => item.canBatchExclude && !processed.has(item.key) && excludeOverrides[item.key] !== false);
  const pending = group.editions.filter((item) => !isDone(item) && !processed.has(item.key));
  const done = group.editions.filter((item) => isDone(item) || processed.has(item.key));
  const selectionCount = group.editions.reduce((sum, item) => sum + count(validSelection(item)), 0);
  const invalidSelections = group.editions.filter((item) => selections[item.key] && !validSelection(item));
  const rows = useMemo(() => suspended ? [] : buildGroupAlignment(group.keeper, compared, relations, evidence.supplements), [suspended, group.keeper, compared, relations, evidence.supplements]);
  const kind = (page: number | undefined, matched: boolean | undefined) => page ? matched ? "matched" : "unique" : "gap";
  const keeperPairs = useMemo(() => new Map(active?.candidate.pagePairs.map((pair) =>
    [active.keeperIsIncoming ? pair.incomingSourcePage : pair.existingSourcePage, pair]) ?? []), [active]);
  const activePair = (page?: number) => page ? keeperPairs.get(page) : undefined;
  const visibleRows = useMemo(() => rows.filter((row) => visibility[kind(row.keeperPage, row.keeperMatched)]
    || compared.some((item) => visibility[kind(row.pages[item.key]?.page, row.pages[item.key]?.matched)])), [rows, visibility, compared]);
  // Alignment slots run left to right; editions remain independent rows.
  // Only viewport columns mount thumbnails. Selection outlives virtualization.
  const pageWidth = 128, labelWidth = Math.min(276, Math.max(220, viewportWidth * .28)), rowHeight = 184;
  const windowSize = Math.ceil(Math.max(pageWidth, viewportWidth - labelWidth) / pageWidth) + 4;
  const start = Math.min(Math.max(0, visibleRows.length - windowSize), Math.max(0, Math.floor(scrollLeft / pageWidth) - 2));
  const end = Math.min(visibleRows.length, start + windowSize);
  const gridStyle = { minWidth: Math.max(viewportWidth, labelWidth + visibleRows.length * pageWidth),
    "--comparison-page-width": pageWidth + "px", "--comparison-label-width": labelWidth + "px",
    "--comparison-row-height": rowHeight + "px" } as CSSProperties;
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const measure = () => { if (element.clientWidth > 0) setViewportWidth(element.clientWidth); };
    measure();
    if (typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(measure); observer.observe(element);
    return () => observer.disconnect();
  }, [compared.length > 0, suspended]);
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => { if (routeOverlapWheel(event, element)) clearPreview.current(); };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [compared.length > 0, suspended]);
  const resetScroll = () => { if (scroller.current) scroller.current.scrollLeft = 0; setScrollLeft(0); onClearPreview(); };
  const toggleShown = (key: string) => {
    setShown((current) => { const next = new Set(current); if (next.has(key)) next.delete(key); else next.add(key); return next; });
    setActiveKey(key); resetScroll();
  };
  const togglePage = (item: GroupEdition, side: Side, page: number, _pair?: DownloadOverlapPagePair, range = false) => {
    if (disabled || !item.canDecide || !onMerge) return;
    try { mergeOutputPages(item.candidate, item.review.incoming.pageCount, { sourceSide: side, selectedPages: emptyMergeSelection() }); }
    catch { setHint("#" + item.excluded.galleryId + " · 페이지 대응 다시 확인"); return; }
    setMode(item, side, "partial");
    setSelections((current) => {
      const saved = current[item.key]?.evidence === item.evidence ? current[item.key] : undefined;
      const pages = selectMergePages({...item.candidate, pagePairs:pairsFor(item)}, saved?.pages ?? emptyMergeSelection(), side, page, saved?.anchor, range);
      if (pages.existing.length + pages.incoming.length > 4000) { setHint("최대 4,000장 선택"); return current; }
      return { ...current, [item.key]: { evidence: item.evidence, reviewId: item.review.reviewId,
        reviewRevision: item.review.revision, candidateId: item.candidate.candidateId,
        pages, anchor: range && saved ? saved.anchor : { side, page } } };
    });
    setHint(null);
  };
  const applySingleDisposition = () => {
    if (!active || disabled || !active.canDecide) return;
    if (selections[active.key] && !activeSelection) { setHint('페이지가 변경되어 다시 선택해야 합니다.'); return; }
    const keeperSide: Side = active.keeperIsIncoming ? 'incoming' : 'existing';
    const galleryFor = (side:Side) => side === keeperSide ? group.keeper : active.excluded;
    try {
      const plan = planEditionDisposition(modesFor(active), activeSelection?.pages ?? emptyMergeSelection(), keeperSide);
      if (plan.kind === 'decision') {
        if(plan.action !== 'keep_both_continue') {
          const removed = galleryFor(plan.action === 'remove_incoming'?'incoming':'existing');
          if(!window.confirm('#'+removed.galleryId+' 판본을 제외할까요? 원본 파일은 보존됩니다.'))return;
        }
        onDecision({reviewId:active.review.reviewId,expectedRevision:active.review.revision,candidateId:active.candidate.candidateId,action:plan.action});
        return;
      }
      if(!onMerge)return;
      const originalPairs = new Set(active.candidate.pagePairs.map(p=>p.incomingSourcePage+':'+p.existingSourcePage));
      const selectedRecovered = pairsFor(active).some(p=>!originalPairs.has(p.incomingSourcePage+':'+p.existingSourcePage)
        && (plan.selectedPages.incoming.includes(p.incomingSourcePage)||plan.selectedPages.existing.includes(p.existingSourcePage)));
      if(selectedRecovered){setHint('순서가 다른 선택 페이지는 병합할 수 없습니다. 선택은 유지됩니다.');return;}
      const {kind:_,...composition}=plan;
      const request:DownloadOverlapMergeRequest={reviewId:active.review.reviewId,expectedRevision:active.review.revision,candidateId:active.candidate.candidateId,...composition,excludeSource:true};
      const output=mergeOutputPages(active.candidate,active.review.incoming.pageCount,request);
      if(!output.length)throw Error('남길 페이지를 하나 이상 선택해 주세요.');
      const target=galleryFor(plan.sourceSide==='incoming'?'existing':'incoming'),source=galleryFor(plan.sourceSide);
      if(window.confirm('#'+target.galleryId+'에 '+output.length+'장을 남기고 #'+source.galleryId+' 판본을 제외할까요? 원본 파일은 보존됩니다.'))onMerge(request);
    }catch(error){setHint(error instanceof Error?error.message:'페이지 대응을 다시 확인해 주세요.');}
  };
  const applyDisposition = async () => {
    if (!onApplyEditions) { applySingleDisposition(); return; }
    if (disabled || !applyScope.length) return;
    try {
      const operations: EditionBatchOperation[] = applyScope.map(item => {
        const selection = validSelection(item);
        if ((selections[item.key] && !selection) || (dispositions[item.key] && dispositions[item.key]!.evidence !== item.evidence))
          throw Error('#'+item.excluded.galleryId+' · 페이지가 변경되어 다시 선택해야 합니다.');
        const keeperSide: Side = item.keeperIsIncoming ? 'incoming' : 'existing';
        const plan = planEditionDisposition(modesFor(item),selection?.pages ?? emptyMergeSelection(),keeperSide);
        const common = {key:item.key,evidence:item.evidence,galleryId:Number(item.excluded.galleryId),incomingEntryId:item.review.incoming.entryId,existingEntryId:item.candidate.existing.entryId};
        const identity = {reviewId:item.review.reviewId,expectedRevision:item.review.revision,candidateId:item.candidate.candidateId};
        if(plan.kind === 'decision') return {...common,kind:'decision',request:{...identity,action:plan.action}};
        const originalPairs = new Set(item.candidate.pagePairs.map(p=>p.incomingSourcePage+':'+p.existingSourcePage));
        if(pairsFor(item).some(p=>!originalPairs.has(p.incomingSourcePage+':'+p.existingSourcePage)
          && (plan.selectedPages.incoming.includes(p.incomingSourcePage)||plan.selectedPages.existing.includes(p.existingSourcePage))))
          throw Error('순서가 다른 선택 페이지는 병합할 수 없습니다. 선택은 유지됩니다.');
        const {kind:_,...composition}=plan;
        const request:DownloadOverlapMergeRequest={...identity,...composition,excludeSource:true};
        if(!mergeOutputPages(item.candidate,item.review.incoming.pageCount,request).length)throw Error('남길 페이지를 하나 이상 선택해 주세요.');
        return {...common,kind:'merge',request};
      });
      const ordered=orderEditionBatch(operations);
      const descriptions=ordered.map(item=>'#'+item.galleryId+' '+(item.kind==='merge'?'선택 병합':item.request.action==='keep_both_continue'?'남기기':'제거'));
      if(!window.confirm(descriptions.join(' · ')+'\n'+ordered.length+'개 판본에 적용할까요? 원본 파일은 보존됩니다.'))return;
      setHint(null);setBatchBusy(true);setEditionProgress({completedKeys:[],total:ordered.length});
      setEditionProgress(await onApplyEditions(ordered,setEditionProgress));
    } catch(error) {setHint(error instanceof Error?error.message:'처리 결과를 확인하지 못했습니다.');}
    finally {setBatchBusy(false);}
  };
  const editionRow = (item: GroupEdition, completed = false) => <article key={item.key} className={"overlap-group-edition is-" + item.status} data-edition-id={item.excluded.galleryId}>
    {!completed && item.canBatchExclude && onApply ? <input type="checkbox" aria-label={"#" + item.excluded.galleryId + " 제외 선택"} title="완전 포함 판본 제외 선택"
      checked={!completed && item.canBatchExclude && excludeOverrides[item.key] !== false}
      disabled={disabled}
      onChange={(event) => setExcludeOverrides((current) => ({ ...current, [item.key]: event.target.checked }))} /> : null}
    {renderCover(item)}
    <div className="overlap-group-edition-info">
      <strong title={item.excluded.title}>{item.excluded.title}</strong>
      <span>#{item.excluded.galleryId} · {item.excluded.pageCount}p
        <small title={"SHA-256 " + item.candidate.exactPages + " · 시각 일치 " + item.candidate.visualPages}>
          일치 {item.candidate.matchedPages}/{item.excluded.pageCount}
        </small>
      </span>
    </div>
    <div className="overlap-group-edition-actions"><span className={"overlap-group-badge is-" + (progress?.failedKey === item.key ? "failed" : item.status)}>{progress?.failedKey === item.key ? "처리 실패" : completed && item.status === "contained" ? "제외 완료" : groupEditionStatus[item.status]}</span>
    {count(validSelection(item)) > 0 ? <span className="overlap-group-selection-count" title="이 판본과 기준 판본의 선택">선택 {count(validSelection(item))}</span> : null}
    <button type="button" className="text-button" aria-label={"#" + item.excluded.galleryId + " 비교"} aria-pressed={shown.has(item.key)} disabled={disabled || !item.keeperCompatible}
      onClick={() => toggleShown(item.key)}>{shown.has(item.key) ? "비교 닫기" : "비교"}</button></div>
  </article>;

  if(suspended)return null;
  return <section className="overlap-group" aria-label={group.editions.length === 1 ? "두 판본 검토" : "합본 중심 판본 검토"} aria-busy={loading || disabled}>
    <div className="overlap-group-overview" aria-label="관련 판본">
    <article className="overlap-group-keeper">
      {(group.editions.find((item) => item.keeperIsIncoming && item.keeperCompatible) ?? group.editions.find((item) => item.keeperCompatible)) && renderCover((group.editions.find((item) => item.keeperIsIncoming && item.keeperCompatible) ?? group.editions.find((item) => item.keeperCompatible))!, true)}
      <div><span><FluentIcon glyph="\uE73E" /> {group.confirmed ? group.editions.length > 1 ? "보존 합본" : "포함 판본" : "기준 판본"}</span>
        <strong>{group.keeper.title}</strong><small>#{group.keeper.galleryId} · {group.keeper.pageCount}p</small></div>
      <span className="overlap-group-total">판본 {group.editions.length}</span>
    </article>
    {pending.map((item) => editionRow(item))}
    </div>
    {done.length ? <details className="overlap-group-completed"><summary>이미 처리된 판본 {done.length}</summary><div className="overlap-group-completed-grid">{done.map((item) => editionRow(item, true))}</div></details> : null}
    {onApply && group.editions.some((item) => item.canBatchExclude || processed.has(item.key)) ? <div className="overlap-group-batch">
      <span role="status">{progress ? progress.completed + "/" + progress.total + " 처리" : "완전 포함 " + selectedExclusions.length + "개"}
        {disabled ? <span className="spinner" /> : null}</span>
      <button type="button" className="danger-button text-button" disabled={disabled || loading || !selectedExclusions.length}
        onClick={() => { const keys = selectedExclusions.map((item) => item.key); setSubmitted(keys); onApply(keys); }}>
        {disabled ? "처리 중" : selectedExclusions.length + "개 제외"}
      </button>
    </div> : null}
    {progress?.failure ? <div className="inline-error" role="alert">{progress.failure}</div> : null}
    {compared.length ? <section className="overlap-group-comparison" aria-label="여러 판본 페이지 비교">
      <header><strong>페이지 비교 <small>{compared.length + 1}개 판본</small></strong>
        <span role="status" className="overlap-group-selection-count">{selectionCount ? selectionCount + "장 선택" : ""}</span>
        {evidence.pending ? <small role="status">대응 확인 중</small> : evidence.failed ? <button type="button" className="text-button" onClick={evidence.retry}>대응 다시 확인</button> : null}
        <div role="group" aria-label="페이지 표시 필터">
          {([["matched", "일치 보기"], ["unique", "추가 보기"]] as const).map(([key, label]) =>
            <button type="button" key={key} className="download-overlap-page-filter" aria-pressed={visibility[key]}
              onClick={() => { setVisibility((current) => ({ ...current, [key]: !current[key] })); resetScroll(); }}>{label}</button>)}
        </div>
      </header>
      {onMerge && compared.some((item) => item.canDecide) ? <p className="overlap-group-selection-guide"><kbd>Ctrl</kbd>과 <kbd>Shift</kbd>를 이용해 원하는 페이지를 선택할 수 있습니다.</p> : null}
      {invalidSelections.length ? <div className="inline-error" role="alert">페이지 변경 · {invalidSelections.map((item) => "#" + item.excluded.galleryId).join(", ")} 재선택 필요</div> : null}
      {hint ? <div className="inline-error" role="alert">{hint}</div> : null}
      <div className="overlap-group-grid-scroll" ref={scroller} data-thumbnail-scroll-root tabIndex={0}
        style={{ "--comparison-height": ((compared.length + 1) * rowHeight + 18) + "px" } as CSSProperties}
        aria-label="합본 기준 페이지 비교표" onScroll={(event) => { setScrollLeft(event.currentTarget.scrollLeft); onClearPreview(); }}>
        {visibleRows.length ? [null, ...compared].map((item) => {
          const edition = item ?? active;
          if (!edition) return null;
          const keeper = item === null, selected = keeper ? activeSelection : validSelection(edition);
          return <div className={"overlap-group-grid-row" + (keeper ? " is-keeper" : active?.key === edition.key ? " is-active" : "")}
            style={gridStyle} key={keeper ? "keeper" : edition.key} data-edition-row={keeper ? group.keeper.galleryId : edition.excluded.galleryId}>
            <div className="overlap-group-row-head">
              <button type="button" className="overlap-group-row-title" aria-label={'#'+(keeper?group.keeper.galleryId:edition.excluded.galleryId)+' 페이지 선택 활성화'} aria-pressed={keeper || active?.key===edition.key} onClick={()=>setActiveKey(edition.key)}>
                <span className="overlap-group-row-meta"><b>#{keeper?group.keeper.galleryId:edition.excluded.galleryId}</b><span>{(keeper?group.keeper:edition.excluded).pageCount}p</span>{keeper?<span>{group.confirmed ? group.editions.length > 1 ? '보존 합본' : '포함 판본' : '기준 판본'}</span>:null}</span>
                <strong title={(keeper?group.keeper:edition.excluded).title}>{(keeper?group.keeper:edition.excluded).title}</strong>
              </button>
              <div className="overlap-edition-modes" role="group" aria-label={'#'+(keeper?group.keeper.galleryId:edition.excluded.galleryId)+' 남길 범위'} data-mode={modesFor(edition)[keeper===edition.keeperIsIncoming?'incoming':'existing']}>
                {([['keep','남기기'],['partial','일부만'],['remove','제거']] as const).map(([mode,label])=><button type="button" key={mode} aria-pressed={modesFor(edition)[keeper===edition.keeperIsIncoming?'incoming':'existing']===mode}
                  disabled={disabled||!edition.canDecide||(mode==='partial'&&!onMerge)} onClick={()=>setMode(edition,keeper===edition.keeperIsIncoming?'incoming':'existing',mode)}>{label}</button>)}
              </div>
            </div>
            <div className="overlap-group-page-spacer" style={{ width: start * pageWidth }} aria-hidden="true" />
            {visibleRows.slice(start, end).map((row, offset) => {
              const cell = row.pages[edition.key], page = keeper ? row.keeperPage : cell?.page;
              const pair = keeper ? activePair(page) : cell?.pair;
              const matched = keeper ? row.keeperMatched : cell?.matched;
              const displayMatch = {matched:Boolean(matched),reordered:keeper?row.keeperReordered:cell?.reordered,
                counterparts:keeper?row.keeperCounterparts:cell?.counterparts, pending:!matched&&evidence.pending,
                unverified:!matched&&Boolean(loadPageEvidence)&&(evidence.failed||!allPairsKnown)};
              return <div className="overlap-group-grid-cell" key={row.key} data-page-slot={row.key}>
                {renderPage({ edition, side: keeper === edition.keeperIsIncoming ? "incoming" : "existing",
                  page, pair, index: start + offset, label: "#" + (keeper ? group.keeper.galleryId : edition.excluded.galleryId),
                  visible: visibility[kind(page, matched)], displayMatch, selection: selected, canSelect: edition.canDecide && Boolean(onMerge),
                  onToggle: (...args) => togglePage(edition, ...args) })}
              </div>;
            })}
            <div className="overlap-group-page-spacer" style={{ width: (visibleRows.length - end) * pageWidth }} aria-hidden="true" />
          </div>;
        }) : <div className="overlap-group-empty">표시할 페이지 없음</div>}
      </div>
      {(onApplyEditions ? applyScope.length > 0 || editionProgress !== null : active?.canDecide) ? <footer className="overlap-group-actions">
        <strong>{onApplyEditions ? applyScope.length+"개 판본" : "#"+active?.excluded.galleryId}</strong>
        <span>#{group.keeper.galleryId}와 비교</span>
        {editionProgress ? <span role="status">{editionProgress.completedKeys.length}/{editionProgress.total} 처리</span> : null}
        {count(activeSelection)>0?<><span>{count(activeSelection)}장 선택</span><button type="button" className="text-button" disabled={disabled} onClick={()=>setSelections(current=>{const next={...current};if(active)delete next[active.key];return next;})}>선택 해제</button></>:null}
        {active?.canDecide ? <button type="button" className="text-button" disabled={disabled || selectionCount > 0} onClick={() => onDecision({ reviewId:active.review.reviewId, expectedRevision:active.review.revision, candidateId:active.candidate.candidateId, action:"false_positive_continue" })}>중복 아님</button> : null}
        <button type="button" className="primary-button" disabled={disabled || (Boolean(onApplyEditions) && !applyScope.length)} onClick={()=>void applyDisposition()}>{onApplyEditions ? applyScope.length+"개 적용" : "적용"}</button>
      </footer> : null}
      {editionProgress?.failure ? <div className="inline-error" role="alert">{editionProgress.failure} · 처리하지 못한 선택은 유지됩니다.</div> : null}
    </section> : null}
  </section>;
}
