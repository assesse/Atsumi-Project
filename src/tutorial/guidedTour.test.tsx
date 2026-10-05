import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TutorialDialog } from "../components/TutorialDialog";
import { tutorialStepsBySource, type TourStep } from "./tourSteps";
import { beginTutorialAction } from "./tourActions";
import { nativeModalOcclusion } from "../features/streaming/nativeOverlayGeometry";
let container: HTMLDivElement, root: Root;
const settle = () => new Promise(resolve => setTimeout(resolve, 12));
const steps = (...ids: string[]) => ids.map(id => Object.values(tutorialStepsBySource).flat().find(step => step.id === id)!);
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) => setTimeout(() => fn(0), 0));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(function (this: HTMLElement) { return [this.getBoundingClientRect()] as unknown as DOMRectList; });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 100, y: 100, left: 100, top: 100, right: 280, bottom: 140, width: 180, height: 40, toJSON() {} });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function click(label: string) {
  await act(async () => { [...container.querySelectorAll<HTMLButtonElement>(".tutorial-callout button")].find(b => b.textContent === label)?.click(); await settle(); });
}
describe("guided tour interactions", () => {
  it("uses the concise right-click instruction and no longer advertises review search", () => {
    expect(steps("follow")[0]!.description).toBe("우클릭을 통해 즐겨찾기에 등록하세요\n즐겨찾기는 자동 탐색 대상에 포함됩니다.");
    expect(steps("community")[0]!.description).not.toContain("검색");
  });
  it("lets only the spotlight action through and blocks settings edits and background clicks", async () => {
    const setting = vi.fn(), outside = vi.fn(), edit = vi.fn(), start = vi.fn(), step = vi.fn();
    await act(async () => { root.render(<><button data-tour="hitomi-settings" onClick={setting}>setting</button><button onClick={outside}>outside</button><div data-tour="download-folder"><button onClick={edit}>edit</button></div><TutorialDialog open onClose={vi.fn()} onStart={start} onStepChange={step} /></>); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    expect(start).toHaveBeenCalledOnce(); expect(step).toHaveBeenLastCalledWith("settings");
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    expect(step).toHaveBeenLastCalledWith("settings");
    await act(async () => { container.querySelectorAll<HTMLButtonElement>("button")[1]!.click(); }); expect(outside).not.toHaveBeenCalled();
    await act(async () => { container.querySelector<HTMLButtonElement>('[data-tour="hitomi-settings"]')!.click(); await settle(); });
    expect(setting).toHaveBeenCalledOnce(); expect(step).toHaveBeenLastCalledWith("folder");
    await act(async () => { container.querySelector<HTMLButtonElement>('[data-tour="download-folder"] button')!.click(); }); expect(edit).not.toHaveBeenCalled();
    expect(container.querySelector(".tutorial-connector")).not.toBeNull();
    await click("다음"); expect(step).toHaveBeenLastCalledWith("privacy");
    expect(container).toHaveTextContent("기본값은 켜짐");
    await click("다음"); expect(step).toHaveBeenLastCalledWith("search");
  });
  it("opens the card body with double click or Enter, never its nested action", async () => {
    const open = vi.fn(), download = vi.fn();
    await act(async () => { root.render(<><section data-tour="hitomi-albums"><article tabIndex={0} data-tour="hitomi-album" onDoubleClick={open}>Album<button onClick={download}>download</button></article></section><TutorialDialog open onClose={vi.fn()} steps={steps("album", "detail")} /></>); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    const card = container.querySelector<HTMLElement>("article")!;
    await act(async () => { card.querySelector("button")!.click(); }); expect(download).not.toHaveBeenCalled();
    await act(async () => { card.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); await settle(); });
    expect(open).toHaveBeenCalledOnce();
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "detail");
    expect(container.querySelector("#tutorial-title")?.textContent).toBe("2. 앨범 도구");
    expect(container.querySelector("#tutorial-description")?.textContent).toBe("즐겨찾기 · 코멘트 · 다운로드");
  });
  it("never bypasses a required action when its target is missing or disabled", async () => {
    await act(async () => { root.render(<><button data-tour="hitomi-settings" disabled>setting</button><TutorialDialog open onClose={vi.fn()} /></>); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    await act(async () => { container.querySelector<HTMLButtonElement>('[data-tour="hitomi-settings"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); await settle(); });
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "settings");
  });
  it("permits explanatory steps, completion and cleanup", async () => {
    const closed = vi.fn();
    await act(async () => { root.render(<TutorialDialog open onClose={closed} steps={steps("finish")} />); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    expect(container).toHaveTextContent("화면을 불러오는 중…");
    await click("마치기"); expect(closed).toHaveBeenCalledWith();
    await act(async () => root.render(<TutorialDialog open={false} onClose={closed} />));
    expect(document.documentElement.dataset.tutorialOpen).toBeUndefined(); expect(container.querySelector(".tutorial-tour")).toBeNull();
  });
  it("restarts at the first step without preparing the old workspace on replay", async () => {
    const start = vi.fn(), step = vi.fn(), close = vi.fn();
    const flow = steps("folder", "finish");
    const render = async (open: boolean) => {
      await act(async () => root.render(<TutorialDialog open={open} onStart={start} onStepChange={step} onClose={close} steps={flow} />));
      await act(async () => { await settle(); });
    };
    await render(true);
    await click("다음");
    expect(step).toHaveBeenLastCalledWith("finish");
    await render(false);
    step.mockClear();
    await render(true);
    expect(step.mock.calls).toEqual([["folder"]]);
    expect(start).toHaveBeenCalledTimes(2);
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "folder");
    expect(close).not.toHaveBeenCalled();
  });
  it("requires right-click registration and waits for success, allowing retry after failure", async () => {
    const search = vi.fn(), save = vi.fn();
    let finish: (error?: string) => void = () => {};
    await act(async () => { root.render(<><div data-tour="hitomi-detail-follow"><button data-metadata-namespace="artist" data-favorite="false" onClick={search} onContextMenu={event => { event.preventDefault(); save(); finish = beginTutorialAction("follow"); }}>artist</button></div><TutorialDialog open onClose={vi.fn()} steps={steps("follow", "auto-find")} /></>); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    const artist = container.querySelector<HTMLButtonElement>('[data-metadata-namespace="artist"]')!;
    await act(async () => artist.click()); expect(search).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
    const register = async () => act(async () => { artist.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })); await settle(); });
    await register(); expect(save).toHaveBeenCalledOnce();
    expect(container.querySelector('[role="status"]')).toHaveTextContent("처리 중…"); expect(container.querySelector("[data-tour-next]")).toBeNull();
    await register(); expect(save).toHaveBeenCalledOnce();
    await act(async () => finish("저장 실패"));
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("저장 실패");
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    await register(); expect(save).toHaveBeenCalledTimes(2);
    await act(async () => { finish(); await settle(); });
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "auto-find");
  });
  it("never removes an existing favorite just to complete the practice", async () => {
    const remove = vi.fn();
    await act(async () => { root.render(<><div data-tour="hitomi-detail-follow"><button data-metadata-namespace="group" data-favorite="true" onContextMenu={remove}>group</button></div><TutorialDialog open onClose={vi.fn()} steps={steps("follow", "auto-find")} /></>); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    await act(async () => { container.querySelector('[data-favorite="true"]')!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, button: 2 })); });
    expect(remove).not.toHaveBeenCalled();
    expect(container.querySelector("[data-tour-next]")).toBeEnabled();
    await click("다음"); expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "auto-find");
  });
  it("offers another album when no artist or group exists, without bypassing registration", async () => {
    await act(async () => root.render(<><section data-tour="hitomi-albums"><article tabIndex={0} data-tour="hitomi-album">Album</article></section><div data-tour="hitomi-detail-follow" /><TutorialDialog open onClose={vi.fn()} steps={steps("album", "follow", "auto-find")} /></>));
    await act(async () => { await settle(); });
    await act(async () => { container.querySelector("article")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); await settle(); });
    await act(async () => { await settle(); });
    expect(container).toHaveTextContent("이 앨범에는 작가·그룹 정보가 없습니다.");
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    await click("다른 앨범 선택");
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "album");
  });
  it("requires a real refresh result and discards old results after closing and replaying", async () => {
    const flow: TourStep[] = [{ ...steps("folder")[0]!, id: "intro" }, ...steps("auto-find-refresh", "downloads")];
    let finish: (error?: string) => void = () => {};
    const render = async (open: boolean) => {
      await act(async () => root.render(<><button data-tour="hitomi-auto-find-refresh" onClick={() => { finish = beginTutorialAction("auto-find-refresh"); }}>refresh</button><TutorialDialog open={open} onClose={vi.fn()} steps={flow} /></>));
      await act(async () => { await settle(); });
    };
    await render(true);
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    await click("다음");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-tour="hitomi-auto-find-refresh"]')!.click());
    expect(container.querySelector("[data-tour-next]")).toBeNull();
    const oldFinish = finish;
    await render(false); await render(true); await click("다음");
    await act(async () => { oldFinish(); await settle(); });
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "auto-find-refresh");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-tour="hitomi-auto-find-refresh"]')!.click());
    await act(async () => { finish(); await settle(); });
    expect(container.querySelector(".tutorial-tour")).toHaveAttribute("data-tour-step", "downloads");
  });
  it("Escape closes immediately without asking for a replay preference", async () => {
    const closed = vi.fn();
    await act(async () => { root.render(<TutorialDialog open onClose={closed} />); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true })); });
    expect(closed).toHaveBeenCalledWith();
  });
  it("preserves background playback while covering the native CHZZK surface", async () => {
    await act(async () => { root.render(<TutorialDialog open onClose={vi.fn()} />); await settle(); });
    // React commits after act's callback; wait for the subsequent measurement frame.
    await act(async () => { await settle(); });
    const layer = container.querySelector<HTMLElement>(".tutorial-tour")!;
    layer.style.display = "block"; // JSDOM has no showPopover() or loaded stylesheet.
    const surface = container.querySelector<SVGSVGElement>(".tutorial-shade")!;
    const bounds = { x: 0, y: 0, left: 0, top: 0, width: 1280, height: 820, right: 1280, bottom: 820, toJSON() {} };
    vi.spyOn(layer, "getBoundingClientRect").mockReturnValue(bounds);
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue(bounds);
    const stage = document.createElement("div");
    vi.spyOn(stage, "getBoundingClientRect").mockReturnValue(bounds);
    const viewport = { x: 0, y: 0, width: 1280, height: 820, visible: true };
    expect(nativeModalOcclusion(stage, viewport)).toMatchObject({ occluded: true, preserveBackground: true, occlusions: [{ x: 0, y: 0, width: 1280, height: 820 }] });
  });
});
