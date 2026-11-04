import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppShell, useAppShell, type AppShellApi } from "./AppShell";
import { createRecordingUpdateGuard } from "../api/recordingUpdateGuard";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), check: vi.fn(), relaunch: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, Channel: class {} }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));
vi.mock("../hooks/useSettings", () => ({ useSettings: () => ({
  hasSnapshot: false, settings: {}, error: "data unavailable", save: vi.fn(),
}) }));
vi.mock("./useStartup", () => ({ useStartup: () => ({
  phase: "failed", backgroundReady: false, statusError: null, cancel: vi.fn(),
}) }));
vi.mock("../hooks/useWindowPlacement", () => ({ useWindowPlacement: () => {} }));
vi.mock("./useRecordingNotifications", () => ({ useRecordingNotifications: () => {} }));
vi.mock("./usePreferenceQueue", () => ({ usePreferenceQueue: () => ({}) }));
vi.mock("./useAppExit", () => ({ useAppExit: () => ({ open: false, dialogProps: {} }) }));
vi.mock("../components/ExitConfirmDialog", () => ({ ExitConfirmDialog: () => null }));

const previousDialogMethods = ["showModal", "close"].map(name => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)] as const);
const disposers: Array<() => Promise<void>> = [];
beforeEach(() => {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "app_update_mode") return "installed";
    if (["app_update_reserve", "app_update_release"].includes(command)) return;
    throw new Error("data-backed commands unavailable");
  });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0));
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value(this: HTMLDialogElement) { this.setAttribute("open", ""); } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value(this: HTMLDialogElement) { this.removeAttribute("open"); } });
});
afterEach(async () => {
  for (const dispose of disposers.splice(0)) await dispose();
  for (const [name, descriptor] of previousDialogMethods) {
    if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, name);
  }
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.resetAllMocks();
});
function Probe() {
  const { tutorialOpen, privacyModePending } = useAppShell();
  return <p data-tutorial={tutorialOpen} data-pending={privacyModePending}>shell</p>;
}
async function mount() {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  disposers.push(async () => { await act(async () => root.unmount()); container.remove(); });
  await act(async () => root.render(<AppShell api={{ runtime: "tauri" } as AppShellApi} updateGuard={createRecordingUpdateGuard("tauri")}><Probe /></AppShell>));
  const click = async (label: string) => {
    const button = Array.from(container.querySelectorAll("button")).find(node => node.textContent?.trim() === label);
    expect(button, label).toBeDefined(); expect(button?.disabled).toBe(false);
    await act(async () => button!.click());
  };
  return { container, click };
}

it("shows update discovery and install even when data never loads and a first-run tutorial exists", async () => {
  const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
  mocks.check.mockResolvedValue({ currentVersion: "2.4.1", version: "2.4.2", downloadAndInstall, close: vi.fn().mockResolvedValue(undefined) });
  const { container, click } = await mount();
  expect(container.textContent).toContain("앱 데이터 준비 실패");
  expect(container.querySelector("[data-tutorial]")?.getAttribute("data-tutorial")).toBe("false");
  expect(container.querySelector("[data-pending]")?.getAttribute("data-pending")).toBe("true");
  expect(container.querySelector(".update-dialog")?.hasAttribute("open")).toBe(true);
  await click("다운로드 및 설치");
  expect(mocks.invoke).toHaveBeenCalledWith("app_update_reserve");
  expect(downloadAndInstall).toHaveBeenCalledOnce();
  expect(mocks.relaunch).toHaveBeenCalledOnce();
});

it("retries discovery and failed signature/download without resetting the failed data state", async () => {
  const downloadAndInstall = vi.fn().mockRejectedValueOnce(new Error("signature mismatch")).mockResolvedValue(undefined);
  const update = { currentVersion: "2.4.1", version: "2.4.2", downloadAndInstall, close: vi.fn().mockResolvedValue(undefined) };
  mocks.check.mockRejectedValueOnce(new Error("offline")).mockResolvedValue(update);
  const { container, click } = await mount();
  expect(container.querySelector(".update-dialog")?.hasAttribute("open")).toBe(false);
  await click("업데이트 확인");
  expect(container.querySelector(".update-dialog")?.hasAttribute("open")).toBe(true);
  await click("다운로드 및 설치");
  expect(mocks.invoke).toHaveBeenCalledWith("app_update_release");
  expect(mocks.relaunch).not.toHaveBeenCalled();
  await click("다시 시도");
  expect(downloadAndInstall).toHaveBeenCalledTimes(2);
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "app_update_reserve")).toHaveLength(2);
  expect(mocks.relaunch).toHaveBeenCalledOnce();
  expect(container.textContent).toContain("앱 데이터 준비 실패");
});

it("reports an actual failed manual request rather than saying the app is current", async () => {
  mocks.check.mockRejectedValue(new Error("offline"));
  const { container, click } = await mount();
  await click("업데이트 확인");
  expect(container.querySelector(".toast")?.textContent).toContain("업데이트 정보를 확인하지 못했습니다");
  expect(container.textContent).not.toContain("현재 최신 버전입니다.");
});
