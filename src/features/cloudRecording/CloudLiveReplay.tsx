import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cloudErrorText, type CloudLiveOperation } from '../../api/cloudRecording';
import { cloudLiveReplayApi, liveReplayPlayerApi, type CloudLiveReplayApi, type LiveReplayDescriptor, type LiveReplayPage, type LiveReplaySnapshot } from '../../api/cloudLiveReplay';
import type { YouTubeVodClock } from '../../api/youtubeOriginal';
import { YouTubeOriginalPlayer, type YouTubeOriginalPlayerHandle } from '../youtube/YouTubeOriginalPlayer';
import { PlatformReplayChat, type ReplayChatApi } from '../platforms/PlatformReplayChat';
import { LiveReplayChzzkChat } from '../streaming/RecordingReplayChat';
import { replayFocusableControls } from '../streaming/RecordingReplayModel';
import { canReplayLiveArchive } from './cloudLiveReplayModel';
import './CloudRecording.css';
import './CloudLiveReplay.css';

type Mode = 'current' | 'history' | 'untimed';
function generation(clock: YouTubeVodClock | null | undefined) {
  return clock ? JSON.stringify([clock.documentEpoch, clock.sourceId, clock.sourceGeneration, clock.seekGeneration]) : '';
}
export function sameLivePageClock(page: LiveReplayPage, clock: YouTubeVodClock | null | undefined) {
  return !page.cursorGeneration || generation(page.cursorGeneration as YouTubeVodClock) === generation(clock);
}

