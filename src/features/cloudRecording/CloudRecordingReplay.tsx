import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cloudErrorText, cloudRecordingReplayApi, type CloudRecordingJob, type CloudSourceCoverage, type CloudRecordingReplayApi, type CloudReplayDescriptor } from '../../api/cloudRecording';
import { createReplayApi, type ReplaySession } from '../../api/replay';
import type { YouTubeVodClock } from '../../api/youtubeOriginal';
import { YouTubeOriginalPlayer, type YouTubeOriginalPlayerHandle } from '../youtube/YouTubeOriginalPlayer';
import { PlatformReplayChat, type ReplayChatApi } from '../platforms/PlatformReplayChat';
import { RecordingReplayChat } from '../streaming/RecordingReplayChat';
import { replayFocusableControls } from '../streaming/RecordingReplayModel';
import { acceptCloudClock, cloudDuration, cloudJobPlayable, cloudReplayNeighbor, cloudReplayTarget, cloudReplayTimeline, cloudVisibilityLabel, type CloudReplayExpectedPart, type CloudReplayPosition, type CloudReplayTarget } from './cloudRecordingModel';
import '../streaming/RecordingReplay.css';

type Transition = { id: number; jobId: string; seconds: number; resume: boolean; rate: number; stage: 'clock' | 'command' | 'position' | 'resume'; baseline: number; observed: YouTubeVodClock | null };
type ObservedPosition = CloudReplayPosition & { clock: YouTubeVodClock };

