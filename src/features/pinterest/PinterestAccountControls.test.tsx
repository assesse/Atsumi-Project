import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PinterestAccountControls } from "./PinterestAccountControls";
import { createPinterestAccountApi, type PinterestAccountApi, type PinterestAccountSnapshot } from "./accountApi";

const loggedOut: PinterestAccountSnapshot = { status: "signedOut", busy: false, open: false, generation: 1 };
const loggedIn: PinterestAccountSnapshot = { ...loggedOut, status: "signedIn", generation: 2 };
let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
const buttons = () => [...host.querySelectorAll<HTMLButtonElement>("button")];
const fixture = (): PinterestAccountApi => ({ status: vi.fn().mockResolvedValue(loggedOut), open: vi.fn().mockResolvedValue({ ...loggedOut, open: true, busy: true }) });
const render = async (api: PinterestAccountApi, callback = vi.fn(), active = true) => act(async () => root.render(<PinterestAccountControls api={api} active={active} enabled onSnapshot={callback} />));

it("does not manufacture login success outside the desktop runtime", async () => {
  const api = createPinterestAccountApi("browser-mock");
  expect((await api.status()).status).toBe("unknown");
  await expect(api.open("login")).rejects.toThrow("데스크톱");
});
it("disables logout when logged out and login when positively logged in", async () => {
  const api = fixture(), onSnapshot = vi.fn();
  await render(api, onSnapshot);
  expect(buttons()[0]).toBeEnabled(); expect(buttons()[1]).toBeDisabled();
  expect(host).toHaveTextContent("로그아웃됨");
  vi.mocked(api.status).mockResolvedValue(loggedIn);
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(buttons()[0]).toBeDisabled(); expect(buttons()[1]).toBeEnabled();
  expect(host).toHaveTextContent("로그인됨");
  expect(onSnapshot).toHaveBeenLastCalledWith(loggedIn);
});
it("opens only on user action and polls while the official window awaits user authentication", async () => {
  const api = fixture();
  await render(api);
  expect(api.open).not.toHaveBeenCalled();
  vi.mocked(api.status).mockResolvedValue({ ...loggedOut, open: true, busy: true });
  await act(async () => buttons()[0]!.click());
  expect(api.open).toHaveBeenCalledExactlyOnceWith("login");
  expect(buttons().every(button => button.disabled)).toBe(true);
  vi.mocked(api.status).mockResolvedValue(loggedIn);
  await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
  expect(host).toHaveTextContent("로그인됨");
  expect(buttons()[1]).toBeEnabled();
});
it("does not call an unsuccessful logout complete or clear the last verified status", async () => {
  const api = fixture();
  vi.mocked(api.status).mockResolvedValue(loggedIn);
  vi.mocked(api.open).mockRejectedValue(new Error("private server details must not be rendered"));
  await render(api);
  await act(async () => buttons()[1]!.click());
  expect(host).toHaveTextContent("로그인됨");
  expect(host.querySelector('[role="alert"]')).toHaveTextContent("로그아웃을 완료하지 못했습니다");
  expect(host).not.toHaveTextContent("private server details");
  expect(buttons()[1]).toBeEnabled();
});
it("keeps unknown proof distinct from signed out and stops polling inactive views", async () => {
  const api = fixture();
  vi.mocked(api.status).mockResolvedValue({ ...loggedOut, status: "unknown", error: "상태 확인이 필요합니다." });
  await render(api);
  expect(host).toHaveTextContent("확인 필요");
  expect(host).not.toHaveTextContent("로그아웃됨");
  await render(api, vi.fn(), false);
  const count = vi.mocked(api.status).mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(60000); });
  expect(api.status).toHaveBeenCalledTimes(count);
});
it("rejects an older status response after a user account operation starts", async () => {
  const api = fixture();
  await render(api);
  let finishStatus!: (value: PinterestAccountSnapshot) => void;
  vi.mocked(api.status).mockImplementationOnce(() => new Promise(resolve => { finishStatus = resolve; }));
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  vi.mocked(api.open).mockResolvedValue(loggedIn);
  await act(async () => buttons()[0]!.click());
  await act(async () => finishStatus(loggedOut));
  expect(host).toHaveTextContent("로그인됨");
  expect(buttons()[0]).toBeDisabled();
});
