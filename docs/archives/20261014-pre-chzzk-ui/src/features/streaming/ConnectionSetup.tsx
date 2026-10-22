import { type ReactNode, type Ref } from "react";
import type { OfficialBrowserSnapshot } from "../../api/officialBrowser";
import type { LiveChannelsApi } from "../../api/liveChannels";
import type { AutoRecordingApi } from "../../api/autoRecording";
import { LiveChannelDirectory, useLiveChannelDirectory } from "./LiveChannelDirectory";
import { ChannelDraftField } from "./ChannelDraftField";
import { FluentIcon } from "../../components/FluentIcon";
import { normalizeMultiviewChannel } from "../../api/multiview";
import { ConnectionHelp } from "./ConnectionHelp";

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
  const directory = useLiveChannelDirectory()!;
  const authLabel = { signed_in: "로그인됨", signed_out: "로그아웃됨", unknown: "계정 상태 확인 대기", checking: "계정 상태 확인 중" }[props.auth];
  const channelNames = props.inputs.map((input, index) => {
    const channel = normalizeMultiviewChannel(input);
    if (props.privacy) return `방송 ${index + 1}`;
    return channel ? directory.profiles[channel]?.channelName || directory.channels.find(row => row.channelId === channel)?.channelName || props.knownChannelNames?.[channel] || `채널 ${channel.slice(0, 6)}…` : "주소를 입력하세요";
  });
  const options = props.mode === "mado" ? typeof props.options === "function" ? props.options(channelNames) : props.options : null;
  return <div className="official-browser-setup-card connection-setup" data-tour="chzzk-connection" data-native-dialog-surface="true">
    <div className="connection-setup-heading">
      {props.unified ? <strong>방송 보기</strong> : <div className="streaming-watch-modes" role="group" aria-label="시청 모드">
        <button type="button" aria-pressed={props.mode === "general"} disabled={props.modeDisabled} onClick={() => props.onMode("general")}>일반모드</button>
        <button type="button" data-tour="chzzk-mado-mode" aria-pressed={props.mode === "mado"} disabled={props.modeDisabled} onClick={() => props.onMode("mado")}>마도모드</button>
      </div>}
      <ConnectionHelp>
      <p>주소 입력 후 시청 시작 · +로 최대 4개</p>
      <p>그리드 없이 직접 재생</p>
      <p>즐겨찾기 옆 녹화 아이콘으로 자동 녹화 설정</p>
      <p>{props.receiverBacked ? "시청을 종료해도 녹화는 계속됩니다." : "녹화 중에는 방송·모드를 변경할 수 없습니다."}</p>
      <p>로그인 유지·보안코드는 네이버 창에서 설정</p>
      </ConnectionHelp>
      {props.onClose ? <button ref={props.closeRef} type="button" className="connection-setup-close" data-tour="chzzk-connection-close" aria-label="연결 설정 닫기" title="닫기" onClick={props.onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button> : null}
    </div>
    {props.channelPicker}
    <form onSubmit={(event) => { event.preventDefault(); props.onConnect(); }}>
      <div className="connection-channel-inputs">
        {props.inputs.slice(0, props.mode === "mado" ? 4 : 1).map((value, index) => <div className="connection-channel-edit-row" key={index}><ChannelDraftField value={value} index={index} multiple={props.mode === "mado"} disabled={props.disabled} privacy={props.privacy ?? true} runtime={props.runtime ?? "browser-mock"} onChange={next => props.onInput(index, next)} onRemove={props.unified && props.inputs.length > 1 ? () => props.onRemoveChannel?.(index) : undefined} /></div>)}
      </div>
      {props.unified ? <button type="button" className="connection-channel-add" data-tour="chzzk-add-channel" aria-label="방송 추가" disabled={props.disabled || props.inputs.length >= 4} onClick={props.onAddChannel}><FluentIcon glyph="\uE710" /></button> : null}
      {!props.channelPicker && directory.error ? <small className="official-browser-error" role="alert">{directory.error}</small> : null}
      {!props.unified ? options : null}
    <div className="official-browser-account">
      <button type="button" disabled={props.accountDisabled || props.auth === "signed_in"} title={props.accountReason ?? (props.auth === "signed_in" ? "저장된 계정으로 로그인되어 있습니다" : "네이버 로그인 창을 엽니다")} onClick={props.onLogin}>로그인</button>
      <button ref={props.logoutRef} type="button" disabled={props.accountDisabled || props.logoutDisabled || props.auth === "signed_out"} title={props.accountReason ?? "Atsumi에서 로그아웃합니다. 다른 브라우저 계정과 녹화 파일은 그대로 유지됩니다."} onClick={props.onLogout}>로그아웃</button>
      <span role="status" aria-busy={props.authChecking || props.auth === "checking"}>{authLabel}</span>
    </div>
    {props.authError ? <small className="official-browser-error" role="alert">{props.authError}</small> : null}
    {props.accountReason ? <small className="official-browser-muted" role="status">{props.accountReason}</small> : null}
    {props.children}
      <div className={`connection-actions${props.unified ? " is-unified" : ""}`}>
        {props.unified ? <div className="connection-view-options">{options}</div> : null}
        <div className="connection-session-actions">
          <button type="submit" className={`official-browser-primary${props.unified && props.watching ? " connection-change" : ""}`} aria-busy={props.pending || undefined} disabled={props.disabled || props.connectDisabled || !props.inputs.slice(0, props.mode === "mado" ? 4 : 1).some((value) => value.trim())}>{props.pending ? <span className="spinner" aria-hidden="true" /> : null}{props.unified && props.watching ? "변경" : "시청 시작"}</button>
          {props.unified && props.watching && props.onStop ? <button type="button" className="connection-stop" disabled={props.disabled || props.connectDisabled} title="시청만 종료합니다. 녹화는 계속됩니다." onClick={props.onStop}>종료</button> : null}
        </div>
        {!props.unified ? <span className="official-browser-extension-status" role="status">그리드 없이 직접 재생</span> : null}
      </div>
    </form>
  </div>;
}
