import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { createXMediaSearchApi, isXSearchSnapshot, type XMediaSearchApi, type XSearchSnapshot } from "./xMediaSearchApi";
import "./XMediaSearchPanel.css";

type Props={runtime:"tauri"|"browser-mock";active:boolean;privacyMode:boolean;accountId:string|null;generation:number|null;feed:string;onNotice:(message:string)=>void;api?:XMediaSearchApi};
type Context={api:XMediaSearchApi;accountId:string|null;generation:number|null;feed:string;active:boolean;privacyMode:boolean};
type Session={context:Context;live:boolean;revision:number;busy:boolean;current:XSearchSnapshot|null;retired:Set<string>;cancelled:Set<string>;stop?:()=>void};
const matches=(value:XSearchSnapshot,context:Context)=>value.accountId===context.accountId&&value.generation===context.generation&&value.feed===context.feed;
const same=(a:Context,b:Context)=>a.api===b.api&&a.accountId===b.accountId&&a.generation===b.generation&&a.feed===b.feed&&a.active===b.active&&a.privacyMode===b.privacyMode;
const terminal=(phase:XSearchSnapshot["phase"])=>phase==="complete"||phase==="failed";
const providerName=(provider:XSearchSnapshot["provider"])=>provider==="saucenao"?"SauceNAO":"Google Lens";
const frontmostSearchDialog=(dialog:HTMLElement|null)=>{
  if(!dialog?.isConnected)return false;
  const shown=(node:Element)=>{
    if(!node.isConnected||node.closest('[hidden],[inert],[aria-hidden="true"]'))return false;
    for(let current:Element|null=node;current;current=current.parentElement){
      const style=getComputedStyle(current);
      if(style.display==="none"||style.visibility==="hidden"||style.visibility==="collapse"||style.opacity==="0")return false;
    }
    return true;
  };
  const modals=[...dialog.ownerDocument.querySelectorAll('[aria-modal="true"],dialog[open],[role="alertdialog"]')].filter(shown);
  // Native showModal() dialogs live in the browser top layer, regardless of
  // their earlier DOM position under #root. Conservatively yield to open ones.
  if(modals.some(node=>node!==dialog&&node.tagName==="DIALOG"&&node.hasAttribute("open")))return false;
  return shown(dialog)&&modals.at(-1)===dialog;
};

