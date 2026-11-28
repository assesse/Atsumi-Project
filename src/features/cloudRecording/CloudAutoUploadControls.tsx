import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cloudErrorText, cloudRecordingApi, type CloudAutomaticRule, type CloudAutomaticTarget, type CloudRecordingApi, type CloudSnapshot, type CloudVisibility } from '../../api/cloudRecording';
import './CloudAutoUploadControls.css';
import { CloudLiveControls } from './CloudLiveControls';

export const automaticStatusLabel: Record<CloudAutomaticRule['status'], string> = {
  disabled: '선택 안 함', waiting_account: '계정 연결 대기', watching: '새 녹화 대기', waiting_recording: '녹화 완료 대기',
  waiting_processing: '녹화 마무리 대기', queued: '업로드 대기', validating: '업로드 준비 중', uploading: '업로드 중',
  processing: 'YouTube 처리 중', ready: '보관됨', paused: '일시정지', failed: '확인 필요', source_missing: '녹화 확인 필요',
};
export function automaticTargetKey(target: CloudAutomaticTarget) {
  return `${target.provider}:${target.kind}:${target.kind === 'channel' ? target.channelId : target.recordingId}`;
}

/** Display lifetime never owns automatic admission or the recording receiver. */
export function useCloudAutomatic(scope: string, active: boolean, privacy: boolean, api: CloudRecordingApi = cloudRecordingApi, external?: { snapshot: CloudSnapshot | null; onSnapshot(value: CloudSnapshot): void }) {
  const [snapshot, setSnapshot] = useState<CloudSnapshot | null>(null), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const current = useRef<CloudSnapshot | null>(null), epoch = useRef(0), mounted = useRef(false), acting = useRef(false);
  const focus = useRef<{ target: HTMLButtonElement; scope: string; completed: boolean } | null>(null);
  const [completion, setCompletion] = useState(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; epoch.current++; }; }, []);
  useEffect(() => { epoch.current++; current.current = null; setSnapshot(null); setError(''); focus.current = null; }, [api, scope]);
  const accept = (value: CloudSnapshot, request: number, publish = true) => {
    if (!mounted.current || request !== epoch.current) return;
    if (value.automatic && current.current?.automatic && value.automatic.revision < current.current.automatic.revision) return;
    current.current = value; setSnapshot(value);
    if (publish) external?.onSnapshot(value);
  };
  const sharedSnapshot = external?.snapshot, hasExternal = !!external;
  useEffect(() => { if (active && !privacy && sharedSnapshot) accept(sharedSnapshot, epoch.current, false); }, [sharedSnapshot, scope, active, privacy]);
  useEffect(() => {
    if (!active || privacy) { epoch.current++; focus.current = null; return; }
    if (hasExternal) return;
    let gone = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (!acting.current) {
        const request = epoch.current;
        try { const value = await api.snapshot(); if (!gone) { accept(value, request); if (request === epoch.current) setError(''); } }
        catch (cause) { if (!gone && request === epoch.current) setError(cloudErrorText(cause)); }
      }
      if (!gone) timer = setTimeout(() => void poll(), 3000);
    };
    void poll(); return () => { gone = true; epoch.current++; focus.current = null; clearTimeout(timer); };
  }, [api, scope, active, privacy, hasExternal]);
  useLayoutEffect(() => {
    const request = focus.current;
    if (!request) return;
    if (!active || privacy || request.scope !== scope) { focus.current = null; return; }
    if (busy || !request.completed) return;
    focus.current = null;
    const element = document.activeElement;
    if (document.hasFocus() && (!element || element === document.body || element === document.documentElement || !element.isConnected)
      && request.target.isConnected && !request.target.disabled && !request.target.closest('[hidden],[inert]')) request.target.focus({ preventScroll: true });
  }, [active, privacy, scope, busy, completion]);
  const run = async (action: () => Promise<CloudSnapshot | null>, picker?: HTMLButtonElement) => {
    if (!active || privacy || acting.current) return;
    acting.current = true; setBusy(true); setError(''); const request = ++epoch.current;
    const restore = picker ? { target: picker, scope, completed: false } : null;
    if (restore) focus.current = restore;
    try { const value = await action(); if (value) accept(value, request); }
    catch (cause) {
      if (mounted.current && request === epoch.current) setError(cloudErrorText(cause));
      // A conflicting write is never retried. Fetch the native persisted choice.
      try { accept(await api.snapshot(), request); } catch { /* Keep the actual operation error. */ }
    } finally {
      acting.current = false;
      if (mounted.current) {
        setBusy(false);
        if (restore && focus.current === restore) { restore.completed = true; setCompletion(value => value + 1); }
      }
    }
  };
  const select = (target: CloudAutomaticTarget, enabled: boolean, visibility?: CloudVisibility) => {
    const revision = current.current?.automatic?.revision;
    if (revision === undefined) return;
    void run(() => api.setAutomatic(target, enabled, revision, visibility));
  };
  return { snapshot, error: error || snapshot?.automatic?.error?.message || '', busy, disabled: !active || privacy || busy || !snapshot?.automatic, select, run, api };
}
export type CloudAutomaticModel = ReturnType<typeof useCloudAutomatic>;

