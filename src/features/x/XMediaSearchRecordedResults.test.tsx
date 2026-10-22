import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { XMediaSearchPanel } from "./XMediaSearchPanel";
import { isXSearchSnapshot, type XMediaSearchCandidate, type XSearchSnapshot } from "./xMediaSearchApi";

// Only the transport is mocked: the production API validators/event filter and
// component both run. These are captured provider DTOs, not invented matches.
// This is NOT live native-controller/X-media/provider end-to-end acceptance.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

// Approved neutral-shapes.jpg, SHA256
// 4D2FEB4CF3F6A87DBF830A583B43108A21A25E246AC14DCDA8C68A2C110390FF.
// Copied exactly from outcome.Ok of lens-persistent-probe-1792619668276.json
// (SHA256 411B9AFB2D4597AF9ADE19A79882C1A539CA55A2A7EE0A8FE9D126F3630C6CCE).
// Opaque observed Google redirect URLs are retained, not decoded or guessed.
const LENS: XMediaSearchCandidate[] = [
  { title: "색에 대해-3 (Two Point Color 기법) - 네이버 블로그 - NAVER", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESuQEB6zswFZ-9uUCPC8wXZa1tQB5OFUSMGFuzhOVT1GsLAJpH2u4oB-JKAbbN0IVjL7zqXPvVyz4Xj5aCNS-hz6AsxWaQj1BXWu2sDAVRNjHins47VUeasRVfAAIC5ncFQU2_Xr6DFnsgysCEWIX9lKkufBbd8j_5VxzzEOKwoOYLbyDqPWq3k3MhGp6mVERPtcMFy0tLKoapfQEwi8pFhPKKH66I921uk18ot1rWWZ9Er052DMRxlukB2w" }] },
  { title: "AI, 머신러닝, 딥러닝 이란? 그리고 딥러닝 모델 종류", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESXgHrOzAVNkwkyP5M9a0m-KCNCkdAdtWp1W4jZlZbwMQeIMiUDRhmMwvVmbYlk3dXfq8EUChLrL3eck-vh6FKCkzZ0Q8fOu67GOJcs4o3wjXvAiq9zts1kL2EIedDecc" }] },
  { title: "머신러닝과 딥러닝(알고리즘 종류) - 끄적거림", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAES4AEB6zswFUvpxcb4dvY4YowkXWF3MIfPKR8GAUsEBZIZXBNktBGuYMfLb-0Xeg0LuS3UT5ppYyyg-4KZNp4iAr55yoy4FrucUb4huaJIGUI2hJxVuTLapHYEIXSqvHd_42OT2OC6EflWH08V1kozJU4ld8PMMFzlpulRGJ-4ImnTiyWj1K1MbgGEDuKTNTBIL2naVY9eOYF1c0NR-5Q8Kkp0joHFHR6qGgwQwIHg2DrWhIV9FAr7XuAYvrmHK9dXCaXUXZA7_WlWPToFwlbjesTi8XAsMwS-39avyh-bU_AdsQ" }] },
  { title: "인공지능,머신러닝,딥러닝,빅데이터 개념 및 공부 방향", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESWwHrOzAVgjdhX8XCuVyOgta4OAJUcvLIROQy3XXzhX0DtWmp_MsR48KIetzTTmgV0uHYrA8CVeVuK7iIbNSodPYfhzqIyljJtQZZk48sAl7M-sjQdgOgSLWU2NQ" }] },
  { title: "[ML] 머신러닝 기초 (1) 머신러닝 소개", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESXAHrOzAVdOFevh57ICR5MIECwdxqQegNUwsFCKDSTyArCtuYIVhwkgjuvisR32G6p_a_5CJrV069nwiGwyRI9Ie2k8KEEplbhrDEet5I_kyXm0Kve6UJGRgW8A26" }] },
  { title: "딥러닝(Deep Learning) 이란 | 블로그", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESZgHrOzAVhrCF2iR_rey5zH8y8SM2m4jcakvO2AGE1QMngmbZtfmMAhEMNuMjgw3iKP7ccT6MdO_WnA4y2pYPCkoZ8jV69v1abWGy7A-OM47UtJH-jpBLDjpfv0pgcH7otmznIC4o7g" }] },
  { title: "[트랜D]\"오늘은 어떤 색의 옷을 입을까\"…색상도 골라주는 ...", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESZgHrOzAVOo_03A9zMRC450C_qDVJ2WMKhme4E0yV-u1Y2nb5EGIgUcFSuObT8wZG_XFPPHLUNotsRufPHHHgNvZM1AR7Vi_Y9oLJ1okRPx5ZPqd_dSyGwG59IVZVqFtdw1jAobssBQ" }] },
  { title: "ChatGPT 기술 분석 백서 - 1부 ChatGPT란 | 인사이트리포트", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESegHrOzAVAQSRSH3tnmOoDJT1rgDGO_omTO1UTCZvGFBfhcAtg7ywBXR1Qoh-u8NxdMoP6kE8YmFQiuPyuTsdPErtQWjWmGQ5BvxodIAOOBFm9L48NO_p9e1Y9fzpQPz14KkyTa2Pt-GttM9d6_9orSYsX9t3Xqs0sm8I" }] },
  { title: "색조 효과 - Win32 apps", creator: null, similarity: null, links: [{ label: "www.google.com", url: "https://www.google.com/goto?url=CAESfwHrOzAVRXRwWSmZjSJaFwMIf7OUreg3Sy4fJ7URPZPRJqZuh-r5ETw949jvN4SS46f39o7gK2cW_XWXiQ4WfkUQtxDx9noxZ-Du-tXq0jL0sHD3WpJ2uQqGdhxlJlu6R5YfzLCXKsgH0YPOiGiLvn38nLWWxRzUuS40IpJ0QgA" }] },
];

// outcome.Ok of saucenao-persistent-probe-1792620130992.json
// SHA256 9B65C9B404049FF16C6FC5403A842502A891F250A2C3E9AE9D986B9024D57448.
// This proof used a synthetic multipart file, not production pbs URL retrieval.
const SAUCENAO: XMediaSearchCandidate[] = [
  { title: "【ＧＩＦ】エレベーター争奪戦", creator: "noverun", similarity: 52.56, links: [
    { label: "3596411", url: "https://seiga.nicovideo.jp/seiga/im3596411" },
    { label: "noverun", url: "https://seiga.nicovideo.jp/user/illust/11493614" },
  ] },
  { title: "Coffee table and gadgets", creator: "Han Yang", similarity: 52.25, links: [
    { label: "9el0Eq", url: "https://www.artstation.com/artwork/9el0Eq" },
    { label: "Han Yang", url: "https://www.artstation.com/hanyang1" },
  ] },
];
const FIXTURES = [
  { provider: "lens" as const, candidates: LENS },
  { provider: "saucenao" as const, candidates: SAUCENAO },
];
type Fixture = typeof FIXTURES[number];
const ID = "b48d2ec8-cf91-4ea3-b0ab-a11f81a5f027";
function snapshot(fixture: Fixture, change: Partial<XSearchSnapshot> = {}): XSearchSnapshot {
  // These source/context fields are test scaffolding, not captured X evidence.
  // The URL is never fetched and the JPEG header is never sent to a provider.
  return {
    id: ID, generation: 7, accountId: "legacy", feed: "foryou", pane: "feed",
    kind: fixture.provider === "lens" ? "video" : "image", provider: fixture.provider,
    phase: "complete", imageUrl: fixture.provider === "saucenao" ? "https://pbs.twimg.com/media/NeutralFixture?format=jpg&name=orig" : null,
    previewData: fixture.provider === "lens" ? "data:image/jpeg;base64,/9j/2Q==" : null,
    message: "검색 후보입니다. 유사도가 높아도 원작자나 원본임을 보장하지 않습니다.",
    candidates: structuredClone(fixture.candidates), ...change,
  };
}

let host: HTMLDivElement, root: Root;
const network = vi.fn(() => Promise.reject(new Error("Replay must not request the network")));
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("fetch", network);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.mocked(listen).mockResolvedValue(vi.fn());
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  expect(network).not.toHaveBeenCalled();
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function render(initial: XSearchSnapshot) {
  vi.mocked(invoke).mockImplementation(async command => command === "x_media_search_status" ? initial : undefined);
  await act(async () => root.render(<XMediaSearchPanel runtime="tauri" active privacyMode={false}
    accountId="legacy" generation={7} feed="foryou" onNotice={vi.fn()}/>));
}
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
async function emit(payload: unknown) {
  expect(listen).toHaveBeenCalledWith("x:media-search", expect.any(Function));
  const receive = vi.mocked(listen).mock.calls[0]![1] as (event: { payload: unknown }) => void;
  await act(async () => receive({ payload }));
}
function expectCandidates(fixture: Fixture) {
  const rows = [...dialog()!.querySelectorAll<HTMLLIElement>(".x-media-search-candidates > li")];
  expect(rows).toHaveLength(fixture.candidates.length);
  fixture.candidates.forEach((candidate, index) => {
    const row = rows[index]!;
    expect(row.querySelector("h3")?.textContent).toBe(candidate.title);
    if (candidate.creator === null) expect(row).not.toHaveTextContent("작가 후보");
    else expect(row).toHaveTextContent(`작가 후보: ${candidate.creator}`);
    if (candidate.similarity === null) expect(row).not.toHaveTextContent("유사도");
    else expect(row).toHaveTextContent(`유사도 ${candidate.similarity.toFixed(1)}%`);
    const links = [...row.querySelectorAll<HTMLButtonElement>(".x-media-search-links button")];
    expect(links).toHaveLength(candidate.links.length);
    candidate.links.forEach((link, linkIndex) => {
      expect(links[linkIndex]).toHaveAttribute("title", link.url);
      expect(links[linkIndex]?.querySelector("span")?.textContent).toBe(link.label);
      expect(links[linkIndex]?.querySelector("small")?.textContent).toBe(new URL(link.url).hostname);
    });
  });
  expect(dialog()).toHaveTextContent("원작자임을 보증하지 않습니다");
  expect(dialog()?.querySelector("a,iframe,video,img,script")).toBeNull();
}

it.each(FIXTURES)("accepts and displays all actual $provider neutral-fixture results through production status validation", async fixture => {
  const value = snapshot(fixture);
  expect(isXSearchSnapshot(value)).toBe(true);
  await render(value); expectCandidates(fixture);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("x_media_search_status");
});

it.each(FIXTURES)("delivers actual $provider candidates through the production event filter and opens only native indices", async fixture => {
  await render(snapshot(fixture, { phase: "searching", message: "검색 중…", candidates: [] }));
  expect(dialog()).toHaveTextContent("검색 중");
  await emit(snapshot(fixture)); expectCandidates(fixture);
  const candidateIndex = fixture.candidates.length - 1;
  const linkIndex = fixture.candidates[candidateIndex]!.links.length - 1;
  const row = dialog()!.querySelectorAll("li")[candidateIndex]!;
  await act(async () => row.querySelectorAll<HTMLButtonElement>(".x-media-search-links button")[linkIndex]!.click());
  expect(invoke).toHaveBeenLastCalledWith("x_media_search_open_result", { id: ID, candidateIndex, linkIndex });
  expect(invoke).toHaveBeenCalledTimes(2);
  expectCandidates(fixture);
});

it.each(FIXTURES)("closes actual $provider results once and rejects a delayed replay without sending another search", async fixture => {
  await render(snapshot(fixture));
  await act(async () => dialog()!.querySelector<HTMLButtonElement>('[aria-label="검색 창 닫기"]')!.click());
  expect(dialog()).toBeNull();
  expect(invoke).toHaveBeenLastCalledWith("x_media_search_cancel", { id: ID });
  await emit(snapshot(fixture)); expect(dialog()).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(2);
});

it.each(["unsafe-link", "extra-field"])("filters a corrupted recorded result (%s) before replacing the searching UI", async corruption => {
  const value = snapshot(FIXTURES[1]!);
  if (corruption === "unsafe-link") value.candidates[0]!.links[0]!.url = "javascript:alert(1)";
  const received: unknown = corruption === "extra-field" ? { ...value, privateHtml: "not part of the DTO" } : value;
  expect(isXSearchSnapshot(received)).toBe(false);
  await render(snapshot(FIXTURES[1]!, { phase: "searching", candidates: [], message: "검색 중…" }));
  await emit(received);
  expect(dialog()).toHaveTextContent("검색 중");
  expect(dialog()?.querySelector("li")).toBeNull();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("x_media_search_status");
});

// These terminal states are deliberately synthetic; the captured live probes
// returned candidates. They test UI classification, not real no-match/challenge.
it.each(FIXTURES.flatMap(fixture => [
  { fixture, phase: "complete" as const, message: "검색 서비스에서 일치 후보를 찾지 못했습니다." },
  { fixture, phase: "failed" as const, message: "검색 결과의 화면 구조를 확인하지 못했습니다. 검색 완료로 처리하지 않았습니다." },
]))("keeps synthetic $phase distinct from the captured $fixture.provider successful results", async ({ fixture, phase, message }) => {
  await render(snapshot(fixture, { phase: "searching", candidates: [], message: "검색 중…" }));
  const value = snapshot(fixture, { phase, candidates: [], message,
    ...(phase === "failed" ? { imageUrl: null, previewData: null } : {}),
  });
  expect(isXSearchSnapshot(value)).toBe(true); await emit(value);
  expect(dialog()?.querySelector(`[role="${phase === "failed" ? "alert" : "status"}"]`)).toHaveTextContent(message);
  expect(dialog()?.querySelector("li,.x-media-search-links")).toBeNull();
  expect(dialog()).not.toHaveTextContent("작가 후보");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("x_media_search_status");
});
