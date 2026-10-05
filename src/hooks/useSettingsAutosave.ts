import { useCallback, useEffect, useRef, useState } from "react";
import type { SettingsPatch } from "../api/contracts";

/** Coalesce sliders, serialize writes, and retain failed edits for an explicit retry. */
export function useSettingsAutosave(onSave: (patch: SettingsPatch) => Promise<boolean>) {
  const save = useRef(onSave); save.current = onSave;
  const pending = useRef<SettingsPatch>({});
  const inFlight = useRef<SettingsPatch>({});
  const flight = useRef<Promise<boolean> | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);
  const [status, setStatus] = useState<"idle" | "pending" | "saving" | "saved" | "error">("idle");
  const flush = useCallback((): Promise<boolean> => {
    clearTimeout(timer.current);
    if (flight.current) return flight.current;
    if (!Object.keys(pending.current).length) return Promise.resolve(true);
    flight.current = Promise.resolve().then(async () => {
      if (mounted.current) setStatus("saving");
      while (Object.keys(pending.current).length) {
        const patch = pending.current;
        pending.current = {};
        inFlight.current = patch;
        let ok = false;
        try { ok = await save.current(patch); } catch { /* Keep the edit available for retry. */ }
        inFlight.current = {};
        if (!ok) {
          pending.current = { ...patch, ...pending.current };
          if (mounted.current) setStatus("error");
          return false;
        }
      }
      if (mounted.current) setStatus("saved");
      return true;
    }).finally(() => { flight.current = null; });
    return flight.current;
  }, []);
  const enqueue = useCallback((patch: SettingsPatch, delay = 0) => {
    pending.current = { ...pending.current, ...patch };
    clearTimeout(timer.current);
    if (mounted.current) setStatus(flight.current ? "saving" : "pending");
    if (delay) timer.current = setTimeout(() => { void flush(); }, delay);
    else void flush();
  }, [flush]);
  const unsaved = useCallback(() => ({ ...inFlight.current, ...pending.current }), []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; clearTimeout(timer.current); void flush(); };
  }, [flush]);
  return { status, enqueue, flush, unsaved };
}
