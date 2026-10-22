import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { WorkspaceViewId } from "../../app/workspaceRegistry";
import { createXSurfaceApi, isXArchiveChooseRequest, xError, type XArchiveChooseRequest, type XSurfaceApi, type XFeed, type XSnapshot, type XSurfaceSnapshot } from "./api";
import { measureNativePlayerGeometry, observeNativePlayerGeometry } from "../platforms/nativePlayerGeometry";
import { XArchiveLibrary, XArchiveTools } from "./XArchivePanel";
import type { XArchiveApi } from "./xArchiveApi";
import { XPreviewTabs } from "./XPreviewTabs";
import { XMediaSearchPanel } from "./XMediaSearchPanel";
import { bindXShellMenuDismissal } from "./xShellMenuDismissal";
import "./XWorkspace.css";

// Survives component remounts and fences reordered native commands.
let serial = 0;
const nextOrder = () => serial = Math.max(serial + 1, Date.now());
// Native closure belongs to the X session, not a React instance. StrictMode and
// a replacement API/component must not open a pane before old cleanup finishes.
let nativeSuspension:Promise<void>=Promise.resolve();
const suspendNative=(api:XSurfaceApi,retainDocuments=false)=>{
  const pending=nativeSuspension.catch(()=>{}).then(()=>api.suspend(retainDocuments));
  nativeSuspension=pending;
  void pending.catch(()=>{}); // The admitting operation reports this failure.
  return pending;
};
async function awaitNativeSuspension(){
  let observed:Promise<void>;
  do{observed=nativeSuspension;await observed;}while(observed!==nativeSuspension);
}
function OfficialIcon({name}:{name:"refresh"|"close"|"check"|"more"|"user"}){
  const paths={refresh:"M20 11a8 8 0 1 0-2.3 6M20 4v7h-7",close:"m6 6 12 12M18 6 6 18",check:"m5 12 4 4 10-10",more:"M5 12h.01M12 12h.01M19 12h.01",user:"M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM4 21v-2a8 8 0 0 1 16 0v2"};
  return <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]}/></svg>;
}
function Skeleton({label}:{label:string}) {
  return <div className="x-skeleton" role="status" aria-label={label}>
    {Array.from({length:4},(_,i)=><div className="x-skeleton-post" key={i} aria-hidden="true"><i/><div><b/><span/><span/><em/></div></div>)}
  </div>;
}
function AccountPortrait({account,id,handle}:{account:XSnapshot|null;id:string;handle:string}) {
  const candidate=account?.presentations?.[id];
  const shown=candidate?.handle.toLowerCase()===handle.toLowerCase()?candidate:null;
  const image=shown?.avatarData?.startsWith("data:image/jpeg;base64,")&&shown.avatarData.length<=65536?shown.avatarData:null;
  const [failed,setFailed]=useState(false);
  useEffect(()=>setFailed(false),[image]);
  return <span className="x-account-avatar" aria-hidden="true">{image&&!failed?<img src={image} alt="" onError={()=>setFailed(true)}/>:handle.slice(0,1).toUpperCase()}</span>;
}
export function XOfficialWorkspace({active,runtime,privacyMode,onNotice,api:provided,archiveApi,view="explore",accountPortalTarget}:{active:boolean;runtime:"tauri"|"browser-mock";privacyMode:boolean;onNotice:(message:string)=>void;api?:XSurfaceApi;archiveApi?:XArchiveApi;view?:WorkspaceViewId<"x">;accountPortalTarget?:HTMLElement|null}) {
  const api=useMemo(()=>provided??createXSurfaceApi(runtime),[provided,runtime]);
  const feed:XFeed=view==="likes"||view==="bookmarks"?view:"foryou";
  const local=view==="local";
  const [account,setAccount]=useState<XSnapshot|null>(null);
  const [surface,setSurface]=useState<XSurfaceSnapshot|null>(null),[error,setError]=useState<string|null>(null);
  const [authError,setAuthError]=useState<string|null>(null);
  const [pending,setPending]=useState(false),[busy,setBusy]=useState<"login"|"switch"|null>(null);
  const [menu,setMenu]=useState(false),[reload,setReload]=useState(0);
  const [previewBusy,setPreviewBusy]=useState(false);
  const previewOperation=useRef<symbol|null>(null);
  const previewPanelId=useId();
  const [archiveChoice,setArchiveChoice]=useState<XArchiveChooseRequest|null>(null);
  const archiveAdmission=useRef<string|null>(null);
  const left=useRef<HTMLDivElement>(null),right=useRef<HTMLDivElement>(null),accountRef=useRef<HTMLDivElement>(null);
  const epoch=useRef(0), refresh=useRef(false), flow=useRef(0),notice=useRef(onNotice);notice.current=onNotice;
  const lastViewport=useRef<{api:XSurfaceApi;generation:number}|null>(null);
  const live=useRef({active,privacyMode,feed,local,busy,pending,surface,account,menu});live.current={active,privacyMode,feed,local,busy,pending,surface,account,menu};
  const accept=(value:XSurfaceSnapshot)=>setSurface(previous=>previous&&previous.generation>value.generation||JSON.stringify(previous)===JSON.stringify(value)?previous:value);
  const releaseArchiveChoice=(id:string)=>{
    if(archiveAdmission.current!==id)return;
    archiveAdmission.current=null;setArchiveChoice(previous=>previous?.id===id?null:previous);
  };
  useEffect(()=>{archiveAdmission.current=null;setArchiveChoice(null);},[api,active,privacyMode,local,feed]);
  useEffect(()=>{previewOperation.current=null;setPreviewBusy(false);},[api,active,privacyMode,local,feed]);
  useEffect(()=>bindXShellMenuDismissal(window,()=>{
    const now=live.current;
    return now.active&&!now.privacyMode&&!now.local&&!now.busy&&!now.pending&&now.surface?.feed===now.feed
      ?{generation:now.surface.generation,dismiss:(generation:number)=>api.dismissMenus(generation)}:null;
  }),[api]);

  useEffect(()=>{
    epoch.current++;flow.current++;setMenu(false);setSurface(null);setPending(false);setBusy(null);
    // Ordinary tab/platform switches conceal retained native documents. Privacy
    // and explicit account operations still use the destructive session reset.
    const closing=suspendNative(api,!privacyMode);
    if(!active||privacyMode)void closing.catch(cause=>notice.current(xError(cause)));
    return()=>{epoch.current++;flow.current++;void suspendNative(api,true);};
  },[api,active,privacyMode,local]);

  useEffect(()=>{
    if(!active||privacyMode||busy)return;
    const request=++epoch.current, order=nextOrder(), force=refresh.current;
    refresh.current=false;setPending(true);setError(null);
    const current=()=>request===epoch.current&&live.current.active&&!live.current.privacyMode;
    void (async()=>{
      try{
        await awaitNativeSuspension();if(!current())return;
        const state=await api.status();if(!current())return;
        setAccount(state);
        if(state.error)setAuthError(state.error);
        if(state.open){setBusy("login");return;}
        if(local)return;
        const value=await api.surface(feed,order,force);if(current()&&value.feed===feed)accept(value);
      }catch(cause){if(current()){setSurface(null);if(!local)setError(xError(cause));}}
      finally{if(current())setPending(false);}
    })();
    return()=>{epoch.current++;};
  },[api,active,privacyMode,feed,local,reload,busy]);

  useEffect(()=>{
    if(!active||privacyMode||local||busy||pending||!surface)return;
    let stopped=false,timer:ReturnType<typeof setTimeout>|undefined;
    const request=epoch.current;
    const poll=async()=>{
      try{const value=await api.surfaceStatus();if(!stopped&&request===epoch.current&&value.feed===live.current.feed)accept(value);}
      catch(cause){if(!stopped&&request===epoch.current)setError(xError(cause));}
      if(!stopped)timer=setTimeout(poll,1250);
    };
    timer=setTimeout(poll,1250);
    return()=>{stopped=true;if(timer)clearTimeout(timer);};
  },[api,active,privacyMode,local,busy,pending,surface?.generation]);

  useEffect(()=>{
    if(!active||privacyMode||local||busy||pending||!api.subscribe)return;
    let stopped=false,unlisten:(()=>void)|undefined;
    const request=epoch.current;
    void api.subscribe(()=>{void Promise.all([api.surfaceStatus(),api.status()]).then(([value,identity])=>{if(!stopped&&request===epoch.current&&value.feed===live.current.feed){accept(value);setAccount(identity);}}).catch(()=>{});})
      .then(stop=>{if(stopped)stop();else unlisten=stop;}).catch(()=>{});
    return()=>{stopped=true;unlisten?.();};
  },[api,active,privacyMode,local,feed,busy,pending]);

  useEffect(()=>{
    if(!api.subscribeArchiveChoose)return;
    let stopped=false,unlisten:(()=>void)|undefined;
    void api.subscribeArchiveChoose(request=>{
      const now=live.current,current=now.surface;
      if(stopped||archiveAdmission.current||!isXArchiveChooseRequest(request)||!now.active||now.privacyMode||now.local||now.busy||now.pending||now.menu
        ||!current||current.generation!==request.generation||current.feed!==request.feed||now.feed!==request.feed
        ||now.account?.active!==request.accountId||current.state!=="ready"
        ||request.pane==="preview"&&(!current.preview||current.previewState!=="ready"))return;
      // Admission is synchronous: bursts from both native panes cannot start
      // two inspections before React renders an occluding selection dialog.
      archiveAdmission.current=request.id;setArchiveChoice(request);
    }).then(stop=>{if(stopped)stop();else unlisten=stop;}).catch(()=>{});
    return()=>{stopped=true;unlisten?.();};
  },[api]);

  useEffect(()=>{
    if(busy!=="login"||!active||privacyMode)return;
    let stopped=false,timer:ReturnType<typeof setTimeout>|undefined;
    const poll=async()=>{
      try{const value=await api.status();if(stopped)return;setAccount(value);if(!value.open){setAuthError(value.error??null);setBusy(null);setReload(n=>n+1);return;}}
      catch(cause){if(!stopped)setError(xError(cause));}
      if(!stopped)timer=setTimeout(poll,1250);
    };
    // account() creates its window asynchronously; do not mistake that interval
    // for a completed login. This effect starts only after account() resolves.
    timer=setTimeout(poll,1250);
    return()=>{stopped=true;if(timer)clearTimeout(timer);};
  },[api,busy,active,privacyMode]);

  useLayoutEffect(()=>{
    if(!surface)return;
    const generation=surface.generation,request=epoch.current;
    lastViewport.current={api,generation};
    let disposed=false;
    const update=()=>{if(disposed)return;const viewport=measureNativePlayerGeometry(left.current,right.current,{sequence:nextOrder(),visible:active&&!privacyMode&&!local&&!busy&&!pending&&surface.feed===feed,muted:false});
      void api.viewport(generation,viewport).catch(cause=>{if(!disposed&&request===epoch.current)notice.current(xError(cause));});};
    update();const stop=observeNativePlayerGeometry([left.current,right.current],update);
    // Replacing/loading the right pane is only a remeasurement. Concealing both
    // native documents here would pause an unrelated manually playing feed.
    return()=>{disposed=true;stop();};
  },[api,surface?.generation,surface?.preview,surface?.state,surface?.previewState,active,privacyMode,local,feed,busy,pending]);
  useLayoutEffect(()=>()=>{
    const last=lastViewport.current;
    if(last?.api===api)void api.viewport(last.generation,{sequence:nextOrder(),player:null,chat:null,visible:false,muted:true,occlusions:[]}).catch(()=>{});
  },[api]);

  useEffect(()=>{
    if(!menu)return;
    const dismiss=(event:Event)=>{if(!accountRef.current?.contains(event.target as Node))setMenu(false);};
    const escape=(event:KeyboardEvent)=>{
      // Native child concealment can leave Escape targeted at body/root. A
      // foreground app modal owns that key even without a focused descendant.
      if([...document.querySelectorAll('[aria-modal="true"],dialog[open],[role="alertdialog"]')].some(node=>{
        if(!node.isConnected||node.closest('[hidden],[inert],[aria-hidden="true"]'))return false;
        for(let current:Element|null=node;current;current=current.parentElement){const style=getComputedStyle(current);if(style.display==="none"||style.visibility==="hidden"||style.visibility==="collapse"||style.opacity==="0")return false;}
        return true;
      }))return;
      if(event.key==="Escape"){event.preventDefault();event.stopPropagation();setMenu(false);accountRef.current?.querySelector("button")?.focus();}
    };
    window.addEventListener("pointerdown",dismiss);window.addEventListener("keydown",escape,true);
    return()=>{window.removeEventListener("pointerdown",dismiss);window.removeEventListener("keydown",escape,true);};
  },[menu]);

  const clear=()=>{epoch.current++;setMenu(false);setArchiveChoice(null);setSurface(null);setError(null);setAuthError(null);setPending(false);return suspendNative(api);};
  const login=async(add=false)=>{
    if(!live.current.active||live.current.privacyMode)return;
    const request=++flow.current,closing=clear();setBusy("switch");
    try{await closing;if(request!==flow.current)return;const value=await api.account(add);if(request!==flow.current)return;setAccount(value);setBusy(value.open?"login":null);setReload(n=>n+1);}
    catch(cause){if(request===flow.current){setBusy(null);setAuthError(xError(cause));}}
  };
  const select=async(id:string)=>{
    if(!live.current.active||live.current.privacyMode)return;
    const request=++flow.current,closing=clear();setBusy("switch");
    try{await closing;if(request!==flow.current)return;const value=await api.select(id);if(request!==flow.current)return;setAccount(value);}
    catch(cause){if(request===flow.current)setAuthError(xError(cause));}
    finally{if(request===flow.current){setBusy(null);setReload(n=>n+1);}}
  };
  const changePreview=(id:string,close=false)=>{
    const now=live.current,current=now.surface;
    if(previewOperation.current||!now.active||now.privacyMode||now.local||now.busy||now.pending||!current
      ||current.feed!==now.feed||!current.previews.some(tab=>tab.id===id))return;
    // Even reselecting the active tab is a newer user intent: native uses it
    // to fence a source-page preview creation that has not finished yet.
    const request=epoch.current,token=Symbol(),generation=current.generation;
    previewOperation.current=token;setPreviewBusy(true);
    void (close?api.closePreview(generation,id):api.selectPreview(generation,id)).then(value=>{
      if(previewOperation.current===token&&request===epoch.current&&live.current.active&&!live.current.privacyMode&&value.feed===live.current.feed)accept(value);
    }).catch(cause=>{
      if(previewOperation.current===token&&request===epoch.current&&live.current.surface?.generation===generation)setError(xError(cause));
    }).finally(()=>{if(previewOperation.current===token){previewOperation.current=null;setPreviewBusy(false);}});
  };
  const selected=account?.accounts?.find(item=>item.id===account.active);
  const message=local?null:authError??error??surface?.error;
  const accountControls=<div className="x-sidebar-controls">
    <div className="x-floating-account" ref={accountRef}>
      <button className="x-account-anchor" type="button" title={!privacyMode&&selected?"@"+selected.handle:"X 로그인"} aria-label={!privacyMode&&selected?"계정":"X 로그인"} aria-haspopup="menu" aria-expanded={menu} disabled={!active||!!busy||privacyMode} onClick={()=>account?.accounts?.length?setMenu(v=>!v):void login()}>
        {!privacyMode&&selected?<AccountPortrait account={account} id={selected.id} handle={selected.handle}/>:<span className="x-account-avatar" aria-hidden="true"><OfficialIcon name="user"/></span>}
        <span className="x-account-copy">{!privacyMode&&selected?"@"+selected.handle:"X 로그인"}</span><span className="x-account-more"><OfficialIcon name="more"/></span>
      </button>
      {menu&&active&&!privacyMode?<div className="x-floating-account-menu" data-native-overlay="true" role="menu" aria-label="저장된 X 계정">
        <div className="x-saved-accounts">{account?.accounts?.map(item=><button className="x-saved-account" role="menuitemradio" aria-checked={item.id===account.active} aria-label={"@"+item.handle} type="button" key={item.id} onClick={()=>item.id===account.active?setMenu(false):void select(item.id)}>
          <AccountPortrait account={account} id={item.id} handle={item.handle}/><span className="x-account-copy">{account.presentations?.[item.id]?.handle.toLowerCase()===item.handle.toLowerCase()&&account.presentations[item.id]?.name?<><strong>{account.presentations[item.id]?.name}</strong><small>@{item.handle}</small></>:"@"+item.handle}</span>{item.id===account.active?<span className="x-account-current" aria-hidden="true"><OfficialIcon name="check"/></span>:null}
        </button>)}</div>
        <button role="menuitem" type="button" onClick={()=>void login(true)}>계정 추가</button><button role="menuitem" type="button" onClick={()=>void login()}>현재 계정 정보 복구</button>
        {!local?<button role="menuitem" type="button" aria-label="새로고침" disabled={pending||!!busy} onClick={()=>{setMenu(false);refresh.current=true;setReload(n=>n+1);}}><OfficialIcon name="refresh"/>새로고침</button>:null}
      </div>:null}
    </div>
  </div>;
  return <section className="x-workspace x-official-workspace" aria-label={local?"X 로컬 저장소":"X 피드"}>
    {accountPortalTarget?createPortal(accountControls,accountPortalTarget):<div className="x-sidebar-fallback">{accountControls}</div>}
    {privacyMode?<p className="x-notice">프라이버시 모드</p>:<>
      {message?<p className="x-notice" role="alert">{message}</p>:null}
      {busy&&!local?<p className="x-notice" role="status">{busy==="login"?"공식 X 창에서 로그인해 주세요. 완료되면 자동으로 돌아옵니다.":"계정 전환 중"}</p>:null}
      {local?<XArchiveLibrary runtime={runtime} api={archiveApi} active={active} privacyMode={privacyMode} accounts={account?.accounts??[]}/>:<div className="x-body">
        <div className="x-native-feed" ref={left} aria-label="X 공식 타임라인" aria-busy={pending||surface?.state==="loading"}>
          {active&&!busy&&(pending||surface?.state==="loading")?<Skeleton label="피드 불러오는 중"/>:null}
        </div>
        <aside className={"x-preview"+(surface?.previews.length?" has-selection":"")} aria-label="X 원본 미리보기">
          {surface?.previews.length?<XPreviewTabs tabs={surface.previews} activeId={surface.activePreviewId}
            panelId={previewPanelId} disabled={!active||pending||!!busy||previewBusy||surface.feed!==feed}
            onSelect={id=>changePreview(id)} onClose={id=>changePreview(id,true)}/>:null}
          <div className="x-native-preview" ref={right} id={previewPanelId} role={surface?.preview?"tabpanel":undefined}
            aria-labelledby={surface?.preview?`${previewPanelId}-tab-${surface.previews.findIndex(tab=>tab.id===surface.activePreviewId)}`:undefined}>
            {!surface?.preview?<p className="x-preview-empty">게시물을 선택하면 여기에 표시됩니다.</p>:surface.previewState!=="ready"?<Skeleton label="미리보기 불러오는 중"/>:null}
          </div>
        </aside>
      </div>}
      {!local?(['feed','preview'] as const).map(pane=><XArchiveTools key={pane} runtime={runtime} api={archiveApi} pane={pane}
        generation={surface?.generation??null} contextKey={`${account?.active??""}:${feed}:${surface?.generation??""}:${pane==="preview"?surface?.activePreviewId:"feed"}`}
        active={active&&!busy&&!pending} privacyMode={privacyMode} disabled={!surface||surface.feed!==feed||surface.state!=="ready"||pane==="preview"&&(!surface.preview||surface.previewState!=="ready")}
        showLibrary={false} showControls={false} chooseRequest={archiveChoice?.pane===pane?archiveChoice:null} onChooseDone={releaseArchiveChoice} onNotice={onNotice}/>):null}
    </>}
    <XMediaSearchPanel runtime={runtime} active={active&&!local&&!busy&&!pending} privacyMode={privacyMode}
      accountId={account?.active??null} generation={surface?.generation??null} feed={feed} onNotice={onNotice}/>
  </section>;
}
// The current explicit requirement uses original X documents in both panes.
export { XOfficialWorkspace as XWorkspace };
