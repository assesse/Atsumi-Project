import { useCallback, useEffect, useRef, useState } from "react";
import type { SoopAutoChannel, SoopAutoRecordApi, SoopAutoSnapshot } from "../../api/soopAutoRecord";

const initial: SoopAutoSnapshot = { revision: 0, channels: [], error: null, ready: false };
const errorText = (cause: unknown) => typeof cause === "string" ? cause.slice(0, 220) : cause instanceof Error ? cause.message.slice(0, 220) : "녹화 예약을 확인하지 못했습니다.";

/** Display polling never owns the persisted scheduler or recording lifetime. */
export function useSoopAutoRecord(api: SoopAutoRecordApi | null, active: boolean, onNotice?: (message: string) => void) {
  const [snapshot, setSnapshot] = useState<SoopAutoSnapshot>(initial);
  const snapshotRef = useRef(snapshot), epoch = useRef(0), mounted = useRef(false), busyRef = useRef(false);
  const [pending, setPending] = useState<string | null>(null), [error, setError] = useState(""), [refreshError, setRefreshError] = useState("");
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; epoch.current++; }; }, []);
  const accept = useCallback((next: SoopAutoSnapshot, generation: number) => {
    if (!mounted.current || generation !== epoch.current || next.revision < snapshotRef.current.revision) return;
    snapshotRef.current = next; setSnapshot(next);
  }, []);
  useEffect(() => {
    epoch.current++; busyRef.current = false; snapshotRef.current = initial; setSnapshot(initial); setPending(null); setRefreshError(""); setError("");
  }, [api]);
  useEffect(() => {
    if (!api || !active) return;
    let cancelled = false, timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      const generation = epoch.current;
      if (!busyRef.current) {
        try { const next = await api.snapshot(); if (!cancelled && generation === epoch.current) { accept(next, generation); setRefreshError(""); } }
        catch (cause) { if (!cancelled && generation === epoch.current) setRefreshError(errorText(cause)); }
      }
      if (!cancelled) timer = setTimeout(() => void refresh(), 3000);
    };
    void refresh(); return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [api, active, accept]);
  const toggle = async (channelId: string, name: string) => {
    if (!api || busyRef.current || !snapshotRef.current.ready) return;
    const current = snapshotRef.current, enabled = !current.channels.find(channel => channel.channelId === channelId)?.enabled;
    if (enabled && current.error) return;
    const generation = ++epoch.current; busyRef.current = true; setPending(channelId); setError("");
    try {
      const next = await api.set(channelId, name, enabled, current.revision); accept(next, generation);
      if (mounted.current && generation === epoch.current) { setRefreshError(""); onNotice?.(enabled ? "녹화 예약을 켰습니다." : "녹화 예약을 껐습니다."); }
    } catch (cause) {
      if (mounted.current && generation === epoch.current) setError(errorText(cause));
      // A revision conflict refreshes the current choice; it never retries the write.
      try { const next = await api.snapshot(); accept(next, generation); }
      catch (refreshCause) { if (mounted.current && generation === epoch.current) setRefreshError(errorText(refreshCause)); }
    } finally {
      if (mounted.current && generation === epoch.current) { busyRef.current = false; setPending(null); }
    }
  };
  const canToggle = (channelId: string) => !!api && snapshot.ready && !pending && (!snapshot.error || !!snapshot.channels.find(channel => channel.channelId === channelId)?.enabled);
  return { snapshot, pending, error: error || snapshot.error || refreshError, canToggle, toggle };
}

export function autoRecordLabel(channel?: SoopAutoChannel): string {
  if (!channel) return "예약 꺼짐";
  if (channel.status === "starting") return "준비 중";
  if (channel.status === "recording") return "녹화 중";
  if (channel.status === "stopping") return "마무리 중";
  if (channel.status === "waiting") return "자리 대기";
  if (channel.status === "skipped" && channel.enabled) return "이번 방송 건너뜀";
  if (channel.status === "failed") return "확인 필요";
  return channel.enabled ? "녹화 예약 됨" : "예약 꺼짐";
}