function SearchDialog({snapshot,busy,error,onClose,onConfirm,onResume,onReset,onOpen}:{snapshot:XSearchSnapshot;busy:boolean;error:string;onClose:()=>void;onConfirm:(remember:boolean)=>void;onResume:()=>void;onReset:()=>void;onOpen:(candidate:number,link:number)=>void}){
  const [remember,setRemember]=useState(false),dialog=useRef<HTMLDivElement>(null),close=useRef<HTMLButtonElement>(null);
  const dismiss=useRef(onClose);dismiss.current=onClose;
  useEffect(()=>{
    const previous=document.activeElement,mounted=dialog.current;close.current?.focus({preventScroll:true});
    const escape=(event:KeyboardEvent)=>{
      // Hiding the previously focused native child can return keyboard focus
      // to the main document/body after the initial button.focus(). The current
      // foreground modal still owns Escape; no native/provider focus is taken.
      if(event.key!=="Escape"||event.defaultPrevented||event.isComposing||event.keyCode===229
        ||event.ctrlKey||event.altKey||event.metaKey||event.shiftKey||!frontmostSearchDialog(mounted))return;
      const target=event.target;
      const atMainWindow=event.currentTarget!==null&&target===event.currentTarget;
      if(!atMainWindow&&target!==document&&(!(target instanceof Node)||target.ownerDocument!==document))return;
      if(target instanceof Element){const owner=target.closest('[aria-modal="true"],dialog[open],[role="alertdialog"]');if(owner&&owner!==mounted)return;}
      event.preventDefault();event.stopImmediatePropagation();dismiss.current();
    };
    window.addEventListener("keydown",escape,true);
    return()=>{
      window.removeEventListener("keydown",escape,true);
      if((document.activeElement===document.body||mounted?.contains(document.activeElement))&&previous instanceof HTMLElement&&previous.isConnected&&!previous.closest('[hidden],[inert],[aria-hidden="true"]'))previous.focus({preventScroll:true});
    };
  },[]);
  let content:ReactNode;
  if(snapshot.phase==="consent")content=<>
    <p>선택한 미디어로 원본을 찾습니다. 이미지는 <strong>SauceNAO에 이미지 주소</strong>를 보내 서비스가 이미지를 가져오고, 영상은 <strong>Google Lens에 현재 장면 한 장</strong>을 전송합니다.</p>
    <p className="x-media-search-muted">X 계정·로그인 정보와 영상 전체는 보내지 않습니다. 검색을 직접 선택했을 때만 전송합니다.</p>
    <p className="x-media-search-muted">전송 자료에는 해당 검색 서비스의 보관 정책이 적용됩니다.</p>
    <p className="x-media-search-muted">검색 서비스 세션은 X와 분리해 이 PC에 보관하며, 서비스에서 만료되면 다시 확인이 필요할 수 있습니다.</p>
    <div className="x-media-search-selection"><strong>이번 전송 · {providerName(snapshot.provider)}</strong>
      {snapshot.previewData?<img src={snapshot.previewData} alt="검색할 현재 영상 장면"/>:<code>{snapshot.imageUrl}</code>}
    </div>
    <label className="x-media-search-remember"><input type="checkbox" checked={remember} disabled={busy} onChange={event=>setRemember(event.target.checked)}/>다시 묻지 않기</label>
  </>;
  else if(snapshot.phase==="searching")content=<div className="x-media-search-working" role="status"><span aria-hidden="true"/>{snapshot.message||`${providerName(snapshot.provider)}에서 검색 중…`}</div>;
  else if(snapshot.phase==="verification")content=<>
    <p role="status">{snapshot.message||`${providerName(snapshot.provider)}에서 직접 확인이 필요합니다.`}</p>
    <p>열린 {providerName(snapshot.provider)} 서비스 창에서 직접 확인한 뒤, 아래의 ‘확인 완료 후 계속’을 눌러 주세요. 수동 확인은 최대 10분까지 기다립니다.</p>
    <p className="x-media-search-muted">확인하는 동안 자동 검색은 멈춥니다. 계속하면 이미 열린 결과를 확인하며, 미디어를 다시 전송하지 않습니다.</p>
    <p className="x-media-search-muted">서비스 세션은 X와 분리된 로컬 프로필에 보관됩니다. 만료되면 다시 확인이 필요할 수 있으며, 검색 전송 동의의 ‘다시 묻지 않기’와는 별개입니다.</p>
  </>;
  else content=<>
    {snapshot.message?<p role={snapshot.phase==="failed"?"alert":"status"}>{snapshot.message}</p>:null}
    {snapshot.phase==="failed"?null:snapshot.candidates.length?<>
      <p className="x-media-search-muted">검색 서비스가 제시한 후보입니다. 유사도는 서비스의 비교 점수이며 원작자임을 보증하지 않습니다.</p>
      <ol className="x-media-search-candidates">{snapshot.candidates.map((candidate,index)=><li key={index}>
        <h3>{candidate.title||"제목 정보 없음"}</h3>
        {candidate.creator?<p>작가 후보: {candidate.creator}</p>:null}
        {candidate.similarity!==null?<p className="x-media-search-muted">유사도 {candidate.similarity.toFixed(1)}%</p>:null}
        <div className="x-media-search-links">{candidate.links.map((link,linkIndex)=><button key={linkIndex} type="button" disabled={busy} onClick={()=>onOpen(index,linkIndex)} title={link.url}>
          <span>{link.label||"출처 열기"}</span><small>{new URL(link.url).hostname}</small>
        </button>)}</div>
      </li>)}</ol>
    </>:!snapshot.message?<p role="status">일치하는 후보를 찾지 못했습니다.</p>:null}
  </>;
  return createPortal(<div className="x-media-search-backdrop" data-native-overlay="true" onContextMenu={event=>event.preventDefault()}>
    <div className="x-media-search-dialog" role="dialog" aria-modal="true" aria-label="미디어 원본 찾기" ref={dialog} tabIndex={-1} onKeyDown={event=>{
      if(event.key==="Tab"){
        const controls=[...dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),[tabindex="0"]')??[]];
        const first=controls[0],last=controls.at(-1);
        if(!first){event.preventDefault();dialog.current?.focus();}
        else if(!controls.includes(document.activeElement as HTMLElement)){event.preventDefault();(event.shiftKey?last:first)?.focus();}
        else if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}
        else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}
      }
    }}>
      <header><h2>미디어 원본 찾기</h2><button ref={close} type="button" aria-label="검색 창 닫기" onClick={onClose}>×</button></header>
      <div className="x-media-search-content">{content}{error?<p role="alert">{error}</p>:null}</div>
      <footer>{snapshot.phase==="consent"?<><button type="button" onClick={onClose}>취소</button><button type="button" className="x-media-search-confirm" disabled={busy} onClick={()=>onConfirm(remember)}>확인 후 전송</button></>
        :snapshot.phase==="verification"?<><button type="button" onClick={onClose}>검색 취소</button><button type="button" className="x-media-search-confirm" disabled={busy} onClick={onResume}>확인 완료 후 계속</button></>
        :<><button type="button" className="x-media-search-reset" disabled={busy} onClick={onReset}>검색 전송 동의 초기화</button><button type="button" onClick={onClose}>{snapshot.phase==="searching"?"검색 취소":"닫기"}</button></>}</footer>
    </div>
  </div>,document.body);
}

