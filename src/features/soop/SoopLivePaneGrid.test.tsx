import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SoopLivePaneGrid } from "./SoopLivePaneGrid";

let root: Root, container: HTMLDivElement;
beforeEach(() => { vi.useFakeTimers(); localStorage.clear(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
describe("SOOP presentation-only pane grid", () => {
  it("keeps four receiver nodes mounted during grouped/paired changes and independent split updates", async () => {
    const opened = vi.fn(), closed = vi.fn();
    function Pane({ id }: { id: number }) { useEffect(() => { opened(id); return () => closed(id); }, [id]); return <div data-pane={id} />; }
    const render = (grouped: boolean) => act(async () => root.render(<SoopLivePaneGrid kind="pair" className="soop-channel-pairs" grouped={grouped} onResizing={() => undefined}>{[0, 1, 2, 3].map(id => <div key={id}><Pane id={id} /></div>)}</SoopLivePaneGrid>));
    await render(false); const original = [...container.querySelectorAll('[data-pane]')];
    await render(true);
    const keyboard = async (label: string, key: string) => act(async () => container.querySelector(`[aria-label="${label}"]`)!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key })));
    await keyboard("방송 묶음 좌우 크기", "ArrowRight"); await keyboard("채팅 좌우 크기", "ArrowLeft"); await keyboard("방송·채팅 좌우 크기", "ArrowRight");
    expect(container.querySelector('[aria-label="방송 묶음 좌우 크기"]')).toHaveAttribute("aria-valuenow", "52");
    expect(container.querySelector('[aria-label="채팅 좌우 크기"]')).toHaveAttribute("aria-valuenow", "48");
    expect(container.querySelector('[aria-label="방송·채팅 좌우 크기"]')).toHaveAttribute("aria-valuenow", "67");
    expect([...container.querySelectorAll('[data-pane]')]).toEqual(original);
    expect(opened).toHaveBeenCalledTimes(4); expect(closed).not.toHaveBeenCalled();
    await render(false); expect([...container.querySelectorAll('[data-pane]')]).toEqual(original); expect(closed).not.toHaveBeenCalled();
  });
  it("reads and writes only SOOP pane preferences and clamps corrupt values", async () => {
    localStorage.setItem("atsumi.live.panes.pair.v1", JSON.stringify({ columns: 79, rows: 70 }));
    localStorage.setItem("atsumi.youtube.live.panes.pair.v1", JSON.stringify({ columns: 23, rows: 30 }));
    localStorage.setItem("atsumi.soop.live.panes.pair.v1", JSON.stringify({ columns: 1000, rows: -1, group: 999, chatColumns: -100 }));
    await act(async () => root.render(<SoopLivePaneGrid kind="pair" className="soop-channel-pairs" grouped onResizing={() => undefined}>{[0, 1, 2].map(id => <div key={id} />)}</SoopLivePaneGrid>));
    expect(container.querySelector('[aria-label="방송 묶음 좌우 크기"]')).toHaveAttribute("aria-valuenow", "80");
    expect(container.querySelector('[aria-label="방송 묶음 상하 크기"]')).toHaveAttribute("aria-valuenow", "20");
    await act(async () => vi.advanceTimersByTimeAsync(201));
    expect(JSON.parse(localStorage.getItem("atsumi.soop.live.panes.pair.v1")!)).toMatchObject({ columns: 80, rows: 20, group: 80, chatColumns: 20 });
    expect(JSON.parse(localStorage.getItem("atsumi.live.panes.pair.v1")!)).toEqual({ columns: 79, rows: 70 });
    expect(JSON.parse(localStorage.getItem("atsumi.youtube.live.panes.pair.v1")!)).toEqual({ columns: 23, rows: 30 });
  });
});
