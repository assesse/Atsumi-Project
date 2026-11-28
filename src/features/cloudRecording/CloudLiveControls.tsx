import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { cloudErrorText, cloudLiveApi, type CloudLiveApi, type CloudLiveArchiveVerification, type CloudLiveOperation, type CloudLiveProvider, type CloudLiveSnapshot } from '../../api/cloudRecording';
import './CloudLiveControls.css';
import { canReplayLiveArchive } from './cloudLiveReplayModel';
const CloudLiveReplay = lazy(() => import('./CloudLiveReplay').then(value => ({ default: value.CloudLiveReplay })));

type Props = { provider: CloudLiveProvider; channels: { id: string; name: string }[]; active: boolean; privacy: boolean; api?: CloudLiveApi };
const terminal = (operation: CloudLiveOperation) => ['joined', 'stopped', 'finished', 'failed', 'cancelled', 'recovery_required', 'interrupted'].includes(operation.state);
export function liveStatusLabel(operation: CloudLiveOperation) {
  if (!operation.receiptPersisted) return '전송 상태 저장 실패';
  if (operation.archiveReady) return '보관됨';
  if (operation.outcomeUncertain || operation.state === 'recovery_required') return '전송 결과 확인 필요';
  if (operation.error || operation.interrupted) return '전송 확인 필요';
  if (operation.stopRequested && !operation.managerJoined) return '전송 마무리 중';
  if (operation.state === 'joined' && operation.archiveVerification) return '전송 종료';
  return ({ selected: '전송 준비됨', preparing: 'YouTube 전송 준비 중', starting: '전송 시작 중', streaming: '전송 중', running: '전송 중',
    stopping: '전송 마무리 중', joined: '전송 종료 · 보관 확인 전', stopped: '전송 종료 · 보관 확인 전', finished: '전송 종료 · 보관 확인 전', cancelled: '전송 취소됨' } as Record<string, string>)[operation.state] || '전송 상태 확인 중';
}
export function liveArchiveStatusLabel(verification: CloudLiveArchiveVerification) {
  return ({ checking: 'YouTube 보관 확인 중', pending: 'YouTube 영상 처리 중', needs_check: 'YouTube 보관 확인 대기',
    metadata_verified: '영상 처리 완료 · 재생·동기화 확인 전', inconclusive: 'YouTube 보관 상태 확인 필요', rejected: 'YouTube 보관 조건 확인 필요' })[verification.state];
}
export function canCheckLiveArchive(operation: CloudLiveOperation, account: CloudLiveSnapshot['account'] | undefined) {
  return !!account?.connected && !!account.channelId && account.channelId === operation.destinationChannelId
    && operation.receiptPersisted && !operation.outcomeUncertain && !!operation.runId && !!operation.broadcastId
    && operation.managerJoined && operation.workerJoined && operation.sidecarJoined
    && !['checking', 'pending'].includes(operation.archiveVerification?.state ?? '');
}
export function liveRecordWarnings(operation: CloudLiveOperation): string[] {
  const warnings: string[] = [];
  if (operation.sidecarTextState === 'partial' || operation.sidecarTextState === 'incomplete') warnings.push('채팅 일부 누락');
  else if (operation.managerJoined && operation.sidecarTextState === 'unknown') warnings.push('채팅 기록 확인 필요');
  if (operation.assetsIncomplete) warnings.push('이미지 일부 누락');
  if (operation.timingIncomplete) warnings.push('동기화 정보 일부 누락');
  return warnings;
}

/** Closing this panel stops polling only, never the viewer or a selected transfer. */
export function CloudLiveControls(props: Props) {
  const [expanded, setExpanded] = useState(false);
  const [replay, setReplay] = useState<{ operation: CloudLiveOperation; title: string } | null>(null);
  useEffect(() => setReplay(null), [props.provider]);
  return <><details className="cloud-live-controls" open={expanded} onToggle={event => setExpanded(event.currentTarget.open)}>
    <summary>실시간으로 YouTube에 보관</summary>
    {expanded && props.active ? <LiveControlsBody key={props.provider} {...props} onReplay={(operation, title) => setReplay({ operation, title })} /> : null}
  </details>{replay ? <Suspense fallback={null}><CloudLiveReplay key={replay.operation.selectionId} operation={replay.operation} title={replay.title} active={props.active} privacyMode={props.privacy} onClose={() => setReplay(null)} /></Suspense> : null}</>;
}

