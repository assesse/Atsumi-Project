import { useId, useState, type ReactNode, type Ref } from "react";
import type { OfficialBrowserSnapshot } from "../../api/officialBrowser";

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
  auth: AccountStatus; accountDisabled: boolean; accountReason?: string; onLogin(): void; onLogout(): void;
  onRefresh(): void; onGrid(): void; gridDisabled: boolean; gridStatus: string;
  authChecking?: boolean; authError?: string | null; refreshDisabled?: boolean;
  onInstaller(browser: "chrome" | "edge"): void; installerDisabled: boolean;
  logoutRef?: Ref<HTMLButtonElement>; options?: ReactNode; children?: ReactNode; helpDetails?: string;
};

/** Shared form; changing the draft tab never changes a native playback context. */
export function ConnectionSetup(props: Props) {
  const id = useId();
  const [help, setHelp] = useState(false);
  const authLabel = { signed_in: "로그인됨", signed_out: "로그아웃됨", unknown: "계정 상태 확인 필요", checking: "계정 상태 확인 중" }[props.auth];
  return <div className="official-browser-setup-card connection-setup" data-native-dialog-surface="true" onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setHelp(false); }} onMouseLeave={(event) => { if (!event.currentTarget.contains(document.activeElement)) setHelp(false); }}>
    <div className="connection-setup-heading">
      <div className="streaming-watch-modes" role="group" aria-label="시청 모드">
        <button type="button" aria-pressed={props.mode === "general"} disabled={props.modeDisabled} onClick={() => props.onMode("general")}>일반모드</button>
        <button type="button" aria-pressed={props.mode === "mado"} disabled={props.modeDisabled} onClick={() => props.onMode("mado")}>마도모드</button>
      </div>
      <button type="button" className="connection-help" aria-label="연결 도움말" aria-expanded={help} aria-controls={`${id}-help`} aria-describedby={help ? `${id}-help` : undefined} onMouseEnter={() => setHelp(true)} onClick={() => setHelp(true)} onFocus={() => setHelp(true)} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setHelp(false); } }}>?</button>
      {props.onClose ? <button ref={props.closeRef} type="button" aria-label="연결 설정 닫기" onClick={props.onClose}>설정 닫기</button> : null}
    </div>
    <div className={`connection-help-reveal${help ? " is-open" : ""}`} aria-hidden={!help} inert={!help}>
    <div className="connection-help-clip"><div id={`${id}-help`} className="connection-help-content" role="note">
      <p>주소를 입력하고 시청을 시작하세요. 마도는 최대 네 방송을 배치합니다.</p>
      <p>그리드 연결은 설치된 네이버 확장을 연결합니다. 설치만으로 연결되지는 않습니다. 실제 화질은 플레이어에서 확인하세요.</p>
      <p>공식 설치 안내에서는 ‘설치없이 일반 화질 시청’도 선택할 수 있습니다.</p>
      <p>로그인은 Atsumi 전용 CHZZK 프로필을 사용합니다. 녹화 중에는 방송·계정·모드를 변경할 수 없습니다.</p>
      <p>다음 실행에도 유지하려면 네이버 로그인 화면에서 ‘로그인 상태 유지’를 선택하세요. QR 로그인도 같은 프로필을 사용하지만 유지 기간은 네이버가 발급한 세션 설정을 따릅니다.</p>
      {props.helpDetails ? <p>{props.helpDetails}</p> : null}
      <div className="official-browser-installer-actions"><span>확장 설치</span>{(["chrome", "edge"] as const).map((browser) => <button key={browser} type="button" disabled={props.installerDisabled || !help} onClick={() => props.onInstaller(browser)}>{browser === "chrome" ? "Chrome" : "Edge"}</button>)}</div>
    </div></div></div>
    <form onSubmit={(event) => { event.preventDefault(); props.onConnect(); }}>
      <div className="connection-channel-inputs">
        {props.inputs.slice(0, props.mode === "mado" ? 4 : 1).map((value, index) => <label key={index} htmlFor={props.mode === "general" ? "official-browser-channel" : `mado-channel-${index}`}>
          <span className={props.mode === "mado" ? "connection-input-label-hidden" : undefined}>{props.mode === "mado" ? `방송 ${index + 1} 주소 또는 ID` : "채널 주소 또는 ID"}</span>
          <input id={props.mode === "general" ? "official-browser-channel" : `mado-channel-${index}`} value={value} disabled={props.disabled} autoComplete="off" spellCheck={false} placeholder="https://chzzk.naver.com/live/채널ID" onChange={(event) => props.onInput(index, event.target.value)} />
        </label>)}
      </div>
      {props.mode === "mado" ? props.options : null}
      <div className="connection-actions">
        <button type="submit" className="official-browser-primary" disabled={props.disabled || !props.inputs.slice(0, props.mode === "mado" ? 4 : 1).some((value) => value.trim())}>{props.pending ? "연결 중…" : "시청 시작"}</button>
        <button type="button" disabled={props.gridDisabled} title={props.accountReason} onClick={props.onGrid}>그리드 연결</button>
        <span role="status">확장 · {props.gridStatus}</span>
      </div>
    </form>
    <div className="official-browser-account">
      <button type="button" disabled={props.accountDisabled || props.auth === "signed_in"} title={props.accountReason ?? (props.auth === "signed_in" ? "저장된 계정으로 로그인되어 있습니다" : "네이버 로그인 창을 엽니다")} onClick={props.onLogin}>로그인</button>
      <button ref={props.logoutRef} type="button" disabled={props.accountDisabled || props.auth === "signed_out"} title={props.accountReason ?? "Atsumi에서 로그아웃합니다. 다른 브라우저 계정과 녹화 파일은 그대로 유지됩니다."} onClick={props.onLogout}>로그아웃</button>
      <span role="status">{authLabel}</span>
      <button type="button" disabled={(props.refreshDisabled ?? props.accountDisabled) || props.authChecking || props.auth === "checking"} aria-busy={props.authChecking === true} onClick={props.onRefresh}>{props.authChecking ? "확인 중…" : "상태 확인"}</button>
    </div>
    {props.authError ? <small className="official-browser-error" role="alert">{props.authError}</small> : null}
    {props.accountReason ? <small className="official-browser-muted" role="status">{props.accountReason}</small> : null}
    {props.children}
  </div>;
}
