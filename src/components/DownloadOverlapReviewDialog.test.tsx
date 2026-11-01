import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { DownloadOverlapAutomationHistoryItem, DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import type { DownloadOverlapContainmentGroup } from "../state/downloadOverlapContainment";
import { ThumbnailClient, type ThumbnailRequest } from "../thumbnail";
import { DownloadOverlapReviewDialog } from "./DownloadOverlapReviewDialog";

const fixture = (): DownloadOverlapReview => ({
  reviewId: "review-overlap",
  entryId: "incoming-entry",
  incoming: {
    entryId: "incoming-entry",
    galleryId: galleryId(200),
    title: "New edition",
    artists: ["artist a"],
    pageCount: 12,
  },
  revision: 4,
  state: "pending",
  profileVersion: 1,
  policyVersion: 1,
  incomingFingerprint: "incoming-fingerprint",
  candidates: ["near_equivalent", "incoming_contains_existing", "existing_contains_incoming", "partial_overlap"].map((relation, index) => ({
    candidateId: `candidate-${index + 1}`,
    existing: {
      entryId: `existing-entry-${index + 1}`,
      galleryId: galleryId(100 + index),
      title: `Owned edition ${index + 1}`,
      artists: ["artist a"],
      pageCount: 10,
    },
    existingFingerprint: `fingerprint-${index + 1}`,
    relation: relation as DownloadOverlapReview["candidates"][number]["relation"],
    confidence: 0.94,
    matchedPages: 8,
    exactPages: 3,
    visualPages: 5,
    existingCoverage: 0.8,
    incomingCoverage: 2 / 3,
    existingUniquePages: 2,
    incomingUniquePages: 4,
    longestAlignedRun: 6,
    rank: index + 1,
    pagePairs: [{
      incomingSourcePage: index + 2,
      existingSourcePage: index + 1,
      exactSha256: false,
      dHashDistance: 2,
      pHashDistance: 3,
      detailHashDistance: 19,
      edgeSimilarity: 0.91,
      visualSimilarity: 0.93,
      lowInformation: false,
    }],
  })),
  createdAt: "2026-08-25T00:00:00.000Z",
  updatedAt: "2026-08-25T00:00:00.000Z",
});

const containmentFixture = (): {
  review: DownloadOverlapReview;
  group: DownloadOverlapContainmentGroup;
} => {
  const review = fixture();
  review.incoming = {
    ...review.incoming,
    galleryId: galleryId(3003760),
    title: "Compilation edition",
    pageCount: 155,
  };
  const containedPageCounts = [17, 9, 9];
  const completeCandidates = containedPageCounts.map((pageCount, index) => ({
    ...review.candidates[index]!,
    candidateId: `contained-${index + 1}`,
    existing: {
      ...review.candidates[index]!.existing,
      galleryId: galleryId(3003700 + index),
      title: `Contained edition ${index + 1}`,
      pageCount,
    },
    relation: "incoming_contains_existing" as const,
    confidence: 0.97,
    matchedPages: pageCount,
    exactPages: pageCount,
    visualPages: 0,
    existingCoverage: 1,
    incomingCoverage: pageCount / 155,
    existingUniquePages: 0,
    incomingUniquePages: 155 - pageCount,
    longestAlignedRun: pageCount,
    rank: index + 1,
    pagePairs: Array.from({ length: pageCount }, (_, pageIndex) => ({
      ...review.candidates[index]!.pagePairs[0]!,
      incomingSourcePage: pageIndex + 20 * index + 1,
      existingSourcePage: pageIndex + 1,
      exactSha256: true,
      lowInformation: false,
    })),
  }));
  const ambiguous = {
    ...review.candidates[3]!,
    candidateId: "ambiguous-156",
    existing: {
      ...review.candidates[3]!.existing,
      galleryId: galleryId(3003999),
      title: "Similar 156 page edition",
      pageCount: 156,
    },
    relation: "partial_overlap" as const,
    confidence: 0.91,
    matchedPages: 140,
    exactPages: 80,
    visualPages: 60,
    existingCoverage: 140 / 156,
    incomingCoverage: 140 / 155,
    existingUniquePages: 16,
    incomingUniquePages: 15,
    longestAlignedRun: 60,
    rank: 4,
  };
  review.candidates = [...completeCandidates, ambiguous];
  const group: DownloadOverlapContainmentGroup = {
    keeper: review.incoming,
    items: completeCandidates.map((candidate) => ({
      key: `${review.reviewId}:${candidate.candidateId}`,
      review,
      candidate,
      action: "remove_existing_continue",
      excluded: candidate.existing,
      keeperIsIncoming: true,
    })),
  };
  return { review, group };
};

const pageMergeFixture = (): DownloadOverlapReview => {
  const review = fixture();
  review.incoming = { ...review.incoming, pageCount: 12 };
  const original = review.candidates[0]!;
  const candidate = {
    ...original,
    existing: { ...original.existing, pageCount: 10 },
    matchedPages: 10,
    exactPages: 4,
    visualPages: 6,
    existingCoverage: 1,
    incomingCoverage: 10 / 12,
    existingUniquePages: 0,
    incomingUniquePages: 2,
    longestAlignedRun: 10,
    pagePairs: Array.from({ length: 10 }, (_, index) => ({
      ...original.pagePairs[0]!,
      existingSourcePage: index + 1,
      incomingSourcePage: index + 1,
    })),
  };
  review.candidates = [candidate];
  return review;
};

describe("DownloadOverlapReviewDialog", () => {
  it("keeps horizontal virtualization, covers, selection and sizing independent", async()=>{
    const review=pageMergeFixture(),first=review.candidates[0]!;
    review.incoming={...review.incoming,pageCount:1000};
    review.candidates=[0,1,2].map(n=>({...first,candidateId:"wide-"+n,existing:{...first.existing,entryId:"wide-"+n,galleryId:galleryId(100+n),pageCount:1000},existingFingerprint:"wide-fp-"+n,existingUniquePages:990,incomingUniquePages:990,existingCoverage:.01,incomingCoverage:.01}));
    const client=new ThumbnailClient({resolve:()=>({kind:"image",url:"https://example.test/cover.png",width:120,height:170})});
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    let resize:()=>void=()=>{};
    vi.stubGlobal("ResizeObserver",class{constructor(callback:()=>void){resize=callback;}observe(){}disconnect(){}});
    const click=async(label:string)=>act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="'+label+'"]')!.click());
    try{
      await act(async()=>root.render(<DownloadOverlapReviewDialog open={false} review={review} previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={vi.fn()}/>));
      const scroll=container.querySelector<HTMLDivElement>(".overlap-group-grid-scroll")!;
      const covers=[...container.querySelectorAll(".overlap-group-cover img")];
      const selected=()=>container.querySelector<HTMLElement>('[aria-label^="#100 1페이지"]')!;
      expect([...container.querySelectorAll(".download-overlap-page-filter")].map(b=>b.textContent)).toEqual(["일치 보기","추가 보기"]);
      expect(container.querySelector(".is-gap")).toBeNull();
      expect(container.querySelector('input[type="checkbox"]')).toBeNull();
      await act(async()=>selected().dispatchEvent(new MouseEvent("click",{bubbles:true,ctrlKey:true,button:0})));
      expect(selected()).toHaveAttribute("data-merge-state","source");
      await act(async()=>{scroll.scrollLeft=24000;scroll.dispatchEvent(new Event("scroll",{bubbles:true}));});
      expect(selected()).toBeNull();
      expect(container.querySelectorAll(".overlap-group-grid-cell").length).toBeLessThan(45);
      expect(container.querySelector(".overlap-group-actions")).toHaveTextContent("1장 선택");
      expect([...container.querySelectorAll(".overlap-group-cover img")]).toEqual(covers);
      await click("창 확대");
      expect(container.querySelector("dialog")).toHaveClass("is-expanded");
      Object.defineProperty(scroll,"clientWidth",{value:1800,configurable:true});
      await act(async()=>resize());
      expect(container.querySelectorAll(".overlap-group-grid-cell").length).toBeGreaterThan(45);
      await act(async()=>{scroll.scrollLeft=0;scroll.dispatchEvent(new Event("scroll",{bubbles:true}));});
      expect(selected()).toHaveAttribute("data-merge-state","source");
      const height=parseFloat(scroll.style.getPropertyValue("--comparison-height"));
      await click("#102 비교");
      expect(container.querySelectorAll(".overlap-group-row-head")).toHaveLength(4);
      expect(parseFloat(scroll.style.getPropertyValue("--comparison-height"))).toBeGreaterThan(height);
      await click("창 크기 복원");
      expect(container.querySelector("dialog")).not.toHaveClass("is-expanded");
      expect(selected()).toHaveAttribute("data-merge-state","source");
      expect([...container.querySelectorAll(".overlap-group-cover img")]).toEqual(covers);
    } finally {await act(async()=>root.unmount());client.dispose();container.remove();vi.unstubAllGlobals();}
  });

  it("reattaches scrolling and sizing after a background review reopens without losing selections", async () => {
    const review=pageMergeFixture(),first=review.candidates[0]!;
    review.candidates=[first,{...first,candidateId:"second",existing:{...first.existing,entryId:"second",galleryId:galleryId(98)},existingFingerprint:"second-fp"}];
    const client=new ThumbnailClient({resolve:()=>({kind:"missing",reason:"fixture"})});
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    const observe=vi.fn();vi.stubGlobal("ResizeObserver",class{observe=observe;disconnect(){}});
    const render=async(open:boolean)=>act(async()=>root.render(<DownloadOverlapReviewDialog open={open} preserveDraftOnHide review={review} previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={vi.fn()}/>));
    try {
      await render(true);
      await act(async()=>container.querySelector<HTMLElement>('[aria-label^="#100 1페이지"]')!.dispatchEvent(new MouseEvent("click",{bubbles:true,ctrlKey:true})));
      const old=container.querySelector(".overlap-group-grid-scroll");
      await render(false);expect(container.querySelector(".overlap-group-grid-scroll")).toBeNull();
      await render(true);
      const scroll=container.querySelector<HTMLElement>(".overlap-group-grid-scroll")!;
      expect(scroll).not.toBe(old);expect(observe).toHaveBeenCalledWith(scroll);
      Object.defineProperties(scroll,{scrollWidth:{value:2000},clientWidth:{value:700}});
      const wheel=new WheelEvent("wheel",{bubbles:true,cancelable:true,deltaY:120,shiftKey:true});
      await act(async()=>scroll.dispatchEvent(wheel));
      expect(wheel.defaultPrevented).toBe(true);expect(scroll.scrollLeft).toBe(120);
      expect(container.querySelector('[aria-label^="#100 1페이지"]')).toHaveAttribute("data-merge-state","source");
    } finally {await act(async()=>root.unmount());client.dispose();container.remove();vi.unstubAllGlobals();}
  });

  async function mountPageFilters() {
    const review = pageMergeFixture();
    const onMergePages = vi.fn();
    const resolve = vi.fn(() => ({ kind: "missing" as const, reason: "fixture" }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<DownloadOverlapReviewDialog open={false} review={review} previewWidth={220}
      thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={onMergePages} />));
    const page = (side: "기존 A" | "신규 B", number: number) =>
      container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="${side} ${number}페이지"]`);
    const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-page-filter")].find(item => item.textContent === label)!;
    const toggle = async (label: string) => act(async () => button(label).click());
    const select = async (side: "기존 A" | "신규 B", number: number, shiftKey = false) => act(async () =>
      page(side, number)!.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, ctrlKey: !shiftKey, shiftKey })));
    const dispose = async () => {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    };
    return { container, onMergePages, resolve, page, button, toggle, select, dispose };
  }

  it("toggles matched, unique and gap displays independently while retaining aligned cells", async () => {
    const ui = await mountPageFilters();
    try {
      for (const label of ["일치", "한쪽에만 있음", "대응 없음"]) {
        expect(ui.button(label)).toHaveAttribute("type", "button");
        expect(ui.button(label)).toHaveAttribute("aria-pressed", "true");
      }
      const uniqueCell = ui.page("신규 B", 11);
      const originalRequests = ui.resolve.mock.calls.length;
      await ui.toggle("일치");
      expect(ui.button("일치")).toHaveAttribute("aria-pressed", "false");
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-matched")).toHaveLength(0);
      expect(ui.page("신규 B", 11)).toBe(uniqueCell);
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-gap")).toHaveLength(2);
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-unique")).toHaveLength(2);
      await ui.toggle("대응 없음");
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-gap")).toHaveLength(0);
      expect(ui.container.querySelectorAll('.download-overlap-page-cell.is-filtered[aria-hidden="true"]')).toHaveLength(2);
      expect(ui.page("신규 B", 11)).toBe(uniqueCell);
      expect(ui.resolve.mock.calls).toHaveLength(originalRequests);
      const grid = ui.container.querySelector<HTMLElement>(".download-overlap-alignment-grid")!;
      expect(grid.style.getPropertyValue("--overlap-page-columns")).toBe("2");
      expect([...grid.children].map(node => node.classList.contains("download-overlap-row-label") ? node.textContent : node.getAttribute("aria-label") ?? "hidden"))
        .toEqual(["기존 A", "hidden", "hidden", "신규 B", expect.stringContaining("신규 B 11페이지"), expect.stringContaining("신규 B 12페이지")]);
      await ui.toggle("한쪽에만 있음");
      expect(ui.container.querySelector(".download-overlap-alignment-grid")).toBeNull();
      expect(ui.container.querySelector(".download-overlap-page-filter-empty")).toHaveTextContent("표시할 페이지가 없습니다.");
      await ui.toggle("대응 없음");
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-gap")).toHaveLength(2);
      expect(ui.page("신규 B", 11)).toBeNull();
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-filtered")).toHaveLength(2);
      await ui.toggle("한쪽에만 있음");
      await ui.toggle("일치");
      expect(ui.container.querySelectorAll(".download-overlap-page-cell.is-matched")).toHaveLength(20);
    } finally { await ui.dispose(); }
  });

  it("keeps hidden Ctrl selections in the unchanged merge payload, including when all filters are off", async () => {
    const ui = await mountPageFilters();
    try {
      await ui.select("기존 A", 3);
      await ui.select("신규 B", 11);
      await ui.toggle("일치");
      expect(ui.container.querySelector(".download-overlap-hidden-selection")).toHaveTextContent("선택 1장 숨김");
      expect(ui.page("신규 B", 11)).toHaveAttribute("data-merge-state", "source");
      await ui.toggle("한쪽에만 있음");
      await ui.toggle("대응 없음");
      expect(ui.container.querySelector(".download-overlap-hidden-selection")).toHaveTextContent("선택 2장 숨김");
      expect(ui.container.querySelector(".download-overlap-action-note")).toHaveTextContent("A 1장 · B 1장 선택");
      const merge = [...ui.container.querySelectorAll<HTMLButtonElement>(".download-overlap-merge-apply")].find(b => b.textContent === "B에 병합")!;
      await act(async () => merge.click());
      expect(ui.onMergePages).toHaveBeenCalledExactlyOnceWith({ reviewId: "review-overlap", expectedRevision: 4, candidateId: "candidate-1",
        sourceSide: "existing", sourcePages: [3], selectedPages: { existing: [3], incoming: [11] } });
      await ui.toggle("일치");
      await ui.toggle("한쪽에만 있음");
      expect(ui.page("기존 A", 3)).toHaveAttribute("data-merge-state", "source");
      expect(ui.page("신규 B", 3)).toHaveAttribute("data-merge-state", "target");
      expect(ui.page("신규 B", 11)).toHaveAttribute("data-merge-state", "source");
      expect(ui.container.querySelector(".download-overlap-hidden-selection")).toBeNull();
      await ui.select("신규 B", 3);
      expect(ui.page("기존 A", 3)).toHaveAttribute("data-merge-state", "target");
      expect(ui.page("신규 B", 3)).toHaveAttribute("data-merge-state", "source");
      expect(ui.page("신규 B", 11)).toHaveAttribute("data-merge-state", "source");
    } finally { await ui.dispose(); }
  });

  it("preserves the original Shift anchor and source-page range across hidden matches", async () => {
    const ui = await mountPageFilters();
    try {
      await ui.select("신규 B", 1);
      await ui.toggle("일치");
      await ui.select("신규 B", 12, true);
      expect(ui.container.querySelector(".download-overlap-action-note")).toHaveTextContent("A 0장 · B 12장 선택");
      expect(ui.container.querySelector(".download-overlap-hidden-selection")).toHaveTextContent("선택 10장 숨김");
      await ui.toggle("일치");
      expect(ui.container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(12);
      expect(ui.container.querySelectorAll('[data-merge-state="target"]')).toHaveLength(10);
      for (let page = 1; page <= 12; page++) expect(ui.page("신규 B", page)).toHaveClass("is-merge-source");
    } finally { await ui.dispose(); }
  });

  it("clears a hover preview when filtering and still allows explicit clearing of hidden selections", async () => {
    const ui = await mountPageFilters();
    try {
      await ui.select("기존 A", 2);
      await act(async () => ui.page("기존 A", 2)!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(ui.container.querySelector(".download-overlap-page-hover-preview")).not.toBeNull();
      await ui.toggle("일치");
      expect(ui.container.querySelector(".download-overlap-page-hover-preview")).toBeNull();
      const clear = [...ui.container.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === "선택 해제")!;
      await act(async () => clear.click());
      expect(ui.container.querySelector(".download-overlap-hidden-selection")).toBeNull();
      await ui.toggle("일치");
      expect(ui.container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(0);
      expect(ui.onMergePages).not.toHaveBeenCalled();
    } finally { await ui.dispose(); }
  });

  it("keeps processed evidence read-only and explains unavailable historical images", async () => {
    const requests: ThumbnailRequest[] = [];
    const client = new ThumbnailClient({ resolve: (request) => {
      requests.push(request);
      const error = new Error("판정 당시 원본을 사용할 수 없습니다.");
      error.name = request.key.kind === "overlap-review-page" && request.key.side === "existing"
        ? "THUMBNAIL_evidenceChanged" : "THUMBNAIL_evidenceUnavailable";
      throw error;
    } });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.state = "resolved";
    review.candidates = [{ ...review.candidates[0]!, decision: "existing_removed" }];
    const decide = vi.fn();
    await act(async () => root.render(<DownloadOverlapReviewDialog open={false} review={review} previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={decide} />));
    expect(container).toHaveTextContent("격리·제외된 원본도 복원 없이 비교합니다");
    expect(container).toHaveTextContent("판정 당시 원본 변경됨");
    expect(container).toHaveTextContent("판정 당시 원본 없음");
    expect(requests.every((request) => request.key.kind === "overlap-review-page")).toBe(true);
    expect(decide).not.toHaveBeenCalled();
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "기존 A 제거")).toBe(false);
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });
  it("selects mapped source pages with Ctrl+click and submits a sorted merge request", async () => {
    const review = pageMergeFixture();
    const onMergePages = vi.fn();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = async (decisionPending = false) => act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        decisionPending={decisionPending}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={onMergePages}
      />,
    ));

    await render();
    const existingPage = (page: number) => container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="기존 A ${page}페이지"]`)!;
    const incomingPage = (page: number) => container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="신규 B ${page}페이지"]`)!;
    await act(async () => existingPage(3).dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();

    await act(async () => existingPage(3).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    await act(async () => existingPage(1).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    expect(existingPage(1)).toHaveClass("is-merge-source");
    expect(existingPage(3)).toHaveClass("is-merge-source");
    expect(incomingPage(1)).toHaveClass("is-merge-target");
    expect(incomingPage(3)).toHaveClass("is-merge-target");
    expect(incomingPage(11)).toHaveClass("is-unique");
    expect(incomingPage(11)).not.toHaveClass("is-merge-target");
    expect(container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(2);
    expect(container.querySelectorAll('[data-merge-state="target"]')).toHaveLength(2);
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("A 2장 · B 0장 선택");
    expect(container.textContent).toContain("검토 미루기");
    expect([...container.querySelectorAll(".overlap-destination-button")].map((b)=>b.textContent)).toEqual(["A에 병합","B에 병합"]);
    for (const label of ["오탐 판정", "둘 다 보존"]) {
      expect([...container.querySelectorAll("button")].find((b)=>b.textContent===label)).toBeDisabled();
    }
    await render(true);
    expect(container.querySelector(".download-overlap-merge-apply")).toBeDisabled();
    await render(false);
    const mergeButton = container.querySelectorAll<HTMLButtonElement>(".download-overlap-merge-apply")[1]!;
    await act(async () => mergeButton.click());
    expect(onMergePages).toHaveBeenCalledWith({
      reviewId:"review-overlap", expectedRevision:4,candidateId:"candidate-1",sourceSide:"existing",sourcePages:[1,3],
      selectedPages:{existing:[1,3],incoming:[]},
    });

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("selects unique pages, switches pair sides exclusively and supports Shift ranges", async () => {
    const review = pageMergeFixture();
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={vi.fn()}
      />,
    ));
    const page = (side: "기존 A" | "신규 B", number: number) =>
      container.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="${side} ${number}페이지"]`)!;
    const ctrlClick = async (element: HTMLElement) => act(async () =>
      element.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));

    await ctrlClick(page("신규 B",11));
    expect(page("신규 B",11)).toHaveClass("is-merge-source");
    await ctrlClick(page("기존 A",1));
    expect(page("기존 A",1)).toHaveClass("is-merge-source");
    await ctrlClick(page("신규 B",1));
    expect(page("기존 A",1)).toHaveClass("is-merge-target");
    expect(page("신규 B",1)).toHaveClass("is-merge-source");
    await act(async () => page("신규 B",3).dispatchEvent(new MouseEvent("click",{bubbles:true,shiftKey:true,button:0})));
    expect(page("신규 B",2)).toHaveClass("is-merge-source");
    expect(page("신규 B",3)).toHaveClass("is-merge-source");
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="선택 해제")!.click());
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();
    expect(container.querySelector(".download-overlap-page-cell.is-merge-target")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("blocks a malformed mapping when summary uniqueness says the source is complete", async () => {
    const review = pageMergeFixture();
    const candidate = review.candidates[0]!;
    candidate.existingUniquePages = 0;
    candidate.pagePairs = candidate.pagePairs.slice(0, -1);
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
        onMergePages={vi.fn()}
      />,
    ));

    const mappedCell = container.querySelector<HTMLElement>('.download-overlap-page-cell[aria-label^="기존 A 1페이지"]')!;
    await act(async () => mappedCell.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 })));
    expect(container.querySelector(".download-overlap-merge-guide")).toHaveTextContent("페이지 대응 개수나 순서가 달라졌습니다");
    expect(container.querySelector(".download-overlap-page-cell.is-merge-source")).toBeNull();
    expect(container.querySelector(".download-overlap-merge-apply")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("preserves per-edition Ctrl/Shift selections across columns, filters and unrelated revisions", async () => {
    const review = pageMergeFixture(), first = review.candidates[0]!;
    review.candidates = [first, { ...first, candidateId:"candidate-second", existing:{...first.existing,entryId:"second",galleryId:galleryId(98),title:"Second edition"}, existingFingerprint:"second-fp",rank:2 }];
    const client = new ThumbnailClient({ resolve: () => ({kind:"missing",reason:"fixture"}) });
    const container=document.createElement("div"); document.body.append(container);const root=createRoot(container), onMerge=vi.fn();
    const confirm=vi.spyOn(window,"confirm").mockReturnValue(true);
    const render=async (data=review)=>act(async()=>root.render(<DownloadOverlapReviewDialog open={false} review={data} previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={onMerge}/>));
    const cell=(id:number,page:number)=>container.querySelector<HTMLElement>('.download-overlap-page-cell[aria-label^="#'+id+' '+page+'페이지"]')!;
    const click=async(el:HTMLElement)=>act(async()=>el.click());
    const select=async(id:number,page:number,shift=false)=>act(async()=>cell(id,page).dispatchEvent(new MouseEvent("click",{bubbles:true,button:0,ctrlKey:!shift,shiftKey:shift})));
    try {
      await render();
      expect(container.querySelectorAll(".overlap-group-row-head")).toHaveLength(3);
      const mode=(id:number,label:string)=>[...container.querySelectorAll<HTMLButtonElement>('[data-edition-row="'+id+'"] .overlap-edition-modes button')].find(b=>b.textContent===label)!;
      expect(container.querySelector('[aria-label="#100 비교 닫기"]')).toBeNull();
      expect(mode(100,'남기기')).toHaveAttribute('aria-pressed','true');
      await click(mode(100,'제거'));expect(onMerge).not.toHaveBeenCalled();
      await select(100,1);expect(mode(100,'일부만')).toHaveAttribute('aria-pressed','true');
      await click(mode(100,'남기기'));expect(cell(100,1)).toHaveAttribute('data-merge-state','source');
      await select(98,2); await select(100,3,true);
      expect(mode(100,'일부만')).toHaveAttribute('aria-pressed','true');
      expect(cell(100,1)).toHaveAttribute("data-merge-state","source");
      expect(cell(100,2)).toHaveAttribute("data-merge-state","source");
      expect(cell(100,3)).toHaveAttribute("data-merge-state","source");
      await click(container.querySelector('[aria-label="#100 비교"]')!);
      expect(cell(100,1)).toBeNull();
      expect(container.querySelector('[data-edition-id="100"]')).toHaveTextContent("선택 3");
      await click(container.querySelector('[aria-label="#100 비교"]')!);
      expect(cell(100,1)).toHaveAttribute("data-merge-state","source");
      for(const filter of container.querySelectorAll<HTMLButtonElement>(".download-overlap-page-filter")) await click(filter);
      expect(container.querySelectorAll(".overlap-group-grid-row")).toHaveLength(0);
      expect(container.querySelector(".overlap-group-actions")).toHaveTextContent("3장 선택");
      await render({...review,revision:5});
      const merge=[...container.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="적용")!;
      await click(merge);
      expect(onMerge).toHaveBeenCalledWith({reviewId:"review-overlap",expectedRevision:5,candidateId:"candidate-1",sourceSide:"existing",sourcePages:[1,2,3],selectedPages:{existing:[1,2,3],incoming:[]},targetSelectionOnly:false,excludeSource:true});
      for(const filter of container.querySelectorAll<HTMLButtonElement>(".download-overlap-page-filter")) await click(filter);
      expect(cell(98,2)).toHaveAttribute("data-merge-state","source");
      await render({...review,revision:6,incomingFingerprint:"changed"});
      expect(container.querySelector(".overlap-group-comparison")).toHaveTextContent("재선택 필요");
      expect(container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(0);
    } finally { await act(async()=>root.unmount());client.dispose();container.remove();confirm.mockRestore(); }
  });

  it("shows the keeper once and compares uncertain editions together without offering their bulk exclusion", async () => {
    const {review,group}=containmentFixture(); const onApply=vi.fn();
    const client=new ThumbnailClient({resolve:()=>({kind:"missing",reason:"fixture"})});
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    try {
      await act(async()=>root.render(<DownloadOverlapReviewDialog open={false} review={review} containmentGroup={group} previewWidth={220}
        thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onApplyContainmentBatch={onApply}/>));
      expect(container.querySelectorAll(".overlap-group-keeper")).toHaveLength(1);
      expect(container.querySelector(".download-overlap-artifacts")).toBeNull();
      expect(container.querySelector(".download-overlap-actions")).toBeNull();
      expect(container.querySelector(".overlap-group-keeper")).toHaveTextContent("Compilation edition");
      expect(container.querySelector('[aria-label="#3003999 제외 선택"]')).toBeNull();
      await act(async()=>container.querySelector<HTMLInputElement>('[aria-label="#3003701 제외 선택"]')!.click());
      await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="#3003700 비교"]')!.click());
      expect(container.querySelectorAll(".overlap-group-row-head")).toHaveLength(3);
      expect(container.querySelectorAll(".overlap-group-grid-cell").length).toBeLessThanOrEqual(45);
      await act(async()=>container.querySelector<HTMLButtonElement>(".overlap-group-batch button")!.click());
      expect(onApply).toHaveBeenCalledExactlyOnceWith(["review-overlap:contained-1","review-overlap:contained-3"]);
    } finally {await act(async()=>root.unmount());client.dispose();container.remove();}
  });

  it("locks a running batch, folds completed editions and retains unresolved exclusions with fresh thumbnails", async () => {
    const {review,group}=containmentFixture(), onApply=vi.fn();
    const resolve=vi.fn((_request:ThumbnailRequest)=>({kind:"missing" as const,reason:"fixture"}));
    const client=new ThumbnailClient({resolve});const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    const render=async(busy=false,completed?:number,current=review)=>act(async()=>root.render(<DownloadOverlapReviewDialog open={false}
      review={current} containmentGroup={{...group,items:group.items.map((item,i)=>({...item,review:current,candidate:current.candidates[i]!}))}}
      decisionPending={busy} batchProgress={completed===undefined?undefined:{completed,total:2,failure:busy?undefined:"#3003702 처리 실패"}}
      previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onApplyContainmentBatch={onApply}/>));
    try {
      await render();
      await act(async()=>container.querySelector<HTMLInputElement>('[aria-label="#3003701 제외 선택"]')!.click());
      await act(async()=>container.querySelector<HTMLButtonElement>(".overlap-group-batch button")!.click());
      await render(true,1);
      expect(container.querySelector(".overlap-group")).toHaveAttribute("aria-busy","true");
      expect(container.querySelector(".overlap-group-batch button")).toBeDisabled();
      const refreshed={...review,revision:5,candidates:review.candidates.map((c,i)=>i===0?{...c,decision:"existing_removed" as const}:c)};
      await render(false,1,refreshed);
      expect(container.querySelector(".overlap-group-completed")).not.toHaveAttribute("open");
      expect(container.querySelector(".overlap-group-completed")).toHaveTextContent("이미 처리된 판본 1");
      expect(container.querySelector('[aria-label="#3003700 제외 선택"]')).toBeNull();
      expect(container.querySelector('[aria-label="#3003701 제외 선택"]')).not.toBeChecked();
      expect(container.querySelector('[aria-label="#3003702 제외 선택"]')).toBeChecked();
      expect(container.querySelector(".overlap-group")).toHaveTextContent("#3003702 처리 실패");
      const requests=resolve.mock.calls.map(([r])=>r.key).filter(k=>k.kind==="overlap-review-page");
      expect(requests.some(k=>k.reviewRevision===5)).toBe(true);
      await act(async()=>container.querySelector<HTMLButtonElement>(".overlap-group-batch button")!.click());
      expect(onApply).toHaveBeenLastCalledWith(["review-overlap:contained-3"]);
    } finally {await act(async()=>root.unmount());client.dispose();container.remove();}
  });

  it("explains stale evidence as a read-only recheck instead of another actionable failure", async () => {
    const review = fixture();
    review.state = "stale";
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".review-signal")).toHaveTextContent("보유 목록 재검사 중");
    expect(container.querySelector(".download-overlap-stale-note")).toHaveTextContent("이미 제외된 상대를 정리하고 현재 보유 목록으로 다시 검사 중입니다");
    expect(container.textContent).not.toContain("기존 A 제거");
    expect(container.textContent).toContain("읽기 전용 판정 기록");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("shows the strict recommendation only for an eligible review", async () => {
    const review = fixture();
    review.incoming.pageCount = 25;
    const eligible = review.candidates[1]!;
    eligible.existing.pageCount = 20;
    eligible.matchedPages = 20;
    eligible.exactPages = 20;
    eligible.visualPages = 0;
    eligible.existingCoverage = 1;
    eligible.incomingCoverage = 0.8;
    eligible.existingUniquePages = 0;
    eligible.incomingUniquePages = 5;
    eligible.longestAlignedRun = 20;
    eligible.pagePairs = Array.from({ length: 20 }, (_, index) => ({
      ...eligible.pagePairs[0]!,
      incomingSourcePage: index + 1,
      existingSourcePage: index + 1,
      exactSha256: true,
      lowInformation: false,
    }));
    review.candidates = [eligible];
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        autoMode="recommend"
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    const banner = container.querySelector(".download-overlap-auto-recommendation")!;
    expect(banner).toHaveTextContent("추천");
    expect(banner).toHaveTextContent("신규 B 유지 · 기존 #101 제외 (완전 포함)");
    expect(banner).toHaveTextContent("직접 선택해야 적용됩니다.");
    expect(banner.textContent!.length).toBeLessThan(100);
    expect(banner).not.toHaveTextContent("신뢰도 85%");
    expect(container.textContent).not.toContain("일반 판본: 포함률 95% 이상");

    const help = banner.querySelector<HTMLButtonElement>('[aria-label="자동 판정 기준"]')!;
    expect(help).not.toHaveAttribute("aria-describedby");
    await act(async () => help.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    let tooltip = document.getElementById(help.getAttribute("aria-describedby")!);
    expect(tooltip).toHaveAttribute("role", "tooltip");
    expect(tooltip?.parentElement).toBe(container.querySelector("dialog"));
    expect(tooltip).toHaveTextContent("작은 판본의 모든 페이지 대응");
    expect(tooltip).toHaveTextContent("일반 판본: 포함률 95% 이상");
    expect(tooltip).toHaveTextContent("제목 표식(무검열 > 미표시 > 검열) → 페이지 수 → 기존본");
    expect(banner).not.toContainElement(tooltip);
    await act(async () => help.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
    expect(help).not.toHaveAttribute("aria-describedby");
    await act(async () => help.focus());
    tooltip = document.getElementById(help.getAttribute("aria-describedby")!);
    expect(tooltip).toHaveTextContent("횟수 제한 없이 재검증 후 적용");
    expect(tooltip).toHaveTextContent("영구 삭제하지 않습니다");
    expect(tooltip).not.toHaveTextContent("하루 10건");
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(help).not.toHaveAttribute("aria-describedby");

    const renderWith = async (autoMode: "off" | "recommend" | "strict_quarantine", current = review) => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false} review={current} autoMode={autoMode} previewWidth={220}
          thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()}
        />,
      ));
    };
    await renderWith("strict_quarantine");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toHaveTextContent("자동 정리 예정");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toHaveTextContent("닫으면 재검증 후 적용 · 영구 삭제 없음");
    const defer = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "검토 미루기")!;
    expect(document.getElementById(defer.getAttribute("aria-describedby")!)).toHaveTextContent("자동 모드에서는 자동 기준에 맞는 항목이 이후 처리될 수 있습니다");
    await renderWith("off");
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    for (const state of ["resolved", "cancelled", "stale"] as const) {
      await renderWith("recommend", { ...review, state });
      expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    }
    await renderWith("recommend", { ...review, candidates: [{ ...eligible, confidence: 0.5 }] });
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();
    await renderWith("recommend", { ...review, candidates: [{ ...eligible, decision: "keep_both" }] });
    expect(container.querySelector(".download-overlap-auto-recommendation")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("renders dark-theme-ready vertical A/B summaries and an aligned page lane", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({ kind: "missing" as const, reason: "fixture" }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    const review = fixture();
    review.candidates = [review.candidates[0]!];
    review.incoming.pageCount = 16;
    const active = review.candidates[0]!;
    active.existing.pageCount = 11;
    active.pagePairs = [
      ...Array.from({ length: 8 }, (_, index) => ({
        ...active.pagePairs[0]!,
        existingSourcePage: index + 1,
        incomingSourcePage: index + 1,
      })),
      ...[14, 15, 16].map((incomingSourcePage, index) => ({
        ...active.pagePairs[0]!,
        existingSourcePage: index + 9,
        incomingSourcePage,
      })),
    ];
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(container.textContent).toContain("거의 같은 판본");
    expect(container.textContent).toContain("기존 A 범위");
    expect(container.textContent).toContain("80%");
    expect(container.textContent).toContain("기존 앨범 A");
    expect(container.textContent).toContain("신규 앨범 B");
    expect(container.textContent).toContain("신규 B에만 9~13p · 5장");
    expect(container.querySelector(".download-overlap-artifacts")?.children).toHaveLength(2);
    expect(container.querySelectorAll(".download-overlap-page-cell.is-gap")).toHaveLength(5);
    expect(container.querySelectorAll(".download-overlap-page-cell.is-unique")).toHaveLength(5);
    expect(container.querySelector(".download-overlap-result")).toBeNull();
    expect(container.querySelector(".download-overlap-artifact.is-kept")).toBeNull();
    const artifactPages = resolve.mock.calls
      .map(([request]) => request.key)
      .filter((key) => key.kind === "overlap-review-page")
      .map((key) => [key.reviewId, key.candidateId, key.reviewRevision, key.side, key.page]);
    expect(artifactPages).toEqual(expect.arrayContaining([
      ["review-overlap", "candidate-1", 4, "existing", 1],
      ["review-overlap", "candidate-1", 4, "incoming", 1],
    ]));

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it.each(["resolved", "cancelled", "pending", "stale"] as const)("allows restore but requires approval only for manual %s evidence", async (state) => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = { ...fixture(), state };
    const removedGalleryIds = state === "cancelled"
      ? [review.incoming.galleryId]
      : [review.candidates[0]!.existing.galleryId, review.candidates[2]!.existing.galleryId];
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId,
      incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title,
      occurredAt: review.updatedAt,
      reviewState: state,
      removeIncomingCount: state === "cancelled" ? 1 : 0,
      removeExistingCount: state === "cancelled" ? 0 : 2,
      removedGalleryIds,
    };
    const onRestore = vi.fn();
    const onAcknowledge = vi.fn();
    const onClose = vi.fn();
    const onDecision = vi.fn();
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false} review={review} previewWidth={220} thumbnailClient={client}
        automationHistoryItem={history} onRestoreAutomationExclusions={onRestore}
        onAcknowledgeAutomationHistory={onAcknowledge}
        onClose={onClose} onRetry={vi.fn()} onDecision={onDecision}
      />,
    ));
    const footer = container.querySelector('.download-overlap-history-actions')!;
    const buttons = [...footer.querySelectorAll<HTMLButtonElement>("button")];
    const restore = buttons.find((button) => button.textContent?.includes("복원"))!;
    const acknowledge = buttons.find((button) => button.textContent?.includes("승인"))!;
    expect(restore).toBeEnabled();
    const manual = state === "pending" || state === "stale";
    if (manual) expect(acknowledge).toBeEnabled();
    else expect(acknowledge).toBeUndefined();
    expect(document.getElementById(restore.getAttribute("aria-describedby")!)).toHaveTextContent("격리된 실제 파일은 복원하지 않습니다");
    if (manual) expect(document.getElementById(acknowledge.getAttribute("aria-describedby")!)).toHaveTextContent("앨범·파일은 변경하지 않습니다");
    expect(container.querySelector('.download-overlap-readonly-actions')).toBeNull();
    expect(container.querySelectorAll('button[aria-label="닫기"]')).toHaveLength(1);
    expect([...container.querySelectorAll(".review-actions button")].some((button) => button.textContent === "닫기")).toBe(false);
    expect(Boolean(container.querySelector(".download-overlap-actions"))).toBe(state === "pending");

    // Selecting another comparison must not replace the persisted removal targets.
    await act(async () => container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[1]!.click());
    await act(async () => restore.click());
    await act(async () => acknowledge?.click());
    expect(onRestore).toHaveBeenCalledExactlyOnceWith(review.reviewId, removedGalleryIds);
    if (manual) expect(onAcknowledge).toHaveBeenCalledExactlyOnceWith(review.reviewId);
    else expect(onAcknowledge).not.toHaveBeenCalled();
    expect(onDecision).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("navigates sequential evidence without acknowledging, including loading or failed items", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const previous = vi.fn();
    const next = vi.fn();
    const acknowledge = vi.fn();
    const review = { ...fixture(), state: "resolved" as const };
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId, incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title, occurredAt: review.updatedAt, reviewState: review.state,
      removeIncomingCount: 0, removeExistingCount: 1, removedGalleryIds: [galleryId(100)],
    };
    const render = async (position: number, busy = false, loadState: "ready" | "loading" | "error" = "ready") => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog open review={loadState === "ready" ? review : undefined}
          loading={loadState === "loading"} error={loadState === "error" ? "근거를 불러오지 못했습니다" : null}
          previewWidth={220} thumbnailClient={client} automationHistoryItem={history}
          automationHistoryPending={busy} onAcknowledgeAutomationHistory={acknowledge}
          automationReviewSequence={{ position, total: 3, canPrevious: position > 1, canNext: position < 3 }}
          onPreviousAutomationReview={previous} onNextAutomationReview={next}
          onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} />,
      ));
    };
    await render(2);
    const nav = container.querySelector('nav[aria-label="자동 분류 순차 검토"]')!;
    const previousButton = nav.querySelector<HTMLButtonElement>('[aria-label="이전 자동 분류"]')!;
    const nextButton = nav.querySelector<HTMLButtonElement>('[aria-label="다음 자동 분류"]')!;
    expect(nav).toHaveTextContent("순차 검토 · 2 / 3");
    const acknowledgeButton = [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-history-actions button")]
      .find((button) => button.textContent?.includes("승인"))!;
    expect(acknowledgeButton).toBeUndefined();
    await act(async () => { previousButton.click(); nextButton.click(); });
    expect(previous).toHaveBeenCalledOnce();
    expect(next).toHaveBeenCalledOnce();
    expect(acknowledge).not.toHaveBeenCalled();
    await render(1);
    expect(previousButton).toBeDisabled();
    await render(3);
    expect(nextButton).toBeDisabled();
    await render(2, true);
    expect(previousButton).toBeDisabled();
    expect(nextButton).toBeDisabled();
    for (const state of ["loading", "error"] as const) {
      await render(2, false, state);
      expect(nav).toBeInTheDocument();
      expect(previousButton).toBeEnabled();
      expect(nextButton).toBeEnabled();
      expect(container.querySelector(".download-overlap-history-actions")).toBeNull();
    }
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("guards history actions while busy and hides them for acknowledged or unrelated records", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    const history: DownloadOverlapAutomationHistoryItem = {
      reviewId: review.reviewId, incomingGalleryId: review.incoming.galleryId,
      title: review.incoming.title, occurredAt: review.updatedAt, reviewState: "pending",
      removeIncomingCount: 0, removeExistingCount: 1,
      removedGalleryIds: [review.candidates[0]!.existing.galleryId],
    };
    const onRestore = vi.fn();
    const onAcknowledge = vi.fn();
    const onDecision = vi.fn();
    const render = async (item = history, pending = false, loading = false) => {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false} review={review} previewWidth={220} thumbnailClient={client}
          automationHistoryItem={item} automationHistoryPending={pending} loading={loading}
          onRestoreAutomationExclusions={onRestore} onAcknowledgeAutomationHistory={onAcknowledge}
          onClose={vi.fn()} onRetry={vi.fn()} onDecision={onDecision}
        />,
      ));
    };
    await render(history, true);
    expect(container.querySelector('.download-overlap-history-actions')).toHaveAttribute("aria-busy", "true");
    expect(container.querySelector('.download-overlap-history-status')).toHaveTextContent("처리 중");
    const blocked = [...container.querySelectorAll<HTMLButtonElement>(".download-overlap-history-actions button, .download-overlap-actions .danger-button")];
    for (const button of blocked) {
      expect(button).toBeDisabled();
      await act(async () => button.click());
    }
    expect(onRestore).not.toHaveBeenCalled();
    expect(onAcknowledge).not.toHaveBeenCalled();
    expect(onDecision).not.toHaveBeenCalled();
    await render(history, false, true);
    for (const button of container.querySelectorAll(".download-overlap-history-actions button")) expect(button).toBeDisabled();
    await render({ ...history, acknowledgedAt: "2026-09-10T00:00:00Z" });
    expect(container.querySelector('.download-overlap-history-actions')).toHaveTextContent("승인 완료");
    expect(container.querySelectorAll(".download-overlap-history-actions button")).toHaveLength(0);
    await render({ ...history, removedGalleryIds: [] });
    const onlyAction = container.querySelectorAll(".download-overlap-history-actions button");
    expect(onlyAction).toHaveLength(1);
    expect(onlyAction[0]).toHaveTextContent("승인");
    await render({ ...history, reviewId: "different-review" });
    expect(container.querySelector('.download-overlap-history-actions')).toBeNull();
    await render({ ...history, incomingGalleryId: galleryId(999) });
    expect(container.querySelector('.download-overlap-history-actions')).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("makes the surviving edition unmistakable in processed multi-candidate evidence", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.incoming.title = "New edition [Decensored]";
    review.state = "resolved";
    review.resolvedAt = "2026-08-25T00:02:00.000Z";
    review.candidates = review.candidates.slice(0, 2).map((candidate, index) => ({
      ...candidate,
      decision: index === 0 ? "existing_removed" as const : "keep_both" as const,
    }));
    review.decisions = [{
      candidateId: review.candidates[0]!.candidateId,
      action: "remove_existing_continue",
      actor: "automation",
      reasonCode: "balanced_overlap_v3",
      ruleVersion: 3,
      featureSnapshotJson: JSON.stringify({
        candidateId: review.candidates[0]!.candidateId,
        incomingGalleryId: review.incoming.galleryId,
        existingGalleryId: review.candidates[0]!.existing.galleryId,
        winner: "incoming",
        preferenceReason: "uncensored",
        metrics: { pageDifference: 2 },
      }),
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    const tabs = [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs[0]).toHaveTextContent("A 제외");
    expect(tabs[1]).toHaveTextContent("둘 다 유지");
    expect(container.querySelector(".download-overlap-result")).toBeNull();
    let artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-excluded");
    expect(artifacts[0]).toHaveTextContent("이 검토에서 제외");
    expect(artifacts[0]).toHaveTextContent("상대 판본의 무검열 표식 우선");
    expect(artifacts[1]).toHaveClass("is-kept");
    expect(artifacts[1]).toHaveTextContent("이 검토에서 보존");
    expect(artifacts[1]).toHaveTextContent("무검열 표식 우선 · 신뢰도 94%");
    expect(container.querySelector(".download-overlap-readonly-actions")).toHaveTextContent("읽기 전용 판정 기록");
    expect(container.querySelector(".download-overlap-readonly-actions")).toHaveTextContent("이 창에서는 판정을 바꾸거나 제외를 복구하지 않습니다");

    await act(async () => tabs[1]!.click());

    artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-kept");
    expect(artifacts[1]).toHaveClass("is-kept");
    expect(artifacts[0]).toHaveTextContent("둘 다 보존 선택");
    expect(artifacts[1]).toHaveTextContent("둘 다 보존 선택");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("shows existing A as the survivor when incoming B was cancelled", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.state = "cancelled";
    review.resolvedAt = "2026-08-25T00:02:00.000Z";
    review.candidates = [review.candidates[0]!];
    review.decisions = [{
      action: "remove_incoming",
      actor: "human",
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".download-overlap-result")).toBeNull();
    const artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
    expect(artifacts[0]).toHaveClass("is-kept");
    expect(artifacts[0]).toHaveAttribute("aria-label", expect.stringContaining("이 검토에서 보존"));
    expect(artifacts[0]).toHaveTextContent("신규 B 제거 선택");
    expect(artifacts[1]).toHaveClass("is-excluded");
    expect(artifacts[1]).toHaveAttribute("aria-label", expect.stringContaining("이 검토에서 제외"));
    expect(artifacts[1]).toHaveTextContent("수동 제거 선택");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it.each(["keep_both", "false_positive"] as const)(
    "shows a later incoming removal as the final result after %s",
    async (firstDecision) => {
      const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      const review = fixture();
      review.state = "cancelled";
      review.resolvedAt = "2026-08-25T00:03:00.000Z";
      review.candidates = review.candidates.slice(0, 2).map((candidate, index) => index === 0
        ? { ...candidate, decision: firstDecision }
        : candidate);
      review.decisions = [{
        candidateId: review.candidates[0]!.candidateId,
        action: firstDecision === "keep_both" ? "keep_both_continue" : "false_positive_continue",
        actor: "human",
        createdAt: "2026-08-25T00:01:00.000Z",
      }, {
        candidateId: review.candidates[1]!.candidateId,
        action: "remove_incoming",
        actor: "human",
        createdAt: "2026-08-25T00:02:00.000Z",
      }];

      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false}
          review={review}
          previewWidth={220}
          thumbnailClient={client}
          onClose={vi.fn()}
          onRetry={vi.fn()}
          onDecision={vi.fn()}
        />,
      ));

      const firstTab = container.querySelectorAll<HTMLButtonElement>('[role="tab"]')[0]!;
      expect(firstTab).toHaveTextContent("B 제외");
      await act(async () => firstTab.click());

      const artifacts = [...container.querySelectorAll<HTMLElement>(".download-overlap-artifact")];
      expect(artifacts[0]).toHaveClass("is-kept");
      expect(artifacts[0]).toHaveTextContent("신규 B 최종 제외로 유지");
      expect(artifacts[1]).toHaveClass("is-excluded");
      expect(artifacts[1]).toHaveTextContent("다른 후보 판정으로 신규 B 최종 제외");
      expect(container.textContent).not.toContain("둘 다 보존 선택");
      expect(container.textContent).not.toContain("오탐 판정 · 둘 다 보존");

      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    },
  );

  it("keeps durable candidate results visible while a multi-candidate review continues", async () => {
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const review = fixture();
    review.candidates = review.candidates.slice(0, 2).map((candidate, index) => index === 0
      ? { ...candidate, decision: "existing_removed" as const }
      : candidate);
    review.decisions = [{
      candidateId: review.candidates[0]!.candidateId,
      action: "remove_existing_continue",
      actor: "human",
      createdAt: "2026-08-25T00:01:00.000Z",
    }];

    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={review}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={vi.fn()}
      />,
    ));

    expect(container.querySelector(".overlap-group-completed")).toHaveTextContent("이미 처리된 판본 1");
    expect(container.querySelector(".overlap-group-completed")).not.toHaveAttribute("open");
    const completedRow = container.querySelector('[data-edition-id="100"]')!;
    expect(completedRow).toHaveTextContent("제외 완료");
    expect(completedRow.querySelector("input")).toBeNull();
    await act(async () => completedRow.querySelector<HTMLButtonElement>("button")!.click());
    expect(container.querySelectorAll(".overlap-group-row-head")).toHaveLength(3);
    expect(container.querySelector(".overlap-group-actions")).toBeNull();
    expect(container.textContent).not.toContain("A 제외 · 계속 검토");
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("submits five revision-checked actions with candidate scope and removal confirmation", async () => {
    const onDecision = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <DownloadOverlapReviewDialog
        open={false}
        review={(() => { const r=fixture(); return {...r,candidates:[r.candidates[0]!]}; })()}
        previewWidth={220}
        thumbnailClient={client}
        onClose={vi.fn()}
        onRetry={vi.fn()}
        onDecision={onDecision}
      />,
    ));
    const click = async (label: string) => {
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes(label));
      if (!button) throw new Error(`${label} button missing`);
      await act(async () => button.click());
    };
    expect(container.querySelectorAll('[role="tooltip"]')).toHaveLength(5);
    const actionButtons = ["검토 미루기", "기존 A 제거", "신규 B 제거", "오탐 판정", "둘 다 보존"]
      .map((label) => [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === label));
    expect(actionButtons.every((button) => Boolean(button?.getAttribute("aria-describedby")))).toBe(true);
    await click("기존 A 제거");
    await click("신규 B 제거");
    await click("오탐 판정");
    await click("둘 다 보존");
    await click("검토 미루기");
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "remove_existing_continue",
      candidateId: "candidate-1",
    });
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "remove_incoming",
    });
    expect(onDecision).toHaveBeenCalledWith({
      reviewId: "review-overlap",
      expectedRevision: 4,
      action: "false_positive_continue",
      candidateId: "candidate-1",
    });
    expect(onDecision).toHaveBeenCalledWith(expect.objectContaining({
      action: "keep_both_continue",
      candidateId: "candidate-1",
    }));
    expect(container.querySelector(".download-overlap-action-note")).toHaveTextContent("현재 A/B에만 적용");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
    confirm.mockRestore();
  });

  it("enlarges aligned pages on hover using the configured card preview width without leaving the dialog", async () => {
    const resolve = vi.fn((_request: ThumbnailRequest) => ({
      kind: "image" as const,
      url: "data:image/png;base64,fixture",
      width: 800,
      height: 1200,
    }));
    const client = new ThumbnailClient({ resolve });
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.matches("dialog.download-overlap-dialog")) {
        return { x: 20, y: 20, width: 500, height: 700, top: 20, right: 520, bottom: 720, left: 20, toJSON: () => ({}) };
      }
      if (this.classList.contains("download-overlap-page-cell")) {
        return { x: 425, y: 590, width: 82, height: 112, top: 590, right: 507, bottom: 702, left: 425, toJSON: () => ({}) };
      }
      return { x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0, toJSON: () => ({}) };
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadOverlapReviewDialog
          open={false}
          review={(() => { const r=fixture(); return {...r,candidates:[r.candidates[0]!]}; })()}
          previewWidth={280}
          thumbnailClient={client}
          onClose={vi.fn()}
          onRetry={vi.fn()}
          onDecision={vi.fn()}
        />,
      ));
      const dialog = container.querySelector<HTMLDialogElement>(".download-overlap-dialog")!;
      const page = container.querySelector<HTMLElement>(".download-overlap-page-cell:not(.is-gap)")!;

      await act(async () => page.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

      const enlarged = dialog.querySelector<HTMLElement>(".download-overlap-page-hover-preview")!;
      expect(enlarged).not.toBeNull();
      expect(enlarged.parentElement).toBe(dialog);
      expect(enlarged).toHaveAttribute("data-preview-width", "280");
      expect(enlarged.style.position).toBe("fixed");
      expect(enlarged.style.width).toBe("280px");
      expect(enlarged.style.height).toBe("420px");
      expect(enlarged.textContent).toContain("기존 A 1p · 시각 93%");
      expect(Number.parseFloat(enlarged.style.left) + Number.parseFloat(enlarged.style.width)).toBeLessThanOrEqual(510);
      expect(Number.parseFloat(enlarged.style.top) + Number.parseFloat(enlarged.style.height)).toBeLessThanOrEqual(710);

      await act(async () => page.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
      expect(dialog.querySelector(".download-overlap-page-hover-preview")).toBeNull();

      const gap = container.querySelector<HTMLElement>(".download-overlap-page-cell.is-gap")!;
      await act(async () => gap.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(dialog.querySelector(".download-overlap-page-hover-preview")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
      rect.mockRestore();
    }
  });
});

