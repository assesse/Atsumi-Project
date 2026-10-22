import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PixivMetadataText } from "./PixivMetadataText";
import { pixivReadings } from "./readingService";

let host: HTMLDivElement, root: Root;
async function hover(node: Element) { await act(async () => node.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }))); await act(async () => vi.advanceTimersByTime(300)); }
const tooltip = () => document.querySelector('[role="tooltip"]');
describe("Pixiv metadata hover interaction", () => {
  beforeEach(() => { vi.useFakeTimers(); Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
  it("keeps canonical labels and Ctrl-click intact, without a duplicate native title", async () => {
    const click = vi.fn(), read = vi.spyOn(pixivReadings, "read");
    await act(async () => root.render(<PixivMetadataText as="button" text="風景" kind="tag" hints={[{ tag: "風景", en: "landscape", romaji: "fuukei" }]} onClick={click} />));
    const button = host.querySelector("button")!; expect(button.textContent).toBe("風景"); expect(button.title).toBe(""); expect(tooltip()).toBeNull();
    await hover(button); expect(tooltip()?.textContent).toContain("풍경"); expect(read).not.toHaveBeenCalled();
    expect([...tooltip()!.querySelectorAll(".pixiv-metadata-line>span:first-child")].map(node => node.textContent)).toEqual(["한국어 뜻", "영어 해석", "영어 발음"]);
    await act(async () => button.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(click.mock.calls[0]![0].ctrlKey).toBe(true); expect(host.querySelectorAll("button")).toHaveLength(1);
  });
  it("labels Kanji as estimated and never shows a late result after privacy or target change", async () => {
    let resolve!: (result: { text: string; partial: boolean }) => void;
    const read = vi.spyOn(pixivReadings, "read").mockImplementation(() => new Promise(done => { resolve = done; }));
    await act(async () => root.render(<PixivMetadataText as="strong" text="山田" kind="artist" />)); await hover(host.querySelector("strong")!);
    expect(tooltip()?.textContent).toContain("발음 확인 중");
    await act(async () => resolve({ text: "yamada", partial: false })); expect(tooltip()?.textContent).toContain("영어 발음yamada (추정)");
    await act(async () => root.render(<PixivMetadataText as="strong" text="田中" kind="artist" />)); expect(tooltip()).toBeNull();
    await hover(host.querySelector("strong")!); const signal = read.mock.calls.at(-1)![1]!;
    await act(async () => root.render(<PixivMetadataText as="strong" text="田中" kind="artist" private />));
    expect(signal.aborted).toBe(true); expect(host.textContent).toBe("작가"); expect(tooltip()).toBeNull();
    await act(async () => resolve({ text: "private", partial: false })); expect(tooltip()).toBeNull();
  });
  it("adds missing tag reading even when a meaning is already available", async () => {
    const read = vi.spyOn(pixivReadings, "read").mockResolvedValue({ text: "fuukei", partial: false });
    await act(async () => root.render(<PixivMetadataText as="button" text="風景" kind="tag" hints={[{ tag: "風景", en: "landscape" }]} />));
    expect(read).not.toHaveBeenCalled(); await hover(host.querySelector("button")!);
    expect(read).toHaveBeenCalledTimes(1);
    expect(tooltip()?.textContent).toContain("한국어 뜻풍경");
    expect(tooltip()?.textContent).toContain("영어 해석landscape");
    expect(tooltip()?.textContent).toContain("영어 발음fuukei (추정)");
  });
  it("uses one reading heading with a value-level partial qualification", async () => {
    vi.spyOn(pixivReadings, "read").mockResolvedValue({ text: "yamada 𠮷", partial: true });
    await act(async () => root.render(<PixivMetadataText as="button" text="山田𠮷" kind="artist" />)); await hover(host.querySelector("button")!);
    expect(tooltip()?.querySelector(".pixiv-metadata-line>span:first-child")?.textContent).toBe("영어 발음");
    expect(tooltip()?.querySelector(".pixiv-metadata-note")?.textContent).toBe(" (일부 추정)");
    expect(tooltip()?.textContent).toContain("yamada 𠮷");
  });
  it.each(["작가", "Artist"])("keeps readable %s original without redundant or invented meanings", async text => {
    const read = vi.spyOn(pixivReadings, "read");
    await act(async () => root.render(<PixivMetadataText as="button" text={text} kind="artist" />)); await hover(host.querySelector("button")!);
    expect(tooltip()?.textContent).toBe(text); expect(read).not.toHaveBeenCalled();
  });
  it("dismisses on scroll/blur/Escape without swallowing Escape or altering selection", async () => {
    await act(async () => root.render(<PixivMetadataText as="button" text="ミドリ" kind="artist" aria-pressed="true" />));
    const button = host.querySelector("button")!;
    for (const event of [new Event("scroll"), new Event("blur"), new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })]) {
      await hover(button); expect(tooltip()).not.toBeNull();
      await act(async () => (event.type === "blur" ? window : document).dispatchEvent(event));
      expect(tooltip()).toBeNull(); expect(event.defaultPrevented).toBe(false); expect(button.getAttribute("aria-pressed")).toBe("true");
    }
  });
  it("supports keyboard focus and clears a pending hover on unmount", async () => {
    const read = vi.spyOn(pixivReadings, "read");
    await act(async () => root.render(<PixivMetadataText as="button" text="ミドリ" kind="group" />));
    await act(async () => host.querySelector("button")!.dispatchEvent(new FocusEvent("focusin", { bubbles: true })));
    await act(async () => vi.advanceTimersByTime(300)); expect(tooltip()?.textContent).toContain("midori");
    await act(async () => root.render(<PixivMetadataText as="button" text="山田" kind="artist" />));
    await act(async () => host.querySelector("button")!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    await act(async () => root.render(null)); await act(async () => vi.advanceTimersByTime(301));
    expect(tooltip()).toBeNull(); expect(read).not.toHaveBeenCalled();
  });
  it("does not expose metadata behind a modal; uses a dialog portal when inside it", async () => {
    await act(async () => root.render(<><PixivMetadataText as="button" text="イラスト" kind="tag" /><dialog open><PixivMetadataText as="button" text="ミドリ" kind="artist" /></dialog></>));
    await hover(host.querySelector("button")!); expect(tooltip()).toBeNull();
    await hover(host.querySelector("dialog button")!); expect(tooltip()?.closest("dialog")).not.toBeNull();
  });
  it("renders translations only as text and leaves unresolved names visibly unavailable", async () => {
    await act(async () => root.render(<PixivMetadataText as="button" text="example" kind="tag" hints={[{ tag: "example", ko: "<img src=x onerror=alert(1)>" }]} />));
    await hover(host.querySelector("button")!); expect(tooltip()?.querySelector("img")).toBeNull(); expect(tooltip()?.textContent).toContain("<img src=x");
    vi.spyOn(pixivReadings, "read").mockResolvedValue(undefined);
    await act(async () => root.render(<PixivMetadataText as="button" text="𠮷𠮷" kind="artist" />)); await hover(host.querySelector("button")!);
    expect(tooltip()?.textContent).toContain("번역·발음 정보 없음");
  });
  it("removes the tooltip when its reader chrome hides or a top-level modal appears", async () => {
    await act(async () => root.render(<header><PixivMetadataText as="strong" text="ミドリ" kind="artist" /></header>));
    await hover(host.querySelector("strong")!); expect(tooltip()).not.toBeNull();
    await act(async () => { host.querySelector("header")!.style.opacity = "0"; }); expect(tooltip()).toBeNull();
    await act(async () => { host.querySelector("header")!.style.opacity = "1"; });
    await hover(host.querySelector("strong")!); expect(tooltip()).not.toBeNull();
    const modal = document.createElement("dialog"); modal.open = true;
    await act(async () => { document.body.append(modal); }); expect(tooltip()).toBeNull(); modal.remove();
  });
});
