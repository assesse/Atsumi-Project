import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserRecording } from "../../api/officialBrowser";
import { RecordingLibrary, recordingChatNote } from "./RecordingLibrary";
import { groupRecordings } from "./recordingGroups";

const token = "a".repeat(32);
const expansionStorageKey = "atsumi.recording-library.expanded.v1";
const item = (id: string, patch: Partial<BrowserRecording> = {}): BrowserRecording => ({ id, channelId: `channel-${id}`, title: `방송 ${id}`, startedAt: 1800000000000, updatedAt: 1800000001000, status: "stopped", mimeType: "video/webm", outputDir: "C:\\SyntheticOnly", segmentCount: 2, bytesWritten: 2000, durationSeconds: 30, lastError: null, segments: [{ index: 0, file: "segment-000000000000.webm", bytes: 1000, durationSeconds: 15 }], ...patch });
const merged = (id: string) => item(id, { merge: { status: "complete", segmentCount: 2, updatedAt: 1, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 1500, durationSeconds: 30 } });
let root: Root, container: HTMLDivElement;
const callbacks = { onSelect: vi.fn(), onFolder: vi.fn(), onReplay: vi.fn(), onOpenMerged: vi.fn(), onRetryMerge: vi.fn(), onOpenSegment: vi.fn(), onDelete: vi.fn<Parameters<typeof RecordingLibrary>[0]["onDelete"]>() };
// File-action regressions retain explicit expansion preferences from earlier sessions.
// The new closed-by-default layout is exercised independently below.
const seedExpandedFiles = (recordings: BrowserRecording[]) => {
  try { if (localStorage.getItem(expansionStorageKey) === null) localStorage.setItem(expansionStorageKey,
    JSON.stringify(Object.fromEntries(groupRecordings(recordings).filter(group => group.recordings.length === 1).map(group => [group.key, true])))); } catch {}
};
const render = async (recordings: BrowserRecording[], selectedId: string | null = null, privacy = false, disabled = false) => {
  seedExpandedFiles(recordings);
  await act(async () => root.render(<RecordingLibrary recordings={recordings} selectedId={selectedId} privacy={privacy} disabled={disabled} retrying={false} openingFolder={false} {...callbacks} />));
};
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(element => element.textContent === text)!;
const cards = () => [...container.querySelectorAll<HTMLButtonElement>(".recording-library-card")];
const select = async (id: string, modifiers: MouseEventInit = { ctrlKey: true }) => act(async () => container.querySelector<HTMLButtonElement>(`[data-recording-id="${id}"]`)!.dispatchEvent(new MouseEvent("click", { bubbles: true, ...modifiers })));
const selectAll = async () => act(async () => container.querySelector('.recording-library')!.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'a', ctrlKey: true })));
const deleteSelection = () => button("선택 삭제") ?? button("녹화 삭제");
const search = async (value: string) => { await act(async () => { const input = container.querySelector<HTMLInputElement>('input[type="search"]')!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); }); };
beforeEach(() => { vi.clearAllMocks(); callbacks.onRetryMerge.mockReset(); localStorage.removeItem("atsumi.recording-library.retry-batch.v1.chzzk"); localStorage.removeItem(expansionStorageKey); callbacks.onDelete.mockResolvedValue({ deletedIds: [], failures: [] }); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); HTMLDialogElement.prototype.showModal = function () { this.open = true; }; HTMLDialogElement.prototype.close = function () { this.open = false; }; container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.removeItem(expansionStorageKey); });

