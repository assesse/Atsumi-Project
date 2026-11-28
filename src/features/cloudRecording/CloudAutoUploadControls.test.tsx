import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudAutomaticRule, CloudAutomaticTarget, CloudRecordingApi, CloudSnapshot } from '../../api/cloudRecording';
import { CloudAutomaticAccount, CloudAutomaticChannelsDialog, CloudAutomaticChoice, automaticTargetKey, useCloudAutomatic } from './CloudAutoUploadControls';

const channel: CloudAutomaticTarget = { kind: 'channel', provider: 'soop', channelId: 'selected-channel' };
const record: CloudAutomaticTarget = { kind: 'recording', provider: 'chzzk', recordingId: 'selected-record' };
const rule = (target: CloudAutomaticTarget = channel): CloudAutomaticRule => ({ id: 'rule', target, enabled: true, generation: 1, activatedAtMs: 100, status: target.kind === 'channel' ? 'watching' : 'waiting_recording' });
const snapshot = (rules: CloudAutomaticRule[] = [], revision = 4): CloudSnapshot => ({ account: { configured: true, connected: true, authorizing: false }, jobs: [], automatic: { revision, rules } });
function client(value = snapshot()): CloudRecordingApi {
  return { snapshot: vi.fn().mockResolvedValue(value), describe: vi.fn(), getPlan: vi.fn(), importClient: vi.fn().mockResolvedValue(null), authorize: vi.fn().mockResolvedValue(value), disconnect: vi.fn().mockResolvedValue(value), enqueue: vi.fn(), enqueueRecording: vi.fn(), pause: vi.fn(), resume: vi.fn(), setAutomatic: vi.fn().mockResolvedValue(snapshot([rule()], 5)) };
}
function Harness({ api, target = channel, active = true, privacy = false }: { api: CloudRecordingApi; target?: CloudAutomaticTarget; active?: boolean; privacy?: boolean }) {
  const model = useCloudAutomatic(automaticTargetKey(target), active, privacy, api);
  return <><CloudAutomaticAccount model={model} privacy={privacy} /><CloudAutomaticChoice model={model} target={target} name="선택 항목" privacy={privacy} /></>;
}
let root: Root, container: HTMLDivElement;
const checkbox = () => container.querySelector<HTMLInputElement>('input[type=checkbox]')!;
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find(node => node.textContent === text)!;
beforeEach(() => { vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); vi.useFakeTimers(); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('explicit automatic archive selection', () => {
  it('defaults a new rule to private and only stores unlisted on explicit opt-in', async () => {
    const api = client(); await act(async () => root.render(<Harness api={api} />));
    const select = container.querySelector<HTMLSelectElement>('select')!;
    expect(select.value).toBe('private');
    await act(async () => { select.value = 'unlisted'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(api.setAutomatic).not.toHaveBeenCalled();
    await act(async () => checkbox().click());
    expect(api.setAutomatic).toHaveBeenCalledExactlyOnceWith(channel, true, 4, 'unlisted');
  });
  it('preserves a legacy rule and changes only its future upload setting on an explicit selection', async () => {
    const api = client(snapshot([rule()])); await act(async () => root.render(<Harness api={api} />));
    const select = container.querySelector<HTMLSelectElement>('select')!;
    expect(select.value).toBe('unlisted'); expect(api.setAutomatic).not.toHaveBeenCalled();
    vi.mocked(api.setAutomatic).mockResolvedValue(snapshot([{ ...rule(), visibility: 'private' }], 5));
    await act(async () => { select.value = 'private'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(api.setAutomatic).toHaveBeenCalledExactlyOnceWith(channel, true, 4, 'private');
    expect(select.value).toBe('private'); expect(api.enqueueRecording).not.toHaveBeenCalled(); expect(api.resume).not.toHaveBeenCalled();
  });
  it('restores the persisted visibility after an update fails without retrying the mutation', async () => {
    const api = client(snapshot([{ ...rule(), visibility: 'private' }]));
    vi.mocked(api.setAutomatic).mockRejectedValue(new Error('설정 변경 실패'));
    await act(async () => root.render(<Harness api={api} />));
    const select = container.querySelector<HTMLSelectElement>('select')!;
    await act(async () => { select.value = 'unlisted'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(select.value).toBe('private'); expect(api.setAutomatic).toHaveBeenCalledTimes(1);
  });
  it('keeps the old rule intent while describing selected future uploads', async () => {
    const ready = rule(); ready.status = 'ready';
    const api = client(snapshot([ready]));
    await act(async () => root.render(<Harness api={api} />));
    expect(document.body.textContent).toContain('새 녹화만 · 공개 범위 선택');
    expect(document.querySelector('.cloud-automatic-choice [role="status"]')).toHaveTextContent(/^보관됨$/);
    expect(api.setAutomatic).not.toHaveBeenCalled(); expect(api.enqueueRecording).not.toHaveBeenCalled();
  });
  it('never selects or uploads on mount, polling, or account connection', async () => {
    const initial = snapshot(); initial.account.connected = false;
    const api = client(initial); vi.mocked(api.authorize).mockResolvedValue(snapshot());
    await act(async () => root.render(<Harness api={api} />));
    expect(checkbox().checked).toBe(false);
    await act(async () => button('계정 연결').click());
    await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(api.setAutomatic).not.toHaveBeenCalled(); expect(api.enqueueRecording).not.toHaveBeenCalled();
    expect(checkbox().checked).toBe(false);
  });
  it('sends only the selected channel and observed revision on the explicit choice', async () => {
    const api = client(); await act(async () => root.render(<Harness api={api} />));
    await act(async () => checkbox().click());
    expect(api.setAutomatic).toHaveBeenCalledExactlyOnceWith(channel, true, 4, 'private');
    expect(checkbox().checked).toBe(true); expect(document.body.textContent).toContain('새 녹화 대기');
    expect(api.enqueueRecording).not.toHaveBeenCalled();
  });
  it('records an exact unfinished selection without starting an upload or authentication', async () => {
    const initial = snapshot(); initial.account.connected = false;
    const waiting = rule(record); waiting.status = 'waiting_account';
    const api = client(initial); vi.mocked(api.setAutomatic).mockResolvedValue(snapshot([waiting], 5));
    await act(async () => root.render(<Harness api={api} target={record} />));
    await act(async () => checkbox().click());
    expect(api.setAutomatic).toHaveBeenCalledExactlyOnceWith(record, true, 4, 'private');
    expect(document.body.textContent).toContain('계정 연결 대기');
    expect(api.authorize).not.toHaveBeenCalled(); expect(api.enqueueRecording).not.toHaveBeenCalled();
  });
  it('refreshes a revision conflict without retrying the rejected write', async () => {
    const api = client(); vi.mocked(api.setAutomatic).mockRejectedValue(new Error('설정이 변경됐습니다.'));
    await act(async () => root.render(<Harness api={api} />));
    vi.mocked(api.snapshot).mockResolvedValue(snapshot([], 9));
    await act(async () => checkbox().click());
    expect(api.setAutomatic).toHaveBeenCalledTimes(1); expect(checkbox().checked).toBe(false);
    expect(document.body.textContent).toContain('설정이 변경됐습니다.');
    await act(async () => checkbox().click());
    expect(api.setAutomatic).toHaveBeenLastCalledWith(channel, true, 9, 'private');
  });
  it('ignores a prior target response after selection context changes', async () => {
    const api = client(); let resolve!: (value: CloudSnapshot) => void;
    vi.mocked(api.setAutomatic).mockImplementation(() => new Promise(done => { resolve = done; }));
    await act(async () => root.render(<Harness api={api} />));
    await act(async () => checkbox().click());
    await act(async () => root.render(<Harness api={api} target={record} />));
    await act(async () => resolve(snapshot([rule()], 5)));
    expect(checkbox().checked).toBe(false); expect(document.body.textContent).not.toContain('새 녹화 대기');
    expect(api.setAutomatic).toHaveBeenCalledTimes(1);
  });
  it('disables new selection during privacy or missing automatic-state support', async () => {
    const api = client(); await act(async () => root.render(<Harness api={api} privacy />));
    expect(checkbox().disabled).toBe(true); await act(async () => checkbox().click());
    expect(api.setAutomatic).not.toHaveBeenCalled();
    const old = snapshot(); delete old.automatic; vi.mocked(api.snapshot).mockResolvedValue(old);
    await act(async () => root.render(<Harness api={api} />));
    expect(checkbox().disabled).toBe(true);
  });
  it('keeps an enabled channel manageable after removal from favorites without creating rules for other channels', async () => {
    const api = client(snapshot([rule()]));
    await act(async () => root.render(<CloudAutomaticChannelsDialog provider="soop" channels={[{ id: 'new-favorite', name: '새 즐겨찾기' }]} active privacy={false} onClose={() => {}} api={api} />));
    const choices = [...document.querySelectorAll<HTMLInputElement>('.cloud-automatic-choice input')];
    expect(choices).toHaveLength(2); expect(choices.filter(node => node.checked)).toHaveLength(1);
    expect(api.setAutomatic).not.toHaveBeenCalled();
    vi.mocked(api.setAutomatic).mockResolvedValue(snapshot([], 5));
    await act(async () => choices.find(node => node.checked)!.click());
    expect(api.setAutomatic).toHaveBeenCalledExactlyOnceWith(channel, false, 4, 'unlisted');
  });
});
