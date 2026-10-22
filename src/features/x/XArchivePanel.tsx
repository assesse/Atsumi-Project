import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { isXArchiveChooseRequest, xError, type XArchiveChooseRequest } from "./api";
import { createXArchiveApi, isXArchiveProgress, type XArchiveApi, type XArchiveDraft, type XArchiveIntent, type XArchivePane, type XArchiveRecord, type XArchiveSummary, type XCapturePost, type XArchiveProgress } from "./xArchiveApi";
import {ArchiveMediaTiles,ArchivePost,ArchiveProvenance,ArchiveSymbol,archiveRoot,XArchiveReader} from "./XArchivePost";
export {XArchiveReader} from "./XArchivePost";
import "./XArchivePanel.css";

const date = (value:string|number) => {
  const at=new Date(value);
  return Number.isFinite(at.getTime())?at.toLocaleString("ko-KR",{year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit",second:"2-digit",hour12:false}):"시각 정보 없음";
};
const statusId = (url:string) => url.match(/^https:\/\/x\.com\/[A-Za-z0-9_]{1,15}\/status\/([0-9]{1,24})$/)?.[1];
const authorLabel = (value:string) => value.replace(/\s+/g," ").trim();
const hasMedia = (post:XCapturePost) => post.media.some(media=>["image","video","unavailable_image","unavailable_video"].includes(media.kind));
class ArchiveReadTimeout extends Error {
  constructor(){super("보관 데이터를 읽는 데 시간이 오래 걸립니다. 다시 시도해 주세요.");}
}
const readDeadline = <T,>(work:Promise<T>,discardLate?:(value:T)=>void):Promise<T> => new Promise((resolve,reject)=>{
  let expired=false;
  const timer=setTimeout(()=>{expired=true;reject(new ArchiveReadTimeout());},15000);
  work.then(value=>{if(expired)discardLate?.(value);else resolve(value);},reject).finally(()=>clearTimeout(timer));
});
function ArchiveIcon({name}:{name:"save"|"download"|"library"}) {
  const path={save:"M5 3h12l3 3v15H4V3h1m3 0v6h8V3M8 21v-8h8v8",download:"M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4",library:"M3 4h5v16H3V4Zm7 0h5v16h-5V4Zm7 1 4 15"};
  return <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={path[name]}/></svg>;
}
function ArchiveDialog({title,busy,onClose,children}:{title:string;busy:boolean;onClose:()=>void;children:ReactNode}) {
  const dialog=useRef<HTMLDivElement>(null),close=useRef<HTMLButtonElement>(null);
  useEffect(()=>{
    const previous=document.activeElement;close.current?.focus({preventScroll:true});
    return()=>{if(previous instanceof HTMLElement&&previous.isConnected&&!previous.closest('[hidden],[inert]'))previous.focus({preventScroll:true});};
  },[]);
  return createPortal(<div className="x-archive-backdrop" data-native-overlay="true">
    <div className="x-archive-dialog" role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={dialog} onKeyDown={event=>{
      if(event.key==="Escape"){event.preventDefault();event.stopPropagation();if(!busy)onClose();}
      if(event.key==="Tab"){
        const controls=[...dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),video[controls],summary,[tabindex="0"]')??[]].filter(control=>{
          const closed=control.closest('details:not([open])');return !closed||closed.firstElementChild===control;
        });
        const first=controls[0],last=controls.at(-1);
        if(!first){event.preventDefault();dialog.current?.focus();}
        else if(!controls.includes(document.activeElement as HTMLElement)){event.preventDefault();(event.shiftKey?last:first)?.focus();}
        else if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}
        else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}
      }
    }}>
      <header><h2>{title}</h2><button ref={close} type="button" aria-label="보관 창 닫기" disabled={busy} onClick={onClose}>×</button></header>
      {children}
    </div>
  </div>,document.body);
}

type LibraryContentProps={record:XArchiveRecord|null;items:XArchiveSummary[];pending:boolean;error:string;api:XArchiveApi;onList:()=>void;onRead:(id:string)=>void;onFolder:()=>void};
function ArchiveLibraryContent({record,items,pending,error,api,onList,onRead,onFolder}:LibraryContentProps) {
  return record?<><div className="x-archive-library-actions"><button type="button" disabled={pending} onClick={onList}>목록</button><button type="button" disabled={pending} aria-label="보관 폴더 열기" onClick={onFolder}>폴더 열기</button></div><XArchiveReader record={record} api={api}/></>:<>
    <div className="x-archive-library-actions"><span>{items.length}개 보관</span><button type="button" disabled={pending} onClick={onList}>새로고침</button></div>
    {pending?<p className="x-archive-empty" role="status">보관 데이터 불러오는 중</p>:items.length?<div className="x-archive-list">{items.map(item=><button type="button" key={item.id} onClick={()=>onRead(item.id)}><strong>{date(item.capturedAt)}</strong><span>{item.posts}개 항목 · {item.assets}개 파일{item.selectedContentComplete?"":" · 일부 보관"}</span><small>{item.sourceUrl}</small>{item.issues>0?<span>누락 {item.issues}건</span>:null}</button>)}</div>:!error?<p className="x-archive-empty">보관된 항목이 없습니다.</p>:null}
  </>;
}