describe("recording library", () => {
  it("starts folded and gives single and multi-file groups the same exclusive overlay interaction", async () => {
    const records = [merged("single"), ...["a", "b"].map(id => ({ ...merged(id), channelId: "shared", broadcastKey: "id:42" }))];
    await act(async () => root.render(<RecordingLibrary recordings={records} selectedId={null} privacy={false} disabled={false} retrying={false} openingFolder={false} {...callbacks} />));
    const headings = [...container.querySelectorAll<HTMLButtonElement>('.recording-broadcast-heading')];
    expect(cards()).toHaveLength(0);
    await act(async () => headings[0]!.click()); expect(cards().map(card => card.dataset.recordingId)).toEqual(["single"]);
    await act(async () => headings[1]!.click()); expect(cards().map(card => card.dataset.recordingId)).toEqual(["a", "b"]);
    expect(headings[0]).toHaveAttribute("aria-expanded", "false");
    await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }))); expect(cards()).toHaveLength(0);
    await act(async () => headings[0]!.click());
    await act(async () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))); expect(cards()).toHaveLength(0);
    expect(callbacks.onDelete).not.toHaveBeenCalled(); expect(callbacks.onReplay).not.toHaveBeenCalled();
  });
  it("Ctrl selects folded groups, preserves previous groups, and toggles mixed groups without opening them", async () => {
    const records = [...["a", "b"].map(id => ({ ...merged(id), channelId: "one", broadcastKey: "id:1" })), merged("c"), item("live", { status: "recording" })];
    localStorage.setItem(expansionStorageKey, "{}"); await render(records);
    const headings = [...container.querySelectorAll<HTMLButtonElement>('.recording-library-grid .recording-broadcast-heading')];
    await act(async () => headings[0]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(cards()).toHaveLength(0); expect(container).toHaveTextContent("2개 선택");
    await act(async () => headings[1]!.dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true })));
    expect(cards()).toHaveLength(0); expect(container).toHaveTextContent("3개 선택");
    await act(async () => headings[0]!.click()); await select("a");
    expect(container.querySelector('.recording-library-grid .recording-broadcast-group')).toHaveAttribute("data-selection", "some");
    await act(async () => headings[1]!.click()); expect(container).toHaveTextContent("2개 선택");
    await act(async () => headings[0]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container).toHaveTextContent("3개 선택"); expect(headings[0]).toHaveAttribute("aria-expanded", "false");
    await act(async () => headings[0]!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container).toHaveTextContent("1개 선택");
    expect(callbacks.onDelete).not.toHaveBeenCalled(); expect(callbacks.onReplay).not.toHaveBeenCalled();
  });
  it("retry-all includes offscreen failures and tracks actual queue, phase and completion without blocking selection", async () => {
    let done!: (value: boolean) => void;
    callbacks.onRetryMerge.mockReturnValueOnce(new Promise<boolean>(resolve => { done = resolve; })).mockResolvedValueOnce(true);
    const a = item("a", { merge: { status: "failed", segmentCount: 2, updatedAt: 1, lastError: "space" } });
    const b = item("b");
    const history = { ...merged("old-gap"), status: "interrupted" as const, lastError: "capture gap" };
    localStorage.setItem(expansionStorageKey, "{}");
    const records = [a, ...Array.from({ length: 25 }, (_, i) => merged("ok-" + i)), b, history, item("live", { status: "recording", progressive: { partCount: 1, segmentCount: 1, durationSeconds: 1, lastError: "failed writer" } }), item("queued", { merge: { status: "queued", segmentCount: 2, updatedAt: 2 } })];
    await render(records);
    const all = container.querySelector<HTMLButtonElement>('.recording-library-retry-all')!;
    expect(all).toHaveTextContent("(2)"); await act(async () => { all.click(); all.click(); });
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("a");
    expect(container.querySelector('input[type=search]')).toBeEnabled();
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-library-grid .recording-broadcast-heading')!.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(container).toHaveTextContent("1개 선택");
    await act(async () => done(true)); expect(callbacks.onRetryMerge.mock.calls.map(call => call[0])).toEqual(["a", "b"]);
    await render(records.map(record => record.id === "a" ? { ...record, merge: { status: "merging", segmentCount: 2, updatedAt: 3 }, workProgress: { phase: "merging", completed: 40, total: 100, unit: "seconds", updatedAt: 4 } } : record.id === "b" ? { ...record, merge: { status: "queued", segmentCount: 2, updatedAt: 3 } } : record));
    expect(container).toHaveTextContent("완료 0/2 · 진행 1 · 대기 1"); expect(container.querySelector('.recording-library-retry-progress')).toHaveTextContent("병합 40%");
    await render(records.map(record => record.id === "a" ? merged("a") : record.id === "b" ? { ...record, merge: { status: "failed", segmentCount: 2, updatedAt: 5, lastError: "still no space" } } : record));
    expect(container.querySelector('.recording-library-retry-progress')).toHaveTextContent("재처리 결과완료 1/2 · 실패 1");
    expect(container.querySelector('.recording-library-retry-progress progress')).toHaveAttribute("value", "2");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("shows the verified file duration instead of summing overlapping capture packets", async () => {
    await render([{ ...merged("a"), durationSeconds: 32.4 }], "a");
    expect(container.querySelector(".recording-library-length")).toHaveTextContent("00:00:30");
    expect(container.querySelector(".recording-library-detail")).toHaveTextContent("00:00:30");
    expect(container.querySelector(".recording-library-detail")).not.toHaveTextContent("00:00:32");
  });
  it("restores each broadcast's last open or closed state after leaving the library", async () => {
    const records = [merged("a"), merged("b"), ...["c", "d"].map(id => ({ ...merged(id), channelId: "shared", broadcastKey: "id:42" }))];
    await render(records);
    const headings = [...container.querySelectorAll<HTMLButtonElement>('.recording-broadcast-heading')];
    await act(async () => headings[0]!.click());
    await act(async () => headings[2]!.click());
    expect(cards().map(card => card.dataset.recordingId)).toEqual(["c", "d"]);
    await act(async () => root.render(null));
    await render(records.map(value => ({ ...value })));
    expect(cards().map(card => card.dataset.recordingId)).toEqual(["c", "d"]);
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    expect(container.querySelector('.recording-library-detail')).toBeNull();
    expect(callbacks.onReplay).not.toHaveBeenCalled();
  });
  it("keeps all broadcasts closed across remounts, empty loading snapshots and stale selected IDs", async () => {
    const records = [merged("a"), merged("b")];
    await render(records);
    for (const heading of container.querySelectorAll<HTMLButtonElement>('.recording-broadcast-heading')) await act(async () => heading.click());
    await act(async () => root.render(null));
    await render([]);
    await render(records);
    expect(cards()).toHaveLength(0);
    await act(async () => root.render(null));
    await render(records, "a");
    expect(cards()).toHaveLength(0);
    expect(container.querySelectorAll('.recording-broadcast-heading[aria-expanded="true"]')).toHaveLength(0);
  });
  it("retains closed states through filters and new files in the same broadcast", async () => {
    const original = { ...merged("a"), channelId: "shared", broadcastKey: "id:42" };
    await render([original]);
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    await search("no match"); await search("");
    expect(cards()).toHaveLength(0);
    await act(async () => root.render(null));
    await render([original, { ...merged("b"), channelId: "shared", broadcastKey: "id:42", title: "새 제목" }, merged("new")]);
    expect(cards()).toHaveLength(0);
  });
  it.each(["broken", "null", "[]", "123", JSON.stringify({ '["a"]': "false" })])("ignores malformed expansion preferences: %s", async (value: string) => {
    localStorage.setItem(expansionStorageKey, value);
    await render([merged("a")]);
    expect(cards()).toHaveLength(0);
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    expect(cards()).toHaveLength(1);
  });
  it("keeps expansion usable when browser storage is unavailable", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("storage unavailable"); });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("storage unavailable"); });
    await render([merged("a")]);
    expect(cards()).toHaveLength(0);
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    expect(cards()).toHaveLength(1);
  });
  it("allows all broadcasts and the selected detail to close, keeping a reserved empty slot", async () => {
    const records = [merged("a"), merged("b")];
    function Controlled() {
      const [selectedId, onSelect] = useState<string | null>(null);
      return <RecordingLibrary recordings={records} selectedId={selectedId} privacy={false} disabled={false} retrying={false} openingFolder={false} {...callbacks} onSelect={onSelect} />;
    }
    seedExpandedFiles(records);
    await act(async () => root.render(<Controlled />));
    expect(container.querySelector('.recording-library-detail')).toBeNull();
    expect(container.querySelector('.recording-library-detail-placeholder')).not.toBeNull();
    await select("a", {});
    expect(container.querySelector('.recording-library-detail')).toHaveTextContent("방송 a");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="녹화 상세 닫기"]')!.click());
    expect(container.querySelector('.recording-library-detail')).toBeNull();
    await select("b", {}); await select("b", {});
    expect(container.querySelector('.recording-library-detail')).toBeNull();
    for (const heading of container.querySelectorAll<HTMLButtonElement>('.recording-broadcast-heading')) await act(async () => heading.click());
    expect(container.querySelectorAll('.recording-broadcast-heading[aria-expanded="true"]')).toHaveLength(0);
    expect(cards()).toHaveLength(0);
    await act(async () => root.render(<Controlled />));
    expect(cards()).toHaveLength(0);
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    expect(cards().map(card => card.dataset.recordingId)).toEqual(["a"]);
  });
  it("keeps a plain-clicked card in selection and reveals checkboxes only from the second card", async () => {
    await render([merged("a"), merged("b"), merged("c"), merged("d")]);
    expect(button("선택")).toBeUndefined();
    await select("a", {});
    expect(callbacks.onSelect).toHaveBeenCalledExactlyOnceWith("a");
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    expect(cards()[0]).toHaveAttribute("aria-pressed", "true");
    expect(button("녹화 삭제")).toBeEnabled();
    await select("c", { shiftKey: true });
    expect(container).toHaveTextContent("3개 선택");
    expect(callbacks.onSelect).toHaveBeenCalledTimes(1);
    await select("b"); expect(container).toHaveTextContent("2개 선택");
    await select("d", { shiftKey: true, ctrlKey: true }); expect(container).toHaveTextContent("4개 선택");
    await select("d", {});
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    expect(callbacks.onSelect).toHaveBeenLastCalledWith("d");
    expect(cards()[3]).toHaveAttribute("aria-pressed", "true");
    await select("d", {});
    expect(cards()[3]).toHaveAttribute("aria-pressed", "false");
    expect(button("녹화 삭제")).toBeUndefined();
  });
  it("plain then Ctrl adds a second card; Space toggles; Ctrl+Shift+A clears", async () => {
    await render([merged("a"), merged("b"), merged("c")]);
    await select("a", {}); await select("b");
    expect(container).toHaveTextContent("2개 선택");
    expect(cards().filter(card => card.getAttribute("aria-pressed") === "true").map(card => card.dataset.recordingId)).toEqual(["a", "b"]);
    await act(async () => cards()[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
    expect(cards()[0]).toHaveAttribute("aria-pressed", "true");
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "a", ctrlKey: true, shiftKey: true, bubbles: true })));
    expect(cards().every(card => card.getAttribute("aria-pressed") === "false")).toBe(true);
  });
  it("does not toggle on the second double-click event or delete until explicitly confirmed", async () => {
    await render([merged("a"), merged("b")]);
    await select("a", { detail: 1 }); await select("a", { detail: 2 });
    await act(async () => cards()[0]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })));
    expect(cards()[0]).toHaveAttribute("aria-pressed", "true");
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })));
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container.querySelector("dialog ul")).toHaveTextContent("방송 a");
    expect(container.querySelector("dialog ul")).not.toHaveTextContent("방송 b");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("retains selected files when their broadcast is collapsed without extending ranges through hidden files", async () => {
    const a = { ...merged("a"), channelId: "shared", broadcastKey: "id:42", startedAt: 2 };
    const b = { ...merged("b"), channelId: "shared", broadcastKey: "id:42", startedAt: 3 };
    await render([a, b, { ...merged("c"), startedAt: 1 }], "a");
    await select("a"); await select("b"); expect(container).toHaveTextContent("2개 선택");
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    expect(container.querySelector('.recording-library-selection')).toHaveTextContent("2개 선택");
    await select("c", { shiftKey: true }); expect(container).toHaveTextContent("2개 선택");
    expect(container.querySelector('[data-recording-id="a"]')).toBeNull();
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("does not extend a range into closed broadcasts, filtered siblings, or unloaded pages", async () => {
    const a = { ...merged("a"), startedAt: 4 };
    const hidden = ["hidden1", "hidden2"].map((id, index) => ({ ...merged(id), channelId: "shared", broadcastKey: "id:42", startedAt: 2 + index }));
    await render([a, ...hidden, { ...merged("b"), startedAt: 1 }]);
    expect(cards().map(card => card.dataset.recordingId)).toEqual(["a", "b"]);
    await select("a", {}); await select("b", { shiftKey: true });
    expect(container).toHaveTextContent("2개 선택");
    await act(async () => deleteSelection().click());
    expect(container.querySelector('dialog')).not.toHaveTextContent("hidden");
  });
  it("clears multiple selection with Escape without deleting or changing playback", async () => {
    await render([merged("a"), merged("b")]); await selectAll();
    await act(async () => cards()[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    expect(callbacks.onSelect).not.toHaveBeenCalled(); expect(callbacks.onDelete).not.toHaveBeenCalled(); expect(callbacks.onReplay).not.toHaveBeenCalled();
  });
  it("serializes batch retry, skips healthy and busy files, and fences repeated submits", async () => {
    let resolve!: (value: boolean) => void;
    callbacks.onRetryMerge.mockReturnValueOnce(new Promise<boolean>(done => { resolve = done; })).mockResolvedValueOnce(true);
    await render([item("retry-a"), merged("healthy"), item("busy", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } }), item("retry-b", { merge: { status: "failed", segmentCount: 2, updatedAt: 1 } })]);
    await selectAll(); expect(container).toHaveTextContent("3개 선택");
    await act(async () => { button("병합 재개").click(); button("병합 재개")?.click(); });
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("retry-a");
    expect(button("선택 삭제")).toBeDisabled(); expect(button("선택 해제")).toBeEnabled();
    await act(async () => resolve(true));
    expect(callbacks.onRetryMerge.mock.calls.map(([id]) => id)).toEqual(["retry-a", "retry-b"]);
    expect(container).toHaveTextContent("완료 0/2 · 상태 확인 2");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("shows failed batch requests without reporting success and permits explicit retry", async () => {
    callbacks.onRetryMerge.mockResolvedValueOnce(false).mockRejectedValueOnce(new Error("fixture"));
    await render([item("a"), item("b")]); await selectAll();
    await act(async () => button("병합 재개").click());
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("2개 재처리 실패");
    expect(container).not.toHaveTextContent("재개를 요청했습니다.");
    expect(button("병합 재개")).toBeEnabled();
  });
  it("revalidates each queued retry against newer recording snapshots", async () => {
    let resolve!: (value: boolean) => void;
    callbacks.onRetryMerge.mockReturnValueOnce(new Promise<boolean>(done => { resolve = done; }));
    await render([item("a"), item("b")]); await selectAll();
    await act(async () => button("병합 재개").click());
    await render([item("a"), item("b", { status: "recording" })]);
    await act(async () => resolve(true));
    expect(callbacks.onRetryMerge).toHaveBeenCalledExactlyOnceWith("a");
  });
  it("rejects a delete confirmation if polling made a selected file busy", async () => {
    await render([merged("a")]); await selectAll(); await act(async () => deleteSelection().click());
    await render([item("a", { status: "recording" })]);
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).not.toHaveBeenCalled();
    expect(container.querySelector('dialog')).toHaveTextContent("상태가 변경되었습니다");
  });
  it("uses the same channel-inside-group shell for one file and multiple files, with file health beside size", async () => {
    const single = { ...merged("single"), captureChat: true, chatStatus: "partial" };
    await render([single, { ...merged("a"), channelId: "shared", broadcastKey: "id:42" }, { ...merged("b"), channelId: "shared", broadcastKey: "id:42" }]);
    expect(container.querySelectorAll('.recording-broadcast-group')).toHaveLength(2);
    expect(container.querySelectorAll('.recording-broadcast-group > .recorded-channel')).toHaveLength(2);
    expect(container.querySelector('.recording-library-card-wrap > .recorded-channel')).toBeNull();
    const bottom = container.querySelector('.recording-library-card-bottom');
    expect(bottom).toHaveTextContent('MB 정상');
    expect(bottom?.querySelector('.recording-library-state.is-success')).toHaveTextContent('정상');
    expect(bottom?.querySelector('.official-browser-saved-chat-warning')).not.toBeNull();
    expect(container).not.toHaveTextContent('종료 정보');
  });
  it("groups a broadcast across a restart without combining files or changing replay IDs", async () => {
    const old = { ...merged("old"), channelId: "channel", broadcastKey: "id:42", startedAt: 1 };
    const latest = { ...merged("latest"), channelId: "channel", broadcastKey: "id:42", startedAt: 2, title: "변경된 제목" };
    await render([latest, old], "old");
    expect(container.querySelectorAll(".recording-broadcast-group")).toHaveLength(1);
    expect(container.querySelector(".recording-broadcast-heading")).toHaveTextContent("변경된 제목파일 2개");
    expect(cards().map(card => card.querySelector("strong")!.textContent)).toEqual(["파일 1", "파일 2"]);
    await act(async () => cards()[1]!.click()); expect(callbacks.onSelect).toHaveBeenCalledWith("latest");
    await act(async () => button("재생").click()); expect(callbacks.onReplay).toHaveBeenCalledWith("latest");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
    await act(async () => container.querySelector<HTMLButtonElement>(".recording-broadcast-heading")!.click());
    expect(cards()).toHaveLength(0);
    await act(async () => container.querySelector<HTMLButtonElement>(".recording-broadcast-heading")!.click()); expect(cards()).toHaveLength(2);
    await select("old", {});
    await act(async () => deleteSelection().click());
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["old"], failures: [] });
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).toHaveBeenCalledExactlyOnceWith(["old"]);
  });
  it("keeps active and stopped files of the same broadcast in one group and protects the active file", async () => {
    const old = { ...merged("old"), channelId: "channel", broadcastKey: "id:42", startedAt: 1 };
    const live = item("live", { channelId: "channel", broadcastKey: "id:42", status: "recording", startedAt: 2 });
    await render([live, old]);
    expect(container.querySelectorAll(".recording-broadcast-group")).toHaveLength(1);
    expect(container.querySelector(".recording-library-active .recording-broadcast-group")).not.toBeNull();
    expect(container.querySelector(".recording-library-browse .recording-broadcast-group")).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    await selectAll();
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    expect(container.querySelector('[data-recording-id="old"]')).toHaveAttribute('aria-pressed', 'true');
    await act(async () => deleteSelection().click());
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["old"], failures: [] });
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).toHaveBeenCalledExactlyOnceWith(["old"]);
  });
  it("filters individual files before grouping so a hidden sibling cannot be selected for deletion", async () => {
    const a = { ...merged("a"), channelId: "channel", broadcastKey: "id:42", title: "일치" };
    const b = { ...merged("b"), channelId: "channel", broadcastKey: "id:42", title: "숨긴 제목" };
    await render([a, b]);
    await act(async () => container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
    await selectAll(); expect(container).toHaveTextContent("2개 선택");
    await search("일치"); expect(container.querySelector('.recording-library-selection')).toBeNull();
    expect(cards()).toHaveLength(1);
    await selectAll();
    await act(async () => deleteSelection().click());
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["a"], failures: [] });
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).toHaveBeenCalledExactlyOnceWith(["a"]);
  });
  it("keeps media-only cleanup in collapsed diagnostics without replay or merge controls", async () => {
    const removed = { ...merged("removed"), mediaRemovedAt: 1234, lastError: "original gap evidence" };
    await render([removed, merged("kept")], "removed");
    expect(container).toHaveTextContent("저장 영상 1개");
    expect(container).toHaveTextContent("영상 삭제 기록 1개");
    expect(container).not.toHaveTextContent("original gap evidence");
    expect(cards()).toHaveLength(1);
    expect(button("재생")).toBeUndefined();
    expect(button("병합 다시 시도")).toBeUndefined();
    expect(button("저장 폴더 열기")).toBeEnabled();
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("shows the saved outcome of cached history, with loading only for missing summaries", async () => {
    await render([
      { ...merged("cached"), storageCheckPending: true },
      item("pending", { status: "interrupted", segmentCount: 0, bytesWritten: 0, durationSeconds: 0, storageCheckPending: true, summaryPending: true }),
    ]);
    expect(container).not.toHaveTextContent("마지막 저장 상태");
    expect(container).toHaveTextContent("기록 불러오는 중");
    expect(container.querySelector(".recording-library-attempts")).toBeNull();
    expect(cards()[1]!.querySelector(".recording-library-state.is-waiting")).toHaveTextContent("기록 불러오는 중");
    expect(cards()[0]!.querySelector(".recording-library-state.is-success")).toHaveTextContent("정상");
    expect(button('재생')).toBeUndefined();
    await select("cached", {});
    expect(button('재생')).toBeEnabled();
    await act(async () => button("확인 필요").click());
    expect(cards()).toHaveLength(0);
  });
  it("distinguishes recording, processing, ready, warning and actual failure by labels and symbols", async () => {
    await render([
      item("live", { status: "recording" }),
      item("merging", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } }),
      merged("ready"),
      item("interrupted", { status: "interrupted" }),
      item("failed", { status: "failed" }),
    ]);
    expect(container.querySelector(".recording-library-state.is-recording")).toHaveTextContent("녹화 중");
    expect(container.querySelector(".recording-library-state.is-processing")).toHaveTextContent("병합·검증 중");
    expect(container.querySelector(".recording-library-state.is-success")).toHaveTextContent("정상");
    expect(container.querySelector(".recording-library-state.is-warning")).toHaveTextContent("중단됨");
    expect(container.querySelector(".recording-library-state.is-error")).toHaveTextContent("저장 오류");
  });
  it("separates empty startup failures from saved broadcasts without deleting their diagnostics", async () => {
    await render([item("retry", { segmentCount: 0, bytesWritten: 0, durationSeconds: 0, status: "interrupted" }), merged("success")]);
    expect(container).toHaveTextContent("저장 영상 1개");
    const attempts = container.querySelector<HTMLDetailsElement>(".recording-library-attempts")!;
    expect(attempts.open).toBe(false);
    expect(attempts).toHaveTextContent("시작 실패 1개");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("shows confirmed broadcast completion separately from manual stopping", async () => {
    await render([item("ended", { ending: { reason: "broadcast_ended", trigger: "video_ended", stoppedAt: 1, confirmedAt: 2 } }), item("manual", { ending: { reason: "user_stopped", trigger: "user_stop", stoppedAt: 1, confirmedAt: null } })]);
    expect(container.querySelector(".recording-playback-problems")).toBeNull();
    await select("ended", {});
    expect(container.querySelector('.recording-library-detail .recording-library-state')).toHaveAttribute("title", "방송 종료");
    await render([item("manual", { ending: { reason: "user_stopped", trigger: "user_stop", stoppedAt: 1, confirmedAt: null } })], "manual");
    expect(container.querySelector('.recording-library-detail .recording-library-state')).toHaveAttribute("title", "직접 중지");
  });
  it("allows explicit selection of an empty attempt without including hidden attempts in select-all", async () => {
    await render([item("empty", { segmentCount: 0, bytesWritten: 0, durationSeconds: 0, status: "interrupted" })]);
    expect(button("선택")).toBeUndefined();
    await selectAll(); expect(container.querySelector('.recording-library-selection')).toBeNull();
    await act(async () => { const details = container.querySelector('details')!; details.open = true; details.dispatchEvent(new Event('toggle', { bubbles: true })); });
    await select("empty");
    expect(container).toHaveTextContent("1개 선택");
    expect(callbacks.onDelete).not.toHaveBeenCalled();
  });
  it("includes unfinished but replayable ranges in the ready filter", async () => {
    await render([item("ranges", { progressive: { partCount: 1, segmentCount: 1, durationSeconds: 15, lastError: null } }), item("waiting")]);
    await act(async () => button("재생 가능").click());
    expect(cards()).toHaveLength(1); expect(cards()[0]!.closest('.recording-broadcast-group')).toHaveTextContent("방송 ranges");
    await select("ranges", {});
    expect(button("재생")).toBeEnabled();
  });
  it("selects multiple saved recordings, confirms exact IDs, and never deletes on cancel", async () => {
    await render([item("live", { status: "recording" }), merged("a"), merged("b")]);
    await select("a");
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    await select("b");
    expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
    expect(container).toHaveTextContent("2개 선택");
    await act(async () => deleteSelection().click());
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container.querySelector("dialog")).toHaveTextContent("복구할 수 없습니다");
    expect(document.activeElement).toBe(button("취소"));
    await act(async () => button("취소").click());
    expect(callbacks.onDelete).not.toHaveBeenCalled();
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["a", "b"], failures: [] });
    await act(async () => deleteSelection().click());
    await act(async () => button("삭제").click());
    expect(callbacks.onDelete).toHaveBeenCalledExactlyOnceWith(["a", "b"]);
    expect(container.querySelector("dialog")).not.toHaveAttribute("open");
    expect(container).toHaveTextContent("2개 녹화를 삭제했습니다.");
  });

  it("select-all includes only loaded expanded files, not active, merging or later pages", async () => {
    const records = Array.from({ length: 30 }, (_, index) => merged(`a${index}`));
    await render([item("live", { status: "recording" }), item("busy", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } }), ...records]);
    await select("a0");
    await select("a1");
    expect(container.querySelector<HTMLInputElement>('input[aria-label="방송 busy 선택"]')).toBeDisabled();
    await selectAll();
    expect(container).toHaveTextContent("23개 선택");
    await search("방송 a2");
    expect(container.querySelector('.recording-library-selection')).toBeNull();
    await selectAll();
    expect(container).toHaveTextContent("11개 선택");
  });

  it("retains only failed selections and reports partial success without hiding failures", async () => {
    await render([merged("a"), merged("b")]);
    callbacks.onDelete.mockResolvedValueOnce({ deletedIds: ["a"], failures: [{ id: "b", error: { code: "BUSY", message: "파일 사용 중", retryable: true } }] });
    await selectAll();
    await act(async () => deleteSelection().click());
    await act(async () => button("삭제").click());
    await render([merged("b")]);
    expect(container).toHaveTextContent("1개 녹화를 삭제했습니다.");
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("방송 b — 파일 사용 중");
    expect(container).toHaveTextContent("1개 선택");
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    expect(cards()[0]).toHaveAttribute("aria-pressed", "true");
    expect(button("녹화 삭제")).toBeEnabled();
  });

  it("fences repeated submits and keeps a transport failure retryable", async () => {
    let reject!: (error: Error) => void;
    callbacks.onDelete.mockReturnValueOnce(new Promise((_, fail) => { reject = fail; }));
    await render([merged("a")]);
    await selectAll();
    await act(async () => deleteSelection().click());
    await act(async () => { button("삭제").click(); button("삭제")?.click(); });
    expect(button("삭제 중…")).toBeDisabled(); expect(button("취소")).toBeDisabled();
    expect(callbacks.onDelete).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error("연결 확인 필요")));
    expect(container.querySelector("dialog")).toHaveAttribute("open");
    expect(container.querySelector("dialog")).toHaveTextContent("연결 확인 필요");
    expect(button("삭제")).toBeEnabled();
  });

  it("drops newly protected selections on polling and hides private titles in confirmation", async () => {
    await render([merged("secret"), merged("busy")]);
    await selectAll();
    await render([merged("secret"), item("busy", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } })]);
    expect(container).toHaveTextContent("1개 선택");
    await act(async () => deleteSelection().click());
    await render([merged("secret")], null, true);
    expect(container.querySelector("dialog")).not.toHaveTextContent("secret");
    expect(container.querySelector("dialog")).toHaveTextContent("녹화 영상");
  });

  it("offers retry deletion, not playback or merge, for interrupted deletion", async () => {
    await render([item("a", { deletionPending: true })]);
    expect(container).toHaveTextContent("삭제 미완료");
    expect(button("재생")).toBeUndefined();
    await selectAll();
    expect(container).toHaveTextContent("1개 선택");
  });
  it("separates active recordings and offers saved playback only on explicit action", async () => {
    await render([item("live", { status: "recording" }), merged("ready")]);
    expect(container.querySelector('.recording-library-active')).toHaveTextContent("방송 live");
    expect(container.querySelector('.recording-library-grid')).not.toHaveTextContent("방송 live");
    expect(container.querySelector('.recording-library-detail')).toBeNull();
    await select("ready");
    expect(container.querySelector('.recording-library-detail')).toHaveTextContent("방송 ready");
    expect(container.querySelector("video,iframe,img")).toBeNull();
    expect(callbacks.onReplay).not.toHaveBeenCalled();
    await act(async () => button("재생").click());
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("ready");
    await act(async () => cards()[0]!.click());
    expect(callbacks.onSelect).toHaveBeenCalledExactlyOnceWith("live");
  });

  it("filters saved metadata while keeping live recordings visible", async () => {
    await render([item("live", { status: "recording" }), merged("ready"), item("failed", { lastError: "문제", status: "failed" }), item("merging", { merge: { status: "merging", segmentCount: 2, updatedAt: 1 } })]);
    await act(async () => button("재생 가능").click());
    expect(cards()).toHaveLength(2); expect(cards()[1]!.closest('.recording-broadcast-group')).toHaveTextContent("방송 ready");
    await act(async () => button("확인 필요").click());
    expect(cards()).toHaveLength(2); expect(cards()[1]!.closest('.recording-broadcast-group')).toHaveTextContent("방송 failed");
    await act(async () => button("전체").click());
    await search("CHANNEL-READY");
    expect(cards()).toHaveLength(2); expect(cards()[1]!.closest('.recording-broadcast-group')).toHaveTextContent("방송 ready");
    await search("일치하지않음");
    expect(cards()).toHaveLength(1); expect(container).toHaveTextContent("조건에 맞는 녹화가 없습니다");
  });

  it("renders 24 metadata cards at a time without opening media and retains more across polling", async () => {
    const recordings = Array.from({ length: 55 }, (_, index) => merged(String(index)));
    await render(recordings);
    expect(cards()).toHaveLength(24);
    await act(async () => button("더 보기 · 31개").click());
    expect(cards()).toHaveLength(48);
    await render(recordings.map(recording => ({ ...recording, updatedAt: recording.updatedAt + 1 })));
    expect(cards()).toHaveLength(48);
    await search("방송 54"); expect(cards()).toHaveLength(1);
    await search(""); expect(cards()).toHaveLength(24);
    expect(callbacks.onOpenSegment).not.toHaveBeenCalled();
    expect(callbacks.onOpenMerged).not.toHaveBeenCalled();
  });

  it("preserves selection across snapshot replacement and routes file actions only to it", async () => {
    await render([merged("a"), merged("b")], "b");
    expect(cards()[1]).toHaveAttribute("aria-pressed", "false");
    await select("b", {});
    expect(cards()[1]).toHaveAttribute("aria-pressed", "true");
    await render([merged("new"), merged("a"), merged("b")], "b");
    expect(container.querySelector('.recording-library-detail')).toHaveTextContent("방송 b");
    await act(async () => button("저장 폴더 열기").click());
    await act(async () => button("재생").click());
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    expect(container.querySelector(".recording-library-heading-actions")).toContainElement(button("저장 폴더 열기"));
    expect(callbacks.onFolder).toHaveBeenCalledExactlyOnceWith("b");
    expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("b");
    await render([merged("b")], "b", false, true);
    expect(button("저장 폴더 열기")).toBeDisabled(); expect(button("재생")).toBeDisabled();
  });

  it("clears private queries and hides broadcast titles, paths and filenames", async () => {
    const values = [merged("secret")]; await render(values); await search("secret");
    await render(values, null, true);
    expect(container).not.toHaveTextContent("secret"); expect(container).not.toHaveTextContent("SyntheticOnly");
    expect(container.querySelector<HTMLInputElement>('input[type="search"]')).toBeDisabled();
    expect(container.querySelector<HTMLInputElement>('input[type="search"]')?.value).toBe("");
    expect(container.querySelector("video,iframe,img")).toBeNull();
  });

  it("does not crash the library when legacy metadata contains an out-of-range date", async () => {
    await render([item("legacy", { startedAt: 1e20 })]);
    expect(cards()).toHaveLength(1);
    expect(cards()[0]?.querySelector("time")).not.toHaveAttribute("datetime");
  });

  it("does not infer saved chat for legacy or explicitly disabled recording data", () => {
    expect(recordingChatNote(item("legacy")).text).toBe("이 기록의 채팅 저장 상태는 확인되지 않았습니다.");
    expect(recordingChatNote(item("off", { captureChat: false, chatCount: 12 })).text).toBe("이 기록은 채팅 저장을 사용하지 않았습니다.");
    expect(recordingChatNote(item("bad", { captureChat: true, chatStatus: "storage_failed", chatCount: 42 }))).toEqual({ warning: true, text: "채팅 저장 실패 · 일부 기록 누락 가능 · 42개 기록" });
  });
});

