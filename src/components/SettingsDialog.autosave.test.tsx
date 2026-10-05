import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsDialog } from "./SettingsDialog";
import { loadDanbooruSearchPreferences } from "../danbooru/searchPreferences";
import type { SettingsPatch, SettingsSnapshot } from "../api/contracts";

const initial: SettingsSnapshot = {
  revision: 1, downloadRoot: "D:\\Albums", folderNameTemplate: "{id}",
  autoFindHistoryMode: "newer_than_latest_owned", downloadOverlapAutoMode: "off",
  explorePageSize: 50, danbooruPageSize: 60, maxColumns: 3, previewWidth: 220,
  danbooruPreviewWidth: 190, relatedPreviewWidth: 240, privacyMode: true, privacyOnStartup: true,
  cacheLimitGb: 10, concurrentImageRequests: 5, downloadAdaptiveConcurrency: true,
  downloadAdaptiveMaxRequests: 8, requestStartIntervalMs: 25, autoFindGrouping: "all", downloadsGrouping: "all",
  exploreDisplayMode: "detail", autoFindDisplayMode: "detail", downloadsDisplayMode: "detail",
  collapsedGroupKeys: [], searchIncludeTags: [], searchExcludeTags: [],
};
let root: Root, container: HTMLDivElement, props: ComponentProps<typeof SettingsDialog>;
const dialogMethods = ["showModal", "close"] as const;
let originalMethods: (PropertyDescriptor | undefined)[];
const render = async (changes: Partial<typeof props> = {}) => { props = { ...props, ...changes }; await act(async () => root.render(<SettingsDialog {...props} />)); };
const tab = async (name: string) => act(async () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(el => el.textContent === name)!.click());
const input = (name: string) => container.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[aria-label="${name}"]`)!;
const enter = async (name: string, text: string) => act(async () => {
  const el = input(name);
  Object.getOwnPropertyDescriptor(el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(el, text);
  el.dispatchEvent(new Event("input", { bubbles: true }));
});
const blur = async (name: string) => act(async () => input(name).dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); localStorage.clear();
  originalMethods = dialogMethods.map(name => Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name));
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.open = true; } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function (this: HTMLDialogElement) { this.open = false; } });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  props = {
    open: true, settings: initial, loading: false, error: null, onClose: vi.fn(),
    onSave: vi.fn(async (_patch: SettingsPatch) => true),
    onLoadStorageUsage: vi.fn(), onPreviewLayout: vi.fn(), onPreviewFolderName: vi.fn(),
    onMaintenance: vi.fn(), onCheckForUpdates: vi.fn(), onTagCatalogRefresh: vi.fn(), tagCatalogRefreshing: false,
    onLoadExplorationExclusions: vi.fn(), onRestoreExplorationExclusions: vi.fn(),
  };
  vi.mocked(props.onLoadStorageUsage).mockResolvedValue({ ok: false, error: { code: "FIXTURE", message: "fixture", retryable: false } });
  vi.mocked(props.onPreviewFolderName).mockResolvedValue({ ok: true, data: "fixture" });
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove();
  dialogMethods.forEach((name, index) => {
    if (originalMethods[index]) Object.defineProperty(HTMLDialogElement.prototype, name, originalMethods[index]!);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, name);
  });
  vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

describe("settings automatic application", () => {
  it("saves switches immediately, input on blur, and the last edits when closed externally", async () => {
    await render();
    expect(props.onSave).not.toHaveBeenCalled();
    expect(container.querySelectorAll('.dialog-header-actions button')).toHaveLength(1);
    await act(async () => input("프라이버시 모드 상태로 시작").click());
    expect(props.onSave).toHaveBeenLastCalledWith({ privacyOnStartup: false });
    await enter("다운로드 폴더", "E:\\Albums");
    expect(props.onSave).toHaveBeenCalledTimes(1);
    await blur("다운로드 폴더");
    expect(props.onSave).toHaveBeenLastCalledWith({ downloadRoot: "E:\\Albums" });
    await tab("Hitomi"); await enter("동시 이미지 요청", "7");
    await render({ open: false });
    expect(props.onSave).toHaveBeenLastCalledWith({ concurrentImageRequests: 7 });
    expect(props.onMaintenance).not.toHaveBeenCalled();
  });
  it("does not persist invalid numbers or contradictory tags, then saves corrected rules atomically", async () => {
    await render(); await tab("Hitomi");
    for (const value of ["", "31", "2.5"]) {
      await enter("동시 이미지 요청", value); await blur("동시 이미지 요청");
      expect(props.onSave).not.toHaveBeenCalled();
    }
    expect(container).toHaveTextContent("1~30 사이의 정수");
    await enter("동시 이미지 요청", "8"); await blur("동시 이미지 요청");
    expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ concurrentImageRequests: 8 });
    await enter("모든 검색 필수 포함 태그", "artist:fixture");
    await enter("모든 검색 제외 태그", "artist:fixture"); await blur("모든 검색 제외 태그");
    expect(props.onSave).toHaveBeenCalledTimes(1);
    expect(container).toHaveTextContent("동시에 지정할 수 없습니다");
    await enter("모든 검색 제외 태그", "group:fixture"); await blur("모든 검색 제외 태그");
    expect(props.onSave).toHaveBeenLastCalledWith({ searchIncludeTags: ["artist:fixture"], searchExcludeTags: ["group:fixture"] });
  });
  it("retains failed edits after closing and supports a retry without reverting the control", async () => {
    const save = vi.fn(async (_patch: SettingsPatch) => false);
    await render({ onSave: save });
    await enter("다운로드 폴더", "E:\\Retained"); await blur("다운로드 폴더");
    expect(container).toHaveTextContent("저장 실패");
    await render({ open: false }); await render({ open: true });
    expect(input("다운로드 폴더").value).toBe("E:\\Retained");
    save.mockResolvedValue(true);
    await act(async () => container.querySelector<HTMLButtonElement>('.settings-save-error button')!.click());
    expect(save).toHaveBeenLastCalledWith({ downloadRoot: "E:\\Retained" });
    expect(container.querySelector('.settings-save-error')).toBeNull();
  });
  it("persists Danbooru filters without a save button or settings revision write", async () => {
    await render(); await tab("Danbooru");
    const checkbox = container.querySelector<HTMLInputElement>('.danbooru-settings-checks input')!;
    expect(checkbox).toBeChecked();
    await act(async () => checkbox.click());
    expect(loadDanbooruSearchPreferences().ratings).not.toContain("g");
    expect(props.onSave).not.toHaveBeenCalled();
  });
});
