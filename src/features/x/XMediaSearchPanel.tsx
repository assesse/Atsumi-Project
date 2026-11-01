import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { createXMediaSearchApi, isXSearchSnapshot, type XCreatorLinks, type XMediaSearchApi, type XSearchSnapshot, type XSearchProvider, type XProviderResult } from "./xMediaSearchApi";
import "./XMediaSearchPanel.css";
import { SourceText, XPageEvidence } from "./XPageEvidence";
import { XWorkEvidence } from "./XWorkEvidence";

type Props={runtime:"tauri"|"browser-mock";active:boolean;privacyMode:boolean;accountId:string|null;generation:number|null;feed:string;localContext?:string|null;navigationRequestId?:string|null;onNotice:(message:string)=>void;api?:XMediaSearchApi};
type Context={api:XMediaSearchApi;accountId:string|null;generation:number|null;feed:string;active:boolean;privacyMode:boolean;localContext:string|null};
type Session={context:Context;live:boolean;revision:number;busy:boolean;current:XSearchSnapshot|null;retired:Set<string>;cancelled:Set<string>;creatorStarted:Set<string>;stop?:()=>void};
const matches=(value:XSearchSnapshot,context:Context)=>value.source==="archive"?context.localContext!==null&&value.localContext===context.localContext:context.localContext===null&&value.accountId===context.accountId&&value.generation===context.generation&&value.feed===context.feed;
const same=(a:Context,b:Context)=>a.api===b.api&&a.accountId===b.accountId&&a.generation===b.generation&&a.feed===b.feed&&a.active===b.active&&a.privacyMode===b.privacyMode&&a.localContext===b.localContext;
const terminal=(phase:XSearchSnapshot["phase"])=>phase==="complete"||phase==="failed";
const providerName=(provider:XSearchSnapshot["provider"])=>provider==="saucenao"?"SauceNAO":"Google Lens";
const providerResults=(snapshot:XSearchSnapshot):XProviderResult[]=>snapshot.providers??(snapshot.phase==="consent"?[]:[{provider:snapshot.provider,phase:snapshot.phase,message:snapshot.message,candidates:snapshot.candidates,originalPageState:snapshot.originalPageState}]);
const originalStateText=(state:XProviderResult["originalPageState"])=>state==="available"?"같은 검색 화면 열기":state==="expired"?"검색 화면 보관 기간이 지났습니다.":state==="closed"?"검색 화면이 닫혔습니다.":"열 수 있는 검색 화면이 없습니다.";
const resultLinkText=(link:{label:string;url:string},provider:XSearchSnapshot["provider"])=>{
  const url=new URL(link.url);
  // Native preparation supplies observed direct destinations when available.
  // An unresolved wrapper remains a truthful fallback, never a guessed source.
  if(provider==="lens"&&url.hostname==="www.google.com"&&["/goto","/url"].includes(url.pathname))return{label:"결과 페이지 열기",detail:"google.com · 주소 확인 필요"};
  return{label:link.label||"출처 열기",detail:url.hostname};
};
// Only this fixed native SETTINGS_ERROR is safe and actionable on confirmation.
// Never display arbitrary rejection text or change the user's consent choice.
const consentSettingsError="검색 동의 설정을 저장하지 못했습니다. 다시 묻지 않기를 해제하고 다시 검색해 주세요.";
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