/** The player owns the lease. Chat reads have no renderer-supplied playback time. */
export function CloudLiveReplay({ operation, title, active, privacyMode, onClose, api = cloudLiveReplayApi }: {
  operation: CloudLiveOperation; title: string; active: boolean; privacyMode: boolean; onClose(): void; api?: CloudLiveReplayApi;
}) {
  const [opened, setOpened] = useState<LiveReplayDescriptor | null>(null), [status, setStatus] = useState<LiveReplaySnapshot | null>(null);
  const [clock, setClock] = useState<YouTubeVodClock | null>(null), [mode, setMode] = useState<Mode>('history');
  const [page, setPage] = useState<LiveReplayPage | null>(null), [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState(''), [measuring, setMeasuring] = useState(false), [retry, setRetry] = useState(0);
  const latest = useRef({ active, privacyMode, clock }); latest.current = { active, privacyMode, clock };
  const alive = useRef(true), player = useRef<YouTubeOriginalPlayerHandle>(null), closeButton = useRef<HTMLButtonElement>(null), surface = useRef<HTMLDivElement>(null);
  const heading = useId();
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const observed = useCallback((value: LiveReplayDescriptor | LiveReplaySnapshot) => {
    if (!alive.current) return;
    setOpened(previous => previous?.token === value.token ? previous : value);
    if ('alignmentState' in value) setStatus(previous => previous?.token === value.token && previous.alignmentState === value.alignmentState
      && previous.historyState === value.historyState && previous.historyItems === value.historyItems && previous.untimedItems === value.untimedItems
      && previous.historyIncomplete === value.historyIncomplete && previous.error?.code === value.error?.code ? previous : value);
  }, []);
  const playerApi = useMemo(() => liveReplayPlayerApi(api, operation.selectionId, operation.revision, observed), [api, operation.selectionId, operation.revision, observed]);
  const onClock = useCallback((value: YouTubeVodClock | null) => {
    latest.current.clock = value;
    // A buffering/ad/stale observation must not rewind the chat or replace
    // the reader's history page. Native page admission still uses fresh time.
    if (!value) return;
    setClock(previous => generation(previous) === generation(value) && previous?.paused === value?.paused && previous?.ended === value?.ended
      && previous?.valid === value?.valid && Math.floor((previous?.currentTime ?? -1) * 5) === Math.floor((value?.currentTime ?? -1) * 5) ? previous : value);
  }, []);
  const clockGeneration = generation(clock);
  const effectiveMode: Mode = mode;
  useEffect(() => { setCursor(null); setPage(null); setError(''); }, [opened?.token, effectiveMode, clockGeneration]);
  const readPage = useCallback(async (token: string, requestedCursor: string | null, limit: number) => {
    const value = await api.page(token, effectiveMode, requestedCursor, limit);
    if (value.provider !== operation.provider || !sameLivePageClock(value, latest.current.clock)) throw new Error('재생 위치가 바뀌었습니다. 기록을 다시 불러와 주세요.');
    return value;
  }, [api, effectiveMode, operation.provider]);
  useEffect(() => {
    if (!opened || operation.provider !== 'chzzk' || !active || privacyMode) return;
    let gone = false, timer: ReturnType<typeof setTimeout>;
    const read = async () => {
      try { const value = await readPage(opened.token, cursor, 120); if (!gone) { setPage(value); setError(''); } }
      catch (cause) { if (!gone) setError(cloudErrorText(cause)); }
      if (!gone && (effectiveMode === 'current' || status?.historyState === 'preparing')) timer = setTimeout(() => void read(), 750);
    };
    void read(); return () => { gone = true; clearTimeout(timer); };
  }, [opened, operation.provider, active, privacyMode, readPage, cursor, retry, effectiveMode, clockGeneration, status?.historyState]);
  const soopApi = useMemo<ReplayChatApi>(() => ({
    page: async (token, _positionMs, requestedCursor, limit) => {
      const value = await readPage(token, effectiveMode === 'current' ? null : requestedCursor, limit);
      if (value.provider !== 'soop') throw new Error('선택한 방송의 채팅이 아닙니다.');
      return value.page;
    },
    asset: async (token, id) => { const dataUrl = await api.asset(token, id); return { state: dataUrl ? 'ready' : 'missing', dataUrl }; },
  }), [api, readPage, effectiveMode]);
  useEffect(() => {
    if (!active) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton.current?.focus({ preventScroll: true });
    return () => { if (previous?.isConnected && !previous.closest('[hidden],[inert]')) previous.focus({ preventScroll: true }); };
  }, [active]);
  const measure = async () => {
    if (!opened || measuring || !active || privacyMode) return;
    setMeasuring(true); setMode('current'); setCursor(null); setError('');
    try { observed(await api.measure(opened.token)); }
    catch (cause) { if (alive.current) setError(cloudErrorText(cause)); }
    finally { if (alive.current) setMeasuring(false); }
  };
  const seek = (seconds: number) => { if (latest.current.active && !latest.current.privacyMode && Number.isFinite(seconds) && seconds >= 0) void player.current?.seek(seconds).catch(cause => setError(cloudErrorText(cause))); };
  const asset = useCallback((token: string, id: string) => api.asset(token, id), [api]);
  const allowed = canReplayLiveArchive(operation), chzzkPage = page?.provider === 'chzzk' ? page.page : null;
  return createPortal(<div className="cloud-recording-replay-backdrop" hidden={!active}>
    <div ref={surface} className="cloud-recording-replay cloud-live-replay" role="dialog" aria-modal="true" aria-labelledby={heading} tabIndex={-1} onKeyDown={event => {
      event.stopPropagation(); if (event.defaultPrevented || event.nativeEvent.isComposing) return;
      if (event.key === 'Escape' && !surface.current?.querySelector('[data-platform-native-fullscreen="true"]')) { event.preventDefault(); onClose(); }
      if (event.key === 'Tab') { const controls = replayFocusableControls(event.currentTarget), first = controls[0], last = controls.at(-1), target = event.nativeEvent.composedPath()[0];
        if (first && (event.shiftKey && target === first || !event.shiftKey && target === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); } }
    }}>
      <header className="cloud-recording-replay-heading"><h2 id={heading}>{privacyMode ? '프라이버시 모드' : title}</h2>
        <span>실시간 보관 영상</span><button ref={closeButton} type="button" className="cloud-recording-close" aria-label="보관 영상 닫기" onClick={onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" /></svg></button></header>
      <div className="cloud-recording-replay-body">
        {allowed ? <YouTubeOriginalPlayer ref={player} channelId={operation.archiveVerification!.videoId!} title={title} playbackKind="vod" api={playerApi}
          allowAccountRecovery active={active} privacyMode={privacyMode} onClock={onClock} onClose={onClose} /> : <p role="alert">YouTube 영상 처리가 끝난 뒤 다시 열어 주세요.</p>}
        <div className="cloud-recording-chat" hidden={privacyMode}>
          {!privacyMode ? <><div className="cloud-live-replay-tools">
            <span role="status">{status?.alignmentState === 'measured' ? '장면 기준 동기화' : status?.alignmentState === 'measuring' ? '재생 장면 비교 중…' : '동기화 확인 전'}</span>
            <button type="button" disabled={!opened || measuring || status?.alignmentState === 'measuring' || !active} onClick={() => void measure()}>동기화 확인</button>
            <nav aria-label="보관 채팅 표시">{([['current', '재생 위치'], ['history', '전체 기록'], ['untimed', '시각 미확인']] as const).map(([value, label]) =>
              <button key={value} type="button" aria-pressed={mode === value} onClick={() => { setMode(value); setCursor(null); }}>{label}</button>)}</nav>
            {status?.historyIncomplete ? <span role="status">일부 기록만 표시됨 · 원본 보존</span> : null}
            {error || status?.error ? <p role="alert">{error || status?.error?.message}<button type="button" onClick={() => { setError(''); setRetry(value => value + 1); }}>다시 시도</button></p> : null}
          </div>
          {opened && operation.provider === 'chzzk' ? <><LiveReplayChzzkChat token={opened.token} items={chzzkPage?.items ?? []} asset={asset} active={active} privacyMode={privacyMode} onSeek={seek} />
            {effectiveMode !== 'current' ? <nav className="cloud-live-replay-pages" aria-label="채팅 기록 페이지"><button type="button" disabled={!chzzkPage?.previousCursor} onClick={() => setCursor(chzzkPage?.previousCursor ?? null)}>이전 기록</button><button type="button" disabled={!chzzkPage?.cursor} onClick={() => setCursor(chzzkPage?.cursor ?? null)}>다음 기록</button></nav> : null}</> : null}
          {opened && operation.provider === 'soop' ? <PlatformReplayChat key={`${opened.token}:${effectiveMode}:${clockGeneration}:${retry}`} provider="soop" libraryId={opened.token} api={soopApi}
            currentTime={clock?.currentTime ?? 0} paused={clock?.paused} ended={clock?.ended} seekVersion={clock?.seekGeneration} active={active} privacyMode={privacyMode} /> : null}
          </> : null}
        </div>
      </div>
    </div>
  </div>, document.body);
}
