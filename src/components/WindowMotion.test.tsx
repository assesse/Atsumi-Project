import { act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MovingTabs, WindowMotion, animateWindowClose } from "./WindowMotion";

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
let animations: Array<{ node: HTMLElement; frames: Keyframe[]; finish(): void; cancel: ReturnType<typeof vi.fn> }>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); animations = [];
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  Object.defineProperty(HTMLElement.prototype, "animate", { configurable: true, value: function(this: HTMLElement, frames: Keyframe[]) {
    let finish!: () => void; const finished = new Promise<void>((resolve) => { finish = resolve; });
    const animation = { node: this, frames, finish, cancel: vi.fn() }; animations.push(animation);
    return { finished, cancel: animation.cancel };
  } });
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); Reflect.deleteProperty(HTMLElement.prototype, "animate"); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("album window motion", () => {
  it("keeps a noninteractive exit surface and cancels stale completion on immediate restore", async () => {
    const dock = createRef<HTMLButtonElement>(), exited = vi.fn();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
      return { left: 0, top: 0, width: this.tagName === "BUTTON" ? 160 : 800, height: this.tagName === "BUTTON" ? 44 : 600 } as DOMRect;
    });
    const render = (show: boolean) => root.render(<><button ref={dock}>상세 탭</button><WindowMotion show={show} anchor={dock} travel={!show} onExited={exited}>{show ? <section>detail</section> : null}</WindowMotion></>);
    await act(async () => render(true));
    await act(async () => animations.at(-1)!.finish());
    await act(async () => render(false));
    const exit = animations.at(-1)!;
    expect(host.querySelector("section")).not.toBeNull(); expect(host.querySelector(".window-motion")).toHaveAttribute("inert");
    expect(exit.frames[1]!.transform).toContain("scale(0.2, 0.07333333333333333)");
    await act(async () => render(true));
    await act(async () => exit.finish());
    expect(exit.cancel).toHaveBeenCalled(); expect(exited).not.toHaveBeenCalled();
    expect(host.querySelector("section")).not.toBeNull(); expect(host.querySelector(".window-motion")).not.toHaveAttribute("inert");
    await act(async () => render(false)); await act(async () => animations.at(-1)!.finish());
    expect(host.querySelector("section")).toBeNull(); expect(exited).toHaveBeenCalledOnce();
  });
  it("removes individual tabs after collapse, including reduced motion without an animation", async () => {
    const render = (ids: string[]) => root.render(<MovingTabs>{ids.map((id) => <button key={id}>{id}</button>)}</MovingTabs>);
    await act(async () => render(["a", "b"]));
    await act(async () => render(["b"]));
    expect(host.querySelectorAll("button")).toHaveLength(2);
    await act(async () => animations.at(-1)!.finish());
    expect(host.querySelectorAll("button")).toHaveLength(1); expect(host.textContent).toBe("b");
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    await act(async () => render([])); expect(host.querySelectorAll("button")).toHaveLength(0);
    await act(async () => render(["a"])); expect(host.textContent).toBe("a");
  });
  it("does not finish a cancelled native preview close", async () => {
    const done = vi.fn(), cancel = animateWindowClose(host, done);
    cancel(); await act(async () => animations.at(-1)!.finish()); expect(done).not.toHaveBeenCalled();
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    animateWindowClose(host, done); expect(done).toHaveBeenCalledOnce();
  });
});