function SearchDialog({api,snapshot,busy,error,creator,creatorLoading,creatorError,onClose,onConfirm,onResume,onReset,onOpen,onOriginal,onLensSettings,onLensRefresh,onCreatorLink}:{api:XMediaSearchApi;snapshot:XSearchSnapshot;busy:boolean;error:string;creator:XCreatorLinks|null;creatorLoading:boolean;creatorError:string;onClose:()=>void;onConfirm:(remember:boolean)=>void;onResume:(provider?:XSearchProvider)=>void;onReset:()=>void;onOpen:(candidate:number,link:number,provider?:XSearchProvider)=>void;onOriginal:(provider?:XSearchProvider)=>void;onLensSettings:()=>void;onLensRefresh:()=>void;onCreatorLink:(index:number)=>void}){
  const [remember,setRemember]=useState(false),dialog=useRef<HTMLDivElement>(null),close=useRef<HTMLButtonElement>(null);
  const [phrase,setPhrase]=useState("");
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
  const consent=snapshot.phase==="consent";
  const results=providerResults(snapshot);
  const mediaLabel=snapshot.kind==="image"?"이미지":snapshot.kind==="thumbnail"?"썸네일":"장면";
  const mediaAlt=consent?(snapshot.kind==="image"?"검색할 다운로드 이미지":snapshot.kind==="thumbnail"?"검색할 영상 썸네일":"검색할 현재 영상 장면"):(snapshot.kind==="image"?"검색에 사용한 다운로드 이미지":snapshot.kind==="thumbnail"?"검색에 사용한 영상 썸네일":"검색에 사용한 영상 장면");
  return createPortal(<div className="x-media-search-backdrop" data-native-overlay="true" onContextMenu={event=>event.preventDefault()}>
    <div className="x-media-search-dialog" role="dialog" aria-modal="true" aria-label="미디어 원본 찾기" ref={dialog} tabIndex={-1} onKeyDown={event=>{
      if(event.key==="Tab"){
        const controls=[...dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),summary,[tabindex="0"]')??[]];
        const first=controls[0],last=controls.at(-1);
        if(!first){event.preventDefault();dialog.current?.focus();}
        else if(!controls.includes(document.activeElement as HTMLElement)){event.preventDefault();(event.shiftKey?last:first)?.focus();}
        else if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}
        else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}
      }
    }}>
      <header><h2>미디어 원본 찾기</h2><button ref={close} type="button" aria-label="검색 창 닫기" onClick={onClose}>×</button></header>
      <div className="x-media-search-content">
        <div className="x-media-search-context">
          <div className="x-media-search-selection">
            <strong>{consent?"검색할":"검색에 사용한"} {mediaLabel}</strong>
            {snapshot.previewData?<img src={snapshot.previewData} alt={mediaAlt}/>:snapshot.imageUrl?<code>{snapshot.imageUrl}</code>:null}
          </div>
          {snapshot.sourcePostUrl?<section className="x-media-search-creator" aria-label="게시자 프로필에 등록된 링크" aria-busy={creatorLoading}>
            <div className="x-media-search-creator-heading"><h3>게시자 프로필</h3><span>{creator?new URL(creator.profileUrl).pathname.replace("/","@"):new URL(snapshot.sourcePostUrl).pathname.split("/")[1]&&`@${new URL(snapshot.sourcePostUrl).pathname.split("/")[1]}`}</span></div>
            {creatorLoading?<span className="x-media-search-inline-status" role="status"><i className="x-media-search-spinner" aria-hidden="true"/>링크 확인 중</span>:null}
            {creatorError?<span className="x-media-search-muted" role="status">{creatorError}</span>:null}
            {creator?<><div className="x-media-search-links">{creator.links.map((link,index)=><button key={index} type="button" disabled={busy} title={link.url} onClick={()=>onCreatorLink(index)}><span>{link.label||new URL(link.url).hostname}</span><small>{link.url}</small></button>)}</div>{!creator.links.length?<span className="x-media-search-muted">등록된 링크 없음</span>:null}</>:null}
          </section>:null}
        </div>
        {!consent&&results.some(result=>result.candidates.length)?<label className="x-media-search-phrase">문구 찾기<input type="search" value={phrase} maxLength={128} placeholder="결과·페이지 정보에서 찾기" onChange={event=>setPhrase(event.target.value)}/></label>:null}
        {!consent&&terminal(snapshot.phase)&&!creatorLoading&&(!snapshot.sourcePostUrl||creator||creatorError)&&(results.some(result=>result.candidates.length)||creator?.links.length)?<XWorkEvidence api={api} jobId={snapshot.id} sourceKey={JSON.stringify([results.map(result=>[result.provider,result.phase,result.candidates.map(candidate=>candidate.links.map(link=>link.url))]),creator?.links.map(link=>link.url)])} phrase={phrase} busy={busy}/>:null}
        {consent?<div className="x-media-search-consent">
          <div className="x-media-search-destinations" aria-label="동시 검색 서비스"><span>SauceNAO</span><span>Google Lens</span></div>
          <p>{snapshot.source==="archive"?"다운로드한 파일의":"선택한"} {mediaLabel} 한 장을 두 서비스에 보내 검색합니다.</p>
          <details><summary>전송 정보</summary><p className="x-media-search-muted">X 계정·로그인 정보, 파일 경로와 영상 전체는 보내지 않습니다. 서비스별 보관 정책이 적용되며 검색 서비스 세션은 X와 분리됩니다.</p></details>
          <label className="x-media-search-remember"><input type="checkbox" checked={remember} disabled={busy} onChange={event=>setRemember(event.target.checked)}/>다시 묻지 않기</label>
        </div>:<div className="x-media-search-providers">
          {results.map(result=>{
            const provider=snapshot.providers?result.provider:undefined;
            const searching=result.phase==="searching",verification=result.phase==="verification";
            return <section key={result.provider} className="x-media-search-provider" aria-label={`${providerName(result.provider)} 검색 결과`}>
              <header><h3>{providerName(result.provider)}</h3><span className={`x-media-search-provider-state is-${result.phase}`} role="status">{searching?"검색 중":verification?"확인 필요":result.phase==="failed"?"검색 실패":`${result.candidates.length}개`}</span>
                <div className="x-media-search-provider-actions">
                  {result.provider==="lens"&&terminal(result.phase)?<>
                    <button type="button" aria-label="Google Lens 흐림 설정" title="Lens 전용 세이프서치에서 흐림을 조정합니다" disabled={busy||result.originalPageState!=="available"} onClick={onLensSettings}>흐림 설정</button>
                    <button type="button" aria-label="Google Lens 결과 새로고침" title="이미지를 다시 전송하지 않고 기존 검색 결과를 갱신합니다" disabled={busy||!terminal(snapshot.phase)||!["available","closed"].includes(result.originalPageState)} onClick={onLensRefresh}>새로고침</button>
                  </>:null}
                  <button type="button" className="x-media-search-original" aria-label={`${providerName(result.provider)} 원본 검색 화면`} title={originalStateText(result.originalPageState)} disabled={busy||result.originalPageState!=="available"} onClick={()=>onOriginal(provider)}>검색 화면 ↗</button>
                </div>
              </header>
              {searching?<div className="x-media-search-working"><span aria-hidden="true"/>{result.message||"검색 중…"}</div>:null}
              {verification?<div className="x-media-search-provider-notice"><p>{result.message||"서비스 창에서 확인을 완료해 주세요."}</p><button type="button" disabled={busy} onClick={()=>onResume(provider)}>확인 완료 후 계속</button></div>:null}
              {result.phase==="failed"?<p className="x-media-search-provider-notice" role="alert">{result.message||"검색 결과를 불러오지 못했습니다."}</p>:null}
              {result.phase==="complete"&&!result.candidates.length?<div className="x-media-search-empty" role="status">{result.message||"검색 결과 없음"}</div>:null}
              {result.candidates.length?<ol className="x-media-search-candidates">{result.candidates.map((candidate,index)=><li key={index}>
                {candidate.thumbnailData?<img className="x-media-search-result-image" src={candidate.thumbnailData} alt={`${providerName(result.provider)} 검색 결과 ${index+1} 미리보기`}/>:null}
                <div className="x-media-search-result-details"><h3>
                  <button type="button" className="x-media-search-title" aria-label={candidate.title||"제목 정보 없음"} title={candidate.links[0]?.url} disabled={busy||!candidate.links.length} onClick={()=>onOpen(index,0,provider)}><span className="x-media-search-title-text"><SourceText text={candidate.title||"제목 정보 없음"} phrase={phrase}/></span>{candidate.links[0]?<> <small>{resultLinkText(candidate.links[0],result.provider).detail}</small></>:null}</button>
                  {candidate.links.slice(1).map((link,offset)=><button key={offset} type="button" className="x-media-search-destination" disabled={busy} title={link.url} onClick={()=>onOpen(index,offset+1,provider)}>{resultLinkText(link,result.provider).detail}</button>)}
                </h3>
                  {candidate.creator?<p className="x-media-search-candidate-author"><SourceText text={candidate.creator} phrase={phrase}/></p>:null}
                  {candidate.similarity!==null?<span className="x-media-search-score" title="서비스의 유사도 점수 · 원작자 확인 아님">유사도 {candidate.similarity}%</span>:null}
                  {candidate.sourceMetadata.length?<dl className="x-media-search-metadata">{candidate.sourceMetadata.map((item,field)=><div key={field}><dt>{item.label}</dt><dd><SourceText text={item.value} phrase={phrase}/></dd></div>)}</dl>:null}
                  <XPageEvidence api={api} jobId={snapshot.id} candidateIndex={index} provider={provider} links={candidate.links} phrase={phrase}/>
                </div>
              </li>)}</ol>:null}
            </section>;
          })}
          {!results.length&&snapshot.phase==="failed"?<p role="alert">{snapshot.message||"검색을 시작하지 못했습니다."}</p>:null}
        </div>}
        {error?<p role="alert">{error}</p>:null}
      </div>
      <footer>{consent?<><button type="button" onClick={onClose}>취소</button><button type="button" className="x-media-search-confirm" disabled={busy} onClick={()=>onConfirm(remember)}>확인 후 전송</button></>
        :<><button type="button" className="x-media-search-reset" disabled={busy} onClick={onReset}>검색 전송 동의 초기화</button><span className="x-media-search-action-status x-media-search-muted" role={busy?"status":undefined} aria-live="polite">{busy?"처리 중":""}</span><button type="button" onClick={onClose}>{snapshot.phase==="searching"||snapshot.phase==="verification"?"검색 취소":"닫기"}</button></>}</footer>
    </div>
  </div>,document.body);
}

