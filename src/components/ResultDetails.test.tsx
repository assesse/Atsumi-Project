import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { ResultDetails } from "./ResultDetails";

describe("ResultDetails", () => {
  it("keeps evidence folded and closes with Escape or outside pointer without affecting its contents", async () => {
    const host = document.createElement("div"); document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => root.render(<ResultDetails count={8977}><p>검사 완료</p><button>검사 취소</button></ResultDetails>));
      const details = host.querySelector("details")!;
      const summary = host.querySelector("summary")!;
      expect(details.open).toBe(false);
      expect(summary).toHaveTextContent("8977개 결과");
      expect(summary).not.toHaveTextContent("검사 완료");
      details.open = true;
      const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      await act(async () => host.querySelector("button")!.dispatchEvent(escape));
      expect(details.open).toBe(false);
      expect(escape.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(summary);
      details.open = true;
      await act(async () => host.querySelector("p")!.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(details.open).toBe(true);
      await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(details.open).toBe(false);
    } finally { await act(async () => root.unmount()); host.remove(); }
  });
});
