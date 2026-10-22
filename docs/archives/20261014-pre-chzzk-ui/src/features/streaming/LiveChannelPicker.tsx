import { useRef, useState } from "react";
import type { LiveChannelsApi } from "../../api/liveChannels";
import type { AutoRecordingApi } from "../../api/autoRecording";
import { normalizeMultiviewChannel } from "../../api/multiview";
import { LiveChannelDirectory, safeChannelPortrait, useLiveChannelDirectory } from "./LiveChannelDirectory";
import "./LiveChannelPicker.css";
import { RecordingLoadStatus } from "./RecordingLoadStatus";
import { AutoRecordingManagerDialog } from "./AutoRecordingManagerDialog";

type Props = {
  runtime: "tauri" | "browser-mock"; inputs: string[]; multiple: boolean; privacy: boolean; disabled: boolean;
  onInputs(inputs: string[]): void; api?: LiveChannelsApi; autoApi?: AutoRecordingApi;
};
export function LiveChannelPicker(props: Props) {
  const directory = useLiveChannelDirectory();
  // Standalone callers/tests get the same directory. ConnectionSetup shares it
  // with its numbered input rows, so either favorite button updates immediately.
  if (!directory) return <LiveChannelDirectory {...props}><LiveChannelPicker {...props} /></LiveChannelDirectory>;
  return <ChannelChoices {...props} />;
}
function ChannelChoices({ runtime, inputs, multiple, privacy, disabled, onInputs }: Props) {
  const { channels: entries, profiles, profileErrors, pending, error, pollError, hasSnapshot, toggle, toggleAuto, managementApi, acceptManagedSnapshot } = useLiveChannelDirectory()!;
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [managementOpen, setManagementOpen] = useState(false);
  const selected = inputs.slice(0, multiple ? 4 : 1).map(normalizeMultiviewChannel);
  const choose = (id: string) => {
    setSelectionError(null);
    if (!multiple) { onInputs([id, ...inputs.slice(1)]); return; }
    const existing = selected.indexOf(id);
    if (existing >= 0) { onInputs(inputs.map((value, index) => index === existing ? "" : value)); return; }
    const empty = inputs.findIndex(value => !value.trim());
    if (empty < 0) {
      if (inputs.length < 4) { onInputs([...inputs, id]); return; }
      setSelectionError("최대 네 방송까지 선택할 수 있습니다."); return;
    }
    onInputs(inputs.map((value, index) => index === empty ? id : value));
  };
  const channelOrder = useRef<string[]>([]);
  const channelIds = new Set(entries.map(row => row.channelId));
  const previousIds = new Set(channelOrder.current);
  channelOrder.current = [...channelOrder.current.filter(id => channelIds.has(id)), ...entries.filter(row => !previousIds.has(row.channelId)).map(row => row.channelId)];
  const ranks = new Map(channelOrder.current.map((id, index) => [id, index]));
  const channels = [...entries].sort((a, b) => ranks.get(a.channelId)! - ranks.get(b.channelId)!);
  return <section className="live-channel-picker" aria-label="저장한 채널" data-tour="chzzk-auto">
    <div className="live-channel-picker-title"><strong>즐겨찾기</strong><span title="녹화 아이콘을 켜면 현재 방송부터 자동 녹화합니다. 끄면 자동 녹화를 마무리합니다.">자동 녹화 ◉</span><button type="button" className="live-channel-manage" aria-label="자동 녹화 관리" title="예약 관리 · 이번 방송만 중지, 등록 해제, 실패 기록" disabled={pending || runtime !== "tauri"} onClick={() => setManagementOpen(true)}>⋯</button></div>
    <div className="live-channel-list">
      {channels.map((row, index) => {
        const profile = privacy ? undefined : profiles[row.channelId];
        const name = privacy ? `채널 ${index + 1}` : profile?.channelName || row.channelName || row.channelId;
        const status = row.scheduled?.status;
        const badge = ({ recording: "녹화 중", starting: "녹화 준비", stopping: "마무리 중", ending: "방송 종료 확인 중", queued: "자리 대기", retry: "연결 재시도 대기", skipped: "이번 방송 건너뜀", attention: "녹화 확인 필요" } as Record<string, string>)[status ?? ""]
          ?? (row.scheduled?.enabled ? "녹화 예약 됨" : "예약 꺼짐");
        const live = profileErrors.has(row.channelId) ? "unknown" : profile?.liveStatus ?? "unknown";
        return <div className={`live-channel-row${row.favorite ? " is-favorite" : ""}`} key={row.channelId}>
          <button type="button" className="live-channel-choice" disabled={disabled} aria-pressed={selected.includes(row.channelId)} aria-label={`${name} 선택`} onClick={() => choose(row.channelId)}>
            <span className="live-channel-avatar" aria-hidden="true">{safeChannelPortrait(profile?.image) ? <img src={profile!.image!} alt="" decoding="async" onError={event => { event.currentTarget.hidden = true; }} /> : null}<span>{privacy ? "·" : Array.from(name)[0]}</span></span>
            <span className="live-channel-identity"><span className="live-channel-name" title={name}>{name}</span><span className={`live-channel-presence is-${live}`}>{live === "live" ? "● 방송 중" : live === "offline" ? "방송 종료" : "방송 확인 중"}</span><small className={status === "recording" ? "is-recording" : status === "attention" || status === "retry" ? "is-problem" : ""} title={row.scheduled?.message ?? row.scheduled?.lastFailure?.message ?? undefined}>{badge}</small></span>
          </button>
          <div className="live-channel-actions">
            <button type="button" className="live-channel-auto" data-tour="chzzk-auto-register" disabled={pending || runtime !== "tauri"} aria-pressed={row.scheduled?.enabled === true} aria-busy={pending} aria-label={`${name} 자동 녹화 ${row.scheduled?.enabled ? "끄기" : "켜기"}`} title={row.scheduled?.enabled ? "자동 녹화 끄기 · 현재 자동 녹화도 마무리합니다" : "자동 녹화 켜기 · 방송 중이면 바로 저장합니다"} onClick={() => void toggleAuto(row.channelId, !row.scheduled?.enabled)}>{pending ? <span className="spinner" /> : <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" strokeWidth="1.6" /><circle cx="12" cy="12" r="4.5" fill="currentColor" /></svg>}</button>
            <button type="button" className="live-channel-star" disabled={pending || runtime !== "tauri"} aria-pressed={row.favorite} aria-label={`${name} 즐겨찾기 ${row.favorite ? "해제" : "추가"}`} title="즐겨찾기만 변경합니다. 녹화 예약은 유지됩니다." onClick={() => void toggle(row.channelId, !row.favorite)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.78 5.63L21 9.54l-4.5 4.39 1.06 6.2L12 17.2l-5.56 2.93 1.06-6.2L3 9.54l6.22-.91Z" fill={row.favorite ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" /></svg></button>
          </div>
        </div>;
      })}
      {hasSnapshot && !channels.length ? <p>즐겨찾기와 녹화 예약 채널이 여기에 표시됩니다.</p> : null}
    </div>
    <RecordingLoadStatus error={pollError} hasSnapshot={hasSnapshot} subject="저장한 채널" />
    {error || selectionError ? <small role="alert">{error || selectionError}</small> : null}
    {managementOpen ? <AutoRecordingManagerDialog runtime={runtime} privacy={privacy} api={managementApi} onChange={acceptManagedSnapshot} onClose={() => setManagementOpen(false)} /> : null}
  </section>;
}