it("preserves selected pages when unordered evidence arrives and blocks a merge that would duplicate them",async()=>{
 const review=pageMergeFixture();review.incoming={...review.incoming,pageCount:3};
 const base=review.candidates[0]!,pair=base.pagePairs[0]!;
 review.candidates=[0,1].map(n=>({...base,candidateId:"reorder-"+n,
  existing:{...base.existing,entryId:"reorder-"+n,galleryId:galleryId(100+n),pageCount:3},
  existingFingerprint:"reorder-fp-"+n,relation:"partial_overlap" as const,
  matchedPages:2,exactPages:2,visualPages:0,existingUniquePages:1,incomingUniquePages:1,
  existingCoverage:2/3,incomingCoverage:2/3,longestAlignedRun:2,
  pagePairs:[{...pair,incomingSourcePage:1,existingSourcePage:2,exactSha256:true},
   {...pair,incomingSourcePage:2,existingSourcePage:3,exactSha256:true}]}));
 let complete!:()=>void;
 const ready=new Promise<void>(resolve=>{complete=resolve;});
 const loader=vi.fn(async(request:{candidateId:string})=>{
  await ready;const c=review.candidates.find(c=>c.candidateId===request.candidateId)!;
  return {ok:true as const,data:{reviewId:review.reviewId,revision:review.revision,candidateId:c.candidateId,
   incomingFingerprint:review.incomingFingerprint,existingFingerprint:c.existingFingerprint,
   additionalPairs:[{...pair,incomingSourcePage:3,existingSourcePage:1,exactSha256:true,visualSimilarity:1}]}};
 });
 const client=new ThumbnailClient({resolve:()=>({kind:"missing",reason:"fixture"})});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host),onMerge=vi.fn();
 const page=(g:number,p:number)=>host.querySelector<HTMLElement>('[aria-label^="#'+g+' '+p+'페이지"]')!;
 try{
  await act(async()=>root.render(<DownloadOverlapReviewDialog open review={review} previewWidth={220}
   thumbnailClient={client} loadPageEvidence={loader} onClose={vi.fn()} onRetry={vi.fn()} onDecision={vi.fn()} onMergePages={onMerge}/>));
  await act(async()=>page(100,1).dispatchEvent(new MouseEvent("click",{bubbles:true,button:0,ctrlKey:true})));
  expect(page(100,1)).toHaveAttribute("data-merge-state","source");
  expect(page(100,1)).toHaveTextContent("확인 중");
  await act(async()=>complete());
  expect(page(100,1)).toHaveAttribute("data-merge-state","source");
  expect(page(100,1)).toHaveTextContent("일치");
  expect(page(200,3)).toHaveTextContent("일치");
  expect(page(100,1).querySelector('[aria-label="순서 다름"]')).not.toBeNull();
  const merge=[...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="적용")!;
  await act(async()=>merge.click());
  expect(onMerge).not.toHaveBeenCalled();
  expect(host).toHaveTextContent("순서가 다른 선택 페이지는 병합할 수 없습니다. 선택은 유지됩니다.");
  expect(page(100,1)).toHaveAttribute("data-merge-state","source");
  const active=host.querySelector<HTMLButtonElement>('[aria-label="#101 페이지 선택 활성화"]')!;
  await act(async()=>active.click());
  expect(page(200,3)).toHaveTextContent("일치");
  expect(page(100,1)).toHaveAttribute("data-merge-state","source");
 }finally{await act(async()=>root.unmount());client.dispose();host.remove();}
});
