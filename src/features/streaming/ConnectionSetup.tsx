import { useId, useState, type ReactNode, type Ref } from "react";
import type { OfficialBrowserSnapshot } from "../../api/officialBrowser";
import type { LiveChannelsApi } from "../../api/liveChannels";
import type { AutoRecordingApi } from "../../api/autoRecording";
import { LiveChannelDirectory, useLiveChannelDirectory } from "./LiveChannelDirectory";
import { ChannelDraftField } from "./ChannelDraftField";
import { FluentIcon } from "../../components/FluentIcon";
import { normalizeMultiviewChannel } from "../../api/multiview";

export type ConnectionMode = "general" | "mado";
export type AccountStatus = "signed_in" | "signed_out" | "unknown" | "checking";
export function accountStatus(snapshot: OfficialBrowserSnapshot): AccountStatus {
  const status = snapshot.authStatus ?? snapshot.loginStatus;
  if (status === "signed_in" || status === "logged_in") return "signed_in";
  if (status === "signed_out" || status === "logged_out") return "signed_out";
  if (status === "checking" || status === "opening" || status === "login_pending") return "checking";
  return "unknown";
}

type Props = {
  mode: ConnectionMode; onMode(mode: ConnectionMode): void; modeDisabled: boolean;
  inputs: string[]; onInput(index: number, value: string): void; disabled: boolean;
  pending: boolean; onConnect(): void; onClose?: () => void; closeRef?: Ref<HTMLButtonElement>;
  connectDisabled?: boolean;
  watching?: boolean; onStop?: () => void;
  auth: AccountStatus; accountDisabled: boolean; accountReason?: string; onLogin(): void; onLogout(): void;
  logoutDisabled?: boolean;
  onGrid(): void; gridDisabled: boolean; gridStatus: string;
  authChecking?: boolean; authError?: string | null;
  onInstaller(browser: "chrome" | "edge"): void; installerDisabled: boolean;
  logoutRef?: Ref<HTMLButtonElement>; options?: ReactNode | ((channelNames: string[]) => ReactNode); children?: ReactNode; helpDetails?: string;
  channelPicker?: ReactNode;
  receiverBacked?: boolean;
  unified?: boolean; onAddChannel?: () => void; onRemoveChannel?: (index: number) => void;
  runtime?: "tauri" | "browser-mock"; privacy?: boolean;
  channelsApi?: LiveChannelsApi; autoApi?: AutoRecordingApi;
  knownChannelNames?: Record<string, string>;
};

