import { useEffect, useState, type CSSProperties, type ReactNode, type Ref, type UIEvent } from 'react';
import type { MediaReplayChatItem, MediaReplayChatPage } from '../platforms/platformMedia';
import { PlatformChatParts } from '../platforms/PlatformChatParts';
import { OriginalSoopGiftMessage, validSoopGift } from './OriginalSoopGiftMessage';
import { OriginalSoopEmoticon } from './OriginalSoopEmoticon';
import { validSoopEmoticon, soopNicknameClass } from './soopEmoticon';
import { OriginalSoopSubscriptionMessage } from './OriginalSoopSubscriptionMessage';
import { validSoopSubscription } from './soopSubscription';
import { OriginalSoopMissionMessage } from './OriginalSoopMissionMessage';
import { isSoopMissionRow } from './soopMission';
import { OriginalSoopPersonalcon } from './OriginalSoopPersonalcon';
import { isSoopNightbot } from './soopPersonalcon';

const roles: Record<string, string> = { streamer:'스트리머', manager:'매니저', vip:'열혈팬', fan:'팬클럽', support:'서포터', subscribe:'구독', nightbot:'나이트봇' };
const time = (ms: number) => { const seconds=Math.max(0,Math.floor(ms/1000));return `${Math.floor(seconds/60)}:${String(seconds%60).padStart(2,'0')}`; };
function Action({ children, label, className, disabled, expanded, onClick }: { children:ReactNode; label:string; className?:string; disabled?:boolean; expanded?:boolean; onClick():void }) {
  return <a role="button" tabIndex={disabled?-1:0} aria-label={label} aria-disabled={disabled||undefined} aria-expanded={expanded} title={label} className={className}
    onClick={event=>{event.preventDefault();if(!disabled)onClick();}} onKeyDown={event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation();if(!disabled)onClick();}}}>{children}</a>;
}
function Toggle({ id, label, value, change }: { id:string;label:string;value:boolean;change(value:boolean):void }) {
  return <li><div className="checkbox_wrap"><input id={id} type="checkbox" checked={value} onChange={event=>change(event.target.checked)}/><label htmlFor={id}>{label}</label></div></li>;
}
export interface OriginalSoopReplayChatViewProps {
  page:MediaReplayChatPage|null; images:ReadonlyMap<string,string>; privacyMode:boolean; loading:boolean; error:boolean;
  settings:boolean; setSettings(value:boolean):void; profile:MediaReplayChatItem|null; setProfile(value:MediaReplayChatItem|null):void;
  showTimes:boolean; setShowTimes(value:boolean):void; fontLevel:number; setFontLevel(value:number):void; showEmoji:boolean; setShowEmoji(value:boolean):void;
  smallEmoji:boolean; setSmallEmoji(value:boolean):void; animateEmoji:boolean; setAnimateEmoji(value:boolean):void;
  following:boolean; preparationPercent:number|null; scrollRef:Ref<HTMLDivElement>; onScroll(event:UIEvent<HTMLDivElement>):void;
  onFollow():void; onNavigate(cursor:string|null):void; onRetry():void; onClose?:()=>void;
  theme?:'light'|'dark'; randomNicknameColor?:boolean; presentationLabel?:string;
  paletteStyle?:CSSProperties;
  rowDisplay?:(row:MediaReplayChatItem)=>{fanMessageColor:boolean;showEmoticons:boolean;smallOgq:boolean};
}
/** LIVE markup and original CSS; saved values only, never an online chat connection. */
export function OriginalSoopReplayChatView(p:OriginalSoopReplayChatViewProps) {
  const [section,setSection]=useState<'main'|'person'|'mark'>('main');
  useEffect(()=>{if(!p.settings)setSection('main');},[p.settings]);
  const openSettings=()=>{p.setSettings(!p.settings);p.setProfile(null);};
  const profile=p.privacyMode?null:p.profile;
  const capturedForeground=!!(p.paletteStyle as Record<string,string>|undefined)?.['--soop-captured-foreground'];
  return <div id="chatting_area" font-level={String(p.fontLevel)}><div id="chatbox" className={`chatbox${p.randomNicknameColor !== false ? ' random-color-on' : ''}`}>
    <div className="area_header"><div className="chat_title platform-replay-chat-header"><h2><span>채팅</span></h2><ul>
      <li className="set"><Action label="채팅 표시 설정" disabled={p.privacyMode} expanded={p.settings} onClick={openSettings}>설정</Action></li>
      {p.onClose?<li className="close"><Action label="재생 닫기" onClick={p.onClose}>닫기</Action></li>:null}
    </ul></div></div>
    {p.privacyMode?<p className="soop-replay-status">프라이버시 모드</p>:<>
      {p.settings?<div role="dialog" aria-label="채팅 설정" className={`chat_layer on ${section==='main'?'setting_chat':`sub ${section}`}`}>
        <div className="area_header">{section!=='main'?<Action className="history_back" label="설정으로 돌아가기" onClick={()=>setSection('main')}>뒤로가기</Action>:null}<h2><span>{section==='main'?'설정':section==='mark'?'이모티콘 표시 방법':'개인 설정'}</span></h2></div>
        <div className="contents">{section==='main'?<>
          <ul><li><Action className="more_layer" label="이모티콘 표시 방법" onClick={()=>setSection('mark')}>이모티콘 표시 방법</Action></li><li><Action className="more_layer" label="개인 설정" onClick={()=>setSection('person')}>개인 설정</Action></li></ul>
          <div className="font-level"><span>채팅 크기 설정</span><div className="ctrl"><output aria-label="채팅 글자 크기">{p.fontLevel}</output><button type="button" className="minus" aria-label="글자 크기 줄이기" disabled={p.fontLevel<=1} onClick={()=>p.setFontLevel(Math.max(1,p.fontLevel-1))}>-</button><button type="button" className="plus" aria-label="글자 크기 키우기" disabled={p.fontLevel>=8} onClick={()=>p.setFontLevel(Math.min(8,p.fontLevel+1))}>+</button></div><div className="preview" font-level={String(p.fontLevel)}><div className="chatting-list-item"><div className="message-container"><div className="message-text"><p className="msg">채팅 글자 크기를 확인하세요.</p></div></div></div></div></div>
        </>:<ul>{section==='mark'?<><Toggle id="replay_emoticon" label="이모티콘 보기" value={p.showEmoji} change={p.setShowEmoji}/><Toggle id="replay_emoticon_ani" label="움직이는 이모티콘 보기" value={p.animateEmoji} change={p.setAnimateEmoji}/><Toggle id="replay_ogq" label="OGQ 이모티콘 작게 보기" value={p.smallEmoji} change={p.setSmallEmoji}/></>:<Toggle id="replay_time" label="시각 표시" value={p.showTimes} change={p.setShowTimes}/>}</ul>}</div>
        {p.presentationLabel?<p className="soop-replay-timing">{p.presentationLabel}</p>:null}
        {p.page?.timelineQuality==='receive-time-approximate'?<p className="soop-replay-timing" title="영상 조각과 채팅을 받은 시각으로 맞췄습니다. 실제 장면과 차이가 날 수 있습니다.">수신 시각 기준</p>:null}
        <Action className="close" label="채팅 설정 닫기" onClick={()=>p.setSettings(false)}>닫기</Action>
      </div>:null}
      {p.page?.state==='unmapped'?<p className="soop-replay-note">시간 정보 없음 · 기록 순서</p>:null}
      {p.page&&(p.page.malformedLines>0||p.page.partialFile)?<p className="soop-replay-note" title={p.page.malformedLines>0?'읽을 수 없는 기록은 건너뛰었습니다. 원본 기록은 변경하지 않습니다.':'채팅 보존 상태가 완전히 확인되지 않았습니다. 원본 기록은 변경하지 않습니다.'}>일부 기록만 표시될 수 있습니다.</p>:null}
      {p.page?.displayMetadataUnavailable?<p className="soop-replay-note" title="배지용 보조 기록을 읽지 못했습니다. 저장된 채팅은 계속 재생됩니다.">배지 정보를 불러오지 못했습니다.</p>:null}
      <div className="chatting-item-wrap"><div id="chatArea" ref={p.scrollRef} className={`chatting-viewer platform-replay-chat-messages${capturedForeground?' soop-captured-foreground':''}`} style={p.paletteStyle} role="log" aria-live="off" onScroll={p.onScroll}><div className="chat_memoyo">
        {!p.page&&p.loading?<p role="status" className="soop-replay-status">불러오는 중</p>:null}
        {p.page?.state==='indexing'?!p.error&&<p role="status" className="soop-replay-status">채팅 준비 중{p.preparationPercent!==null?` ${p.preparationPercent}%`:''}</p>:p.page?.state==='missing'?<p className="soop-replay-status">저장된 채팅이 없습니다.</p>:p.page?.state==='empty'?<p className="soop-replay-status">표시할 채팅이 없습니다.</p>:p.page?.items.map(row=>{
          if(row.kind==='chat'&&(row.soop?.messageType==='staff'||row.soop?.messageType==='police')){
            const style=row.soop.messageType==='staff'?'notice':'warning';
            return <div key={row.id} data-chat-row-id={row.id} className="chatting-list-item platform-replay-chat-row">
              {p.showTimes&&row.timeMs!==null?<time className="soop-replay-time">{time(row.timeMs)}</time>:null}
              <div className={`bubble-container ${style}`}><strong className="title"><i className={`ic-${style}`}/>{row.author} 안내</strong><p className="msg platform-replay-chat-text">{row.text}</p></div>
            </div>;
          }
          if(isSoopMissionRow(row))return <OriginalSoopMissionMessage key={row.id} row={row} images={p.images} showTimes={p.showTimes} onProfile={()=>{p.setProfile(row);p.setSettings(false);}}/>;
          if(row.kind==='membership' &&validSoopSubscription(row.soopSubscription))return <OriginalSoopSubscriptionMessage key={row.id} row={{...row,soopSubscription:row.soopSubscription}} images={p.images} showTimes={p.showTimes} onProfile={()=>{p.setProfile(row);p.setSettings(false);}}/>;
          if(validSoopGift(row))return <OriginalSoopGiftMessage key={row.id} row={row} images={p.images} showTimes={p.showTimes} onProfile={()=>{p.setProfile(row);p.setSettings(false);}}/>;
          const role=row.soop?.role&&roles[row.soop.role]?row.soop.role:'';
          const badge=row.soop?.badge&&roles[row.soop.badge]?row.soop.badge:'';
          const display=p.rowDisplay?.(row)??{fanMessageColor:true,showEmoticons:p.showEmoji,smallOgq:p.smallEmoji};
          // SOOP's ordinary message renderer keeps Nightbot's own color; the
          // OGQ renderer only excludes the broadcaster from its palette.
          const nicknameClass=isSoopNightbot(row.soop)&&!(row.kind==='chat'&&validSoopEmoticon(row.soopEmoticon))?'':soopNicknameClass(row.soop?.nicknameColorKey,role);
          const system=['gap','connected','ended','system'].includes(row.kind),gift=['donation','membership'].includes(row.kind);
          const messageColor=display.fanMessageColor&&row.soop&&((row.soop.flag1&32)!==0)&&row.soop.messageColor&&/^#[a-f\d]{6}$/i.test(row.soop.messageColor)?row.soop.messageColor:undefined;
          return <div key={row.id} data-chat-row-id={row.id} className={`chatting-list-item platform-replay-chat-row${row.kind==='chat'?' soop-replay-ordinary':''}${system?` soop-replay-system is-${row.kind}`:''}${gift?' soop-replay-gift':''}`} user-type={role}>
            <div className="message-container">{p.showTimes&&row.timeMs!==null?<time className="soop-replay-time">{time(row.timeMs)}</time>:null}
              {row.author?<div className="username"><button type="button" className="platform-replay-chat-author" onClick={()=>{p.setProfile(row);p.setSettings(false);}}>
                <OriginalSoopPersonalcon metadata={row.soop} images={p.images}/>
                {badge&&badge!=='nightbot'?<span className={`grade-badge-${badge}`} title={roles[badge]} aria-label={roles[badge]}>{roles[badge]}</span>:null}
                <span className={`author ${nicknameClass}`}>{row.author}</span>
              </button></div>:null}
              <div className="message-text">{row.kind==='chat'&&validSoopEmoticon(row.soopEmoticon)?<OriginalSoopEmoticon value={row.soopEmoticon} images={p.images} show={display.showEmoticons} small={display.smallOgq} animate={p.animateEmoji} theme={p.theme}/>:null}{row.amount?<strong className="soop-replay-gift-amount">{row.amount}</strong>:null}<p className="msg platform-replay-chat-text" style={{color:messageColor}}>{display.showEmoticons?<PlatformChatParts text={row.text} parts={row.parts} images={p.images}/>:row.parts?.length?row.parts.slice(0,128).map(part=>part.type==='text'?part.text:part.alt||'[이모티콘]').join(''):row.text}</p></div>
            </div>
          </div>;
        })}
      </div></div>{!p.following&&p.page?.state==='ready'?<div className="chat_scroll_down"><button type="button" className="scroll_down" aria-label="현재 시각으로" title="현재 시각으로" onClick={p.onFollow}>현재 시각으로</button></div>:null}</div>
      {p.page?.state==='unmapped'?<div className="soop-replay-pagination"><button type="button" disabled={!p.page.previousCursor||p.loading} onClick={()=>p.onNavigate(p.page!.previousCursor)}>이전</button><button type="button" disabled={!p.page.cursor||p.loading} onClick={()=>p.onNavigate(p.page!.cursor)}>다음</button></div>:null}
      {p.error?<p role="alert" className="soop-replay-status is-error">채팅을 불러오지 못했습니다.<button type="button" onClick={p.onRetry}>다시 시도</button></p>:null}
      <div className="actionbox soop-replay-readonly" aria-label="녹화 채팅 · 전송할 수 없음">
        <div className="chat_item_list"><ul className="item_box">
          <li className="star"><a role="button" tabIndex={-1} aria-disabled="true" title="녹화 재생에서는 사용할 수 없습니다.">별풍선 선물하기</a></li>
          <li className="adballoon"><a role="button" tabIndex={-1} aria-disabled="true" title="녹화 재생에서는 사용할 수 없습니다.">애드벌룬 후원하기</a></li>
          <li className="sooptore"><a role="button" tabIndex={-1} aria-disabled="true" title="녹화 재생에서는 사용할 수 없습니다.">숲토어</a></li>
        </ul></div>
        <div className="chat_write"><h3>채팅쓰기</h3><div className="input_chat"><div className="write_area" role="textbox" aria-label="녹화된 채팅입니다." aria-readonly="true" aria-disabled="true">녹화된 채팅입니다.</div></div>
          <button type="button" className="btn_emoticon" disabled aria-label="이모티콘 · 녹화 재생에서는 사용할 수 없음">이모티콘 레이어 열기</button>
          <button type="button" className="btn_send" disabled aria-label="보내기 · 녹화 재생에서는 사용할 수 없음"><span>보내기</span></button>
        </div>
      </div>
      {profile?<div role="dialog" aria-label="채팅 사용자 정보" className="chat_layer on setting_chat soop-replay-profile"><div className="area_header"><h2><span>사용자 정보</span></h2></div><div className="contents"><strong>{profile.author}</strong><small>저장 당시 정보</small>{profile.badges.length?<ul>{profile.badges.slice(0,8).map((badge,index)=><li key={index}>{badge}</li>)}</ul>:<p>추가 정보 없음</p>}</div><Action className="close" label="사용자 정보 닫기" onClick={()=>p.setProfile(null)}>닫기</Action></div>:null}
    </>}
  </div></div>;
}
