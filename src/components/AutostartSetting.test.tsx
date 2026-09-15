import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getAutostartStatus, setAutostartEnabled, type AutostartStatus } from "../api/autostart";
import type { ApiResult } from "../api/contracts";
import { AutostartSetting } from "./AutostartSetting";

vi.mock("../api/autostart", () => ({ getAutostartStatus: vi.fn(), setAutostartEnabled: vi.fn() }));

const defaultStatus: AutostartStatus = {
  supported: true, enabled: false, launchMode: "development", needsRepair: false, disabledByWindows: false,
};
const success = (patch: Partial<AutostartStatus> = {}): ApiResult<AutostartStatus> => ({ ok: true, data: { ...defaultStatus, ...patch } });
const failed: ApiResult<AutostartStatus> = { ok: false, error: { code: "AUTOSTART_FAILED", message: "접근 권한을 확인해 주세요.", retryable: true } };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("AutostartSetting", () => {
  let container: HTMLDivElement;
  let root: Root;
  const getStatus = vi.mocked(getAutostartStatus);
  const setEnabled = vi.mocked(setAutostartEnabled);
  const render = async (active = true) => { await act(async () => root.render(<AutostartSetting active={active} />)); };
  const toggle = () => container.querySelector<HTMLInputElement>('input[role="switch"]')!;
  const button = (label: string) => Array.from(container.querySelectorAll("button")).find((item) => item.textContent === label)!;
  const focus = async () => { await act(async () => window.dispatchEvent(new Event("focus"))); };

  beforeEach(() => {
    vi.resetAllMocks();
    getStatus.mockResolvedValue(success());
    setEnabled.mockImplementation(async (enabled) => success({ enabled }));
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("reads status only when active and never enables registration just by opening settings", async () => {
    await render(false);
    await focus();
    expect(container.textContent).toBe("");
    expect(getStatus).not.toHaveBeenCalled();
    await render();
    expect(getStatus).toHaveBeenCalledTimes(1);
    expect(setEnabled).not.toHaveBeenCalled();
    expect(toggle()).not.toBeChecked();
    expect(toggle()).toHaveAccessibleName("Windows 로그인 시 자동 실행");
    expect(container.textContent).toContain("저장 버튼과 별개로 즉시 적용");
    expect(container.textContent).toContain("기본으로 켜지지 않습니다");
    expect(container.textContent).toContain("개발 실행기를 사용");
  });

  it("enables and disables immediately, using only the state confirmed by the native API", async () => {
    const pending = deferred<ApiResult<AutostartStatus>>();
    setEnabled.mockReturnValueOnce(pending.promise);
    await render();
    await act(async () => toggle().click());
    expect(setEnabled).toHaveBeenCalledWith(true);
    expect(toggle()).not.toBeChecked();
    expect(toggle()).toBeDisabled();
    expect(container.textContent).toContain("변경 중");
    await act(async () => pending.resolve(success({ enabled: true })));
    expect(toggle()).toBeChecked();
    expect(toggle()).not.toBeDisabled();
    await act(async () => toggle().click());
    expect(setEnabled).toHaveBeenLastCalledWith(false);
    expect(toggle()).not.toBeChecked();
  });

  it("preserves the last confirmed state after a mutation failure and retries the requested change", async () => {
    getStatus.mockResolvedValue(success({ enabled: true }));
    setEnabled.mockResolvedValueOnce(failed);
    await render();
    await act(async () => toggle().click());
    expect(toggle()).toBeChecked();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("자동 실행 설정을 변경하지 못했습니다");
    await act(async () => button("다시 시도").click());
    expect(setEnabled).toHaveBeenNthCalledWith(2, false);
    expect(toggle()).not.toBeChecked();
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("shows failed reads and unexpected API failures without reporting false success", async () => {
    getStatus.mockResolvedValueOnce(failed);
    await render();
    expect(toggle()).toBeDisabled();
    expect(container.textContent).toContain("자동 실행 상태를 확인하지 못했습니다");
    await act(async () => button("다시 시도").click());
    expect(toggle()).not.toBeDisabled();
    setEnabled.mockRejectedValueOnce(new Error("invoke failed"));
    await act(async () => toggle().click());
    expect(toggle()).not.toBeChecked();
    expect(container.textContent).toContain("자동 실행 설정을 변경하지 못했습니다");
    expect(container.textContent).not.toContain("invoke failed");
  });

  it("refreshes external OS changes on window focus and reopening, not while hidden", async () => {
    await render();
    getStatus.mockResolvedValue(success({ enabled: true, launchMode: "installed" }));
    await focus();
    expect(toggle()).toBeChecked();
    expect(container.textContent).toContain("현재 앱의 실행 파일");
    await render(false);
    await focus();
    expect(getStatus).toHaveBeenCalledTimes(2);
    getStatus.mockResolvedValue(success());
    await render();
    expect(getStatus).toHaveBeenCalledTimes(3);
    expect(toggle()).not.toBeChecked();
    expect(setEnabled).not.toHaveBeenCalled();
  });

  it("disables the switch in browsers without pretending to persist a preference", async () => {
    getStatus.mockResolvedValue(success({ supported: false, launchMode: "unsupported" }));
    await render();
    expect(toggle()).toBeDisabled();
    expect(container.textContent).toContain("브라우저에서는 변경할 수 없습니다");
    await act(async () => toggle().click());
    expect(setEnabled).not.toHaveBeenCalled();
  });

  it("offers an explicit reconnection for a moved app and allows the old registration to be disabled", async () => {
    getStatus.mockResolvedValue(success({ enabled: true, needsRepair: true }));
    await render();
    expect(setEnabled).not.toHaveBeenCalled();
    expect(container.textContent).toContain("이전 앱 위치");
    await act(async () => button("현재 앱에 다시 연결").click());
    expect(setEnabled).toHaveBeenCalledWith(true);
    expect(container.textContent).not.toContain("현재 앱에 다시 연결");
    await focus();
    await act(async () => toggle().click());
    expect(setEnabled).toHaveBeenLastCalledWith(false);
  });

  it("explains Windows startup-app blocking without claiming the app will auto-run", async () => {
    getStatus.mockResolvedValue(success({ enabled: true, disabledByWindows: true }));
    await render();
    expect(container.textContent).toContain("등록됨 · Windows에서 중지");
    expect(container.textContent).toContain("Windows 설정 → 앱 → 시작 프로그램");
    expect(setEnabled).not.toHaveBeenCalled();
    await act(async () => toggle().click());
    expect(setEnabled).toHaveBeenCalledWith(false);
  });

  it("ignores an older read after the setting panel reopens", async () => {
    const pending = deferred<ApiResult<AutostartStatus>>();
    getStatus.mockReturnValueOnce(pending.promise).mockResolvedValue(success({ enabled: true }));
    await render();
    await render(false);
    await render();
    expect(toggle()).toBeChecked();
    await act(async () => pending.resolve(success()));
    expect(toggle()).toBeChecked();
  });

  it("ignores an older focus refresh that resolves after a newer focus refresh", async () => {
    const pending = deferred<ApiResult<AutostartStatus>>();
    await render();
    getStatus.mockReturnValueOnce(pending.promise).mockResolvedValue(success({ enabled: true }));
    await focus();
    await focus();
    expect(toggle()).toBeChecked();
    await act(async () => pending.resolve(success()));
    expect(toggle()).toBeChecked();
  });

  it("serializes repeated clicks and defers focus refresh until a mutation finishes", async () => {
    const pending = deferred<ApiResult<AutostartStatus>>();
    setEnabled.mockReturnValueOnce(pending.promise);
    await render();
    await act(async () => { toggle().click(); toggle().click(); });
    await focus();
    expect(setEnabled).toHaveBeenCalledTimes(1);
    expect(getStatus).toHaveBeenCalledTimes(1);
    getStatus.mockResolvedValue(success({ enabled: true }));
    await act(async () => pending.resolve(success({ enabled: true })));
    expect(getStatus).toHaveBeenCalledTimes(2);
    expect(toggle()).toBeChecked();
  });

  it("uses a fresh OS read instead of a stale mutation response after closing and reopening", async () => {
    const pending = deferred<ApiResult<AutostartStatus>>();
    setEnabled.mockReturnValueOnce(pending.promise);
    await render();
    await act(async () => toggle().click());
    await render(false);
    await render();
    expect(toggle()).toBeDisabled();
    expect(getStatus).toHaveBeenCalledTimes(1);
    getStatus.mockResolvedValue(success());
    await act(async () => pending.resolve(success({ enabled: true })));
    expect(getStatus).toHaveBeenCalledTimes(2);
    expect(toggle()).not.toBeChecked();
    expect(toggle()).not.toBeDisabled();
  });
});