/** Only the official remote player receives video. One recording lease supplies saved events. */
export function CloudRecordingReplay({ jobs, initialJobId, expectedParts, coverage, active, privacyMode, onClose, cloudApi = cloudRecordingReplayApi }: {
  jobs: CloudRecordingJob[]; initialJobId: string; expectedParts?: CloudReplayExpectedPart[]; coverage?: CloudSourceCoverage; active: boolean; privacyMode: boolean; onClose(): void; cloudApi?: CloudRecordingReplayApi;
}) {
  const initial = jobs.find(value => value.id === initialJobId);
  const anchor = useRef(initial); if (initial) anchor.current = initial;
  const recording = anchor.current;
  const [selectedId, setSelectedId] = useState(initialJobId);
  const selected = useRef(initialJobId);
  const requested = jobs.find(value => value.id === selectedId) ?? initial;
  const cohortKey = requested ? JSON.stringify([requested.source.provider, requested.source.recordingId,
    requested.replayCohortId ?? requested.source.planId ?? requested.id]) : '';
  const [lease, setLease] = useState<{ key: string; descriptor: CloudReplayDescriptor } | null>(null);
  const descriptor = lease?.key === cohortKey ? lease.descriptor : null;
  const manifest = descriptor?.description;
  const timeline = useMemo(() => recording ? cloudReplayTimeline(jobs, recording, manifest?.parts ?? expectedParts, manifest?.coverage ?? coverage) : { units: [], issue: '선택한 보관 영상을 찾을 수 없습니다.', end: 0, tailIssue: null }, [jobs, recording, manifest, expectedParts, coverage]);
  const job = timeline.units.find(unit => unit.job?.id === selectedId)?.job ?? jobs.find(value => value.id === selectedId);
  const replayApi = useMemo(() => createReplayApi('tauri'), []);
  const [session, setSession] = useState<ReplaySession | null>(null), [chatError, setChatError] = useState(''), [error, setError] = useState('');
  const platformChatApi = useMemo<ReplayChatApi | undefined>(() => descriptor && descriptor.provider !== 'chzzk' ? {
    page: (_id, positionMs, cursor, limit) => cloudApi.page(descriptor.token, positionMs, cursor, limit),
    asset: (_id, assetId) => cloudApi.asset(descriptor.token, assetId),
  } : undefined, [cloudApi, descriptor]);
  const [notice, setNotice] = useState(''), [position, setPosition] = useState<ObservedPosition | null>(null), [clockAvailable, setClockAvailable] = useState(false);
  const [transitionRevision, setTransitionRevision] = useState(0), [transitionChecking, setTransitionChecking] = useState(false);
  const previous = useRef<CloudReplayPosition | null>(null), player = useRef<YouTubeOriginalPlayerHandle>(null), lastClock = useRef<YouTubeVodClock | null>(null);
  const closeButton = useRef<HTMLButtonElement>(null), surface = useRef<HTMLDivElement>(null), headingId = useId();
  const lifetime = useRef(0), offsetSaving = useRef(false), transitionId = useRef(0), pending = useRef<Transition | null>(null), alive = useRef(true);
  const waitingAfter = useRef<string | null>(null), rate = useRef(1), playIntent = useRef(true);
  const drive = useRef<(clock: YouTubeVodClock) => void>(() => {});
  const latest = useRef({ active, privacyMode }); latest.current = { active, privacyMode };

  useEffect(() => { alive.current = true; return () => { alive.current = false; transitionId.current++; pending.current = null; }; }, []);
  useEffect(() => {
    selected.current = initialJobId; setSelectedId(initialJobId); pending.current = null; transitionId.current++; waitingAfter.current = null; lastClock.current = null;
    setError(''); setNotice(''); setClockAvailable(false); setTransitionChecking(false);
  }, [initialJobId]);
  useEffect(() => {
    const generation = ++lifetime.current; let gone = false, release: (() => void) | undefined;
    setLease(null); setSession(null); setChatError(''); previous.current = null; setPosition(null); setClockAvailable(false);
    pending.current = null; transitionId.current++; waitingAfter.current = null; lastClock.current = null; setTransitionChecking(false); setNotice('');
    if (!requested) return;
    void cloudApi.open(requested.id).then(value => {
      release = () => { void cloudApi.close(value.provider, value.token).catch(() => {}); };
      if (gone || generation !== lifetime.current) { release(); return; }
      if (value.provider !== requested.source.provider || value.description.source.provider !== requested.source.provider
        || value.description.source.recordingId !== requested.source.recordingId
        || (value.description.planId ?? null) !== (requested.source.planId ?? null)
        || requested.replayCohortId && value.cohortId !== requested.replayCohortId
        || value.provider === 'chzzk' && (!value.session || value.session.token !== value.token || value.session.recordingId !== requested.source.recordingId)) {
        release(); release = undefined; throw new Error('선택한 영상의 채팅 기록을 확인하지 못했습니다. 다시 열어 주세요.');
      }
      setLease({ key: cohortKey, descriptor: value }); setSession(value.session ?? null);
    }).catch(cause => { if (!gone && generation === lifetime.current) setChatError(cloudErrorText(cause)); });
    return () => { gone = true; ++lifetime.current; release?.(); };
  }, [cohortKey, cloudApi]);
  useEffect(() => {
    if (!active) return;
    const old = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton.current?.focus({ preventScroll: true });
    return () => { if (old?.isConnected && !old.closest('[hidden],[inert]')) old.focus({ preventScroll: true }); };
  }, [active]);
  const commit = useCallback((next: CloudReplayPosition, clock: YouTubeVodClock) => {
    const observed: ObservedPosition = { ...next, clock };
    previous.current = observed; setClockAvailable(true);
    setPosition(old => old && (clock.paused || clock.ended ? old.time === next.time : Math.floor(old.time * 5) === Math.floor(next.time * 5)) && old.generation === next.generation
      && old.clock.paused === clock.paused && old.clock.ended === clock.ended ? old : observed);
  }, []);
  const failTransition = useCallback((expected: Transition, message: string) => {
    if (!alive.current || pending.current !== expected || selected.current !== expected.jobId) return;
    pending.current = null; waitingAfter.current = null; setNotice(''); setError(message); setClockAvailable(false); setTransitionChecking(false);
    // A rejected seek/rate must not continue playing an unintended position.
    void player.current?.pause().catch(() => {});
  }, []);
  drive.current = clock => {
    const intent = pending.current, handle = player.current;
    if (!intent || !job || selected.current !== job.id || intent.jobId !== job.id || !handle || !latest.current.active || latest.current.privacyMode) return;
    const accepted = acceptCloudClock(previous.current, clock, job);
    if (!accepted) return;
    intent.observed = clock;
    const current = () => alive.current && pending.current === intent && selected.current === intent.jobId;
    if (intent.stage === 'clock') {
      intent.stage = 'command'; intent.baseline = clock.sequence;
      void (async () => {
        if (!clock.paused) await handle.pause();
        if (!current()) return;
        if (Math.abs(clock.playbackRate - intent.rate) > .000001) await handle.setRate(intent.rate);
        if (!current()) return;
        await handle.seek(Math.min(intent.seconds, clock.duration!));
        if (!current()) return;
        intent.stage = 'position'; setTransitionChecking(true); if (intent.observed) drive.current(intent.observed);
      })().catch(() => failTransition(intent, '영상 위치나 재생속도를 적용하지 못했습니다. 해당 구간을 다시 선택해 주세요.'));
    } else if (intent.stage === 'position' && clock.sequence > intent.baseline
      && Math.abs(clock.currentTime! - intent.seconds) <= .75 && Math.abs(clock.playbackRate - intent.rate) <= .000001) {
      // The command acknowledgement alone is insufficient. Only this observed
      // new content position/rate may advance saved chat or restart playback.
      commit(accepted, clock); rate.current = clock.playbackRate;
      if (!intent.resume) { pending.current = null; playIntent.current = false; setNotice(''); setTransitionChecking(false); return; }
      intent.stage = 'resume';
      void handle.resume().then(() => { if (current()) { pending.current = null; playIntent.current = true; setNotice(''); setTransitionChecking(false); } })
        .catch(() => failTransition(intent, '다음 구간의 재생을 시작하지 못했습니다. 공식 플레이어에서 재생해 주세요.'));
    }
  };
  const start = useCallback((target: CloudReplayTarget, resume: boolean) => {
    if (!latest.current.active || latest.current.privacyMode) return;
    if (target.kind === 'blocked') { setError(target.reason); return; }
    if (target.kind === 'end') return;
    const oldId = selected.current;
    const intent: Transition = { id: ++transitionId.current, jobId: target.job.id, seconds: target.seconds, resume, rate: rate.current, stage: 'clock', baseline: 0, observed: null };
    pending.current = intent; waitingAfter.current = null; selected.current = target.job.id; setSelectedId(target.job.id);
    setError(''); setNotice('영상 위치를 연결하고 있습니다…'); setClockAvailable(false); setTransitionChecking(false); setTransitionRevision(intent.id);
    if (oldId === target.job.id && lastClock.current) drive.current(lastClock.current); else lastClock.current = null;
  }, []);
  const advance = useCallback(() => {
    if (!job || selected.current !== job.id || pending.current || !latest.current.active || latest.current.privacyMode || !playIntent.current) return;
    const target = cloudReplayNeighbor(timeline, job.id, 1);
    if (target.kind === 'blocked') { waitingAfter.current = job.id; setNotice(target.reason); return; }
    waitingAfter.current = null;
    if (target.kind === 'ready') start(target, true);
    else setNotice(''); // Final EOF remains on the original replayable player.
  }, [job, timeline, start]);
  const onClock = useCallback((clock: YouTubeVodClock | null) => {
    if (!job || selected.current !== job.id) return;
    if (!clock) { setClockAvailable(false); setTransitionChecking(false); return; }
    const next = acceptCloudClock(previous.current, clock, job);
    if (!next) { setClockAvailable(false); setTransitionChecking(false); return; }
    lastClock.current = clock;
    if (pending.current) { if (pending.current.stage === 'position') setTransitionChecking(true); drive.current(clock); return; }
    commit(next, clock); rate.current = clock.playbackRate;
    if (!clock.ended) {
      playIntent.current = !clock.paused;
      if (waitingAfter.current) { waitingAfter.current = null; setNotice(''); }
    } else advance();
  }, [job, commit, advance]);
  useEffect(() => { if (waitingAfter.current === selected.current && lastClock.current?.ended && !pending.current) advance(); }, [timeline, active, privacyMode, advance]);
  useEffect(() => {
    if (!transitionChecking || !notice || !pending.current || !active || privacyMode) return;
    const expected = pending.current;
    const timer = setTimeout(() => failTransition(expected, '새 구간의 재생 위치나 재생속도를 확인하지 못했습니다. 공식 플레이어 상태를 확인해 주세요.'), 15000);
    return () => clearTimeout(timer);
  }, [transitionChecking, transitionRevision, notice, selectedId, active, privacyMode, failTransition]);
  const seek = useCallback((target: number) => start(cloudReplayTarget(timeline, target), playIntent.current), [timeline, start]);
  const saveOffset = useCallback(async (value: number) => {
    if (!session || offsetSaving.current) return '채팅 보정을 저장 중입니다.';
    if (!Number.isFinite(value) || Math.abs(value) > 3600) return '채팅 보정은 -3600~3600초로 입력해 주세요.';
    offsetSaving.current = true; const expected = lifetime.current;
    try {
      const result = await replayApi.setOffset(session.token, value);
      if (expected !== lifetime.current) return '다시보기가 종료됐습니다.';
      if (!result.ok) return result.error.message;
      setSession(old => old && { ...old, manualOffsetSeconds: result.data });
      if (previous.current) { const generation = previous.current.generation + 1; previous.current = { ...previous.current, generation }; setPosition(old => old && { ...old, generation }); }
      return null;
    } catch { return '채팅 보정을 저장하지 못했습니다.'; }
    finally { offsetSaving.current = false; }
  }, [replayApi, session]);
  const unitIndex = timeline.units.findIndex(unit => unit.job?.id === selectedId);
  return createPortal(<div className="cloud-recording-replay-backdrop" hidden={!active}>
    <div ref={surface} className="cloud-recording-replay" role="dialog" aria-modal="true" aria-labelledby={headingId} tabIndex={-1} onKeyDown={event => {
      event.stopPropagation();
      if (event.defaultPrevented || event.nativeEvent.isComposing) return;
      if (event.key === 'Escape' && !surface.current?.querySelector('[data-platform-native-fullscreen="true"]')) { event.preventDefault(); onClose(); }
      if (event.key === 'Tab') {
        const controls = replayFocusableControls(event.currentTarget), first = controls[0], last = controls.at(-1), target = event.nativeEvent.composedPath()[0];
        if (first && (event.shiftKey && target === first || !event.shiftKey && target === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); }
      }
    }}>
      <header className="cloud-recording-replay-heading"><h2 id={headingId}>{privacyMode ? '프라이버시 모드' : recording?.title || 'YouTube 다시보기'}</h2>
        {timeline.units.length > 1 ? <nav className="cloud-recording-unit-navigation" aria-label="녹화 구간">
          <button type="button" aria-label="이전 녹화 구간" disabled={privacyMode || !!pending.current || unitIndex <= 0} onClick={() => start(cloudReplayNeighbor(timeline, selectedId, -1), playIntent.current)}>이전</button>
          <output aria-label="전체 녹화 재생 위치">{unitIndex + 1}/{timeline.units.length} · {cloudDuration(position?.time ?? 0)} / {cloudDuration(timeline.end)}</output>
          <button type="button" aria-label="다음 녹화 구간" disabled={privacyMode || !!pending.current || unitIndex < 0 || unitIndex >= timeline.units.length - 1} onClick={() => start(cloudReplayNeighbor(timeline, selectedId, 1), playIntent.current)}>다음</button>
        </nav> : null}
        <span aria-label="재생 영상 공개 범위">{job ? cloudVisibilityLabel(job) : '공개 범위 확인 필요'}</span>
        <button ref={closeButton} type="button" className="cloud-recording-close" aria-label="YouTube 다시보기 닫기" onClick={onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" /></svg></button></header>
      {!privacyMode && (manifest?.coverage?.captureWarning ?? coverage?.captureWarning) ? <p role="status" className="cloud-recording-error cloud-recording-capture-warning">{manifest?.coverage?.captureWarning ?? coverage?.captureWarning}</p> : null}
      <div className="cloud-recording-replay-body">
        {job && cloudJobPlayable(job) && !timeline.issue ? <YouTubeOriginalPlayer key={`${job.id}:${cohortKey}`} ref={player} channelId={job.videoId!} title={job.title} playbackKind="vod" allowAccountRecovery active={active} privacyMode={privacyMode} onClock={onClock} onClose={onClose} /> : <p role="alert" className="cloud-recording-error">{timeline.issue || '선택한 영상이 아직 준비되지 않았습니다.'}</p>}
        <div className="cloud-recording-chat" hidden={privacyMode}>
          {descriptor?.provider === 'chzzk' && session && position ? <RecordingReplayChat key={descriptor.token} api={replayApi} session={session} time={position.time} seekVersion={position.generation} onSeek={seek}
            onIndexReady={quality => setSession(old => old?.token === descriptor.token && (old.indexState !== 'ready' || old.syncQuality !== quality) ? { ...old, indexState: 'ready', syncQuality: quality } : old)} onSetOffset={saveOffset} /> : null}
          {descriptor && descriptor.provider !== 'chzzk' && platformChatApi && position ? <PlatformReplayChat key={descriptor.token} provider={descriptor.provider} libraryId={descriptor.token} currentTime={position.time} paused={position.clock.paused} ended={position.clock.ended} seekVersion={position.generation} active={active} privacyMode={privacyMode} api={platformChatApi} /> : null}
          {!position && !chatError && !error && !notice ? <p role="status">영상이 재생되면 저장된 채팅이 표시됩니다.</p> : null}
          {chatError || error ? <p role="alert" className="cloud-recording-error">{chatError || error}</p> : null}
          {notice ? <p role="status">{notice}</p> : position && !clockAvailable ? <span className="cloud-recording-clock-wait" role="status">영상 대기 중</span> : null}
        </div>
      </div>
    </div>
  </div>, document.body);
}
