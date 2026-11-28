import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cloudErrorText, cloudRecordingApi, cloudRecordingReplayApi, type CloudPreparationProgress, type CloudRecordingApi, type CloudRecordingReplayApi, type CloudRecordingJob, type CloudSnapshot, type CloudSourceDescription, type CloudSourceRef, type CloudVisibility } from '../../api/cloudRecording';
import { CloudRecordingReplay } from './CloudRecordingReplay';
import { CloudAutomaticChoice, useCloudAutomatic } from './CloudAutoUploadControls';
import { cloudDuration, cloudJobCanPause, cloudJobCanResume, cloudJobPlayable, cloudProgress, cloudSize, cloudStateLabel, cloudVisibilityLabel, sameCloudPart, sameCloudRecording } from './cloudRecordingModel';
import './CloudRecording.css';

const preparationLabels: Record<CloudPreparationProgress['phase'], string> = { checking: '저장 구간 확인 중', preparing: '업로드 영상 준비 중', verifying: '준비된 영상 검증 중' };
function preparationMeter(progress: CloudPreparationProgress) {
  const seconds = Number.isFinite(progress.totalSeconds) && progress.totalSeconds! > 0;
  const total = seconds ? progress.totalSeconds : progress.totalBytes;
  const done = seconds ? progress.completedSeconds : progress.completedBytes;
  if (!Number.isFinite(total) || total! <= 0 || !Number.isFinite(done) || done! < 0) return { value: undefined, text: '' };
  const completed = Math.min(total!, done!);
  return { value: completed / total! * 100, text: seconds ? `${cloudDuration(completed)} / ${cloudDuration(total!)}` : `${cloudSize(completed)} / ${cloudSize(total!)}` };
}
function PreparationMeter({ progress }: { progress: CloudPreparationProgress }) {
  const meter = preparationMeter(progress);
  return <><progress max={100} value={meter.value} aria-label="업로드 준비 진행" />{meter.text ? <small>{meter.text}</small> : null}</>;
}