function LiveControlsBody({ provider, channels, active, privacy, api = cloudLiveApi, onReplay }: Props & { onReplay(operation: CloudLiveOperation, title: string): void }) {
  const [snapshot, setSnapshot] = useState<CloudLiveSnapshot | null>(null), [sources, setSources] = useState<string[]>([]);
  const [selected, setSelected] = useState(''), [title, setTitle] = useState(''), [audience, setAudience] = useState('');
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const epoch = useRef(0), pending = useRef(false), mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; epoch.current++; }; }, []);
  useEffect(() => {
    ++epoch.current;
    setSnapshot(null); setSources([]); setSelected(''); setTitle(''); setAudience(''); setError('');
    if (!active || privacy) return;
    let gone = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const request = epoch.current;
      if (!pending.current) {
        const results = await Promise.allSettled([api.snapshot(), api.sources(provider)]);
        if (!gone && request === epoch.current && !pending.current) {
          const [status, available] = results;
          if (status.status === 'fulfilled') setSnapshot(status.value); else setError(cloudErrorText(status.reason));
          if (available.status === 'fulfilled') setSources(available.value); else { setSources([]); setError(cloudErrorText(available.reason)); }
        }
      }
      if (!gone) timer = setTimeout(() => void poll(), 3000);
    };
    void poll();
    return () => { gone = true; epoch.current++; clearTimeout(timer); };
  }, [api, provider, active, privacy]);
  const run = async (action: (isCurrent: () => boolean) => Promise<CloudLiveSnapshot>) => {
    if (!active || privacy || pending.current) return;
    pending.current = true; setBusy(true); setError(''); const request = ++epoch.current;
    const isCurrent = () => mounted.current && request === epoch.current;
    try { const value = await action(isCurrent); if (isCurrent()) setSnapshot(value); }
    catch (cause) {
      if (isCurrent()) {
        setError(cloudErrorText(cause));
        // Refresh a conflict/uncertain receipt once. Never retry a remote create.
        try { const value = await api.snapshot(); if (isCurrent()) setSnapshot(value); } catch { /* Keep the original action error. */ }
      }
    } finally { pending.current = false; if (mounted.current) setBusy(false); }
  };
  const name = (id: string) => channels.find(channel => channel.id === id)?.name || id;
  const account = snapshot?.account, disabled = busy || privacy || !active;
  const operations = snapshot?.operations.filter(operation => operation.provider === provider) ?? [];
  const occupied = operations.some(operation => operation.channelId === selected && (operation.outcomeUncertain || !operation.receiptPersisted || (!terminal(operation) && operation.state !== 'selected')));
  const start = () => void run(async isCurrent => {
    const value = await api.select(provider, selected);
    if (!isCurrent()) return value;
    const candidates = value.operations.filter(operation => operation.provider === provider && operation.channelId === selected && operation.state === 'selected');
    const operation = candidates[0];
    if (!operation || candidates.length !== 1 || value.account.generation !== account?.generation || !value.account.connected) throw new Error('방송 또는 연결 계정이 변경됐습니다. 다시 선택해 주세요.');
    return api.start(operation.selectionId, operation.revision, { title: title.trim(), scheduledStartMs: Date.now() + 60_000, madeForKids: audience === 'yes' });
  });
  return <div className="cloud-live-body">
    <div className="cloud-live-account"><span>{privacy ? '프라이버시 모드' : account?.authorizing ? '브라우저에서 연결 중…' : account?.connected ? `${account.channelName || 'YouTube 계정'} · 실시간 전송 연결됨` : '실시간 전송 계정 연결 필요'}</span>
      <div>
        {!account?.configured ? <button type="button" disabled={disabled || !account} onClick={() => void run(() => api.importClient())}>설정 파일 선택</button> : null}
        <button type="button" disabled={disabled || !account?.configured || account.connected || account.authorizing} onClick={() => void run(() => api.authorize())}>실시간 전송 연결</button>
        <button type="button" disabled={disabled || !account || (!account.connected && !account.authorizing)} onClick={() => void run(() => api.disconnect())}>{account?.authorizing ? '연결 취소' : '연결 해제'}</button>
      </div>
    </div>
    <p className="cloud-live-caption">일부 공개 · 영상은 YouTube로 전송 · 채팅·후원 기록은 앱에 보관</p>
    <label>시청 중인 방송<select value={sources.includes(selected) ? selected : ''} disabled={disabled || !account?.connected} onChange={event => { setSelected(event.target.value); setTitle(event.target.value ? name(event.target.value).slice(0, 100) : ''); setAudience(''); }}>
      <option value="">{sources.length ? '방송 선택' : '시청 중인 방송이 없습니다'}</option>
      {!privacy ? sources.map(id => <option key={id} value={id}>{name(id)}</option>) : null}
    </select></label>
    <label>보관 제목<input value={privacy ? '' : title} maxLength={100} disabled={disabled || !selected} onChange={event => setTitle(event.target.value)} /></label>
    <label>시청자층<select value={audience} disabled={disabled || !selected} onChange={event => setAudience(event.target.value)}>
      <option value="">선택해 주세요</option><option value="no">아동용이 아닙니다</option><option value="yes">아동용입니다</option>
    </select></label>
    <button type="button" className="cloud-live-start" disabled={disabled || !account?.connected || !sources.includes(selected) || !title.trim() || !audience || occupied} onClick={start}>전송 시작</button>
    {!privacy && (error || account?.error) ? <p role="alert">{error || account?.error?.message}</p> : null}
    {!privacy && operations.length ? <ul className="cloud-live-operations">{operations.map(operation => <li key={operation.selectionId}>
      <div><strong>{name(operation.channelId)}</strong><span role="status">{liveStatusLabel(operation)}</span>
        {operation.archiveVerification ? <span role="status">{liveArchiveStatusLabel(operation.archiveVerification)}</span> : null}
        {liveRecordWarnings(operation).map(warning => <span className="cloud-live-record-warning" key={warning}>{warning}</span>)}</div>
      {!terminal(operation) ? <button type="button" disabled={disabled || operation.stopRequested} onClick={() => void run(() => api.stop(operation.selectionId, operation.revision))}>전송 중지</button> : null}
      {operation.managerJoined && operation.workerJoined && operation.sidecarJoined ? <button type="button" disabled={disabled || !canCheckLiveArchive(operation, account)} onClick={() => void run(() => api.verifyArchive(operation.selectionId, operation.revision))}>보관 확인</button> : null}
      {canReplayLiveArchive(operation) ? <button type="button" disabled={disabled} onClick={() => onReplay(operation, name(operation.channelId))}>아츠미에서 재생</button> : null}
      {operation.error ? <p role="alert">{operation.error.message}</p> : null}
    </li>)}</ul> : null}
  </div>;
}
