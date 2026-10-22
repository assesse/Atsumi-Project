import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import type { SoopAutoChannel, SoopAutoRecordApi, SoopAutoSnapshot } from "../../api/soopAutoRecord";
import { autoRecordLabel, useSoopAutoRecord } from "./useSoopAutoRecord";

const off: SoopAutoSnapshot = { revision: 1, channels: [], ready: true, error: null };
const channel: SoopAutoChannel = { channelId: "channel", name: "채널", enabled: true, status: "idle", liveStatus: "offline", jobId: null, checkedAt: 1, error: null };
const on: SoopAutoSnapshot = { ...off, revision: 2, channels: [channel] };
const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (error: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); });
function View({ api, active = true }: { api: SoopAutoRecordApi; active?: boolean }) {
  const model = useSoopAutoRecord(api, active);
  return <><button disabled={!model.canToggle("channel")} aria-pressed={!!model.snapshot.channels[0]?.enabled} aria-busy={!!model.pending} onClick={() => void model.toggle("channel", "채널")}>예약</button><output>{model.snapshot.revision}</output><p role="alert">{model.error}</p></>;
}
const render = async (api: SoopAutoRecordApi, active = true) => act(async () => root.render(<View api={api} active={active} />));
const click = async () => act(async () => host.querySelector("button")!.click());

it("only explicit toggles write and stop polling while inactive without altering a reservation", async () => {
  const api = { snapshot: vi.fn().mockResolvedValue(off), set: vi.fn().mockResolvedValue(on) };
  await render(api); await act(async () => vi.advanceTimersByTimeAsync(6000)); expect(api.set).not.toHaveBeenCalled();
  await click(); expect(api.set).toHaveBeenCalledExactlyOnceWith("channel", "채널", true, 1);
  await render(api, false); const calls = api.snapshot.mock.calls.length;
  await act(async () => vi.advanceTimersByTimeAsync(30_000)); expect(api.snapshot).toHaveBeenCalledTimes(calls);
  expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "true"); expect(api.set).toHaveBeenCalledTimes(1);
});

it("ignores an old polling response after a successful write", async () => {
  const late = deferred<SoopAutoSnapshot>();
  const api = { snapshot: vi.fn().mockResolvedValueOnce(off).mockReturnValueOnce(late.promise), set: vi.fn().mockResolvedValue(on) };
  await render(api); await act(async () => vi.advanceTimersByTimeAsync(3000)); await click();
  await act(async () => late.resolve(off));
  expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "true"); expect(host.querySelector("output")).toHaveTextContent("2");
});

it("locks duplicate clicks, keeps the same toggle node, and waits for acknowledged state", async () => {
  const write = deferred<SoopAutoSnapshot>(); const api = { snapshot: vi.fn().mockResolvedValue(off), set: vi.fn().mockReturnValue(write.promise) };
  await render(api); const button = host.querySelector("button")!; await click(); await click();
  expect(api.set).toHaveBeenCalledTimes(1); expect(button).toBeDisabled(); expect(button).toHaveAttribute("aria-pressed", "false");
  await act(async () => write.resolve(on)); expect(host.querySelector("button")).toBe(button); expect(button).not.toBeDisabled(); expect(button).toHaveAttribute("aria-pressed", "true");
});

it("refreshes after a revision conflict without repeating the write or erasing its message", async () => {
  const api = { snapshot: vi.fn().mockResolvedValueOnce(off).mockResolvedValue(on), set: vi.fn().mockRejectedValue("예약 상태가 변경됐습니다.") };
  await render(api); await click(); await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(api.set).toHaveBeenCalledTimes(1); expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "true");
  expect(host.querySelector('[role="alert"]')).toHaveTextContent("예약 상태가 변경됐습니다.");
});

it("does not treat failed refresh as disabled or enable an unloaded/corrupt configuration", async () => {
  const api = { snapshot: vi.fn().mockResolvedValueOnce(on).mockRejectedValue("네트워크 오류"), set: vi.fn() };
  await render(api); await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "true"); expect(host.querySelector('[role="alert"]')).toHaveTextContent("네트워크 오류");
  api.snapshot.mockResolvedValue({ ...on, channels: [], error: "설정 손상" }); await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(host.querySelector("button")).toBeDisabled(); await click(); expect(api.set).not.toHaveBeenCalled();
});

it("permits an explicit fault-time off but never claims an unsaved stop was durably saved", async () => {
  const fault = "설정 손상", stopped = "이번 실행에서만 해제됐습니다. 다시 시작하면 예약이 재개될 수 있습니다.";
  const api = { snapshot: vi.fn().mockResolvedValueOnce({ ...on, error: fault }).mockResolvedValue({ ...on, revision: 3, channels: [{ ...channel, enabled: false }], error: stopped }), set: vi.fn().mockRejectedValue(stopped) };
  await render(api); const control = host.querySelector("button")!;
  expect(control).not.toBeDisabled(); expect(control).toHaveAttribute("aria-pressed", "true");
  await click(); expect(api.set).toHaveBeenCalledExactlyOnceWith("channel", "채널", false, 2);
  expect(host.querySelector("button")).toBe(control); expect(control).toHaveAttribute("aria-pressed", "false"); expect(control).toBeDisabled();
  expect(host.querySelector('[role="alert"]')).toHaveTextContent(stopped);
  await click(); expect(api.set).toHaveBeenCalledTimes(1);
});

it("late writes from an old API cannot unlock or overwrite a newer pending operation", async () => {
  const oldWrite = deferred<SoopAutoSnapshot>(), nextWrite = deferred<SoopAutoSnapshot>();
  const oldApi = { snapshot: vi.fn().mockResolvedValue(off), set: vi.fn().mockReturnValue(oldWrite.promise) };
  const nextApi = { snapshot: vi.fn().mockResolvedValue(off), set: vi.fn().mockReturnValue(nextWrite.promise) };
  await render(oldApi); await click(); await render(nextApi); await click();
  await act(async () => oldWrite.resolve(on)); expect(host.querySelector("button")).toBeDisabled(); expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "false");
  await act(async () => nextWrite.resolve(on)); expect(host.querySelector("button")).not.toBeDisabled(); expect(host.querySelector("button")).toHaveAttribute("aria-pressed", "true");
});

it("uses short distinct labels without calling a failed recording complete", () => {
  expect(autoRecordLabel()).toBe("예약 꺼짐"); expect(autoRecordLabel(channel)).toBe("녹화 예약 됨");
  for (const [status, label] of [["starting", "준비 중"], ["recording", "녹화 중"], ["stopping", "마무리 중"], ["waiting", "자리 대기"], ["skipped", "이번 방송 건너뜀"], ["failed", "확인 필요"]] as const) expect(autoRecordLabel({ ...channel, status })).toBe(label);
});
