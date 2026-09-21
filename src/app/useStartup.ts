import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type StartupSnapshot = { phase: "starting" | "ready" | "failed" | "cancelling"; elapsedMs: number };

/** Marks a committed React frame AND a round-trip through the native UI loop.
 * Settings failure/loading must never be reported as a usable startup. */
export function useStartup(runtime: "tauri" | "browser-mock", settingsReady: boolean) {
  const desktop = runtime === "tauri";
  const [phase, setPhase] = useState<StartupSnapshot["phase"]>(desktop ? "starting" : "ready");
  const [backgroundReady, setBackgroundReady] = useState(!desktop);

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const snapshot = await invoke<StartupSnapshot>("app_startup_snapshot");
        if (cancelled) return;
        setPhase(snapshot.phase);
        if (snapshot.phase === "ready" || snapshot.phase === "failed") return;
      } catch { if (cancelled) return; }
      timer = window.setTimeout(() => void poll(), 200);
    };
    void poll();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [desktop]);

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    let first = 0;
    let second = 0;
    let scheduled = false;
    const start = () => {
      if (scheduled || document.visibilityState === "hidden") return;
      scheduled = true;
      first = requestAnimationFrame(() => {
        second = requestAnimationFrame(() => {
          if (cancelled) return;
          void invoke<StartupSnapshot>("app_startup_frame", { settingsReady: false }).catch(() => undefined);
        });
      });
    };
    start();
    document.addEventListener("visibilitychange", start);
    return () => { cancelled = true; cancelAnimationFrame(first); cancelAnimationFrame(second); document.removeEventListener("visibilitychange", start); };
  }, [desktop]);

  useEffect(() => {
    if (!desktop || !settingsReady || phase !== "ready") return;
    let cancelled = false;
    let first = 0;
    let second = 0;
    let timer: number | undefined;
    let scheduled = false;
    const start = () => {
      if (scheduled || document.visibilityState === "hidden") return;
      scheduled = true;
      first = requestAnimationFrame(() => {
        second = requestAnimationFrame(() => {
          if (cancelled) return;
          void invoke<StartupSnapshot>("app_startup_frame", { settingsReady: true }).then(() => {
            if (!cancelled) timer = window.setTimeout(() => setBackgroundReady(true), 150);
          }).catch(() => {
            // Diagnostics must not disable normal operation if their IPC fails.
            if (!cancelled) setBackgroundReady(true);
          });
        });
      });
    };
    start();
    document.addEventListener("visibilitychange", start);
    return () => { cancelled = true; cancelAnimationFrame(first); cancelAnimationFrame(second); window.clearTimeout(timer); document.removeEventListener("visibilitychange", start); };
  }, [desktop, settingsReady, phase]);

  return {
    phase, backgroundReady,
    cancel: async () => {
      if (desktop && await invoke<boolean>("app_startup_cancel")) setPhase("cancelling");
    },
  };
}
