import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { useSettings, type SettingsApi } from "./useSettings";
import type { SettingsSnapshot } from "../api/contracts";

describe("settings startup readiness", () => {
  it("retains a valid snapshot despite a subscription or later save failure", async () => {
    const api: SettingsApi = {
      on: vi.fn().mockRejectedValue(new Error("listener unavailable")),
      settingsGet: vi.fn().mockResolvedValue({ ok: true, data: { revision: 1 } as SettingsSnapshot }),
      settingsUpdate: vi.fn().mockRejectedValue(new Error("save unavailable")),
    };
    const container = document.createElement("div");
    const root = createRoot(container);
    let value!: ReturnType<typeof useSettings>;
    function Probe() { value = useSettings(api); return null; }
    try {
      await act(async () => root.render(<Probe />));
      expect(value.hasSnapshot).toBe(true);
      expect(value.error).not.toBeNull();
      await act(async () => { await value.save({ privacyMode: true }); });
      expect(value.hasSnapshot).toBe(true);
      expect(value.error).not.toBeNull();
    } finally { await act(async () => root.unmount()); }
  });
  it("does not count fallback defaults as a successful settings load", async () => {
    const api: SettingsApi = {
      on: vi.fn().mockResolvedValue(() => undefined),
      settingsGet: vi.fn().mockRejectedValue(new Error("not available")),
      settingsUpdate: vi.fn(),
    };
    const root = createRoot(document.createElement("div"));
    let value!: ReturnType<typeof useSettings>;
    function Probe() { value = useSettings(api); return null; }
    try {
      await act(async () => root.render(<Probe />));
      expect(value.hasSnapshot).toBe(false);
      expect(value.loading).toBe(false);
    } finally { await act(async () => root.unmount()); }
  });
});