/** A main-window overlay; never navigates, mounts or replaces an X document. */
export function XMediaSearchPanel({runtime,active,privacyMode,accountId,generation,feed,onNotice,api:provided}:Props){
  const api=useMemo(()=>provided??createXMediaSearchApi(runtime),[provided,runtime]);
  const context:Context={api,active,privacyMode,accountId,generation,feed};
  const live=useRef(context),notify=useRef(onNotice);live.current=context;notify.current=onNotice;
  const session=useRef<Session|null>(null),[snapshot,setSnapshot]=useState<XSearchSnapshot|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState("");
  const eligible=active&&!privacyMode&&accountId!==null&&generation!==null;
  const current=(token:Session)=>token.live&&session.current===token&&same(token.context,live.current);
  const cancel=(token:Session,id:string)=>{
    if(token.cancelled.has(id))return;token.cancelled.add(id);
    void token.context.api.cancel(id).catch(()=>{if(current(token))notify.current("검색 취소 상태를 확인하지 못했습니다.");});
  };
  const accept=(token:Session,value:XSearchSnapshot|null)=>{
    if(value!==null&&!isXSearchSnapshot(value))return;
    if(!current(token)){
      if(value&&matches(value,token.context))cancel(token,value.id);
      return;
    }
    if(value&&!matches(value,token.context))return;
    if(value&&token.retired.has(value.id))return;
    const old=token.current;
    if(value&&old?.id===value.id&&(terminal(old.phase)||old.phase!=="consent"&&value.phase==="consent"))return;
    if(old&&old.id!==value?.id){token.retired.add(old.id);if(value)cancel(token,old.id);}
    // Limit retained tombstones without re-admitting an old request in this context.
    if(value&&old?.id!==value.id&&token.retired.size>=256){cancel(token,value.id);return;}
    if(old?.id!==value?.id)token.busy=false;
    token.current=value;setSnapshot(value);setError("");setBusy(token.busy);
  };
  useEffect(()=>{
    setSnapshot(null);setBusy(false);setError("");
    if(!eligible)return;
    const token:Session={context:{api,active,privacyMode,accountId,generation,feed},live:true,revision:0,busy:false,current:null,retired:new Set(),cancelled:new Set()};session.current=token;
    void (async()=>{
      try{
        const stop=await api.subscribe(value=>{token.revision++;accept(token,value);});
        if(!current(token)){stop();return;}token.stop=stop;
        const revision=token.revision,value=await api.status();
        if(revision===token.revision)accept(token,value);
        else if(!current(token)&&value&&matches(value,token.context))cancel(token,value.id);
      }catch{if(current(token))notify.current("검색 상태를 불러오지 못했습니다.");}
    })();
    return()=>{token.live=false;token.stop?.();if(token.current)cancel(token,token.current.id);if(session.current===token)session.current=null;};
  },[api,active,privacyMode,accountId,generation,feed]);
  const close=(expectedId:string)=>{
    const token=session.current;if(!token||!current(token)||!token.current||token.current.id!==expectedId)return;
    const id=token.current.id;token.retired.add(id);token.current=null;token.revision++;token.busy=false;setSnapshot(null);setError("");setBusy(false);cancel(token,id);
  };
  const command=async(action:"confirm"|"resume"|"reset"|"open",remember=false,candidateIndex=0,linkIndex=0)=>{
    const token=session.current,value=token?.current;
    if(!token||!value||!current(token)||value!==snapshot||token.busy)return;
    if(action==="confirm"&&value.phase!=="consent"||action==="resume"&&value.phase!=="verification"||action==="open"&&value.phase!=="complete")return;
    const revision=token.revision;token.busy=true;setBusy(true);setError("");
    try{
      if(action==="confirm"||action==="resume"){
        const result=await (action==="confirm"?api.confirm(value.id,remember):api.resume(value.id));
        if(current(token)&&token.current?.id===value.id&&revision===token.revision)accept(token,result);
        else if(!current(token)&&result&&matches(result,token.context))cancel(token,result.id);
      }else if(action==="reset"){
        await api.resetConsent();if(current(token)&&token.current===value)notify.current("다음 검색부터 전송 동의를 다시 확인합니다.");
      }else await api.openResult(value.id,candidateIndex,linkIndex);
    }catch{if(current(token)&&token.current?.id===value.id)setError(action==="confirm"?"검색을 시작하지 못했습니다. 다시 확인해 주세요.":action==="resume"?"검색을 계속하지 못했습니다. 서비스 창의 확인 상태를 다시 확인해 주세요.":action==="reset"?"전송 동의를 초기화하지 못했습니다.":"검색 링크를 열지 못했습니다.");}
    finally{if(current(token)&&token.current?.id===value.id){token.busy=false;setBusy(false);}}
  };
  if(!eligible||!snapshot||!matches(snapshot,context)||!session.current||!current(session.current))return null;
  return <SearchDialog key={snapshot.id} snapshot={snapshot} busy={busy} error={error} onClose={()=>close(snapshot.id)} onConfirm={remember=>void command("confirm",remember)} onResume={()=>void command("resume")} onReset={()=>void command("reset")} onOpen={(candidate,link)=>void command("open",false,candidate,link)}/>;
}