/** A main-window overlay; never navigates, mounts or replaces an X document. */
export function XMediaSearchPanel({runtime,active,privacyMode,accountId,generation,feed,localContext=null,navigationRequestId=null,onNotice,api:provided}:Props){
  const lastNavigationRequest=useRef(navigationRequestId);
  const api=useMemo(()=>provided??createXMediaSearchApi(runtime),[provided,runtime]);
  const context:Context={api,active,privacyMode,accountId,generation,feed,localContext};
  const live=useRef(context),notify=useRef(onNotice);live.current=context;notify.current=onNotice;
  const session=useRef<Session|null>(null),[snapshot,setSnapshot]=useState<XSearchSnapshot|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(""),[creator,setCreator]=useState<XCreatorLinks|null>(null);
  const [creatorLoading,setCreatorLoading]=useState(false),[creatorError,setCreatorError]=useState("");
  const eligible=active&&!privacyMode&&(localContext!==null||accountId!==null&&generation!==null);
  const current=(token:Session)=>token.live&&session.current===token&&same(token.context,live.current);
  const cancel=(token:Session,id:string)=>{
    if(token.cancelled.has(id))return;token.cancelled.add(id);
    void token.context.api.cancel(id).catch(()=>{if(current(token))notify.current("검색 취소 상태를 확인하지 못했습니다.");});
  };
  const accept=(token:Session,value:XSearchSnapshot|null,lensRefresh=false)=>{
    if(value!==null&&!isXSearchSnapshot(value))return;
    if(!current(token)){
      if(value&&matches(value,token.context))cancel(token,value.id);
      return;
    }
    if(value&&!matches(value,token.context))return;
    if(value&&token.retired.has(value.id))return;
    const old=token.current;
    if(value&&old?.id===value.id){
      if(old.phase!=="consent"&&value.phase==="consent")return;
      if(terminal(old.phase)){
        if(lensRefresh){
          const lens=providerResults(value).find(item=>item.provider==="lens"&&item.phase==="complete");
          if(!lens)return;
          // Only this explicit same-job reply may replace terminal Lens data.
          // Preserve SauceNAO and every frozen source/context field.
          value={...old,phase:"complete",message:"",...(old.provider==="lens"?{candidates:lens.candidates,originalPageState:lens.originalPageState}:{}),...(old.providers?{providers:old.providers.map(item=>item.provider==="lens"?lens:item)}:{})};
        }else{
          if(value.phase!==old.phase)return;
          value={...old,originalPageState:value.originalPageState,...(old.providers?{providers:old.providers.map(item=>({...item,originalPageState:value!.providers?.find(next=>next.provider===item.provider)?.originalPageState??item.originalPageState}))}:{})};
        }
      }
    }
    if(old&&old.id!==value?.id){token.retired.add(old.id);if(value)cancel(token,old.id);}
    // Limit retained tombstones without re-admitting an old request in this context.
    if(value&&old?.id!==value.id&&token.retired.size>=256){cancel(token,value.id);return;}
    if(old?.id!==value?.id){token.busy=false;setCreator(null);setCreatorLoading(false);setCreatorError("");}
    token.current=value;setSnapshot(value);setError("");setBusy(token.busy);
  };
  useEffect(()=>{
    setSnapshot(null);setBusy(false);setError("");setCreator(null);setCreatorLoading(false);setCreatorError("");
    if(!eligible)return;
    const token:Session={context:{api,active,privacyMode,accountId,generation,feed,localContext},live:true,revision:0,busy:false,current:null,retired:new Set(),cancelled:new Set(),creatorStarted:new Set()};session.current=token;
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
  },[api,active,privacyMode,accountId,generation,feed,localContext]);
  useEffect(()=>{
    const token=session.current,value=token?.current;
    if(!eligible||!token||!value||value.id!==snapshot?.id||!value.sourcePostUrl||!current(token)||token.creatorStarted.has(value.id))return;
    token.creatorStarted.add(value.id);setCreatorLoading(true);setCreatorError("");
    // Lookup has its own lifetime and never acquires the search/action busy lock.
    // Only the proven original publisher is read, never unrelated match candidates.
    void api.creatorLinks(value.id).then(result=>{
      if(current(token)&&token.current?.id===value.id)setCreator(result);
    }).catch(()=>{
      if(current(token)&&token.current?.id===value.id)setCreatorError("프로필 링크를 불러오지 못함");
    }).finally(()=>{
      if(current(token)&&token.current?.id===value.id)setCreatorLoading(false);
    });
  },[api,active,privacyMode,accountId,generation,feed,localContext,eligible,snapshot?.id,snapshot?.sourcePostUrl]);
  const close=(expectedId:string)=>{
    const token=session.current;if(!token||!current(token)||!token.current||token.current.id!==expectedId)return;
    const id=token.current.id;token.retired.add(id);token.current=null;token.revision++;token.busy=false;setSnapshot(null);setError("");setBusy(false);cancel(token,id);
  };
  useEffect(()=>{
    if(lastNavigationRequest.current===navigationRequestId)return;
    lastNavigationRequest.current=navigationRequestId;
    // Selecting the already-open X destination does not advance its generation.
    // Close this search through the same cancellation path so it cannot cover it.
    const id=session.current?.current?.id;
    if(navigationRequestId&&id)close(id);
  },[navigationRequestId]);
  const command=async(action:"confirm"|"resume"|"reset"|"open"|"original"|"settings"|"refresh",remember=false,candidateIndex=0,linkIndex=0,provider?:XSearchProvider)=>{
    const token=session.current,value=token?.current;
    if(!token||!value||!current(token)||value!==snapshot||token.busy)return;
    const selected=provider===undefined?value:providerResults(value).find(item=>item.provider===provider);
    if(!selected||action==="confirm"&&value.phase!=="consent"||action==="resume"&&selected.phase!=="verification"||action==="open"&&selected.phase!=="complete"||action==="original"&&selected.originalPageState!=="available")return;
    if((action==="settings"||action==="refresh")&&(selected.provider!=="lens"||!terminal(selected.phase)))return;
    if(action==="settings"&&selected.originalPageState!=="available"||action==="refresh"&&(!terminal(value.phase)||!["available","closed"].includes(selected.originalPageState)))return;
    const revision=token.revision;token.busy=true;setBusy(true);setError("");
    try{
      if(action==="confirm"||action==="resume"){
        const result=await (action==="confirm"?api.confirm(value.id,remember):provider===undefined?api.resume(value.id):api.resume(value.id,provider));
        if(current(token)&&token.current?.id===value.id&&revision===token.revision)accept(token,result);
        else if(!current(token)&&result&&matches(result,token.context))cancel(token,result.id);
      }else if(action==="refresh"){
        const result=await api.refreshLensResults(value.id);
        if(result.id!==value.id)throw new Error("Lens 검색이 변경됐습니다.");
        if(current(token)&&token.current?.id===value.id)accept(token,result,true);
      }else if(action==="settings"){
        await api.openLensSettings(value.id);
        if(current(token)&&token.current?.id===value.id)notify.current("세이프서치에서 ‘사용 안함’을 선택한 뒤 새로고침해 주세요. 구글이 연령 확인을 요구하면 해당 창에서 직접 진행해 주세요.");
      }else if(action==="reset"){
        await api.resetConsent();if(current(token)&&token.current===value)notify.current("다음 검색부터 전송 동의를 다시 확인합니다.");
      }else if(action==="original")await (provider===undefined?api.openOriginal(value.id):api.openOriginal(value.id,provider));
      else await (provider===undefined?api.openResult(value.id,candidateIndex,linkIndex):api.openResult(value.id,candidateIndex,linkIndex,provider));
    }catch(cause){
      if(current(token)&&token.current?.id===value.id){
        if(action==="open"&&(cause instanceof Error?cause.message:cause)==="SEARCH_RESULT_REDIRECT_UNRESOLVED"){
          setError("이동 주소를 확인하지 못했습니다. 열린 검색 화면에서 링크를 확인해 주세요.");
          return;
        }
        const knownConsentFailure=action==="confirm"&&(cause instanceof Error?cause.message:cause)===consentSettingsError;
        setError(knownConsentFailure?consentSettingsError:action==="confirm"?"검색을 시작하지 못했습니다. 다시 확인해 주세요.":action==="resume"?"검색을 계속하지 못했습니다. 서비스 창의 확인 상태를 다시 확인해 주세요.":action==="settings"?"Lens 설정 창을 열지 못했습니다. 검색 화면이 남아 있는지 확인해 주세요.":action==="refresh"?"Lens 결과를 갱신하지 못했습니다. 설정·인증 상태나 검색 보관 시간을 확인해 주세요. 미디어를 다시 전송하지 않았습니다.":action==="reset"?"전송 동의를 초기화하지 못했습니다.":action==="original"?"원본 검색 창이 닫혔거나 보관 시간이 지났습니다. 미디어를 다시 전송하지 않았습니다.":"검색 링크를 열지 못했습니다.");
      }
    }
    finally{if(current(token)&&token.current?.id===value.id){token.busy=false;setBusy(false);}}
  };
  const openCreator=async(index:number)=>{
    const token=session.current,value=token?.current;
    if(!token||!value||!current(token)||value.id!==snapshot?.id||token.busy||!creator)return;
    token.busy=true;setBusy(true);setError("");
    try{
      await api.openCreatorLink(value.id,index);
    }catch{
      if(current(token)&&token.current?.id===value.id)setError("프로필 링크를 열지 못했습니다.");
    }finally{if(current(token)&&token.current?.id===value.id){token.busy=false;setBusy(false);}}
  };
  if(!eligible||!snapshot||!matches(snapshot,context)||!session.current||!current(session.current))return null;
  return <SearchDialog key={snapshot.id} api={api} snapshot={snapshot} busy={busy} error={error} creator={creator} creatorLoading={creatorLoading} creatorError={creatorError} onClose={()=>close(snapshot.id)} onConfirm={remember=>void command("confirm",remember)} onResume={provider=>void command("resume",false,0,0,provider)} onReset={()=>void command("reset")} onOpen={(candidate,link,provider)=>void command("open",false,candidate,link,provider)} onOriginal={provider=>void command("original",false,0,0,provider)} onLensSettings={()=>void command("settings",false,0,0,"lens")} onLensRefresh={()=>void command("refresh",false,0,0,"lens")} onCreatorLink={index=>void openCreator(index)}/>;
}