/** Shared form; changing the draft tab never changes a native playback context. */
export function ConnectionSetup(props: Props) {
  return <LiveChannelDirectory runtime={props.runtime ?? "browser-mock"} privacy={props.privacy ?? true} inputs={props.inputs.slice(0, props.mode === "mado" ? 4 : 1)} api={props.channelsApi} autoApi={props.autoApi}>
    <ConnectionSetupForm {...props} />
  </LiveChannelDirectory>;
}
function ConnectionSetupForm(props: Props) {
  const id = useId();
  const directory = useLiveChannelDirectory()!;
  const [help, setHelp] = useState(false);
  const authLabel = { signed_in: "로그인됨", signed_out: "로그아웃됨", unknown: "계정 상태 확인 대기", checking: "계정 상태 확인 중" }[props.auth];
  const channelNames = props.inputs.map((input, index) => {
    const channel = normalizeMultiviewChannel(input);
    if (props.privacy) return `방송 ${index + 1}`;
    return channel ? directory.profiles[channel]?.channelName || directory.channels.find(row => row.channelId === channel)?.channelName || props.knownChannelNames?.[channel] || `채널 ${channel.slice(0, 6)}…` : "주소를 입력하세요";
  });
  const options = props.mode === "mado" ? typeof props.options === "function" ? props.options(channelNames) : props.options : null;
  return <div className="official-browser-setup-card connection-setup" data-tour="chzzk-connection" data-native-dialog-surface="true" onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setHelp(false); }} onMouseLeave={(event) => { if (!event.currentTarget.contains(document.activeElement)) setHelp(false); }}>
    <div className="connection-setup-heading">
      {props.unified ? <strong>방송 보기</strong> : <div className="streaming-watch-modes" role="group" aria-label="시청 모드">
        <button type="button" aria-pressed={props.mode === "general"} disabled={props.modeDisabled} onClick={() => props.onMode("general")}>일반모드</button>
        <button type="button" data-tour="chzzk-mado-mode" aria-pressed={props.mode === "mado"} disabled={props.modeDisabled} onClick={() => props.onMode("mado")}>마도모드</button>
      </div>}
      <button type="button" className="connection-help" aria-label="연결 도움말" aria-expanded={help} aria-controls={`${id}-help`} aria-describedby={help ? `${id}-help` : undefined} onMouseEnter={() => setHelp(true)} onClick={() => setHelp(true)} onFocus={() => setHelp(true)} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setHelp(false); } }}>?</button>
      {props.onClose ? <button ref={props.closeRef} type="button" data-tour="chzzk-connection-close" aria-label="연결 설정 닫기" onClick={props.onClose}>설정 닫기</button> : null}
    </div>
    <div className={`connection-help-reveal${help ? " is-open" : ""}`} aria-hidden={!help} inert={!help}>
    <div className="connection-help-clip"><div id={`${id}-help`} className="connection-help-content" role="note">
      <p>주소를 입력하고 시청을 시작하세요. + 버튼으로 최대 네 방송을 추가합니다.</p>
      <p>브라우저 호환 모드로 그리드 없이 직접 재생합니다. Chrome·Edge 확장이나 네이버 커넥터를 따로 설치할 필요가 없습니다.</p>
      <p>실제 화질은 방송과 공식 플레이어가 제공하는 범위에 따릅니다. 로그인·연령 확인이 필요한 방송은 먼저 로그인하세요.</p>
      <p>{props.receiverBacked ? "시청 종료·배치 변경은 녹화를 중지하지 않습니다. 자동 녹화는 즐겨찾기 옆의 녹화 아이콘으로 설정합니다. 녹화 중에도 로그인할 수 있지만 로그아웃은 녹화 종료 후 가능합니다." : "로그인은 Atsumi 전용 CHZZK 프로필을 사용합니다. 녹화 중에는 방송·모드를 변경할 수 없습니다."}</p>
      <p>다음 실행에도 유지하려면 네이버 로그인 화면에서 ‘로그인 상태 유지’를 선택하세요. QR 로그인도 같은 프로필을 사용하지만 유지 기간은 네이버가 발급한 세션 설정을 따릅니다.</p>
      {props.helpDetails ? <p>{props.helpDetails}</p> : null}
    </div></div></div>
    <form onSubmit={(event) => { event.preventDefault(); props.onConnect(); }}>
      <div className="connection-channel-inputs">
        {props.inputs.slice(0, props.mode === "mado" ? 4 : 1).map((value, index) => <div className="connection-channel-edit-row" key={index}><ChannelDraftField value={value} index={index} multiple={props.mode === "mado"} disabled={props.disabled} privacy={props.privacy ?? true} runtime={props.runtime ?? "browser-mock"} onChange={next => props.onInput(index, next)} onRemove={props.unified && props.inputs.length > 1 ? () => props.onRemoveChannel?.(index) : undefined} /></div>)}
      </div>
      {props.unified ? <button type="button" className="connection-channel-add" data-tour="chzzk-add-channel" aria-label="방송 추가" disabled={props.disabled || props.inputs.length >= 4} onClick={props.onAddChannel}><FluentIcon glyph="\uE710" /></button> : null}
      {!props.channelPicker && directory.error ? <small className="official-browser-error" role="alert">{directory.error}</small> : null}
      {!props.unified ? options : null}
      <div className={`connection-actions${props.unified ? " is-unified" : ""}`}>
        {props.unified ? <div className="connection-view-options">{options}</div> : null}
        <div className="connection-session-actions">
          <button type="submit" className="official-browser-primary" aria-busy={props.pending || undefined} disabled={props.disabled || props.connectDisabled || !props.inputs.slice(0, props.mode === "mado" ? 4 : 1).some((value) => value.trim())}>{props.pending ? <span className="spinner" aria-hidden="true" /> : null}{props.unified && props.watching ? "변경" : "시청 시작"}</button>
          {props.unified && props.watching && props.onStop ? <button type="button" disabled={props.disabled || props.connectDisabled} title="시청만 종료합니다. 녹화는 계속됩니다." onClick={props.onStop}>종료</button> : null}
        </div>
        {!props.unified ? <span className="official-browser-extension-status" role="status">그리드 없이 직접 재생</span> : null}
      </div>
    </form>
    {props.channelPicker}
    <div className="official-browser-account">
      <button type="button" disabled={props.accountDisabled || props.auth === "signed_in"} title={props.accountReason ?? (props.auth === "signed_in" ? "저장된 계정으로 로그인되어 있습니다" : "네이버 로그인 창을 엽니다")} onClick={props.onLogin}>로그인</button>
      <button ref={props.logoutRef} type="button" disabled={props.accountDisabled || props.logoutDisabled || props.auth === "signed_out"} title={props.accountReason ?? "Atsumi에서 로그아웃합니다. 다른 브라우저 계정과 녹화 파일은 그대로 유지됩니다."} onClick={props.onLogout}>로그아웃</button>
      <span role="status" aria-busy={props.authChecking || props.auth === "checking"}>{authLabel}</span>
    </div>
    {props.authError ? <small className="official-browser-error" role="alert">{props.authError}</small> : null}
    {props.accountReason ? <small className="official-browser-muted" role="status">{props.accountReason}</small> : null}
    {props.children}
  </div>;
}
