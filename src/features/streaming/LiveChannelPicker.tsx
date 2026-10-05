import { useRef, useState } from "react";
import type { LiveChannelsApi } from "../../api/liveChannels";
import type { AutoRecordingApi } from "../../api/autoRecording";
import { normalizeMultiviewChannel } from "../../api/multiview";
import { LiveChannelDirectory, safeChannelPortrait, useLiveChannelDirectory } from "./LiveChannelDirectory";
import "./LiveChannelPicker.css";
import { RecordingLoadStatus } from "./RecordingLoadStatus";

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
  const { channels: entries, profiles, pending, error, pollError, hasSnapshot, toggle } = useLiveChannelDirectory()!;
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const selected = inputs.slice(0, multiple ? 4 : 1).map(normalizeMultiviewChannel);
  const choose = (id: string) => {
    setSelectionError(null);
    if (!multiple) { onInputs([id, ...inputs.slice(1)]); return; }
    const existing = selected.indexOf(id);
    if (existing >= 0) { onInputs(inputs.map((value, index) => index === existing ? "" : value)); return; }
    const empty = inputs.findIndex(value => !value.trim());
    if (empty < 0) { setSelectionError("마도는 최대 네 방송까지 선택할 수 있습니다."); return; }
    onInputs(inputs.map((value, index) => index === empty ? id : value));
  };
  const channelOrder = useRef<string[]>([]);
  const channelIds = new Set(entries.map(row => row.channelId));
  const previousIds = new Set(channelOrder.current);
  channelOrder.current = [...channelOrder.current.filter(id => channelIds.has(id)), ...entries.filter(row => !previousIds.has(row.channelId)).map(row => row.channelId)];
  const ranks = new Map(channelOrder.current.map((id, index) => [id, index]));
  const channels = [...entries].sort((a, b) => ranks.get(a.channelId)! - ranks.get(b.channelId)!);
  return <section className="live-channel-picker" aria-label="저장한 채널">
    <div className="live-channel-picker-title"><strong>즐겨찾기 · 녹화 예약</strong></div>
    <div className="live-channel-list">
      {channels.map((row, index) => {
        const profile = privacy ? undefined : profiles[row.channelId];
        const name = privacy ? `채널 ${index + 1}` : profile?.channelName || row.channelName || row.channelId;
        const status = row.scheduled?.status;
        const badge = status === "recording" ? "녹화 중" : status === "starting" ? "녹화 준비" : status === "stopping" ? "마무리 중" : row.scheduled ? row.scheduled.enabled ? "녹화 예약" : "예약 꺼짐" : "즐겨찾기";
        return <div className={`live-channel-row${row.favorite ? " is-favorite" : ""}`} key={row.channelId}>
          <button type="button" className="live-channel-choice" disabled={disabled} aria-pressed={selected.includes(row.channelId)} aria-label={`${name} 선택`} onClick={() => choose(row.channelId)}>
            <span className="live-channel-avatar" aria-hidden="true">{safeChannelPortrait(profile?.image) ? <img src={profile!.image!} alt="" decoding="async" onError={event => { event.currentTarget.hidden = true; }} /> : null}<span>{privacy ? "·" : Array.from(name)[0]}</span></span>
            <span className="live-channel-identity"><span className="live-channel-name" title={name}>{name}</span><small className={status === "recording" ? "is-recording" : ""}>{badge}</small></span>
          </button>
          <button type="button" className="live-channel-star" disabled={pending || runtime !== "tauri"} aria-pressed={row.favorite} aria-label={`${name} 즐겨찾기 ${row.favorite ? "해제" : "추가"}`} title="즐겨찾기만 변경합니다. 녹화 예약은 유지됩니다." onClick={() => void toggle(row.channelId, !row.favorite)}>{row.favorite ? "★" : "☆"}</button>
        </div>;
      })}
      {hasSnapshot && !channels.length ? <p>즐겨찾기와 녹화 예약 채널이 여기에 표시됩니다.</p> : null}
    </div>
    <RecordingLoadStatus error={pollError} hasSnapshot={hasSnapshot} subject="저장한 채널" />
    {error || selectionError ? <small role="alert">{error || selectionError}</small> : null}
  </section>;
}