export function CloudRecordingDialog({ source, title, active = true, privacyMode, pendingRecording = false, allowAutomatic = true, onClose, api = cloudRecordingApi, replayApi = cloudRecordingReplayApi }: {
  source: CloudSourceRef; title: string; active?: boolean; privacyMode: boolean; pendingRecording?: boolean; allowAutomatic?: boolean; onClose(): void; api?: CloudRecordingApi; replayApi?: CloudRecordingReplayApi;
}) {
  const [snapshot, setSnapshot] = useState<CloudSnapshot | null>(null), [description, setDescription] = useState<CloudSourceDescription | null>(null);
  const [error, setError] = useState(''), [pollError, setPollError] = useState(''), [sourceError, setSourceError] = useState(''), [busy, setBusy] = useState(false);
  const [playing, setPlaying] = useState<CloudRecordingJob | null>(null);
  const [visibility, setVisibility] = useState<CloudVisibility>('private');
  const [playbackDescription, setPlaybackDescription] = useState<CloudSourceDescription | null>(null);
  const [sourceRefresh, setSourceRefresh] = useState(0), [sourceLoading, setSourceLoading] = useState(false);
  const [pickerCompletion, setPickerCompletion] = useState(0);
  const pickerFocus = useRef<{ target: HTMLButtonElement; sourceKey: string; completed: boolean } | null>(null);
  const revision = useRef(0), mounted = useRef(true), inFlight = useRef(false), acting = useRef(false);
  const closeButton = useRef<HTMLButtonElement>(null), panel = useRef<HTMLDivElement>(null), headingId = useId();
  const sourceKey = `${source.provider}:${source.recordingId}`;
  const automatic = useCloudAutomatic(sourceKey, active && allowAutomatic && source.provider !== 'youtube', privacyMode, api, { snapshot, onSnapshot: setSnapshot });
  const currentSourceKey = useRef(sourceKey); currentSourceKey.current = sourceKey;
  const jobs = (snapshot?.jobs ?? []).filter(job => sameCloudRecording(job.source, source)).sort((a, b) => a.mediaOffsetSeconds - b.mediaOffsetSeconds || a.createdAt - b.createdAt);
  const latestPlanId = [...jobs].sort((a, b) => b.createdAt - a.createdAt).find(job => job.source.planId)?.source.planId;
  const readyLegacyId = !latestPlanId ? [...jobs].sort((a, b) => b.createdAt - a.createdAt).find(job => cloudJobPlayable(job))?.id : undefined;
  const snapshotReady = snapshot !== null;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; revision.current++; }; }, []);
  useEffect(() => {
    setDescription(null); setSourceError(''); setPlaying(null); setPlaybackDescription(null);
    setVisibility('private');
  }, [sourceKey]);
  useEffect(() => {
    if (pendingRecording) { setDescription(null); setSourceError(''); setSourceLoading(false); return; }
    // Resolve an existing remote receipt before considering local upload input.
    // A first-render local describe could otherwise open/recover removed video.
    if (!snapshotReady) { setSourceLoading(true); return; }
    let gone = false; setSourceLoading(true);
    const request = latestPlanId ? api.getPlan(latestPlanId) : readyLegacyId ? replayApi.describe(readyLegacyId) : api.describe({ provider: source.provider, recordingId: source.recordingId });
    void request.then(value => {
      if (gone) return;
      if (!sameCloudRecording(value.source, source) || latestPlanId && value.planId !== latestPlanId) throw new Error('이 녹화의 업로드 구간을 확인하지 못했습니다. 다시 확인해 주세요.');
      setDescription(value); setSourceError('');
    }).catch(cause => { if (!gone) setSourceError(cloudErrorText(cause)); })
      .finally(() => { if (!gone) setSourceLoading(false); });
    return () => { gone = true; };
  }, [api, replayApi, sourceKey, latestPlanId, readyLegacyId, snapshotReady, sourceRefresh, pendingRecording]);
  useEffect(() => {
    if (!active) return;
    let gone = false;
    const refresh = async () => {
      if (inFlight.current || acting.current) return;
      inFlight.current = true; const expected = revision.current;
      try { const value = await api.snapshot(); if (!gone && expected === revision.current) { setSnapshot(value); setPollError(''); } }
      catch (cause) { if (!gone && expected === revision.current) setPollError(cloudErrorText(cause)); }
      finally { inFlight.current = false; }
    };
    void refresh(); const timer = setInterval(() => void refresh(), 2000);
    return () => { gone = true; clearInterval(timer); };
  }, [api, active]);
  useEffect(() => {
    if (!active || playing) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton.current?.focus({ preventScroll: true });
    return () => { if (previous?.isConnected && !previous.closest('[hidden],[inert]')) previous.focus({ preventScroll: true }); };
  }, [active, !!playing]);
  useLayoutEffect(() => {
    const request = pickerFocus.current;
    if (!request) return;
    if (!active || privacyMode || playing || request.sourceKey !== sourceKey) {
      pickerFocus.current = null;
      return;
    }
    if (busy || !request.completed) return;
    pickerFocus.current = null;
    const focused = document.activeElement;
    const orphaned = !focused || focused === document.body || focused === document.documentElement || !focused.isConnected;
    if (mounted.current && document.hasFocus() && orphaned && request.target.isConnected
      && !request.target.disabled && panel.current?.contains(request.target)
      && !request.target.closest('[hidden],[inert]')) {
      request.target.focus({ preventScroll: true });
    }
  }, [active, privacyMode, playing, sourceKey, busy, pickerCompletion]);
  const run = async (action: () => Promise<CloudSnapshot | null>) => {
    if (acting.current) return;
    acting.current = true; setBusy(true); setError(''); const expected = ++revision.current;
    try { const value = await action(); if (mounted.current && expected === revision.current && value) setSnapshot(value); }
    catch (cause) { if (mounted.current && expected === revision.current) setError(cloudErrorText(cause)); }
    finally { acting.current = false; if (mounted.current) setBusy(false); }
  };
  const importClient = (target: HTMLButtonElement) => {
    if (acting.current) return;
    const request = { target, sourceKey, completed: false };
    pickerFocus.current = request;
    void run(() => api.importClient()).finally(() => {
      if (mounted.current && pickerFocus.current === request) {
        request.completed = true;
        // Commit after busy clears: browsers drop focus when the trigger disables.
        setPickerCompletion(value => value + 1);
      }
    });
  };
  const account = snapshot?.account;
  const parts = description?.parts ?? [];
  const incomplete = description?.coverage?.state === 'incomplete';
  const pendingParts = parts.filter(part => !jobs.some(job => sameCloudPart(job.source, part.source ?? { ...source, partIndex: part.partIndex })));
  const planning = jobs.some(job => job.recordingPlan);
  const canEnqueue = !pendingRecording && !incomplete && !planning && !sourceLoading && (pendingParts.length > 0 || !!description?.preparationRequired);
  const enqueue = () => run(async () => {
    const value = await api.enqueueRecording({ provider: source.provider, recordingId: source.recordingId }, visibility);
    try {
      const committed = await api.describe({ provider: source.provider, recordingId: source.recordingId });
      if (mounted.current && currentSourceKey.current === sourceKey) { setDescription(committed); setSourceError(''); }
    } catch (cause) {
      if (mounted.current && currentSourceKey.current === sourceKey) setSourceError(cloudErrorText(cause));
    }
    return value;
  });
  const play = (job: CloudRecordingJob) => run(async () => {
    const manifest = await replayApi.describe(job.id);
    if (!sameCloudRecording(manifest.source, job.source) || (manifest.planId ?? null) !== (job.source.planId ?? null)) {
      throw new Error('이 녹화의 업로드 구간을 확인하지 못했습니다. 다시 열어 주세요.');
    }
    if (mounted.current && currentSourceKey.current === sourceKey) { setPlaybackDescription(manifest); setPlaying(job); }
    return null;
  });
  const playingJob = playing && jobs.find(job => job.id === playing.id) || playing;
  return <>{createPortal(<div className="cloud-recording-backdrop" hidden={!active || !!playing}>
    <div ref={panel} role="dialog" aria-modal="true" aria-labelledby={headingId} className="cloud-recording-dialog" onKeyDown={event => {
      event.stopPropagation();
      if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); onClose(); }
      if (event.key === 'Tab') {
        const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled),a[href],input:not(:disabled),select:not(:disabled)')];
        const first = controls[0], last = controls.at(-1);
        if (first && (event.shiftKey && document.activeElement === first || !event.shiftKey && document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); }
      }
    }}>
      <header><div><h2 id={headingId}>YouTube 보관</h2><span className="cloud-recording-private">로컬 원본·채팅·후원 기록 유지</span></div><button ref={closeButton} type="button" className="cloud-recording-close" aria-label="YouTube 보관 닫기" onClick={onClose}><CloseIcon /></button></header>
      <h3 className="cloud-recording-title">{privacyMode ? '프라이버시 모드' : title}</h3>
      <section className="cloud-recording-account" aria-label="업로드 계정">
        <div><strong>{privacyMode ? '업로드 계정' : account?.channelName || '업로드 계정'}</strong><span>{account?.authorizing ? '브라우저에서 연결 중…' : account?.connected ? '연결됨' : '연결 필요'}</span></div>
        <div className="cloud-recording-actions">
          <button type="button" disabled={busy || !account || account.authorizing || account.connected || privacyMode} onClick={event => importClient(event.currentTarget)}>{account?.configured ? '설정 파일 변경' : '설정 파일 선택'}</button>
          <button type="button" disabled={busy || !account?.configured || account.connected || account.authorizing || privacyMode} onClick={() => void run(() => api.authorize())}>계정 연결</button>
          <button type="button" disabled={busy || !account?.connected || account.authorizing || privacyMode} onClick={() => void run(() => api.disconnect())}>연결 해제</button>
        </div>
        {account && !account.configured ? <p>Google Cloud에서 만든 데스크톱 앱 OAuth JSON을 선택해 주세요.</p> : null}
        {account?.error ? <p role="alert" className="cloud-recording-error">{account.error.message}</p> : null}
      </section>
      {allowAutomatic && source.provider !== 'youtube' ? <section className="cloud-recording-automatic" aria-label="선택한 녹화 자동 보관">
        <CloudAutomaticChoice model={automatic} target={{ kind: 'recording', provider: source.provider, recordingId: source.recordingId }} name={title} privacy={privacyMode} />
        <small>{pendingRecording ? '녹화와 검증이 끝나면 이 녹화만 업로드합니다.' : '선택한 공개 범위는 새 업로드에 적용됩니다.'} 기존 영상·원본·채팅 기록은 유지합니다.</small>
        {automatic.error ? <p role="alert" className="cloud-recording-error">{automatic.error}</p> : null}
      </section> : null}
      <div className="cloud-recording-jobs" aria-label="YouTube 업로드 작업">
        {jobs.map((job, index) => <article key={job.id} className={`cloud-recording-job is-${job.state}`}>
          <div className="cloud-recording-job-heading"><strong>{jobs.length > 1 || parts.length > 1 ? `구간 ${index + 1} · ` : ''}{job.durationSeconds > 0 ? `${cloudDuration(job.mediaOffsetSeconds)}–${cloudDuration(job.mediaOffsetSeconds + job.durationSeconds)}` : '선택한 녹화'}</strong><span role="status">{job.state === 'validating' ? job.preparation ? preparationLabels[job.preparation.phase] : '업로드 준비 중' : cloudStateLabel[job.state]}</span></div>
          <small aria-label="업로드 공개 범위">{cloudVisibilityLabel(job)}</small>
          {job.state === 'validating' && job.preparation ? <PreparationMeter progress={job.preparation} /> : ['uploading', 'paused', 'retry_wait', 'queued', 'validating'].includes(job.state) ? <><progress max={100} value={cloudProgress(job)} aria-label="업로드 진행" /><small>{cloudSize(job.uploadedBytes)} / {job.totalBytes > 0 ? cloudSize(job.totalBytes) : '확인 중'}</small></> : null}
          <div className="cloud-recording-actions">
            {cloudJobCanPause(job) ? <button type="button" disabled={busy} onClick={() => void run(() => api.pause(job.id))}>일시정지</button> : null}
            {cloudJobCanResume(job) ? <button type="button" disabled={busy || !account?.connected || privacyMode} onClick={() => void run(() => api.resume(job.id))}>{job.state === 'failed' ? '다시 시도' : '계속'}</button> : null}
            {cloudJobPlayable(job) ? <button type="button" disabled={privacyMode || busy} onClick={() => void play(job)}>아츠미에서 재생</button> : null}
          </div>
          {job.error ? <p className="cloud-recording-error" role="alert">{job.error.message}</p> : null}
        </article>)}
        {!jobs.length && description ? <p className="cloud-recording-empty">{parts.length > 1 ? `${parts.length}개 구간 · ` : ''}{cloudDuration(description.durationSeconds ?? parts.reduce((sum, part) => sum + part.durationSeconds, 0))} · {cloudSize(description.bytes ?? parts.reduce((sum, part) => sum + part.bytes, 0))}{description.preparationRequired ? ' · 업로드 시 구간 준비' : ''}</p> : null}
        {!description && !sourceError ? <p role="status">{pendingRecording ? '녹화 완료 대기' : '저장 구간을 확인하고 있습니다…'}</p> : null}
        {incomplete ? <p role="alert" className="cloud-recording-error">{description.coverage?.reason || '아직 준비되지 않은 녹화 구간이 있습니다. 전체 구간을 확인한 뒤 업로드할 수 있습니다.'}</p> : null}
        {!privacyMode && description?.coverage?.captureWarning ? <p role="status" className="cloud-recording-error cloud-recording-capture-warning">{description.coverage.captureWarning}</p> : null}
        {sourceError ? <div className="cloud-recording-source-error"><p role="alert" className="cloud-recording-error">{sourceError}</p><button type="button" disabled={busy || sourceLoading} onClick={() => setSourceRefresh(value => value + 1)}>{sourceLoading ? '확인 중…' : '다시 확인'}</button></div> : null}
        {error ? <p role="alert" className="cloud-recording-error">{error}</p> : null}
        {pollError ? <p role="alert" className="cloud-recording-error">{pollError}</p> : null}
      </div>
      <footer><label className="cloud-visibility-choice"><span>이번 업로드</span><select aria-label="이번 업로드 공개 범위" value={visibility} disabled={!active || busy || privacyMode || !canEnqueue} onChange={event => setVisibility(event.target.value as CloudVisibility)}><option value="private">비공개</option><option value="unlisted">일부 공개 · 링크로 시청</option></select></label><button type="button" className="cloud-recording-primary" disabled={!active || busy || !account?.connected || !canEnqueue || privacyMode || !!sourceError} onClick={() => void enqueue()}>{busy ? '처리 중…' : `${visibility === 'private' ? '비공개' : '일부 공개'} 업로드${pendingParts.length > 1 ? ` · ${pendingParts.length}개 구간` : ''}`}</button></footer>
    </div>
  </div>, document.body)}
    {playingJob ? <CloudRecordingReplay key={playingJob.id} jobs={jobs} initialJobId={playingJob.id} expectedParts={playbackDescription?.parts} coverage={playbackDescription?.coverage} active={active} privacyMode={privacyMode} onClose={() => setPlaying(null)} cloudApi={replayApi} /> : null}
  </>;
}
export function CloseIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" /></svg>; }
