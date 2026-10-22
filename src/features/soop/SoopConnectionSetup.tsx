import type { ReactNode } from "react";
import { FluentIcon } from "../../components/FluentIcon";
import { SoopConnectionHelp } from "./SoopConnectionHelp";

export type SoopLiveArrangement = "channels" | "grouped";
type Props = {
  favorites: ReactNode; children: ReactNode; error?: string;
  active: boolean; watching: boolean; disabled: boolean; ready: boolean; pending: boolean; rowCount: number;
  mode: "all" | "selected"; onMode: (mode: "all" | "selected") => void;
  channels: { channelId: string; name: string }[]; lead: string; onLead: (id: string) => void;
  arrangement: SoopLiveArrangement; onArrangement: (value: SoopLiveArrangement) => void;
  onAdd: () => void; onStart: () => void; onStop: () => void; onClose: () => void;
  onLogin?: () => void; onLogout?: () => void; accountLabel?: string; accountBusy?: boolean; authenticated?: boolean;
};

/** Presentation is copied from ConnectionSetup; all service state remains SOOP-owned. */
export function SoopConnectionSetup(props: Props) {
  return <div className="soop-connection-setup" data-tour="soop-connection">
    <div className="soop-connection-setup-heading">
      <strong>방송 보기</strong>
      <SoopConnectionHelp enabled={props.active}>
        <p>주소 입력 후 시청 시작 · +로 최대 4개</p>
        <p>로그인과 채팅은 SOOP 원본 화면에서 진행</p>
        <p>즐겨찾기 옆 녹화 아이콘으로 자동 녹화 설정</p>
        <p>시청을 종료해도 녹화는 계속됩니다.</p>
        <p>모두 보기 또는 선택 보기로 영상 배치</p>
      </SoopConnectionHelp>
      <button type="button" className="soop-connection-setup-close soop-drawer-close" aria-label="방송 보기 닫기" title="닫기" onClick={props.onClose}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button>
    </div>
    {props.favorites}
    <form className="soop-input-area" onSubmit={event => { event.preventDefault(); props.onStart(); }}>
      <div className="soop-connection-channel-inputs">{props.children}</div>
      <button type="button" className="soop-connection-channel-add" aria-label="방송 추가" disabled={props.disabled || props.rowCount >= 4} onClick={props.onAdd}><FluentIcon glyph={'\uE710'} /></button>
      <div className="soop-connection-account">
        <button type="button" disabled={!props.onLogin || props.disabled || props.accountBusy || props.authenticated === true} onClick={props.onLogin}>로그인</button>
        <button type="button" disabled={!props.onLogout || props.disabled || props.accountBusy || props.authenticated !== true} onClick={props.onLogout}>로그아웃</button>
        <span role="status">{props.accountLabel || "계정 확인 전"}</span>
      </div>
      {props.error && !props.disabled ? <p className="soop-inline-error" role="alert">{props.error}</p> : null}
      <div className="soop-watch-actions soop-connection-actions">
        <div className="soop-view-choice soop-connection-view-options">
          <div className="soop-mode" role="group" aria-label="영상 표시" data-mode={props.mode}>
            <button type="button" aria-pressed={props.mode === "all"} disabled={props.disabled} onClick={() => props.onMode("all")}>모두 보기</button>
            <button type="button" aria-pressed={props.mode === "selected"} disabled={props.disabled} onClick={() => props.onMode("selected")}>선택 보기</button>
          </div>
          <label className="soop-broadcast-select">
            <span>{props.mode === "selected" ? "영상 방송" : "화면 배치"}</span>
            <span className="soop-broadcast-select-control">
              {props.mode === "selected" ? <select aria-label="볼 영상" value={props.lead} disabled={!props.channels.length || props.disabled} onChange={event => props.onLead(event.target.value)}>
                <option value="" disabled>채널 선택</option>
                {props.channels.map(channel => <option key={channel.channelId} value={channel.channelId}>{props.disabled ? "채널" : channel.name}</option>)}
              </select> : <select aria-label="화면 배치" value={props.arrangement} disabled={props.disabled} onChange={event => props.onArrangement(event.target.value as SoopLiveArrangement)}>
                <option value="channels">방송 · 채팅씩</option><option value="grouped">방송끼리 · 채팅끼리</option>
              </select>}
              <FluentIcon glyph={'\uE70D'} />
            </span>
          </label>
        </div>
        <div className="soop-connection-session-actions">
          <button type="submit" className={`soop-primary ${props.watching ? "soop-change" : "soop-start"}`} disabled={!props.ready || props.disabled} aria-busy={props.pending || undefined}>{props.pending ? <span className="soop-spinner" aria-hidden="true" /> : null}{props.watching ? "변경" : "시청 시작"}</button>
          {props.watching ? <button type="button" className="soop-stop" title="시청만 종료합니다. 녹화는 계속됩니다." onClick={props.onStop}>종료</button> : null}
        </div>
      </div>
    </form>
  </div>;
}
