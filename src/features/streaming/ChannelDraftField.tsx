import { useState } from "react";
import { normalizeChannelDraft, normalizeMultiviewChannel } from "../../api/multiview";
import { safeChannelPortrait, useLiveChannelDirectory } from "./LiveChannelDirectory";

export function ChannelDraftField({ value, index, multiple, disabled, privacy, runtime, onChange }: {
  value: string; index: number; multiple: boolean; disabled: boolean; privacy: boolean;
  runtime: "tauri" | "browser-mock"; onChange(value: string): void;
}) {
  const directory = useLiveChannelDirectory()!;
  const [failedImage, setFailedImage] = useState<string | null>(null);
  const channelId = normalizeMultiviewChannel(value);
  const profile = !privacy && channelId ? directory.profiles[channelId] : undefined;
  const favorite = directory.favorites.some(row => row.channelId === channelId);
  const failed = !!channelId && directory.profileErrors.has(channelId);
  const loading = !!channelId && !privacy && !profile && !failed && runtime === "tauri";
  const label = multiple ? `방송 ${index + 1} 주소 또는 ID` : "채널 주소 또는 ID";
  const inputId = multiple ? `mado-channel-${index}` : "official-browser-channel";
  const name = profile?.channelName || (channelId && !privacy ? directory.channels.find(row => row.channelId === channelId)?.channelName : "");
  const skeleton = !value.trim() || loading;
  const description = !value.trim() ? "채널 미선택" : !channelId ? "채널 주소 또는 32자리 ID를 입력하세요." : privacy ? "프라이버시 모드 · 채널 정보 숨김" : name || (loading ? "채널 확인 중…" : failed ? "채널 정보를 불러오지 못했습니다." : runtime !== "tauri" ? "데스크톱 앱에서 채널 확인" : "이름 정보 없음");
  return <div className="connection-channel-row">
    <span className="connection-channel-number" aria-hidden="true">{index + 1}</span>
    <div className={`connection-channel-profile${skeleton ? " is-skeleton" : ""}`} aria-live="polite" aria-busy={loading} aria-label={description} title={description}>
      {safeChannelPortrait(profile?.image) && failedImage !== profile?.image ? <img src={profile!.image!} alt="" decoding="async" onError={() => setFailedImage(profile!.image!)} /> : <span className="connection-channel-avatar" aria-hidden="true">{skeleton ? "" : privacy ? "·" : Array.from(name || "?")[0]}</span>}
      <span className="connection-channel-name" aria-hidden={skeleton || undefined}>{skeleton ? "" : description}</span>
    </div>
    <div className="connection-channel-draft">
      <label className="connection-input-label-hidden" htmlFor={inputId}>{label}</label>
      <input id={inputId} value={value} disabled={disabled} autoComplete="off" spellCheck={false} placeholder="방송·채널 주소 또는 채널 ID"
        onChange={event => onChange(normalizeChannelDraft(event.target.value))}
        onPaste={event => { const channel = normalizeMultiviewChannel(event.clipboardData.getData("text")); if (channel) { event.preventDefault(); onChange(channel); } }} />
    </div>
    <button type="button" className="connection-channel-favorite" aria-pressed={favorite} aria-busy={directory.pending} aria-label={`방송 ${index + 1} 즐겨찾기 ${favorite ? "해제" : "추가"}`}
      title={favorite ? "즐겨찾기 해제 · 녹화 예약은 유지됩니다" : "즐겨찾기에 추가"} disabled={disabled || !channelId || !directory.hasFavorites || directory.pending || runtime !== "tauri"}
      onClick={() => { if (channelId) void directory.toggle(channelId, !favorite); }}>{directory.pending ? <span className="spinner" aria-hidden="true" /> : favorite ? "★" : "☆"}</button>
  </div>;
}