type LibraryProps={runtime:"tauri"|"browser-mock";active?:boolean;privacyMode?:boolean;api?:XArchiveApi;accounts?:ReadonlyArray<{id:string;handle:string}>};
type LibraryTab="body"|"media";
const accountKey=(accountId:string|undefined)=>accountId?`account:${accountId}`:"unknown";
const inLibraryScope=(item:XArchiveSummary,tab:LibraryTab,account:string)=>item.intent===(tab==="body"?"archive":"media")&&(account==="all"||accountKey(item.accountId)===account);
const LIBRARY_PAGE_SIZE=4,LIBRARY_READ_CONCURRENCY=2;
type LibraryReadLane={work:Promise<XArchiveRecord>|null;wake:(()=>void)|null};
type LibraryCapture={record:XArchiveRecord;error?:never}|{record?:never;error:string};
function ArchiveTimelinePost({record,api,onOpen,selected}:{record:XArchiveRecord;api:XArchiveApi;onOpen:()=>void;selected:boolean}) {
  const post=archiveRoot(record);
  return <div className="x-archive-timeline-post" data-archive-id={record.id}>
    {post?<ArchivePost record={record} post={post} api={api} onOpen={onOpen} selected={selected}/>:<p>보관된 내용이 없습니다.</p>}
    <ArchiveProvenance record={record}/>
  </div>;
}
/** Standalone sidebar view: local records never require a live X account or native pane. */
export function XArchiveLibrary({runtime,active=true,privacyMode=false,api:provided,accounts=[]}:LibraryProps) {
  const api=useMemo(()=>provided??createXArchiveApi(runtime),[provided,runtime]);
  const [items,setItems]=useState<XArchiveSummary[]>([]),[record,setRecord]=useState<XArchiveRecord|null>(null);
  const [tab,setTab]=useState<LibraryTab>("body"),[accountFilter,setAccountFilter]=useState("all"),[focusPost,setFocusPost]=useState<string|null>(null);
  const [captures,setCaptures]=useState<ReadonlyMap<string,LibraryCapture>>(new Map()),[visibleCount,setVisibleCount]=useState(0);
  const [pending,setPending]=useState<"list"|"hydrate"|"folder"|null>(null),[error,setError]=useState("");
  const epoch=useRef(0),flight=useRef(false),alive=useRef(false);
  const readLanes=useRef<LibraryReadLane[]>(Array.from({length:LIBRARY_READ_CONCURRENCY},()=>({work:null,wake:null})));
  const timeline=useRef<HTMLDivElement>(null),timelineScroll=useRef<Record<string,number>>({}),scopeCounts=useRef<Record<string,number>>({});
  const scope=useRef({tab,accountFilter});scope.current={tab,accountFilter};
  const cached=useRef(captures);cached.current=captures;
  const scopedItems=items.filter(item=>inLibraryScope(item,tab,accountFilter));
  const accountOptions=[...new Set(items.map(item=>item.accountId))].map(id=>({key:accountKey(id),label:id?(accounts.find(account=>account.id===id)?.handle?`@${accounts.find(account=>account.id===id)!.handle}`:`저장 계정 (${id.slice(0,8)})`):"계정 미확인"}));
  const live=useRef({active,privacyMode,api});live.current={active,privacyMode,api};
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;epoch.current++;};},[]);
  const current=(request:number)=>alive.current&&request===epoch.current&&live.current.active&&!live.current.privacyMode&&live.current.api===api;
  const start=(operation:NonNullable<typeof pending>)=>{if(flight.current||!live.current.active||live.current.privacyMode||live.current.api!==api)return null;flight.current=true;setPending(operation);setError("");return ++epoch.current;};
  const finish=(request:number)=>{if(current(request)){flight.current=false;setPending(null);}};
  const hydrate=async(request:number,batch:XArchiveSummary[])=>{
    let cursor=0,blocked=false;
    const worker=async(slot:number)=>{
      while(current(request)&&!blocked&&cursor<batch.length){
        const item=batch[cursor++]!;
        const existing=cached.current.get(item.id)?.record;
        if(existing&&existing.intent===item.intent&&(!item.accountId||existing.accountId===item.accountId))continue;
        const lane=readLanes.current[slot]!;
        try{
          if(lane.work){
            // One removable UI waiter per real lane: repeated scope changes
            // or retries cannot append a never-settling promise chain.
            lane.wake?.();let wake!:()=>void;
            const ready=new Promise<void>(resolve=>{wake=resolve;lane.wake=wake;});
            try{await readDeadline(ready);}finally{if(lane.wake===wake)lane.wake=null;}
          }
          if(!current(request))return;if(blocked)throw new ArchiveReadTimeout();
          const work=api.read(item.id);lane.work=work;
          // UI timeout does not release or cancel the actual native read.
          const release=()=>{if(lane.work!==work)return;lane.work=null;const wake=lane.wake;lane.wake=null;wake?.();};
          void work.then(release,release);
          const value=await readDeadline(work);
          if(!current(request))return;
          if(value.id!==item.id)throw new Error("보관 항목이 일치하지 않습니다.");
          if(value.intent!==item.intent||item.accountId&&value.accountId!==item.accountId)throw new Error("보관 항목의 종류나 계정이 일치하지 않습니다.");
          if(value.version!==1||value.document.coverage!=="loaded_dom_only")throw new Error("지원하지 않는 보관 형식입니다.");
          setCaptures(previous=>new Map(previous).set(item.id,{record:value}));
        }catch(cause){if(cause instanceof ArchiveReadTimeout)blocked=true;if(current(request))setCaptures(previous=>new Map(previous).set(item.id,{error:xError(cause)}));}
      }
    };
    await Promise.all(Array.from({length:Math.min(LIBRARY_READ_CONCURRENCY,batch.length)},(_,index)=>worker(index)));
    if(blocked&&current(request)&&cursor<batch.length)setCaptures(previous=>{
      const next=new Map(previous);for(const item of batch.slice(cursor))if(!next.get(item.id)?.record)next.set(item.id,{error:"이전 읽기 작업이 끝나지 않았습니다. 다시 시도해 주세요."});return next;
    });
  };
  const loadList=async()=>{
    const request=start("list");if(request===null)return;timelineScroll.current={};scopeCounts.current={};setRecord(null);setFocusPost(null);setItems([]);setCaptures(new Map());cached.current=new Map();setVisibleCount(0);
    try{
      const value=await readDeadline(api.list());if(!current(request))return;
      const matching=value.filter(item=>inLibraryScope(item,scope.current.tab,scope.current.accountFilter));
      setItems(value);setVisibleCount(Math.min(LIBRARY_PAGE_SIZE,matching.length));setPending("hydrate");
      await hydrate(request,matching.slice(0,LIBRARY_PAGE_SIZE));
    }
    catch(cause){if(current(request))setError(xError(cause));}
    finally{finish(request);}
  };
  const loadMore=async()=>{
    if(visibleCount>=scopedItems.length)return;
    const request=start("hydrate");if(request===null)return;
    const next=Math.min(scopedItems.length,visibleCount+LIBRARY_PAGE_SIZE),batch=scopedItems.slice(visibleCount,next);setVisibleCount(next);
    try{await hydrate(request,batch);}
    finally{finish(request);}
  };
  const retry=async(item:XArchiveSummary)=>{
    const request=start("hydrate");if(request===null)return;setCaptures(previous=>{const next=new Map(previous);next.delete(item.id);return next;});
    try{await hydrate(request,[item]);}
    finally{finish(request);}
  };
  const openFolder=async()=>{
    if(!record)return;
    const request=start("folder");if(request===null)return;
    try{await api.openFolder(record.id);}
    catch(cause){if(current(request))setError(xError(cause));}
    finally{finish(request);}
  };
  const openRecord=(value:XArchiveRecord,postId?:string)=>{setRecord(value);setFocusPost(postId??null);};
  const changeScope=(nextTab:LibraryTab,nextAccount:string)=>{
    if(nextTab===scope.current.tab&&nextAccount===scope.current.accountFilter)return;
    timelineScroll.current[`${scope.current.accountFilter}:${scope.current.tab}`]=timeline.current?.scrollTop??0;
    scopeCounts.current[`${scope.current.accountFilter}:${scope.current.tab}`]=visibleCount;
    scope.current={tab:nextTab,accountFilter:nextAccount};setTab(nextTab);setAccountFilter(nextAccount);
    if(record&&nextAccount!=="all"&&accountKey(record.accountId)!==nextAccount){setRecord(null);setFocusPost(null);}
    // An in-flight list uses the latest scope when it arrives. A stale scoped
    // read may finish, but never paint another account or append its next page.
    if(pending==="list")return;
    epoch.current++;flight.current=false;setPending(null);
    const request=start("hydrate");if(request===null)return;
    const matching=items.filter(item=>inLibraryScope(item,nextTab,nextAccount));
    const count=Math.min(Math.max(LIBRARY_PAGE_SIZE,scopeCounts.current[`${nextAccount}:${nextTab}`]??0),matching.length);
    setVisibleCount(count);
    void hydrate(request,matching.slice(0,count)).finally(()=>finish(request));
  };
  const changeTab=(next:LibraryTab)=>changeScope(next,scope.current.accountFilter);
  useLayoutEffect(()=>{if(timeline.current)timeline.current.scrollTop=timelineScroll.current[`${accountFilter}:${tab}`]??0;},[tab,accountFilter]);
  useEffect(()=>{
    epoch.current++;flight.current=false;setItems([]);setRecord(null);setFocusPost(null);setCaptures(new Map());setVisibleCount(0);setPending(null);setError("");
    if(active&&!privacyMode)void loadList();
    return()=>{epoch.current++;flight.current=false;};
  },[api,active,privacyMode]);
  useEffect(()=>{
    const viewport=timeline.current;
    if(!viewport||tab!=="media"||!active||privacyMode)return;
    let disposed=false,scheduled=false;
    const check=()=>{
      scheduled=false;
      if(disposed||!alive.current||!live.current.active||live.current.privacyMode||live.current.api!==api
        ||scope.current.tab!==tab||scope.current.accountFilter!==accountFilter||flight.current||pending||error
        ||document.visibilityState==="hidden"||visibleCount>=scopedItems.length||viewport.clientHeight<=0
        ||scopedItems.slice(0,visibleCount).some(item=>captures.get(item.id)?.error))return;
      const height=viewport.clientHeight,remaining=viewport.scrollHeight-height-Math.max(0,viewport.scrollTop);
      // The first batch fills the measured list viewport, not the whole
      // archive. Scrolled views additionally prefetch one bounded near-end batch.
      if(viewport.scrollHeight<=height+1||viewport.scrollTop>0&&remaining<=Math.min(400,Math.max(160,height/2)))void loadMore();
    };
    const schedule=()=>{if(disposed||scheduled)return;scheduled=true;queueMicrotask(check);};
    const observer=typeof ResizeObserver==="undefined"?null:new ResizeObserver(schedule);
    observer?.observe(viewport);if(viewport.firstElementChild)observer?.observe(viewport.firstElementChild);
    viewport.addEventListener("scroll",schedule,{passive:true});viewport.addEventListener("load",schedule,true);
    window.addEventListener("resize",schedule);document.addEventListener("visibilitychange",schedule);schedule();
    return()=>{disposed=true;observer?.disconnect();viewport.removeEventListener("scroll",schedule);viewport.removeEventListener("load",schedule,true);window.removeEventListener("resize",schedule);document.removeEventListener("visibilitychange",schedule);};
  },[api,active,privacyMode,tab,accountFilter,items,captures,visibleCount,pending,error]);
  if(!active||privacyMode)return null;
  return <section className="x-archive-library-view" aria-label="X 로컬 저장소" aria-busy={!!pending} onContextMenu={event=>event.preventDefault()} onPlayCapture={event=>{
    const source=event.target;if(!(source instanceof HTMLMediaElement))return;
    for(const media of event.currentTarget.querySelectorAll<HTMLMediaElement>("video,audio"))if(media!==source&&!media.paused)media.pause();
  }}>
    <div className="x-archive-timeline-shell" aria-label="로컬 저장소 탐색">
      <header className="x-archive-timeline-heading">
        <div role="tablist" aria-label="저장 항목 종류" onKeyDown={event=>{
          if(event.nativeEvent.isComposing||event.ctrlKey||event.altKey||event.metaKey||!["ArrowLeft","ArrowRight","Home","End"].includes(event.key))return;
          event.preventDefault();const next=event.key==="Home"?"body":event.key==="End"?"media":tab==="body"?"media":"body";changeTab(next);
          event.currentTarget.querySelector<HTMLButtonElement>(`#x-archive-${next}-tab`)?.focus();
        }}>{(["body","media"]as const).map(value=><button type="button" key={value} role="tab" id={`x-archive-${value}-tab`} aria-selected={tab===value} tabIndex={tab===value?0:-1} aria-controls="x-archive-items" onClick={()=>changeTab(value)}>{value==="body"?"저장된 본문":"미디어"}</button>)}</div>
        <button className="x-archive-icon-button" type="button" disabled={!!pending} aria-label="새로고침" title="새로고침" onClick={()=>void loadList()}><ArchiveSymbol name="refresh"/></button>
      </header>
      <div className="x-archive-account-filter"><select aria-label="저장 계정" value={accountFilter} onChange={event=>changeScope(tab,event.currentTarget.value)}>
        <option value="all">모든 계정</option>{accountOptions.map(option=><option key={option.key} value={option.key}>{option.label}</option>)}
      </select></div>
      {error?<p className="x-archive-error" role="alert">{error}</p>:null}
      <div ref={timeline} className="x-archive-timeline" id="x-archive-items" role="tabpanel" aria-labelledby={`x-archive-${tab}-tab`} tabIndex={0}>
        <div className={tab==="media"?"x-archive-media-grid":"x-archive-feed-items"} role={tab==="body"?"feed":undefined} aria-label={tab==="body"?"로컬 보관 게시물":"저장된 미디어"}>
        {scopedItems.slice(0,visibleCount).map(item=>{
          const capture=captures.get(item.id);
          return capture?.record?(tab==="body"?<ArchiveTimelinePost key={item.id} record={capture.record} api={api} selected={record?.id===item.id} onOpen={()=>openRecord(capture.record)}/>:<ArchiveMediaTiles key={item.id} record={capture.record} api={api} onOpen={id=>openRecord(capture.record,id)}/>):capture?.error?<article className="x-archive-timeline-post x-archive-timeline-failed" key={item.id}><p role="alert">{capture.error}</p><button type="button" disabled={!!pending} onClick={()=>void retry(item)}>다시 불러오기</button></article>:<article className="x-archive-timeline-post x-archive-card-skeleton" key={item.id} aria-busy="true" aria-label="보관 게시물 불러오는 중"><i/><div><b/><span/><span/><em/></div></article>;
        })}
        </div>
        {pending==="list"?<p className="x-archive-empty" role="status">보관 데이터 불러오는 중</p>:null}
        {!pending&&!scopedItems.length&&!error?<p className="x-archive-empty">{tab==="media"?"다운로드한 미디어가 없습니다.":"보관된 항목이 없습니다."}</p>:null}
        {tab==="media"&&!pending&&scopedItems.length>0&&!scopedItems.slice(0,visibleCount).some(item=>captures.get(item.id)?.record?.assets.some(asset=>["image","poster","video","video_hls"].includes(asset.kind)))?<p className="x-archive-empty">불러온 항목에 저장된 미디어가 없습니다.</p>:null}
        {visibleCount<scopedItems.length?<button className="x-archive-more" type="button" disabled={!!pending} onClick={()=>void loadMore()}>더 보기</button>:null}
      </div>
    </div>
    <aside className="x-archive-preview" aria-label="로컬 게시물 미리보기">
      {record?<><header><h2>게시물</h2><button type="button" className="x-archive-icon-button" aria-label="미리보기 닫기" onClick={()=>{setRecord(null);setFocusPost(null);}}><ArchiveSymbol name="close"/></button></header><XArchiveReader key={record.id} record={record} api={api} onFolder={()=>void openFolder()} focusPostId={focusPost}/></>:<p className="x-archive-empty">게시물을 선택하면 여기에 표시됩니다.</p>}
    </aside>
  </section>;
}

