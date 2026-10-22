import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { createXApi, xError, type XApi, type XCommunity, type XFeed, type XPage, type XPost, type XSnapshot } from "./api";
import { Author, Photo } from "./XMedia";
import { XVideo } from "./XVideo";
import { XImageViewer, type XImageSelection } from "./XImageViewer";
import "./XWorkspace.css";

export function XIcon({name}:{name:"heart"|"bookmark"|"reply"|"repost"|"views"|"share"|"refresh"}) {
  const paths={heart:"M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1.1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z",bookmark:"M6 3h12v18l-6-4-6 4V3Z",reply:"M21 11a9 9 0 0 1-9 9H4l2-4a9 9 0 1 1 15-5Z",repost:"m4 7 3-3 3 3M7 4v12h9m4 1-3 3-3-3m3 3V8H8",views:"M4 20V10m5 10V4m6 16v-7m5 7V7",share:"M12 16V3m-5 5 5-5 5 5M5 13v7h14v-7",refresh:"M20 11a8 8 0 1 0-2.3 6M20 4v7h-7"};
  return <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]}/></svg>;
}
function Skeleton({detail=false}:{detail?:boolean}) {return <div className="x-skeleton x-flow-skeleton" role="status" aria-label={detail?"게시물 불러오는 중":"피드 불러오는 중"}>{Array.from({length:detail?2:4},(_,i)=><div key={i} className="x-skeleton-post" aria-hidden="true"><i/><div><b/><span/><span/><em/></div></div>)}</div>;}
type View={page:XPage|null;loading:boolean;error:string|null;scroll:number;selected:string|null;stale?:boolean};
type ActionWarning={message:string;classification:"pending_confirmation"|"error";owner:string|null;postId:string;handle:string;generation:number;action:"like"|"bookmark";token:number};
type ActionObservation={readonly state:boolean;readonly count:string|null};
const pendingConfirmationMessage="X의 처리 결과를 아직 확인하지 못했습니다. 중복 전송하지 말고 목록을 새로고침해 확인해 주세요.";
const observeActionState=(posts:XPost[],warning:ActionWarning):ActionObservation|null=>{
  for(const post of posts){
    const stats=post.stats;
    if(post.id===warning.postId&&post.handle.toLowerCase()===warning.handle&&stats){
      if(warning.action==="like"&&stats.canLike===true&&typeof stats.liked==="boolean")
        return {state:stats.liked,count:typeof stats.likes==="string"?stats.likes:null};
      if(warning.action==="bookmark"&&stats.canBookmark===true&&typeof stats.bookmarked==="boolean")
        return {state:stats.bookmarked,count:typeof stats.bookmarks==="string"?stats.bookmarks:null};
    }
    const quoted=post.quote?observeActionState([post.quote],warning):null;
    if(quoted)return quoted;
  }
  return null;
};
const syncActionState=(post:XPost,warning:ActionWarning,observed:ActionObservation):XPost=>{
  let result=post;
  if(post.id===warning.postId&&post.handle.toLowerCase()===warning.handle&&post.stats){
    const stats=post.stats;
    if(warning.action==="like"){
      const likes=observed.count??stats.likes;
      if(stats.liked!==observed.state||stats.likes!==likes||stats.canLike!==true)
        result={...post,stats:{...stats,liked:observed.state,likes,canLike:true}};
    }else{
      const bookmarks=observed.count??stats.bookmarks;
      if(stats.bookmarked!==observed.state||stats.bookmarks!==bookmarks||stats.canBookmark!==true)
        result={...post,stats:{...stats,bookmarked:observed.state,bookmarks,canBookmark:true}};
    }
  }
  if(post.quote){
    const quote=syncActionState(post.quote,warning,observed);
    if(quote!==post.quote)result={...result,quote};
  }
  return result;
};
const empty=():View=>({page:null,loading:false,error:null,scroll:0,selected:null});
const merge=(old:XPost[],next:XPost[])=>{const map=new Map(old.map(p=>[p.id,p]));next.forEach(p=>{
  const previous=map.get(p.id),known=previous?.relation==="reply"||previous?.relation==="related";
  map.set(p.id,known&&p.relation!=="reply"&&p.relation!=="related"?{...p,relation:previous.relation}:p);
});return [...map.values()];};
type CardProps={api:XApi;post:XPost;generation:number;active:boolean;autoPlayAllowed:boolean;priority:number;selected?:boolean;detail?:boolean;pending?:Set<string>;onProfile:(handle:string)=>void;onImage:(image:XImageSelection)=>void;onSelect:(p:XPost)=>void;onMenu:(e:MouseEvent,p:XPost)=>void;onAction:(p:XPost,action:string,generation:number)=>void};
function PostCard(props:CardProps) {
  const {post,api,generation,active,detail,onSelect,onMenu,onAction}=props;
  const stats=post.stats;
  const imageIndices=post.media.flatMap((url,i)=>!post.video||!/_video_thumb\//.test(url)?[i]:[]);
  const pending=props.pending?.has(post.id);
  return <article className={"x-post"+(props.selected?" is-selected":"")+(detail?" x-detail-post":"")} onContextMenu={e=>onMenu(e,post)}>
    {post.pinned?<div className="x-repost-context"><svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M9 3h6m-5 0v6l-4 4v2h12v-2l-4-4V3M12 15v7"/></svg>고정됨</div>:null}
    {post.repost?<button className="x-repost-context" aria-label="재게시된 게시물 보기" onClick={()=>onSelect(post)}><XIcon name="repost"/>{post.repost}</button>:null}
    <button className="x-author-select" aria-label={post.name+" 프로필 보기"} onClick={()=>props.onProfile(post.handle)}><Author api={api} post={post} generation={generation} active={active}/></button>
    <button className="x-post-select" type="button" aria-label={post.name+" 게시물 미리보기"} aria-pressed={!!props.selected} onClick={()=>onSelect(post)}>
      <span className="x-text">{post.text}</span>
    </button>
    {imageIndices.length?<div className={"x-images count-"+imageIndices.length}>{imageIndices.map(index=><Photo key={index} api={api} post={post} generation={generation} active={active} index={index} onOpen={()=>props.onImage({post,index,generation})}/>)}</div>:null}
    {post.video?<div className="x-video-slot"><XVideo api={api} post={post} generation={generation} active={active} autoPlayAllowed={props.autoPlayAllowed} priority={props.priority}/></div>:null}
    {post.quote?<div className="x-quote"><PostCard {...props} post={post.quote} detail={false} selected={false}/></div>:null}
    {post.quotePreview?<div className="x-quote x-quote-preview" aria-label="인용 게시물">
      <Author api={api} post={{...post,...post.quotePreview}} generation={generation} active={active} quotePreview/>
      <span className="x-text">{post.quotePreview.text}</span>
      {post.quotePreview.media.length?<div className={"x-images count-"+post.quotePreview.media.length}>{post.quotePreview.media.map((_,index)=><Photo key={index} api={api} post={{...post,...post.quotePreview!}} generation={generation} active={active} index={index} quotePreview onOpen={()=>props.onImage({post:{...post,...post.quotePreview!},index,generation,quotePreview:true})}/>)}</div>:null}
    </div>:null}
    <div className="x-actions" role="group" aria-label="게시물 동작">
      <button title="답글 보기" aria-label="답글 보기" onClick={()=>onSelect(post)}><XIcon name="reply"/><span>{stats?.replies}</span></button>
      <span className="x-repost-count" title="재게시 수" aria-label={"재게시 "+(stats?.reposts||"0")}><XIcon name="repost"/><span>{stats?.reposts}</span></span>
      <button title="마음에 들어요" aria-label="마음에 들어요" aria-pressed={stats?.liked} disabled={pending||!stats?.canLike} onClick={()=>onAction(post,"like",generation)}><XIcon name="heart"/><span>{stats?.likes}</span></button>
      <span className="x-view-count" title="조회수"><XIcon name="views"/>{stats?.views}</span>
      <button title="북마크" aria-label="게시물 북마크" aria-pressed={stats?.bookmarked} disabled={pending||!stats?.canBookmark} onClick={()=>onAction(post,"bookmark",generation)}><XIcon name="bookmark"/></button>
      <button title="링크 복사" aria-label="링크 복사" onClick={()=>onAction(post,"share",generation)}><XIcon name="share"/></button>
    </div>
  </article>;
}

export function XWorkspace({active,runtime,privacyMode,onNotice,api:provided}:{active:boolean;runtime:"tauri"|"browser-mock";privacyMode:boolean;onNotice:(message:string)=>void;api?:XApi}) {
  const api=useMemo(()=>provided??createXApi(runtime),[provided,runtime]);
  const [feed,setFeed]=useState<XFeed>("foryou"),[views,setViews]=useState<Record<string,View>>({});
  const [account,setAccount]=useState<XSnapshot|null>(null),[communities,setCommunities]=useState<XCommunity[]>([]);
  const accountState=useRef(account);accountState.current=account;
  const [menu,setMenu]=useState(false),[busy,setBusy]=useState<"login"|"switch"|null>(null),[authError,setAuthError]=useState<string|null>(null);
  const [context,setContext]=useState<{x:number;y:number;post:XPost}|null>(null);
  const [filter,setFilter]=useState(""),[mediaOnly,setMediaOnly]=useState(false);
  const [detailId,setDetailId]=useState<string|null>(null);
  const [imageSelection,setImageSelection]=useState<XImageSelection|null>(null);
  const [history,setHistory]=useState<{feed:XFeed;filter:string;mediaOnly:boolean}[]>([]);
  const detailSentinel=useRef<HTMLDivElement>(null);
  const [pendingActions,setPendingActions]=useState(new Set<string>()),[actionError,setActionError]=useState<ActionWarning|null>(null);
  const actionWarningToken=useRef(0),latestActionWarning=useRef<ActionWarning|null>(null);
  const acting=useRef(new Set<string>());
  const [ready,setReady]=useState(false);
  const suspension=useRef<Promise<void>>(Promise.resolve());
  const suspend=useCallback(()=>{
    const pending=suspension.current.catch(()=>{}).then(()=>api.suspend());
    suspension.current=pending;
    void pending.catch(()=>{}); // Loads/status below observe and report the failure.
  },[api]);
  const state=useRef(views);state.current=views;
  const live=useRef({active,privacyMode});live.current={active,privacyMode};
  const epoch=useRef(0),requests=useRef(new Map<XFeed,string>()),left=useRef<HTMLDivElement>(null),right=useRef<HTMLDivElement>(null),accountRef=useRef<HTMLDivElement>(null),sentinel=useRef<HTMLDivElement>(null);
  const previousFeed=useRef(feed);
  const notice=useRef(onNotice);notice.current=onNotice;
  const update=useCallback((key:XFeed,change:(value:View)=>View)=>setViews(all=>({...all,[key]:change(all[key]??empty())})),[]);
  const cancelAll=useCallback(()=>{epoch.current++;for(const id of requests.current.values())void api.cancel(id).catch(()=>{});requests.current.clear();},[api]);
  const load=useCallback(async(key:XFeed,append=false)=>{
    if(!live.current.active||live.current.privacyMode||requests.current.has(key))return;
    const previous=state.current[key];
    if(append&&!previous?.page?.cursor)return;
    const id=crypto.randomUUID(),version=epoch.current,owner=accountState.current?.active??null,warningToken=actionWarningToken.current;requests.current.set(key,id);
    let timeout:ReturnType<typeof setTimeout>|undefined;
    update(key,v=>({...v,loading:true,error:null,stale:false}));
    try{
      await suspension.current;
      if(version!==epoch.current||requests.current.get(key)!==id||!live.current.active||live.current.privacyMode)return;
      const read=async()=>{
        try{return await api.load(key,append?previous!.page!.cursor:null,id);}
        catch(error){
          // Revalidate one expired read cursor without losing the retained view.
          // Never reuse this retry path for social writes or bypass native ownership checks.
          if(!append||xError(error)!=="X 탐색 상태가 변경되었거나 만료되었습니다. 새로고침해 주세요."||version!==epoch.current||requests.current.get(key)!==id||!live.current.active||live.current.privacyMode)throw error;
          return api.load(key,null,id);
        }
      };
      const result=await Promise.race([read(),new Promise<never>((_,reject)=>{timeout=setTimeout(()=>{void api.cancel(id).catch(()=>{});reject(new Error("X 응답 대기 시간이 길어 요청을 멈췄습니다. 다시 시도해 주세요."));},35000);})]);
      if(version!==epoch.current||requests.current.get(key)!==id||!live.current.active||live.current.privacyMode)return;
      if(result.status==="signedOut"){
        cancelAll();suspend();setReady(false);setBusy(null);acting.current.clear();setPendingActions(new Set());latestActionWarning.current=null;setActionError(null);
        setAuthError("X 로그인이 필요합니다. 계정 메뉴에서 현재 계정 정보 복구를 선택해 주세요.");setCommunities([]);setViews({});setDetailId(null);
        setImageSelection(null);setHistory([]);setContext(null);setMenu(false);setFilter("");setMediaOnly(false);setFeed("foryou");return;
      }
      if(result.communities?.length)setCommunities(result.communities);
      update(key,v=>({...v,loading:false,error:null,page:{...result,posts:append&&v.page?.generation===result.generation?merge(v.page.posts,result.posts):result.posts}}));
      const warning=latestActionWarning.current;
      // Admit recovery at this response boundary, never inside a state updater.
      // A known current state does not prove that the earlier mutation succeeded.
      if(warning?.classification==="pending_confirmation"&&warning.token===warningToken
        &&version===epoch.current&&owner&&warning.owner===owner&&accountState.current?.active===owner
        &&result.status==="signedIn"&&warning.generation===result.generation){
        const observed=observeActionState(result.posts,warning);
        if(observed){
          setViews(all=>Object.fromEntries(Object.entries(all).map(([viewKey,value])=>{
            const page=value.page;
            if(!page||page.generation!==warning.generation)return [viewKey,value];
            const posts=page.posts.map(post=>syncActionState(post,warning,observed));
            return [viewKey,posts.some((post,index)=>post!==page.posts[index])?{...value,page:{...page,posts}}:value];
          })));
          latestActionWarning.current=null;setActionError(null);
        }
      }
    }catch(cause){if(version===epoch.current&&requests.current.get(key)===id&&live.current.active&&!live.current.privacyMode)update(key,v=>({...v,loading:false,error:xError(cause)}));}
    finally{clearTimeout(timeout);if(requests.current.get(key)===id)requests.current.delete(key);}
  },[api,update,cancelAll,suspend]);
  useEffect(()=>{
    setReady(false);
    if(!active||privacyMode){cancelAll();setImageSelection(null);setMenu(false);setContext(null);setBusy(null);acting.current.clear();setPendingActions(new Set());suspend();setViews(all=>Object.fromEntries(Object.entries(all).map(([k,v])=>[k,{...v,loading:false,stale:true,page:v.page?{...v.page,cursor:null}:null}])));return;}
    let stopped=false;const version=epoch.current;
    void suspension.current.then(()=>stopped?null:api.status()).then(value=>{
      if(stopped||!value||version!==epoch.current)return;
      const previous=accountState.current;
      if(previous&&(previous.active!==value.active||previous.generation!==value.generation)){
        cancelAll();acting.current.clear();setPendingActions(new Set());latestActionWarning.current=null;setActionError(null);setViews({});setCommunities([]);
        setDetailId(null);setImageSelection(null);setHistory([]);setContext(null);setMenu(false);setFilter("");setMediaOnly(false);setFeed("foryou");
      }
      setAccount(value);setAuthError(value.error??null);if(value.open)setBusy("login");else setReady(true);
    }).catch(e=>{if(!stopped)setAuthError(xError(e));});
    return()=>{stopped=true;};
  },[active,privacyMode,api,cancelAll,suspend]); // Feed changes are handled without resetting account/session.
  useEffect(()=>()=>{cancelAll();suspend();},[cancelAll,suspend]);
  useEffect(()=>{
    if(!active||privacyMode||busy||!ready)return;
    if(previousFeed.current!==feed){
      for(const [key,id] of requests.current)if(key!==feed){void api.cancel(id).catch(()=>{});requests.current.delete(key);update(key,v=>({...v,loading:false,stale:true}));}
      previousFeed.current=feed;
    }
    if((!state.current[feed]?.page||state.current[feed]?.stale)&&!requests.current.has(feed))void load(feed);
    if(left.current)left.current.scrollTop=state.current[feed]?.scroll??0;
    setDetailId(state.current[feed]?.selected??null);
    const selected=state.current[feed]?.selected;
    if(selected&&state.current['post:'+selected]?.stale)void load(('post:'+selected) as XFeed);
  },[feed,active,privacyMode,busy,ready,load,api,update]);
  useEffect(()=>{
    if(busy!=="login"||!active||privacyMode)return;
    let stopped=false,timer:ReturnType<typeof setTimeout>;
    const poll=async()=>{try{const value=await api.status();if(stopped)return;setAccount(value);if(!value.open){setBusy(null);setAuthError(value.error??null);setReady(true);return;}}catch(e){if(!stopped)setAuthError(xError(e));}if(!stopped)timer=setTimeout(poll,1250);};
    timer=setTimeout(poll,1250);return()=>{stopped=true;clearTimeout(timer);};
  },[api,busy,active,privacyMode,load]);
  const view=views[feed]??empty(),detailKey=detailId?("post:"+detailId) as XFeed:null,detail=detailKey?views[detailKey]??empty():empty();
  useLayoutEffect(()=>{if(detailKey&&right.current)right.current.scrollTop=state.current[detailKey]?.scroll??0;},[detailKey,active,privacyMode]);
  useEffect(()=>{
    if(!active||privacyMode||busy||view.loading||view.error||!view.page?.cursor||!sentinel.current||typeof IntersectionObserver==="undefined")return;
    const observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting))void load(feed,true);},{root:left.current,rootMargin:"0px 0px 650px 0px"});
    observer.observe(sentinel.current);return()=>observer.disconnect();
  },[active,privacyMode,busy,view.loading,view.error,view.page?.cursor,feed,load]);
  useEffect(()=>{
    if(!active||privacyMode||busy||detail.loading||detail.error||!detail.page?.cursor||!detailKey||!detailSentinel.current||typeof IntersectionObserver==="undefined")return;
    const observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting))void load(detailKey,true);},{root:right.current,rootMargin:"0px 0px 650px 0px"});
    observer.observe(detailSentinel.current);return()=>observer.disconnect();
  },[active,privacyMode,busy,detail.loading,detail.error,detail.page?.cursor,detailKey,load]);
  const backProfile=()=>{const previous=history.at(-1);if(previous){setHistory(all=>all.slice(0,-1));setFilter(previous.filter);setMediaOnly(previous.mediaOnly);setFeed(previous.feed);}};
  useEffect(()=>{
    if(!active||privacyMode)return;
    const escape=(e:KeyboardEvent)=>{if(e.key!=="Escape"||e.defaultPrevented||e.isComposing||e.target instanceof Element&&e.target.closest('input,textarea,[contenteditable="true"]'))return;
      if(imageSelection){e.preventDefault();e.stopPropagation();setImageSelection(null);}
      else if(context){e.preventDefault();e.stopPropagation();setContext(null);}
      else if(menu){e.preventDefault();e.stopPropagation();setMenu(false);}
      else if(detailId){e.preventDefault();e.stopPropagation();setDetailId(null);update(feed,v=>({...v,selected:null}));}
      else if(history.length){e.preventDefault();e.stopPropagation();backProfile();}
    };
    const dismiss=(e:Event)=>{setContext(null);if(!accountRef.current?.contains(e.target as Node))setMenu(false);};
    window.addEventListener("keydown",escape);window.addEventListener("pointerdown",dismiss);
    return()=>{window.removeEventListener("keydown",escape);window.removeEventListener("pointerdown",dismiss);};
  },[active,privacyMode,context,menu,detailId,feed,update,imageSelection,history]);
  const openProfile=(handle:string)=>{if(feed==="profile:"+handle.toLowerCase())return;setHistory(all=>[...all,{feed,filter,mediaOnly}]);setFilter("");setMediaOnly(false);setDetailId(null);setFeed(("profile:"+handle.toLowerCase()) as XFeed);};
  const selectPost=(post:XPost)=>{setDetailId(post.id);update(feed,v=>({...v,selected:post.id}));const key=("post:"+post.id) as XFeed;update(key,v=>({...v,scroll:0}));if(!state.current[key]?.page||state.current[key]?.stale)void load(key);if(right.current)right.current.scrollTop=0;};
  const chooseFeed=(key:XFeed)=>{setHistory([]);setFeed(key);};
  const onMenu=(event:MouseEvent,post:XPost)=>{event.preventDefault();event.stopPropagation();setContext({x:Math.max(8,Math.min(event.clientX,innerWidth-240)),y:Math.max(8,Math.min(event.clientY,innerHeight-60)),post});};
  const submitAction=async(post:XPost,kind:"like"|"bookmark",generation:number)=>{
    if(!active||privacyMode||busy||acting.current.has(post.id))return;
    const version=epoch.current,owner=accountState.current?.active??null;
    acting.current.add(post.id);setPendingActions(new Set(acting.current));
    try{
      if(!api.action)throw new Error("게시물 동작은 데스크톱 앱에서 사용할 수 있습니다.");
      const expected=kind==="like"?!!post.stats?.liked:!!post.stats?.bookmarked;
      const confirmed=await api.action(post.id,kind,generation,expected);
      if(version!==epoch.current||!live.current.active||live.current.privacyMode)return;
      const replace=(p:XPost):XPost=>p.id===confirmed.id?{...confirmed,relation:p.relation}:p.quote?{...p,quote:replace(p.quote)}:p;
      setViews(all=>Object.fromEntries(Object.entries(all).map(([key,v])=>[key,{...v,page:v.page?{...v.page,posts:v.page.posts.map(replace)}:null}])));
    }catch(error){if(version===epoch.current){
      const message=xError(error);
      const warning:ActionWarning={message,classification:message===pendingConfirmationMessage?"pending_confirmation":"error",owner,postId:post.id,handle:post.handle.toLowerCase(),generation,action:kind,token:++actionWarningToken.current};
      latestActionWarning.current=warning;setActionError(warning);
    }}
    finally{if(version===epoch.current){acting.current.delete(post.id);setPendingActions(new Set(acting.current));}}
  };
  const action=async(post:XPost,kind:string,generation:number)=>{
    if(kind==="share"){try{await navigator.clipboard.writeText("https://x.com/"+post.handle+"/status/"+post.id);notice.current("게시물 링크를 복사했습니다.");}catch{notice.current("링크를 복사하지 못했습니다.");}return;}
    if(kind==="like"||kind==="bookmark")await submitAction(post,kind,generation);
  };
  const clear=()=>{cancelAll();setReady(false);acting.current.clear();setPendingActions(new Set());latestActionWarning.current=null;setActionError(null);setViews({});setCommunities([]);setDetailId(null);setImageSelection(null);setHistory([]);setFilter("");setMediaOnly(false);setContext(null);setMenu(false);setAuthError(null);setFeed("foryou");};
  const login=async(add=false)=>{clear();setBusy("switch");const version=epoch.current;try{await suspension.current;const value=await api.account(add);if(version!==epoch.current)return;setAccount(value);setBusy(value.open?"login":null);setReady(!value.open);}catch(e){if(version===epoch.current){setBusy(null);setAuthError(xError(e));}}};
  const selectAccount=async(id:string)=>{clear();setBusy("switch");const version=epoch.current;try{await suspension.current;const value=await api.select(id);if(version!==epoch.current)return;setAccount(value);setReady(true);}catch(e){if(version===epoch.current)setAuthError(xError(e));}finally{if(version===epoch.current)setBusy(null);}};
  const selectedAccount=account?.accounts?.find(a=>a.id===account.active);
  const selectedPost=detail.page?.posts.find(p=>p.id===detailId)??view.page?.posts.find(p=>p.id===detailId);
  const visiblePosts=(view.page?.posts??[]).filter(p=>(!mediaOnly||p.video||p.media.length>0||!!p.quote?.media.length||!!p.quotePreview?.media.length)&&(!filter||[p.name,p.handle,p.text,p.quote?.text??'',p.quotePreview?.text??''].some(s=>s.toLowerCase().includes(filter.toLowerCase()))));
  const cards=(posts:XPost[],generation:number,isDetail=false)=>(ready?posts:[]).map(post=><PostCard key={post.id} api={api} post={post} generation={generation} active={active&&!privacyMode&&ready} autoPlayAllowed={!imageSelection&&(isDetail||!detailId)} priority={isDetail?1:0} onProfile={openProfile} onImage={setImageSelection} pending={pendingActions} selected={detailId===post.id&&!isDetail} detail={isDetail&&post.id===detailId} onSelect={selectPost} onMenu={onMenu} onAction={(p,k,g)=>void action(p,k,g)}/>);
  return <section className="x-workspace" aria-label="X 피드">
    <header className="x-toolbar x-clean-toolbar"><h1>X</h1><div className="x-feeds" role="group" aria-label="피드 종류">
      {([["foryou","추천"],["following","팔로잉"],...(!privacyMode&&ready?communities.map(c=>["community:"+c.id,c.name]):[])] as [XFeed,string][]).map(([id,label])=><button key={id} aria-pressed={feed===id} disabled={!!busy||privacyMode} onClick={()=>chooseFeed(id)} title={label}>{label}</button>)}
    </div>{selectedAccount&&!privacyMode?<div className="x-collections" role="group" aria-label="내 모음">{(["likes","bookmarks"] as const).map(id=><button key={id} aria-pressed={feed===id} disabled={!!busy} onClick={()=>chooseFeed(id)}><XIcon name={id==="likes"?"heart":"bookmark"}/>{id==="likes"?"마음함":"북마크"}</button>)}</div>:null}
      <button className="x-refresh" aria-label="새로고침" title="새로고침" disabled={view.loading||!!busy||privacyMode} onClick={()=>void load(feed)}><XIcon name="refresh"/></button>
      <div className="x-account" ref={accountRef}><button aria-label={selectedAccount?"계정":"X 로그인"} aria-expanded={menu} disabled={!!busy||privacyMode} onClick={()=>account?.accounts?.length?setMenu(v=>!v):void login()}>{selectedAccount&&!privacyMode?"@"+selectedAccount.handle+" ▾":"X 로그인"}</button>
        {menu&&!privacyMode?<div role="menu" className="x-account-menu">{account?.accounts?.map(a=><button role="menuitem" key={a.id} disabled={a.id===account.active} onClick={()=>void selectAccount(a.id)}>@{a.handle}{a.id===account.active?" ✓":""}</button>)}<button role="menuitem" onClick={()=>void login(true)}>계정 추가</button><button role="menuitem" onClick={()=>void login()}>현재 계정 정보 복구</button></div>:null}
      </div>
    </header>
    {privacyMode?<p className="x-notice">프라이버시 모드</p>:<>
      {authError?<p role="alert" className="x-notice">{authError}</p>:null}
      {actionError?<p role="alert" className="x-notice">{actionError.message}</p>:null}
      {busy?<p role="status" className="x-notice">{busy==="login"?"공식 X 창에서 로그인해 주세요. 완료되면 자동으로 돌아옵니다.":"계정 전환 중"}</p>:null}
      <div className="x-body"><div className="x-feed-column">
        {ready&&feed.startsWith("profile:")?<div className="x-profile-heading"><button aria-label="이전 피드로" onClick={backProfile}>←</button><strong>@{feed.slice(8)}</strong></div>:null}
        <div className="x-filterbar"><input aria-label="불러온 게시물 검색" placeholder="불러온 게시물 검색" value={filter} onChange={e=>setFilter(e.target.value)}/><button aria-pressed={mediaOnly} onClick={()=>setMediaOnly(v=>!v)}>미디어</button></div>
        <div className="x-scroll" ref={left} onScroll={e=>{const top=e.currentTarget.scrollTop;update(feed,v=>({...v,scroll:top}));}} aria-busy={view.loading}>
          {!ready&&!authError||view.loading&&!view.page?<Skeleton/>:null}
          {view.error?<div className="x-notice" role="alert">{view.error}<button onClick={()=>void load(feed,!!view.page?.cursor)}>다시 시도</button></div>:null}
          <div className="x-timeline">{cards(visiblePosts,view.page?.generation??0)}</div>
          {!view.loading&&view.page&&!visiblePosts.length?<p className="x-notice">{view.page.posts.length?"일치하는 게시물이 없습니다.":"아직 게시물이 없습니다."}</p>:null}
          <div ref={sentinel} className="x-more">{view.loading&&view.page?<span role="status">더 불러오는 중</span>:view.page?.cursor?<button onClick={()=>void load(feed,true)}>더 보기</button>:view.page?.limited?<button onClick={()=>void load(feed)}>다시 불러오기</button>:null}</div>
        </div>
      </div><aside className={"x-preview"+(detailId?" has-selection":"")} aria-label="X 게시물 미리보기">
        {detailId?<><header className="x-preview-heading"><span>게시물</span><button className="x-preview-close" aria-label="미리보기 닫기" onClick={()=>{setDetailId(null);update(feed,v=>({...v,selected:null}));}}>×</button></header>
          <div className="x-preview-scroll x-thread" ref={right} onScroll={e=>{const top=e.currentTarget.scrollTop;if(detailKey)update(detailKey,v=>({...v,scroll:top}));}}>
            {selectedPost?cards([selectedPost],detail.page?.generation??view.page?.generation??0,true):null}
            {detail.loading&&!detail.page?<Skeleton detail/>:null}
            {detail.error?<div className="x-notice" role="alert">{detail.error}<button onClick={()=>detailKey&&void load(detailKey)}>다시 시도</button></div>:null}
            {(["reply","related","unknown"] as const).map(relation=>{const posts=ready?(detail.page?.posts.filter(p=>p.id!==detailId&&(p.relation??"unknown")===relation)??[]):[];return posts.length?<section className="x-thread-section" key={relation} aria-label={relation==="reply"?"답글":relation==="related"?"관련 게시물":"분류 미확인 게시물"}><h2>{relation==="reply"?"답글":relation==="related"?"관련 게시물":"분류 미확인 게시물"}</h2>{cards(posts,detail.page?.generation??0,true)}</section>:null;})}
            <div ref={detailSentinel} className="x-more">{detail.loading&&detail.page?<span role="status">더 불러오는 중</span>:detail.page?.cursor?<button onClick={()=>detailKey&&void load(detailKey,true)}>답글·관련 게시물 더 보기</button>:null}</div>
          </div></>:<p className="x-preview-empty">게시물을 선택하면 여기에 표시됩니다.</p>}
      </aside></div>
    </>}
    {imageSelection&&active&&!privacyMode?<XImageViewer api={api} selection={imageSelection} onChange={index=>setImageSelection(current=>current?{...current,index}:null)} onClose={()=>setImageSelection(null)}/>:null}
    {context&&!privacyMode?<div role="menu" className="x-context-menu" style={{left:context.x,top:context.y}} onPointerDown={e=>e.stopPropagation()}><button role="menuitem" onClick={()=>{void api.open(context.post.id).catch(e=>notice.current(xError(e)));setContext(null);}}>외부 브라우저에서 열기</button></div>:null}
  </section>;
}