describe("recording card context menu",()=>{
 const context=async(target:Element)=>act(async()=>{target.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true,clientX:40,clientY:40}));});
 const menuButton=(label:string)=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(b=>b.textContent===label)!;
 it.each([".recording-broadcast-group",".recording-broadcast-heading strong",".recording-library-card-wrap",".recording-library-card time"])("opens the same single-file actions from %s",async selector=>{
   await render([merged("a")]); await context(container.querySelector(selector)!);
   expect(menuButton("재생")).not.toBeDisabled();expect(menuButton("삭제")).not.toBeDisabled();
   await act(async()=>menuButton("재생").click());
   expect(callbacks.onReplay).toHaveBeenCalledExactlyOnceWith("a");
   expect(callbacks.onDelete).not.toHaveBeenCalled();
 });
 it("allows a collapsed single file to request deletion, cancel preserves it",async()=>{
   await render([merged("a")]);
   await act(async()=>container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
   await context(container.querySelector('.recording-broadcast-group')!);
   await act(async()=>menuButton("삭제").click());
   expect(container.querySelector("dialog")).toHaveAttribute("open");
   await act(async()=>button("취소").click());
   expect(callbacks.onDelete).not.toHaveBeenCalled();
 });
 it("nested file right-click never targets siblings and processing is protected",async()=>{
   const a={...merged("a"),channelId:"shared",broadcastKey:"id:1"},b={...merged("b"),channelId:"shared",broadcastKey:"id:1",workProgress:{phase:"verifying",completed:1,total:10,unit:"seconds",updatedAt:1}};
   await render([a,b]);await act(async()=>container.querySelector<HTMLButtonElement>('.recording-broadcast-heading')!.click());
   await context(container.querySelector('[data-recording-id="b"]')!);
   expect(document.querySelectorAll('[role="menuitem"]')).toHaveLength(3);
   expect(menuButton("삭제")).toBeDisabled();
   expect(container.querySelector('[role="progressbar"]')).toHaveAttribute("aria-valuenow","10");
 });
});
