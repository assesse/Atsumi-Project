import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { DanbooruDateRange } from "./DanbooruDateRange";

describe("Danbooru date segments", () => {
  it("moves year/month/day input forward, caps the year and has no calendar tab stop", async () => {
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    let values: [string, string] = ["", ""];
    function Fixture() { const [range, setRange] = useState(values); return <><DanbooruDateRange from={range[0]} to={range[1]} onChange={(from, to) => { values = [from, to]; setRange(values); }} /><button>다음 항목</button></>; }
    const type = async (index: number, value: string) => {
      const input = host.querySelectorAll<HTMLInputElement>('input[type="text"]')[index]!;
      await act(async () => { input.focus(); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
    };
    try {
      await act(async () => root.render(<Fixture />));
      const fields = host.querySelectorAll<HTMLInputElement>('input[type="text"]');
      await type(0, "2018"); expect(document.activeElement).toBe(fields[1]);
      await type(1, "10"); expect(document.activeElement).toBe(fields[2]);
      await type(2, "08"); expect(document.activeElement).toBe(fields[3]);
      expect(values[0]).toBe("2018-10-08");
      await type(3, "201888"); expect(fields[3]).toHaveValue("2018"); expect(fields[3]).toHaveAttribute("maxlength", "4");
      expect([...fields].map(field => field.getAttribute("aria-label"))).toEqual(["시작 연도", "시작 월", "시작 일", "종료 연도", "종료 월", "종료 일"]);
      expect(host.querySelectorAll('input[type="date"]')).toHaveLength(2);
      expect(host.querySelectorAll("button")).toHaveLength(3);
      expect([...host.querySelectorAll<HTMLInputElement>('input[type="date"]')].every(field => field.tabIndex === -1)).toBe(true);
      expect([...fields].every(field => field.tabIndex === 0)).toBe(true);
    } finally { await act(async () => root.unmount()); host.remove(); }
  });

  it("opens the native calendar by pointer or Alt+Down and synchronizes a picked date", async () => {
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    let values: [string, string] = ["", ""];
    function Fixture() { const [range, setRange] = useState(values); return <DanbooruDateRange from={range[0]} to={range[1]} onChange={(from, to) => { values = [from, to]; setRange(values); }} />; }
    try {
      await act(async () => root.render(<Fixture />));
      const calendar = host.querySelector<HTMLInputElement>('[aria-label="시작일 달력"]')!;
      let opens = 0; calendar.showPicker = () => { opens += 1; };
      await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="시작일 달력 열기"]')!.click());
      await act(async () => host.querySelector('[aria-label="시작 연도"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true })));
      expect(opens).toBe(2);
      await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(calendar, "2024-02-29"); calendar.dispatchEvent(new Event("input", { bubbles: true })); });
      expect(values[0]).toBe("2024-02-29");
      expect(host.querySelector('[aria-label="시작 연도"]')).toHaveValue("2024");
    } finally { await act(async () => root.unmount()); host.remove(); }
  });

  it("pastes a complete compact date without losing any digits", async () => {
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    let values: [string, string] = ["", ""];
    function Fixture() { const [range, setRange] = useState(values); return <DanbooruDateRange from={range[0]} to={range[1]} onChange={(from, to) => { values = [from, to]; setRange(values); }} />; }
    try {
      await act(async () => root.render(<Fixture />));
      const event = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: { getData: () => "20181008" } });
      await act(async () => host.querySelector("input")!.dispatchEvent(event));
      expect(values[0]).toBe("2018-10-08");
      expect(document.activeElement).toBe(host.querySelector('[aria-label="종료 연도"]'));
      expect(event.defaultPrevented).toBe(true);
    } finally { await act(async () => root.unmount()); host.remove(); }
  });

  it("does not let a partial month's blur overwrite a pasted date", async () => {
    const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
    let values: [string, string] = ["2017-1-02", ""];
    function Fixture() { const [range, setRange] = useState(values); return <DanbooruDateRange from={range[0]} to={range[1]} onChange={(from, to) => { values = [from, to]; setRange(values); }} />; }
    try {
      await act(async () => root.render(<Fixture />));
      const month = host.querySelector<HTMLInputElement>('[aria-label="시작 월"]')!;
      const event = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: { getData: () => "20181008" } });
      await act(async () => { month.focus(); month.dispatchEvent(event); });
      expect(values[0]).toBe("2018-10-08");
      expect(document.activeElement).toBe(host.querySelector('[aria-label="종료 연도"]'));
    } finally { await act(async () => root.unmount()); host.remove(); }
  });
});
