import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { useSettings, type SettingsApi } from "./useSettings";
import type { SettingsSnapshot } from "../api/contracts";

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => vi.unstubAllGlobals());

describe("settings startup readiness", () => {
  it("uses the newest revision for rapid edits and retries a conflicting partial update", async () => {
    let snapshot = { revision: 1, privacyMode: false, previewWidth: 220, explorePageSize: 50 } as SettingsSnapshot;
    let listener!: (snapshot: SettingsSnapshot) => void;
    const api: SettingsApi = {
      on: vi.fn(async (_event, callback) => { listener = callback; return () => {}; }),
      settingsGet: vi.fn<SettingsApi["settingsGet"]>(async () => ({ ok: true, data: snapshot })),
      settingsUpdate: vi.fn<SettingsApi["settingsUpdate"]>(async (patch, revision) => {
        if (revision !== snapshot.revision) return { ok: false, error: { code: "REVISION_CONFLICT", message: "conflict", retryable: true } };
        snapshot = { ...snapshot, ...patch, revision: snapshot.revision + 1 };
        return { ok: true, data: snapshot };
      }),
    };
    const root = createRoot(document.createElement("div"));
    let value!: ReturnType<typeof useSettings>;
    function Probe() { value = useSettings(api); return null; }
    try {
      await act(async () => root.render(<Probe />));
      await act(async () => { await Promise.all([value.save({ previewWidth: 280 }), value.save({ explorePageSize: 80 })]); });
      expect(api.settingsUpdate).toHaveBeenNthCalledWith(1, { previewWidth: 280 }, 1);
      expect(api.settingsUpdate).toHaveBeenNthCalledWith(2, { explorePageSize: 80 }, 2);
      snapshot = { ...snapshot, revision: 4, privacyMode: true }; // Another preference writer.
      await act(async () => { expect((await value.save({ explorePageSize: 90 })).ok).toBe(true); });
      expect(api.settingsUpdate).toHaveBeenLastCalledWith({ explorePageSize: 90 }, 4);
      expect(value.settings).toMatchObject({ revision: 5, privacyMode: true, previewWidth: 280, explorePageSize: 90 });
      await act(async () => listener({ ...snapshot, revision: 2, privacyMode: false }));
      expect(value.settings.revision).toBe(5);
      expect(value.settings.privacyMode).toBe(true);
    } finally { await act(async () => root.unmount()); }
  });
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
