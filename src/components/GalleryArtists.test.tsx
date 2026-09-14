import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { fitGalleryArtistCount, GalleryArtists, orderGalleryArtists, type GalleryArtistsProps } from "./GalleryArtists";

describe("GalleryArtists", () => {
  let host: HTMLDivElement;
  let root: Root;
  let props: GalleryArtistsProps;
  let cardClick: Mock<() => void>;
  let cardKey: Mock<() => void>;
  const reactTestEnvironment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
  const previousEnvironment = reactTestEnvironment.IS_REACT_ACT_ENVIRONMENT;
  beforeAll(() => { reactTestEnvironment.IS_REACT_ACT_ENVIRONMENT = true; });
  afterAll(() => { reactTestEnvironment.IS_REACT_ACT_ENVIRONMENT = previousEnvironment; });
  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    cardClick = vi.fn();
    cardKey = vi.fn();
    props = {
      artist: "Alpha", artists: ["Alpha", "Beta", "Gamma", "Delta"], favoriteMetadata: new Set(["artist:beta"]),
      onSearch: vi.fn(), onToggleFavorite: vi.fn(),
    };
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });
  const render = async (value = props) => { await act(async () => root.render(<div onClick={() => cardClick()} onKeyDown={() => cardKey()}><GalleryArtists {...value} /></div>)); };
  const more = () => host.querySelector<HTMLButtonElement>(".gallery-artists-more:not([data-measure-overflow])")!;
  const popup = () => document.querySelector<HTMLDivElement>(".gallery-artists-popover");
  const click = async (node: HTMLElement, detail = 1) => {
    await act(async () => node.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail })));
  };
  const hover = async (node: HTMLElement) => {
    await act(async () => node.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
  };

  it("prioritizes only matching favorite artists, deduplicates aliases, and preserves source order and callback spelling", () => {
    const source = [" Alpha_One ", "BETA Two", "alpha one", "Gamma", ""];
    const ordered = orderGalleryArtists("unrelated", source, new Set([" ARTIST:beta_two ", "tag:Gamma"]));
    expect(ordered.map(({ name, favorite }) => [name, favorite])).toEqual([["BETA Two", true], ["Alpha One", false], ["Gamma", false]]);
    expect(ordered[0]!.token).toBe("artist:BETA Two");
    expect(source).toEqual([" Alpha_One ", "BETA Two", "alpha one", "Gamma", ""]);
  });

  it("does not invent artists or split the legacy primary value when a complete list is unavailable", async () => {
    await render({ ...props, artist: "Alpha / Beta", artists: undefined });
    expect(host.querySelector(".gallery-artists-label")).toHaveTextContent("Alpha / Beta");
    expect(more()).toBeNull();
    expect(orderGalleryArtists("Alpha", ["", " "], new Set())).toHaveLength(1);
    expect(orderGalleryArtists("", [], new Set())).toHaveLength(0);
    await render({ ...props, artist: "", artists: [], compact: true });
    expect(host.querySelector(".gallery-artists.is-compact")).toHaveTextContent("작가 정보 없음");
    expect(more()).toBeNull();
  });

  it("fits up to three names and reserves the real overflow count width", () => {
    expect(fitGalleryArtistCount([60, 70, 50], 250, [32, 32, 32], 4)).toBe(3);
    expect(fitGalleryArtistCount([60, 70, 50], 180, [32, 32, 32], 4)).toBe(2);
    expect(fitGalleryArtistCount([600, 70, 50], 120, [32, 32, 32], 4)).toBe(1);
    expect(fitGalleryArtistCount([60, 70, 50], 190, [32, 32, 32], 3)).toBe(3);
  });

  it("uses measured card width for detail and one favorite name plus remainder for compact", async () => {
    const measurement = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const width = this.classList.contains("gallery-artists") ? 180 : this.hasAttribute("data-measure-artist") ? 75 : this.hasAttribute("data-measure-overflow") ? 35 : 0;
      return { width, height: 24, x: 0, y: 0, top: 0, bottom: 24, left: 0, right: width, toJSON: () => ({}) };
    });
    await render();
    expect(host.querySelectorAll("button.gallery-artists-name")).toHaveLength(1);
    expect(more()).toHaveTextContent("+3명");
    measurement.mockRestore();
    await render({ ...props, compact: true });
    expect(host.querySelectorAll("button.gallery-artists-name")).toHaveLength(1);
    expect(host.querySelector("button.gallery-artists-name")).toHaveTextContent("★Beta");
    expect(more()).toHaveTextContent("외 3명");
  });

  it("shows all known artists outside the clipped card on hover and permits pointer travel", async () => {
    vi.useFakeTimers();
    await render();
    await hover(more());
    expect(popup()).toHaveAttribute("role", "dialog");
    expect(host.contains(popup())).toBe(false);
    expect(popup()!.querySelectorAll("li")).toHaveLength(4);
    await act(async () => host.querySelector(".gallery-artists")!.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
    await hover(popup()!);
    await act(async () => vi.advanceTimersByTime(250));
    expect(popup()).not.toBeNull();
    await act(async () => popup()!.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
    await act(async () => vi.advanceTimersByTime(250));
    expect(popup()).toBeNull();
  });

  it("pins a clicked popover, updates favorites individually, and searches the exact artist", async () => {
    await render();
    await hover(more());
    await click(more());
    const favorite = popup()!.querySelector<HTMLButtonElement>('[aria-label="Beta 즐겨찾기 해제"]')!;
    expect(favorite).toHaveAttribute("aria-pressed", "true");
    await click(favorite);
    expect(props.onToggleFavorite).toHaveBeenCalledWith("artist:Beta");
    expect(popup()).not.toBeNull();
    const sourceOrder = Array.from(popup()!.querySelectorAll(".gallery-artists-popover-search")).map((button) => button.textContent);
    await render({ ...props, favoriteMetadata: new Set(["artist:gamma"]) });
    expect(Array.from(popup()!.querySelectorAll(".gallery-artists-popover-search")).map((button) => button.textContent)).toEqual(sourceOrder);
    expect(popup()!.querySelector('[aria-label="Gamma 즐겨찾기 해제"]')).toHaveAttribute("aria-pressed", "true");
    await click(popup()!.querySelector<HTMLButtonElement>('[aria-label="Alpha 작가 검색"]')!);
    expect(props.onSearch).toHaveBeenCalledWith("artist:Alpha");
    expect(popup()).toBeNull();
    expect(cardClick).not.toHaveBeenCalled();
  });

  it("opens by keyboard, keeps key events off card handlers, and restores focus after Escape", async () => {
    await render();
    await act(async () => more().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(popup()!.querySelector("button"));
    expect(cardKey).not.toHaveBeenCalled();
    await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(popup()).toBeNull();
    expect(document.activeElement).toBe(more());
  });

  it("honors selection interception without searching or changing favorites", async () => {
    const capture = vi.fn((event) => { event.preventDefault(); });
    await render({ ...props, onClickCapture: capture });
    await click(host.querySelector<HTMLButtonElement>("button.gallery-artists-name")!);
    await click(more());
    expect(props.onSearch).not.toHaveBeenCalled();
    expect(popup()).toBeNull();
    await hover(more());
    await click(popup()!.querySelector<HTMLButtonElement>(".gallery-artists-popover-favorite")!);
    expect(props.onToggleFavorite).not.toHaveBeenCalled();
  });

  it("dismisses outside, on focus leaving, and when the album changes", async () => {
    await render();
    await click(more());
    await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
    expect(popup()).toBeNull();
    await click(more(), 0);
    const other = document.createElement("button"); document.body.append(other);
    await act(async () => other.focus());
    expect(popup()).toBeNull();
    other.remove();
    await click(more());
    await render({ ...props, artist: "Other", artists: ["Other", "New", "Third"] });
    expect(popup()).toBeNull();
  });

  it("does not leak artists from a disabled card through a portal", async () => {
    await render();
    await click(more());
    await render({ ...props, disabled: true });
    expect(popup()).toBeNull();
    expect(more()).toBeDisabled();
    await hover(more());
    await click(more());
    await click(host.querySelector<HTMLButtonElement>("button.gallery-artists-name")!);
    expect(popup()).toBeNull();
    expect(props.onSearch).not.toHaveBeenCalled();
  });

  it("portals inside a native modal's top layer and removes the portal on unmount", async () => {
    const modal = document.createElement("dialog"); modal.open = true;
    document.body.append(modal); modal.append(host);
    await render();
    await click(more());
    expect(popup()!.parentElement).toBe(modal);
    await act(async () => root.render(null));
    expect(popup()).toBeNull();
    document.body.append(host); modal.remove();
  });
});