export function CloudAutomaticChoice({ model, target, name, privacy = false }: { model: CloudAutomaticModel; target: CloudAutomaticTarget; name: string; privacy?: boolean }) {
  const rule = model.snapshot?.automatic?.rules.find(value => automaticTargetKey(value.target) === automaticTargetKey(target));
  const [choice, setChoice] = useState<CloudVisibility>('private');
  const savedVisibility = rule ? rule.visibility ?? 'unlisted' : 'private';
  useEffect(() => { setChoice(savedVisibility); }, [automaticTargetKey(target), rule?.id, savedVisibility]);
  useEffect(() => { if (rule?.enabled && !model.busy) setChoice(savedVisibility); }, [rule?.enabled, model.busy, savedVisibility]);
  const label = target.kind === 'channel' ? '녹화 후 YouTube 보관' : '이 녹화 자동 업로드';
  return <div className="cloud-automatic-choice">
    <label><input type="checkbox" checked={rule?.enabled ?? false} disabled={model.disabled || privacy}
      aria-label={`${privacy ? '선택 항목' : name} ${label}`} onChange={event => model.select(target, event.target.checked, choice)} /><span>{label}</span></label>
    <label className="cloud-visibility-choice"><span>새 업로드</span><select aria-label={`${privacy ? '선택 항목' : name} 자동 업로드 공개 범위`} value={choice} disabled={model.disabled || privacy}
      onChange={event => { const next = event.target.value as CloudVisibility; setChoice(next); if (rule?.enabled) model.select(target, true, next); }}><option value="private">비공개</option><option value="unlisted">일부 공개 · 링크로 시청</option></select></label>
    {rule ? <small role="status" className={rule.error ? 'is-error' : ''}>{privacy ? '' : rule.error?.message || automaticStatusLabel[rule.status]}</small> : null}
  </div>;
}

export function CloudAutomaticAccount({ model, privacy = false }: { model: CloudAutomaticModel; privacy?: boolean }) {
  const account = model.snapshot?.account;
  return <section className="cloud-automatic-account" aria-label="YouTube 보관 계정">
    <div><strong>YouTube 보관</strong><span>{privacy ? '프라이버시 모드' : account?.authorizing ? '브라우저에서 연결 중…' : account?.connected ? `${account.channelName || '업로드 계정'} · 연결됨` : '계정 연결 필요'}</span></div>
    <div className="cloud-automatic-account-actions">
      <button type="button" disabled={model.busy || !account || account.connected || account.authorizing || privacy} onClick={event => void model.run(() => model.api.importClient(), event.currentTarget)}>{account?.configured ? '설정 파일 변경' : '설정 파일 선택'}</button>
      <button type="button" disabled={model.busy || !account?.configured || account.connected || account.authorizing || privacy} onClick={() => void model.run(() => model.api.authorize())}>계정 연결</button>
      <button type="button" disabled={model.busy || !account?.connected || account.authorizing || privacy} onClick={() => void model.run(() => model.api.disconnect())}>연결 해제</button>
    </div>
    <small>선택한 채널의 새 녹화만 · 공개 범위 선택 · 원본 유지</small>
    {model.error || account?.error ? <p role="alert">{model.error || account?.error?.message}</p> : null}
  </section>;
}

export function CloudAutomaticChannelsDialog({ provider, channels, active, privacy, onClose, api = cloudRecordingApi }: {
  provider: 'chzzk' | 'soop'; channels: { id: string; name: string }[]; active: boolean; privacy: boolean; onClose(): void; api?: CloudRecordingApi;
}) {
  const model = useCloudAutomatic(`${provider}:channel-manager`, active, privacy, api), dialog = useRef<HTMLDialogElement>(null), close = useRef<HTMLButtonElement>(null);
  const choices = [...channels];
  for (const rule of model.snapshot?.automatic?.rules ?? []) {
    const target = rule.target;
    if (rule.enabled && target.kind === 'channel' && target.provider === provider && !choices.some(channel => channel.id === target.channelId)) choices.push({ id: target.channelId, name: target.channelId });
  }
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (dialog.current && !dialog.current.open) { if (typeof dialog.current.showModal === 'function') dialog.current.showModal(); else dialog.current.setAttribute('open', ''); }
    close.current?.focus({ preventScroll: true });
    return () => { if (previous?.isConnected && !previous.closest('[hidden],[inert]')) previous.focus({ preventScroll: true }); };
  }, []);
  return createPortal(<dialog ref={dialog} className="cloud-automatic-dialog" aria-label="채널 YouTube 보관 관리" data-native-preserve-video="true"
    onKeyDown={event => event.stopPropagation()} onCancel={event => { event.preventDefault(); onClose(); }} onClose={onClose}>
    <div data-native-dialog-surface="true"><header><h2>채널 YouTube 보관</h2><button ref={close} type="button" aria-label="채널 보관 관리 닫기" onClick={onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button></header>
      <CloudAutomaticAccount model={model} privacy={privacy} />
      <CloudLiveControls provider={provider} channels={channels} active={active} privacy={privacy} />
      <div className="cloud-automatic-channel-list">{choices.map((channel, index) => <article key={channel.id}><strong>{privacy ? `채널 ${index + 1}` : channel.name}</strong><CloudAutomaticChoice model={model} target={{ kind: 'channel', provider, channelId: channel.id }} name={channel.name} privacy={privacy} /></article>)}</div>
      {!choices.length ? <p>즐겨찾기에서 채널을 선택해 주세요.</p> : null}
    </div>
  </dialog>, document.body);
}