type ToolsProps={runtime:"tauri"|"browser-mock";pane:XArchivePane;generation:number|null;contextKey:string;active?:boolean;privacyMode?:boolean;disabled?:boolean;showLibrary?:boolean;showControls?:boolean;chooseRequest?:XArchiveChooseRequest|null;onChooseDone?:(id:string)=>void;api?:XArchiveApi;onNotice?:(message:string)=>void};
type DraftLease={draft:XArchiveDraft;api:XArchiveApi;released:boolean};
function useStableProgressText(text:string,key:string,coalesce:boolean) {
  const latest=useRef({text,key,coalesce});
  const [display,setDisplay]=useState(()=>({text,key,at:Date.now()}));
  // One latest-value timer, not a debounce: continuous chunks cannot postpone
  // the trailing update indefinitely. Scope/phase changes render immediately.
  useLayoutEffect(()=>{latest.current={text,key,coalesce};},[text,key,coalesce]);
  useLayoutEffect(()=>{
    if(display.key!==key||!coalesce){
      if(display.key!==key||display.text!==text)setDisplay({text,key,at:Date.now()});
      return;
    }
    if(display.text===text)return;
    const timer=setTimeout(()=>{
      const next=latest.current;
      if(next.key===key&&next.coalesce)setDisplay({text:next.text,key,at:Date.now()});
    },Math.max(0,750-(Date.now()-display.at)));
    return()=>clearTimeout(timer);
  },[text,key,coalesce,display]);
  return display.key!==key||!coalesce?text:display.text;
}
function ArchiveSaveProgress({value,cancelling}:{value:XArchiveProgress|null;cancelling:boolean}) {
  const stage=value?.stage??"preparing";
  const terminal=["complete","failed","cancelled"].includes(stage),waitingCancel=cancelling&&!terminal;
  const bytes=`받은 ${((value?.downloadedBytes??0)/1048576).toFixed(1)} MB`;
  const label=waitingCancel?"취소 결과를 기다리는 중":stage==="queued"?"대기 중":stage==="preparing"?"저장 준비 중":stage==="downloading"?`${value!.processedMedia}/${value!.totalMedia} · ${bytes}`:stage==="writing"?"파일 검증·보관 중":stage==="complete"?(value?.partial?"일부 보관 완료":"저장 완료"):stage==="cancelled"?"저장 취소됨":"저장 실패";
  const display=useStableProgressText(label,`${value?.requestId??"pending"}:${waitingCancel?"cancelling":stage}:${!!value?.partial}`,stage==="downloading"&&!waitingCancel);
  const percent=stage==="complete"?100:value?.totalBytes&&value.totalBytes>0?Math.min(99,100*value.downloadedBytes/value.totalBytes):stage==="failed"||stage==="cancelled"?0:undefined;
  return <div className="x-archive-save-progress"><span role="status">{display}</span><progress aria-label="보관 진행" aria-valuetext={display} max={100} value={percent}/></div>;
}
export function XArchiveTools({runtime,pane,generation,contextKey,active=true,privacyMode=false,disabled=false,showLibrary=true,showControls=true,chooseRequest,onChooseDone,api:provided,onNotice}:ToolsProps) {
  const api=useMemo(()=>provided??createXArchiveApi(runtime),[provided,runtime]);
  const [mode,setMode]=useState<"select"|"library"|null>(null),[intent,setIntent]=useState<XArchiveIntent>("archive");
  const [draft,setDraft]=useState<XArchiveDraft|null>(null),[selected,setSelected]=useState<Set<string>>(new Set()),[includeMedia,setIncludeMedia]=useState(true);
  const [items,setItems]=useState<XArchiveSummary[]>([]),[record,setRecord]=useState<XArchiveRecord|null>(null);
  const [pending,setPending]=useState<"inspect"|"save"|"list"|"read"|"folder"|null>(null),[error,setError]=useState("");
  const [cancelPending,setCancelPending]=useState(false);
  const [progress,setProgress]=useState<XArchiveProgress|null>(null);
  const progressOwner=useRef<{requestId:string;captureId:string;accountId:string;epoch:number;generation:number|null;sequence:number;terminal:boolean}|null>(null);
  const epoch=useRef(0),flight=useRef(false),alive=useRef(false),saveRequest=useRef<string|null>(null),cancelling=useRef(false);
  const draftLease=useRef<DraftLease|null>(null);
  const releaseDraft=(lease:DraftLease|null=draftLease.current)=>{
    if(!lease||lease.released)return;lease.released=true;
    if(draftLease.current===lease)draftLease.current=null;
    // Exact transient identity only; cleanup never cancels a running job,
    // changes its epoch or touches an archive. A lost IPC remains TTL-bounded.
    try{void lease.api.release(lease.draft.captureId,lease.draft.accountId).catch(()=>{});}catch{}
  };
  const handledChoice=useRef<string|null>(null),activeChoice=useRef<string|null>(null),chooseDone=useRef(onChooseDone);chooseDone.current=onChooseDone;
  const releaseChoice=(id=activeChoice.current)=>{if(!id)return;if(activeChoice.current===id)activeChoice.current=null;chooseDone.current?.(id);};
  const live=useRef({active,privacyMode,disabled,generation});live.current={active,privacyMode,disabled,generation};
  const notice=useRef(onNotice);notice.current=onNotice;
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;epoch.current++;releaseDraft();releaseChoice();};},[]);
  useEffect(()=>{releaseDraft();releaseChoice();epoch.current++;flight.current=false;saveRequest.current=null;progressOwner.current=null;setProgress(null);cancelling.current=false;setCancelPending(false);setMode(null);setDraft(null);setRecord(null);setSelected(new Set());setPending(null);setError("");},[api,active,privacyMode,contextKey]);
  const current=(request:number)=>alive.current&&request===epoch.current&&live.current.active&&!live.current.privacyMode;
  useEffect(()=>{
    if(!api.subscribeProgress)return;
    let stopped=false,stop:(()=>void)|undefined;
    void api.subscribeProgress(value=>{
      const owner=progressOwner.current;
      if(stopped||!isXArchiveProgress(value)||!owner||owner.terminal||!current(owner.epoch)||owner.generation!==live.current.generation
        ||owner.requestId!==value.requestId||owner.captureId!==value.captureId||owner.accountId!==value.accountId||value.sequence<=owner.sequence)return;
      owner.sequence=value.sequence;owner.terminal=["complete","failed","cancelled"].includes(value.stage);setProgress(value);
    }).then(unlisten=>{if(stopped)unlisten();else stop=unlisten;}).catch(()=>{});
    return()=>{stopped=true;stop?.();};
  },[api]);
  const start=(operation:NonNullable<typeof pending>)=>{if(flight.current||!live.current.active||live.current.privacyMode)return null;flight.current=true;setPending(operation);setError("");return ++epoch.current;};
  const finish=(request:number)=>{if(current(request)){flight.current=false;setPending(null);}};
  const inspect=async(nextIntent:XArchiveIntent,choice?:XArchiveChooseRequest)=>{
    if(live.current.disabled||live.current.generation===null)return;
    if(choice&&(!isXArchiveChooseRequest(choice)||choice.pane!==pane||choice.generation!==live.current.generation))return;
    const request=start("inspect");if(request===null){if(choice)releaseChoice(choice.id);return;}
    let opened=false,lease:DraftLease|null=null;
    try{
      const requestedGeneration=live.current.generation;
      const value=await readDeadline(api.inspect(pane,requestedGeneration),late=>releaseDraft({draft:late,api,released:false}));
      lease={draft:value,api,released:false};
      if(!current(request)||live.current.disabled||requestedGeneration!==live.current.generation)return;
      const chosen=choice?value.document.records.find(post=>post.url===`https://x.com${choice.path}`):undefined;
      if(choice&&(value.accountId!==choice.accountId||value.document.sourceUrl!==choice.sourceUrl||!chosen))
        throw new Error("선택한 게시물이나 화면이 변경되었습니다. 다시 시도해 주세요.");
      releaseDraft();draftLease.current=lease;setDraft(value);setIntent(nextIntent);setIncludeMedia(true);setRecord(null);
      const root=statusId(value.document.sourceUrl),first=chosen??value.document.records.find(post=>post.id===root&&(nextIntent==="archive"||hasMedia(post)))??value.document.records.find(post=>nextIntent==="archive"||hasMedia(post));
      const provenRoot=first&&first.id===root&&first.presentation?.relation==="root"&&first.presentation.conversationId===root;
      setSelected(new Set(first?[first.id,...(nextIntent==="archive"&&provenRoot?value.document.records.filter(post=>post.id!==first.id&&post.presentation?.relation==="reply"&&post.presentation.conversationId===first.id).map(post=>post.id):[])]:[]));
      // Do not conceal the native pane until its frozen DOM read is complete.
      opened=true;setMode("select");
    }catch(cause){if(current(request)){setError(xError(cause));notice.current?.(xError(cause));}}
    finally{if(!opened)releaseDraft(lease);finish(request);if(choice&&!opened)releaseChoice(choice.id);}
  };
  useEffect(()=>{
    if(!chooseRequest)return;
    if(handledChoice.current===chooseRequest.id){
      if(activeChoice.current!==chooseRequest.id)releaseChoice(chooseRequest.id);
      return;
    }
    // Consume once even if lifecycle changed before delivery. A stale intent
    // may never spring back to life when its old collection is revisited.
    handledChoice.current=chooseRequest.id;
    if(!isXArchiveChooseRequest(chooseRequest)||chooseRequest.pane!==pane||chooseRequest.generation!==generation
      ||!active||privacyMode||disabled||flight.current||mode){releaseChoice(chooseRequest.id);return;}
    activeChoice.current=chooseRequest.id;
    void inspect("archive",chooseRequest);
  },[chooseRequest,active,privacyMode,disabled,generation,pane,contextKey]);
  const loadList=async()=>{
    const request=start("list");if(request===null)return;setMode("library");setRecord(null);
    try{const value=await readDeadline(api.list());if(current(request))setItems(value);}
    catch(cause){if(current(request))setError(xError(cause));}
    finally{finish(request);}
  };
  const read=async(id:string)=>{
    const request=start("read");if(request===null)return;setRecord(null);
    try{const value=await readDeadline(api.read(id));if(current(request)){if(value.id!==id)throw new Error("보관 항목이 일치하지 않습니다.");setRecord(value);}}
    catch(cause){if(current(request))setError(xError(cause));}
    finally{finish(request);}
  };
  const save=async()=>{
    if(!draft||!selected.size)return;
    const lease=draftLease.current;
    if(!lease||lease.draft!==draft||lease.api!==api||lease.released)return;
    const request=start("save");if(request===null)return;
    const selectedIds=draft.document.records.filter(post=>selected.has(post.id)).map(post=>post.id);
    try{
      const requestId=crypto.randomUUID();saveRequest.current=requestId;cancelling.current=false;setCancelPending(false);
      progressOwner.current={requestId,captureId:draft.captureId,accountId:draft.accountId,epoch:request,generation:live.current.generation,sequence:-1,terminal:false};setProgress(null);
      const value=await api.save({requestId,captureId:draft.captureId,selectedIds,intent,includeMedia:intent==="media"||includeMedia||draft.document.records.some(post=>post.photoOnly&&selected.has(post.id))});
      if(value.captureId!==draft.captureId||value.accountId!==draft.accountId||value.document.sourceUrl!==draft.document.sourceUrl||value.document.records.length!==selectedIds.length||value.document.records.some(post=>!selectedIds.includes(post.id)))throw new Error("저장 결과가 선택한 보관 내용과 일치하지 않습니다. 보관함을 확인해 주세요.");
      releaseDraft(lease);
      if(current(request)){setDraft(null);setRecord(value);setMode("library");notice.current?.(value.selectedContentComplete?(value.document.records.every(post=>post.photoOnly)?"선택한 사진을 보관했습니다. 본문·댓글은 포함하지 않습니다.":"선택한 내용을 보관했습니다."):"일부 내용만 보관됐습니다. 누락 항목을 확인해 주세요.");}
    }catch(cause){if(current(request))setError(xError(cause));}
    finally{if(progressOwner.current?.epoch===request)progressOwner.current=null;if(current(request)){saveRequest.current=null;cancelling.current=false;setCancelPending(false);}finish(request);}
  };
  const cancel=async()=>{
    const id=saveRequest.current,request=epoch.current;
    if(!id||cancelling.current||pending!=="save")return;
    cancelling.current=true;setCancelPending(true);
    try{await api.cancel(id);}
    catch(cause){if(current(request)&&saveRequest.current===id){setError(xError(cause));cancelling.current=false;setCancelPending(false);}}
    // Acknowledgement is not completion: the save result owns the final state.
  };
  const openFolder=async()=>{
    if(!record)return;
    const request=start("folder");if(request===null)return;
    try{await api.openFolder(record.id);}
    catch(cause){if(current(request))setError(xError(cause));}
    finally{finish(request);}
  };
  const close=()=>{if(pending==="save")return;epoch.current++;flight.current=false;releaseDraft();setMode(null);setDraft(null);setRecord(null);setPending(null);setError("");releaseChoice();};
  if(!active||privacyMode)return null;
  const root=draft?statusId(draft.document.sourceUrl):undefined;
  const selectedPhotoOnly=!!draft?.document.records.some(post=>post.photoOnly&&selected.has(post.id));
  return <>
    {showControls?<div className="x-archive-tools" role="group" aria-label={pane==="preview"?"미리보기 보관":"피드 보관"}>
      <button type="button" title="현재 화면 보관" aria-label="현재 화면 보관" disabled={disabled||generation===null||!!pending||!!mode} onClick={()=>void inspect("archive")}><ArchiveIcon name="save"/></button>
      <button type="button" title="미디어 다운로드" aria-label="미디어 다운로드" disabled={disabled||generation===null||!!pending||!!mode} onClick={()=>void inspect("media")}><ArchiveIcon name="download"/></button>
      {showLibrary?<button type="button" title="로컬 보관함" aria-label="로컬 보관함" disabled={!!pending||!!mode} onClick={()=>void loadList()}><ArchiveIcon name="library"/></button>:null}
      {pending==="inspect"?<span role="status">화면 읽는 중</span>:null}
      {!mode&&error?<span role="alert">{error}</span>:null}
    </div>:null}
    {mode?<ArchiveDialog title={mode==="select"?(intent==="media"?"미디어 다운로드":"현재 화면 보관"):"로컬 보관함"} busy={pending==="save"} onClose={close}>
      {error?<p className="x-archive-error" role="alert">{error}</p>:null}
      {mode==="select"&&draft?<>
        <div className="x-archive-intro"><time>캡처 {date(draft.document.capturedAt)}</time><span>지금 불러온 항목만 선택합니다. 전체 댓글이 아닙니다.</span>{draft.document.limitReached?<span>한도에 도달해 일부 항목만 표시합니다.</span>:null}</div>
        <div className="x-archive-selection">{draft.document.records.map(post=>{
          const selectable=intent==="archive"||hasMedia(post);
          return <label className={"x-archive-choice"+(selected.has(post.id)?" is-selected":"")} key={post.id}>
            <input type="checkbox" aria-label={post.photoOnly?`현재 사진 #${post.id}`:`${post.id===root?"본문":"불러온 항목"} ${authorLabel(post.author)} #${post.id}`} checked={selected.has(post.id)} disabled={pending==="save"||!selectable} onChange={()=>setSelected(previous=>{const next=new Set(previous);if(next.has(post.id))next.delete(post.id);else next.add(post.id);return next;})}/>
            <span><strong>{post.photoOnly?"현재 사진":`${post.id===root?"본문 · ":""}${authorLabel(post.author)}`}</strong>{!post.photoOnly?<time>{date(post.timestamp)}</time>:null}<span className="x-archive-choice-body">{post.photoOnly?"현재 사진만 보관 · 본문·댓글 미포함":post.body||"미디어 게시물"}</span>{post.truncated?<small>접힌 본문 · 현재 보이는 내용만</small>:null}{!selectable?<small>다운로드할 이미지·영상 없음</small>:null}</span>
          </label>;
        })}</div>
        <footer>{intent==="archive"?<label className="x-archive-include"><input type="checkbox" checked={includeMedia||selectedPhotoOnly} disabled={pending==="save"||selectedPhotoOnly} onChange={event=>setIncludeMedia(event.currentTarget.checked)}/>미디어 포함</label>:null}<span>{selected.size}개 선택</span><button type="button" disabled={!selected.size||!!pending} onClick={()=>void save()}>{pending==="save"?"저장 중…":intent==="media"?"다운로드":"보관"}</button></footer>
        {pending==="save"?<div className="x-archive-saving"><ArchiveSaveProgress value={progress} cancelling={cancelPending}/><button type="button" disabled={cancelPending} onClick={()=>void cancel()}>저장 취소</button></div>:null}
      </>:null}
      {mode==="library"?<ArchiveLibraryContent record={record} items={items} pending={!!pending} error={error} api={api} onList={()=>void loadList()} onRead={id=>void read(id)} onFolder={()=>void openFolder()}/>:null}
    </ArchiveDialog>:null}
  </>;
}
