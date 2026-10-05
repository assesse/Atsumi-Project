import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, it, expect, vi } from "vitest";
import { ConnectionSetup } from "./ConnectionSetup";

const id = "fedb21a1e1d5f9862eb47b15c69beadd";
describe("channel address drafts", () => {
  it.each(["general", "mado"] as const)("normalizes pasted channel formats in %s without connecting", async (mode) => {
    const container = document.createElement("div"), root = createRoot(container);
    const onInput = vi.fn(), onConnect = vi.fn();
    try {
      await act(async () => root.render(<ConnectionSetup mode={mode} onMode={vi.fn()} modeDisabled={false} inputs={["old draft", "", "", ""]} onInput={onInput} disabled={false} pending={false} onConnect={onConnect}
        auth="unknown" accountDisabled={false} onLogin={vi.fn()} onLogout={vi.fn()} onGrid={vi.fn()} gridDisabled={false} gridStatus="" onInstaller={vi.fn()} installerDisabled={false} />));
      const inputs = [...container.querySelectorAll<HTMLInputElement>(".connection-channel-inputs input")];
      for (const [index, input] of inputs.entries()) for (const text of [`https://chzzk.naver.com/live/${id}`, `https://chzzk.naver.com/${id}`, id.toUpperCase()]) {
        const event = new Event("paste", { bubbles: true, cancelable: true });
        Object.defineProperty(event, "clipboardData", { value: { getData: () => ` ${text} ` } });
        await act(async () => input.dispatchEvent(event));
        expect(event.defaultPrevented).toBe(true);
        expect(onInput).toHaveBeenLastCalledWith(index, id);
      }
      expect(onConnect).not.toHaveBeenCalled();
      onInput.mockClear();
      const invalid = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(invalid, "clipboardData", { value: { getData: () => `https://chzzk.naver.com.evil.test/${id}` } });
      await act(async () => inputs[0]!.dispatchEvent(invalid));
      expect(invalid.defaultPrevented).toBe(false); expect(onInput).not.toHaveBeenCalled();
    } finally { await act(async () => root.unmount()); }
  });
});
