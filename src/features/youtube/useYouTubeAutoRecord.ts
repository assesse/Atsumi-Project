import { useEffect, useMemo, useRef, useState } from 'react';
import type { YouTubeAutoChannel, YouTubeAutoRecordApi, YouTubeAutoSnapshot } from '../../api/youtubeAutoRecord';
import { targetKey, type YouTubeMetadata } from './model';

const idPattern = /^UC[A-Za-z0-9_-]{22}$/;
const directId = (item: YouTubeMetadata) => item.kind === 'channel' && item.id.startsWith('channel/') && idPattern.test(item.id.slice(8)) ? item.id.slice(8) : null;
const message = (cause: unknown) => typeof cause === 'string' ? cause : cause instanceof Error ? cause.message : '녹화 예약을 변경하지 못했습니다.';
export function autoRecordLabel(row?: YouTubeAutoChannel) {
  if (!row?.enabled && !['recording', 'stopping'].includes(row?.status ?? '')) return '예약 꺼짐';
  return ({ starting: '녹화 준비', recording: row && row.recordingCount > 1 ? `${row.recordingCount}개 녹화 중` : '녹화 중', stopping: '마무리 중', waiting: '자리 대기', skipped: '이번 방송 건너뜀', failed: '확인 필요', idle: '녹화 예약 됨' })[row?.status ?? 'idle'];
}

export function useYouTubeAutoRecord(api: YouTubeAutoRecordApi | undefined, active: boolean, favorites: YouTubeMetadata[]) {
  const [snapshot, setSnapshot] = useState<YouTubeAutoSnapshot | null>(null);
  const [aliases, setAliases] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(''), [error, setError] = useState(''), [pollError, setPollError] = useState('');
  const generation = useRef(0), operation = useRef(0), mutation = useRef(false), latest = useRef(snapshot);
  const aliasesRef = useRef(aliases); aliasesRef.current = aliases;
  latest.current = snapshot;
  useEffect(() => { latest.current = null; setSnapshot(null); setAliases({}); setError(''); setPollError(''); }, [api]);
  useEffect(() => {
    const current = ++generation.current; mutation.current = false; setBusy('');
    if (!active || !api) return;
    let pending = false;
    const refresh = () => {
      if (pending || mutation.current) return;
      pending = true; const serial = operation.current;
      void api.snapshot().then(value => { if (current === generation.current && serial === operation.current) { setSnapshot(old => !old || value.revision >= old.revision ? value : old); setPollError(''); } })
        .catch(cause => { if (current === generation.current && serial === operation.current) setPollError(message(cause)); })
        .finally(() => { pending = false; });
    };
    refresh(); const timer = setInterval(refresh, 2000);
    return () => { ++generation.current; clearInterval(timer); };
  }, [active, api]);
  // Resolve old handle-form favorites only when there are native reservations
  // to reconcile. Two bounded metadata requests, no repeated portrait fetching.
  const candidates = favorites.filter(item => item.kind === 'channel' && !directId(item)).slice(0, 100);
  const aliasKey = candidates.map(item => item.url).join('\n');
  const reconcile = !!snapshot?.channels.some(row => row.enabled);
  useEffect(() => {
    if (!active || !api || !reconcile || !candidates.length) return;
    let cancelled = false, next = 0;
    const worker = async () => { while (!cancelled && next < candidates.length) {
      const item = candidates[next++]!;
      if (aliasesRef.current[targetKey(item)]) continue;
      try { const value = await api.resolve(item.url); if (!cancelled && idPattern.test(value.channelId)) setAliases(old => ({ ...old, [targetKey(item)]: value.channelId })); } catch { /* Explicit toggle reports a failed resolution; no false identity. */ }
    } };
    void worker(); void worker(); return () => { cancelled = true; };
  }, [active, api, aliasKey, reconcile]);
  const idFor = (item: YouTubeMetadata) => directId(item) ?? aliases[targetKey(item)];
  const rowFor = (item: YouTubeMetadata) => snapshot?.channels.find(row => row.channelId === idFor(item));
  const rows = useMemo(() => {
    const known = new Set(favorites.map(item => directId(item) ?? aliases[targetKey(item)]).filter(Boolean));
    return [...favorites, ...(snapshot?.channels.filter(row => row.enabled && !known.has(row.channelId)) ?? []).map(row => ({ kind: 'channel' as const, id: `channel/${row.channelId}`, url: `https://www.youtube.com/channel/${row.channelId}`, title: row.name, author: '' }))];
  }, [favorites, snapshot?.channels, aliases]);
  const toggle = async (item: YouTubeMetadata) => {
    if (!active || !api || mutation.current || !latest.current?.ready || item.kind !== 'channel') return;
    if (latest.current.error && !rowFor(item)?.enabled) return;
    mutation.current = true; ++operation.current; const current = generation.current;
    setBusy(targetKey(item)); setError('');
    try {
      const prior = rowFor(item);
      const identity = prior?.enabled ? { channelId: prior.channelId, name: prior.name } : await api.resolve(item.url);
      if (current !== generation.current) return;
      if (!idPattern.test(identity.channelId)) throw new Error('채널 식별자를 확인하지 못했습니다.');
      // A handle may resolve to an existing canonical reservation while its
      // background alias check is still pending. Toggle the verified identity.
      const enabled = !latest.current!.channels.find(row => row.channelId === identity.channelId)?.enabled;
      if (enabled && latest.current!.error) throw new Error(latest.current!.error!);
      setAliases(old => ({ ...old, [targetKey(item)]: identity.channelId }));
      const revision = latest.current!.revision;
      const value = await api.set(identity.channelId, identity.name, enabled, revision);
      if (current === generation.current) setSnapshot(value);
    } catch (cause) {
      if (current === generation.current) { setError(message(cause)); try { const value = await api.snapshot(); if (current === generation.current) setSnapshot(value); } catch { /* Keep the explicit mutation error. */ } }
    } finally { if (current === generation.current) { mutation.current = false; setBusy(''); } }
  };
  const canToggle = (item: YouTubeMetadata) => active && !!api && item.kind === 'channel' && snapshot?.ready === true && !busy && (!snapshot.error || !!rowFor(item)?.enabled);
  return { rows, rowFor, idFor, toggle, busy, canToggle, error: error || snapshot?.error || pollError };
}
