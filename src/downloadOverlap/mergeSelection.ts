import type { DownloadOverlapCandidate, DownloadOverlapMergeRequest } from "../api/contracts";

export type MergeSide = "existing" | "incoming";
export type MergeSelection = { existing: number[]; incoming: number[] };
export const emptyMergeSelection = (): MergeSelection => ({ existing: [], incoming: [] });

/** Range selection follows source page order, not holes on the shared axis. */
export function selectMergePages(candidate: DownloadOverlapCandidate, selection: MergeSelection,
  side: MergeSide, page: number, anchor?: { side: MergeSide; page: number }, range = false): MergeSelection {
  const other = side === "existing" ? "incoming" : "existing";
  const own = new Set(selection[side]);
  const opposite = new Set(selection[other]);
  const start = range && anchor?.side === side ? Math.min(anchor.page, page) : page;
  const end = range && anchor?.side === side ? Math.max(anchor.page, page) : page;
  const remove = !range && own.has(page);
  for (let number = start; number <= end; number++) {
    if (remove) own.delete(number);
    else {
      own.add(number);
      const pair = candidate.pagePairs.find((p) => (side === "existing" ? p.existingSourcePage : p.incomingSourcePage) === number);
      if (pair) opposite.delete(side === "existing" ? pair.incomingSourcePage : pair.existingSourcePage);
    }
  }
  return { [side]: [...own].sort((a,b) => a-b), [other]: [...opposite].sort((a,b) => a-b) } as MergeSelection;
}

/** Selected counterparts override the destination; only selected unique donor pages are inserted. */
export function mergeOutputPages(candidate: DownloadOverlapCandidate, incomingCount: number,
  request: Pick<DownloadOverlapMergeRequest, "sourceSide" | "selectedPages">): { side: MergeSide; page: number }[] {
  const selected = request.selectedPages ?? emptyMergeSelection();
  const counts = { existing: candidate.existing.pageCount, incoming: incomingCount };
  const pairs = [...candidate.pagePairs].sort((a,b) => a.existingSourcePage-b.existingSourcePage);
  if (candidate.matchedPages !== pairs.length || candidate.existingUniquePages !== counts.existing - pairs.length
    || candidate.incomingUniquePages !== counts.incoming - pairs.length) throw new Error("페이지 대응 개수가 달라졌습니다. 다시 대조해 주세요.");
  if (!pairs.length || pairs.some((p,i) => p.existingSourcePage < 1 || p.existingSourcePage > counts.existing
    || p.incomingSourcePage < 1 || p.incomingSourcePage > counts.incoming
    || (i > 0 && (p.existingSourcePage <= pairs[i-1]!.existingSourcePage || p.incomingSourcePage <= pairs[i-1]!.incomingSourcePage)))) throw new Error("페이지 대응 순서를 다시 확인해 주세요.");
  for (const side of ["existing", "incoming"] as const) {
    if (new Set(selected[side]).size !== selected[side].length || selected[side].some((p) => !Number.isSafeInteger(p) || p < 1 || p > counts[side])) throw new Error("선택 페이지가 유효하지 않습니다.");
  }
  if (pairs.some((p) => selected.existing.includes(p.existingSourcePage) && selected.incoming.includes(p.incomingSourcePage))) throw new Error("대응 페이지는 한쪽만 선택해 주세요.");
  const target = request.sourceSide === "existing" ? "incoming" : "existing";
  const source = request.sourceSide;
  const read = (p: typeof pairs[number], side: MergeSide) => side === "existing" ? p.existingSourcePage : p.incomingSourcePage;
  const output: {side: MergeSide; page: number}[] = [];
  let t = 1; let s = 1;
  for (const pair of pairs) {
    while (t < read(pair, target)) output.push({ side: target, page: t++ });
    while (s < read(pair, source)) { if (selected[source].includes(s)) output.push({ side: source, page: s }); s++; }
    output.push(selected[source].includes(s) ? { side: source, page: s } : { side: target, page: t });
    t++; s++;
  }
  while (t <= counts[target]) output.push({ side: target, page: t++ });
  while (s <= counts[source]) { if (selected[source].includes(s)) output.push({ side: source, page: s }); s++; }
  return output;
}
