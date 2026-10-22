import {useEffect,useLayoutEffect,useMemo,useRef,useState} from "react";
import {createPortal} from "react-dom";
import Hls from "hls.js";
import {isXArchiveLocalUrl,validXArchiveAsset,type XArchiveApi,type XArchiveAsset,type XArchiveRecord,type XCapturePost} from "./xArchiveApi";
import {useArchiveContextMenu} from "./XArchiveContextMenu";

export const archiveDate=(value:string|number)=>{
  const time=new Date(value);
  return Number.isFinite(time.getTime())?time.toLocaleString("ko-KR",{year:"numeric",month:"long",day:"numeric",hour:"numeric",minute:"2-digit"}):"";
};
const archiveShortDate=(value:string,capturedAt:string)=>{
  const time=new Date(value),capture=new Date(capturedAt);
  if(!Number.isFinite(time.getTime()))return "";
  // Older captures have no observed display label; retain a fixed date instead
  // of inventing a relative age from the reader's current clock.
  return time.toLocaleDateString("ko-KR",{...(time.getFullYear()!==capture.getFullYear()?{year:"numeric"as const}:{}),month:"long",day:"numeric"});
};
export const archiveRoot=(record:XArchiveRecord)=>record.document.records.find(post=>post.id===record.document.sourceUrl.match(/\/status\/(\d+)(?:\/photo\/\d+)?$/)?.[1])??record.document.records[0];
const label=(value:string)=>value.replace(/\s+/g," ").trim();
// Source labels can already include a localized noun (for example "26만 조회수").
// Keep that observed label, adding our noun only to a count without one.
const viewLabel=(value:string)=>{
  const observed=label(value),parts=/^(\d[\d,.]*(?:\s*[kKmMbB천만억])?)(?:\s+(.+))?$/.exec(observed);
  return parts?{count:parts[1]!,suffix:parts[2]||"조회수"}:{count:observed,suffix:""};
};
export function useArchiveMedia(record:XArchiveRecord,api:XArchiveApi){
  const names=useMemo(()=>new Set(record.assets.filter(asset=>validXArchiveAsset(asset.name)&&/^[a-f0-9]{64}$/.test(asset.sha256)&&asset.bytes>0).map(asset=>asset.name)),[record.assets]);
  const allowed=useMemo(()=>(url:string)=>isXArchiveLocalUrl(url,record.id,names),[record.id,names]);
  const local=(asset:XArchiveAsset|undefined)=>{if(!asset||!names.has(asset.name))return undefined;const url=api.mediaUrl(record.id,asset.name);return allowed(url)?url:undefined;};
  return{local,allowed};
}
const issues:Record<string,string>={
  video_resource_not_observed:"재생 주소가 확인되지 않아 영상은 저장되지 않았습니다.",
  image_resource_not_observed:"이미지 주소가 확인되지 않아 파일은 저장되지 않았습니다.",
  original_image_url_unavailable:"원본 이미지 주소를 확인하지 못해 저장하지 않았습니다.",
  original_image_request_failed:"원본 이미지를 끝까지 받지 못해 저장하지 않았습니다.",
  original_image_validation_failed:"원본 이미지 검증에 실패해 저장하지 않았습니다.",
  capture_text_truncated:"원문이 접힌 상태여서 보이는 본문만 저장됐습니다.",
  media_request_failed:"미디어를 가져오지 못했습니다.",media_response_not_complete:"미디어 응답이 완전하지 않습니다.",
  media_truncated:"미디어가 끝까지 수신되지 않았습니다.",media_size_limit:"미디어 저장 크기 한도를 넘었습니다.",
  hls_live_unsupported:"실시간 재생 목록은 완성된 영상으로 보관하지 않았습니다.",
  hls_subtitle_invalid:"자막 파일을 안전하게 확인하지 못해 영상을 완전히 보관하지 않았습니다.",
  hls_media_url_unsupported:"지원되지 않는 영상 주소가 있어 완전히 보관하지 않았습니다.",
};
export function ArchiveSymbol({name}:{name:"reply"|"repost"|"like"|"bookmark"|"views"|"share"|"info"|"close"|"refresh"|"folder"|"verified"}){
  const paths={reply:"M12 3C6.5 3 3 6.6 3 11c0 4.3 3.5 7.5 8 7.5h1l4.7 3v-4.3C19.4 15.8 21 13.6 21 11c0-4.4-3.5-8-9-8Z",repost:"m3 7 4-4 4 4M7 3v12a3 3 0 0 0 3 3h2m9-1-4 4-4-4m4 4V9a3 3 0 0 0-3-3h-2",like:"M20.8 4.8a5.2 5.2 0 0 0-7.3 0L12 6.3l-1.5-1.5a5.2 5.2 0 0 0-7.3 7.3L12 21l8.8-8.9a5.2 5.2 0 0 0 0-7.3Z",bookmark:"M6 3h12v18l-6-4-6 4V3Z",views:"M4 20V9m5 11V3m6 17v-7m5 7V6",share:"M12 16V3m-5 5 5-5 5 5M4 14v7h16v-7",info:"M12 8h.01M11 11h1v6m0-15a10 10 0 1 0 0 20 10 10 0 0 0 0-20Z",close:"m6 6 12 12M6 18 18 6",refresh:"M20 11a8 8 0 1 0-2 6M20 4v7h-7",folder:"M3 7V5h7l2 2h9v13H3V7Z",verified:"m12 2 3 2 4 .5 1 4 2 3.5-2 3.5-1 4-4 .5-3 2-3-2-4-.5-1-4L2 12l2-3.5 1-4L9 4l3-2Zm-5 10 3 3 7-7"};
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name]}/></svg>;
}
export function ArchiveProvenance({record,onFolder}:{record:XArchiveRecord;onFolder?:()=>void}){
  const photosOnly=record.document.records.length>0&&record.document.records.every(post=>post.photoOnly);
  const status=photosOnly?(record.selectedContentComplete?"선택한 사진 보관됨":"일부 사진만 보관됨"):(record.selectedContentComplete?"선택한 내용 보관됨":"일부 내용만 보관됨");
  return <details className="x-archive-provenance-details">
    <summary aria-label="보관 정보" title="보관 정보"><ArchiveSymbol name="info"/>{!record.selectedContentComplete?<span className="x-archive-partial">일부 보관</span>:null}</summary>
    <div className="x-archive-provenance">
      <strong>{status}</strong><time>캡처 {archiveDate(record.document.capturedAt)}</time><span>저장 {archiveDate(record.savedAt)}</span>
      <span>저장할 때 불러온 내용만 포함합니다.</span>
      {photosOnly?<span>현재 사진만 보관 · 본문·댓글 미포함</span>:null}
      {!record.mediaRequested?<span>미디어 파일은 저장하지 않았습니다.</span>:null}
      {record.document.limitReached?<span>한도에 도달해 일부 항목만 담았습니다.</span>:null}
      <span className="x-archive-source">{record.document.sourceUrl}</span>
      {record.document.records.some(post=>!post.presentation)&&record.document.records.some(post=>post.counts)?<span>{record.document.records.map(post=>post.counts).filter(Boolean).join(" · ")}</span>:null}
      {record.issues.length?<ul className="x-archive-issues">{record.issues.map((issue,index)=><li key={index}>{issues[issue.reason]??`저장 누락 · ${issue.reason}`}</li>)}</ul>:null}
      {onFolder?<button type="button" aria-label="보관 폴더 열기" onClick={onFolder}><ArchiveSymbol name="folder"/>폴더 열기</button>:null}
    </div>
  </details>;
}
export function LocalVideo({url,poster,hls,allowed}:{url:string;poster?:string;hls:boolean;allowed:(url:string)=>boolean}){
  const element=useRef<HTMLVideoElement>(null),[started,setStarted]=useState(false),[error,setError]=useState("");
  useEffect(()=>{
    const video=element.current;if(!started||!video||!allowed(url))return;
    let disposed=false,player:Hls|undefined;const failed=()=>{if(!disposed)setError("저장된 영상을 재생하지 못했습니다.");};
    if(hls&&Hls.isSupported()){
      player=new Hls({enableWorker:true,maxBufferLength:20,maxMaxBufferLength:35,backBufferLength:10,manifestLoadingMaxRetry:0,fragLoadingMaxRetry:0,
        xhrSetup:(xhr,requestUrl)=>{if(!allowed(requestUrl)){xhr.abort();throw new Error("비로컬 보관 요청 차단");}xhr.withCredentials=false;},
        fetchSetup:(context,options)=>{if(!allowed(context.url))throw new Error("비로컬 보관 요청 차단");return new Request(context.url,{...options,credentials:"omit"});}});
      player.on(Hls.Events.ERROR,(_event,data)=>{if(data.fatal)failed();});
      player.on(Hls.Events.MANIFEST_PARSED,()=>{if(!disposed)void video.play().catch(()=>{});});
      player.loadSource(url);player.attachMedia(video);
    }else if(!hls||video.canPlayType("application/vnd.apple.mpegurl")){video.src=url;void video.play().catch(()=>{});}else failed();
    return()=>{disposed=true;player?.destroy();video.pause();video.removeAttribute("src");video.load();};
  },[url,hls,started,allowed]);
  return <figure className="x-archive-video">{started?<video ref={element} controls playsInline preload="metadata" poster={poster} aria-label="저장 영상" onError={()=>setError("저장된 영상을 재생하지 못했습니다.")}/>:<>{poster?<img src={poster} alt="저장된 영상 미리보기" loading="lazy"/>:null}<button type="button" className="x-archive-play" aria-label="저장 영상 재생" onClick={()=>setStarted(true)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 5 11 7-11 7Z"/></svg></button></>}{error?<figcaption role="alert">{error}</figcaption>:null}</figure>;
}
type ImageItem={src:string;alt:string;key:string};
function LocalImageViewer({images,index,onClose}:{images:ImageItem[];index:number;onClose:()=>void}){
  const [current,setCurrent]=useState(index),dialog=useRef<HTMLDivElement>(null),close=useRef<HTMLButtonElement>(null);
  useEffect(()=>{const previous=document.activeElement;close.current?.focus();return()=>{if(previous instanceof HTMLElement&&previous.isConnected)previous.focus({preventScroll:true});};},[]);
  const item=images[current];if(!item)return null;
  return createPortal(<div className="x-archive-image-viewer" role="dialog" aria-modal="true" aria-label="저장된 이미지" ref={dialog} data-native-overlay="true" onKeyDown={event=>{
    if(event.nativeEvent.isComposing||event.ctrlKey||event.altKey||event.metaKey)return;
    if(event.key==="Escape"){event.preventDefault();event.stopPropagation();onClose();}
    if(!event.shiftKey&&(event.key==="ArrowRight"||event.key.toLowerCase()==="d")){event.preventDefault();setCurrent(value=>Math.min(images.length-1,value+1));}
    if(!event.shiftKey&&(event.key==="ArrowLeft"||event.key.toLowerCase()==="a")){event.preventDefault();setCurrent(value=>Math.max(0,value-1));}
    if(event.key==="Tab"){const controls=[...dialog.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")??[]],first=controls[0],last=controls.at(-1);if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first?.focus();}}
  }}>
    <button ref={close} type="button" className="x-archive-image-close" aria-label="이미지 닫기" onClick={onClose}><ArchiveSymbol name="close"/></button>
    {images.length>1?<button type="button" className="x-archive-image-prev" aria-label="이전 이미지" disabled={!current} onClick={()=>setCurrent(value=>value-1)}>‹</button>:null}
    <img src={item.src} alt={item.alt}/>
    {images.length>1?<button type="button" className="x-archive-image-next" aria-label="다음 이미지" disabled={current===images.length-1} onClick={()=>setCurrent(value=>value+1)}>›</button>:null}
  </div>,document.body);
}
function Avatar({src,name}:{src?:string;name:string}){
  const [failed,setFailed]=useState(false);
  return src&&!failed?<img className="x-archive-avatar" src={src} alt="" loading="lazy" onError={()=>setFailed(true)}/>:<span className="x-archive-avatar x-archive-avatar-fallback" aria-hidden="true">{[...name][0]||"·"}</span>;
}
function PostMedia({record,post,api,assets}:{record:XArchiveRecord;post:XCapturePost;api:XArchiveApi;assets:XArchiveAsset[]}){
  const {local,allowed}=useArchiveMedia(record,api),[viewer,setViewer]=useState<number|null>(null),[imageError,setImageError]=useState(false);
  const videos=assets.filter(asset=>asset.kind==="video"||asset.kind==="video_hls");
  const videoPosters=new Set(videos.flatMap(asset=>{const poster=post.media.find(media=>media.url===asset.sourceUrl)?.poster;return poster?[poster]:[];}));
  const images=assets.filter(asset=>asset.kind==="image"||asset.kind==="poster"&&!videoPosters.has(asset.sourceUrl)).flatMap(asset=>{const src=local(asset);return src?[{src,key:asset.name,alt:post.media.find(media=>media.url===asset.sourceUrl)?.alt||"저장된 이미지"}]:[];});
  return <>
    {images.length?<div className={`x-archive-timeline-images count-${Math.min(images.length,4)}`}>{images.map((item,index)=><button type="button" key={item.key} aria-label={`이미지 ${index+1} 크게 보기`} onClick={()=>setViewer(index)}><img src={item.src} alt={item.alt} loading="lazy" onError={event=>{event.currentTarget.hidden=true;setImageError(true);}}/></button>)}</div>:null}
    {assets.some(asset=>["image","poster"].includes(asset.kind)&&!local(asset))?<p role="alert" className="x-archive-error">저장된 이미지 주소를 확인하지 못했습니다.</p>:null}
    {imageError?<p role="alert" className="x-archive-error">저장된 이미지를 표시하지 못했습니다.</p>:null}
    {videos.map(asset=>{const url=local(asset),source=post.media.find(media=>media.url===asset.sourceUrl),poster=local(assets.find(item=>item.kind==="poster"&&item.sourceUrl===source?.poster));return url?<LocalVideo key={asset.name} url={url} poster={poster} hls={asset.kind==="video_hls"} allowed={allowed}/>:<p key={asset.name} role="alert" className="x-archive-error">저장된 영상 주소를 확인하지 못했습니다.</p>;})}
    {viewer!==null?<LocalImageViewer images={images} index={viewer} onClose={()=>setViewer(null)}/>:null}
  </>;
}
function HistoricalActions({post,detail}:{post:XCapturePost;detail:boolean}){
  const counters=post.presentation?.counts;
  return <div className="x-archive-reactions" aria-label="저장 당시 반응">
    {(["reply","repost","like",...(detail?["bookmark"]as const:["views","bookmark"]as const),"share"]as const).map(name=>{
      const key={reply:"replies",repost:"reposts",like:"likes",views:"views",bookmark:"bookmarks",share:null}[name] as "replies"|"reposts"|"likes"|"views"|"bookmarks"|null;
      const title={reply:"답글",repost:"재게시",like:"마음에 들어요",views:"조회수",bookmark:"북마크",share:"공유"}[name],raw=key?counters?.[key]:"",views=name==="views"&&raw?viewLabel(raw):null,value=views?.count??raw;
      const accessible=views?[views.count,views.suffix].filter(Boolean).join(" "):`${title}${value?` ${value}`:""}`;
      return <span key={name} className="x-archive-reaction" aria-label={`${accessible} · 저장 당시 기록`}><ArchiveSymbol name={name}/>{value?<span>{value}</span>:null}</span>;
    })}
  </div>;
}
export function ArchivePost({record,post,api,detail=false,onOpen,selected=false}:{record:XArchiveRecord;post:XCapturePost;api:XArchiveApi;detail?:boolean;onOpen?:()=>void;selected?:boolean}){
  const {local}=useArchiveMedia(record,api);
  const {onContextMenu,contextMenu}=useArchiveContextMenu(record,post,api);
  const assets=record.assets.filter(asset=>asset.postId===post.id),presentation=post.presentation;
  const name=post.photoOnly?"현재 사진":label(presentation?.displayName||post.author)||"작성자 정보 없음";
  const quote=presentation?.quote??presentation?.quotePresentation;
  const avatar=local(assets.find(asset=>asset.kind==="avatar"&&(presentation?.avatarUrl?asset.sourceUrl===presentation.avatarUrl:asset.sourceUrl!==quote?.avatarUrl)));
  const postIssues=record.issues.filter(issue=>issue.postId===post.id),views=presentation?.counts.views?viewLabel(presentation.counts.views):null;
  const urlsWithPosters=(urls:string[])=>new Set([...urls,...post.media.filter(media=>urls.includes(media.url)&&media.poster).map(media=>media.poster!)]);
  const quoteUrls=urlsWithPosters(quote?.mediaUrls??[]),quoteAssets=assets.filter(asset=>quoteUrls.has(asset.sourceUrl));
  const ownUrls=presentation?.mediaUrls?urlsWithPosters(presentation.mediaUrls):null;
  const ownAssets=assets.filter(asset=>ownUrls?ownUrls.has(asset.sourceUrl):!quoteUrls.has(asset.sourceUrl));
  // Capture identity can be incomplete even though its resources were safely
  // saved. Keep unattributed files separate, without assigning them to a quote
  // or erasing an intentionally shared own/quote occurrence.
  const usedAssets=new Set([...ownAssets,...(quote?quoteAssets:[])].map(asset=>asset.name));
  const unassignedAssets=[...new Map(assets.filter(asset=>["image","poster","video","video_hls"].includes(asset.kind)&&!usedAssets.has(asset.name)).map(asset=>[asset.name,asset])).values()];
  const body=!post.photoOnly&&post.body?<span className="x-archive-body">{post.body}</span>:null;
  const timestamp=presentation?.timestampLabel||archiveShortDate(post.timestamp,record.document.capturedAt);
  return <article className={`x-archive-post ${detail?"is-detail":"is-compact"}${selected?" is-selected":""}`} data-post-id={post.id} data-relation={presentation?.relation??"unknown"} onContextMenu={onContextMenu}>
    {post.context?<div className="x-archive-timeline-context"><ArchiveSymbol name="repost"/>{post.context}</div>:null}
    <header><Avatar src={avatar} name={name}/><div className="x-archive-author"><strong>{onOpen?<button type="button" className="x-archive-author-open" aria-label={`${name} 보관 게시물 열기`} onClick={onOpen}>{name}</button>:name}</strong>{presentation?.verified?<ArchiveSymbol name="verified"/>:null}{presentation?.handle?<span>@{presentation.handle.replace(/^@/,"")}</span>:null}{!post.photoOnly&&!detail&&(post.timestamp||presentation?.timestampLabel)?<time title={archiveDate(post.timestamp)} dateTime={post.timestamp||undefined}>{onOpen?<button type="button" className="x-archive-timestamp-open" aria-label={`${presentation?.handle?`@${presentation.handle.replace(/^@/,"")}`:name} 보관 게시물 열기 · ${timestamp}`} onClick={onOpen}>{timestamp}</button>:timestamp}</time>:null}</div></header>
    <div className="x-archive-post-content">
      {onOpen&&body?<button className="x-archive-timeline-open" type="button" aria-label={`${name} 보관 내용 열기`} aria-pressed={selected} onClick={onOpen}>{body}</button>:<div className="x-archive-post-copy">{body}</div>}
      {presentation?.mediaBeforeQuote===true?<PostMedia record={record} post={post} api={api} assets={ownAssets}/>:null}
      {quote?<div className="x-archive-quote-card">
        <div className="x-archive-quote-author"><Avatar src={local(assets.find(asset=>asset.kind==="avatar"&&asset.sourceUrl===quote.avatarUrl))} name={quote.displayName}/><strong>{quote.displayName}</strong>{quote.verified?<ArchiveSymbol name="verified"/>:null}{quote.handle?<span>@{quote.handle.replace(/^@/,"")}</span>:null}{quote.timestamp||quote.timestampLabel?<time title={archiveDate(quote.timestamp)} dateTime={quote.timestamp||undefined}>{quote.timestampLabel||archiveShortDate(quote.timestamp,record.document.capturedAt)}</time>:null}</div><div className="x-archive-body">{quote.body}</div>
        <PostMedia record={record} post={post} api={api} assets={quoteAssets}/>
      </div>:null}
      {(quote?post.quotes.slice(1):post.quotes).map((text,index)=><div className="x-archive-quote-card x-archive-legacy-quote" key={index}>{text}</div>)}
      {presentation?.mediaBeforeQuote!==true?<PostMedia record={record} post={post} api={api} assets={ownAssets}/>:null}
      {unassignedAssets.length?<section className="x-archive-unassigned-media" aria-label="함께 보관된 미디어"><PostMedia record={record} post={post} api={api} assets={unassignedAssets}/></section>:null}
      {detail&&!post.photoOnly?<div className="x-archive-post-date">{post.timestamp||presentation?.timestampLabel?<time dateTime={post.timestamp||undefined}>{presentation?.timestampLabel||archiveDate(post.timestamp)}</time>:null}{views?<span><strong>{views.count}</strong>{views.suffix?` ${views.suffix}`:""}</span>:null}</div>:null}
      {!post.photoOnly?<HistoricalActions post={post} detail={detail}/>:null}
      {postIssues.length?<ul className="x-archive-issues">{postIssues.map((issue,index)=><li key={index}>{issues[issue.reason]??`저장 누락 · ${issue.reason}`}</li>)}</ul>:null}
    </div>
    {contextMenu}
  </article>;
}
/** Frozen data only: no remote embeds, follow/reaction actions or source HTML. */
export function XArchiveReader({record,api,onFolder,focusPostId}:{record:XArchiveRecord;api:XArchiveApi;compactProvenance?:boolean;onFolder?:()=>void;focusPostId?:string|null}){
  const region=useRef<HTMLDivElement>(null);
  useLayoutEffect(()=>{
    const current=region.current;if(!current)return;
    const post=focusPostId?[...current.querySelectorAll<HTMLElement>('[data-post-id]')].find(item=>item.dataset.postId===focusPostId):undefined;
    current.scrollTop=post?Math.max(0,post.offsetTop-current.offsetTop):0;
  },[record.id,focusPostId]);
  if(record.version!==1||record.document.coverage!=="loaded_dom_only")return <p className="x-archive-error" role="alert">지원하지 않는 보관 형식입니다.</p>;
  const root=archiveRoot(record);
  const replies=record.document.records.filter(post=>post!==root&&post.presentation?.relation==="reply"&&post.presentation.conversationId===root?.id).sort((a,b)=>(a.presentation?.order??0)-(b.presentation?.order??0));
  const other=record.document.records.filter(post=>post!==root&&!replies.includes(post));
  return <div className="x-archive-reader" role="region" aria-label="보관된 게시물 내용" tabIndex={0} ref={region} onContextMenu={event=>event.preventDefault()}>
    <div className="x-archive-reader-info"><ArchiveProvenance record={record} onFolder={onFolder}/></div>
    {root?<ArchivePost key={`${record.id}:${root.id}`} record={record} post={root} api={api} detail/>:null}
    {root&&!root.photoOnly?<div className="x-archive-reply-composer" role="group" aria-label="읽기 전용 답글 입력 영역" aria-disabled="true"><span className="x-archive-avatar x-archive-avatar-fallback" aria-hidden="true"><svg viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="8" r="4"/><path d="M4 22v-3a8 8 0 0 1 16 0v3Z"/></svg></span><span>답글 게시하기</span><button type="button" disabled>답글</button></div>:null}
    {replies.length?<section className="x-archive-replies" aria-label="저장된 답글">{replies.map(post=><ArchivePost key={`${record.id}:${post.id}`} record={record} post={post} api={api}/>)}</section>:null}
    {other.length?<section aria-label="함께 보관된 게시물">{other.map(post=><ArchivePost key={`${record.id}:${post.id}`} record={record} post={post} api={api}/>)}</section>:null}
  </div>;
}
function ArchiveMediaTile({record,post,api,src,alt,isVideo,onOpen}:{record:XArchiveRecord;post:XCapturePost;api:XArchiveApi;src?:string;alt:string;isVideo:boolean;onOpen:()=>void}){
  const {onContextMenu,contextMenu}=useArchiveContextMenu(record,post,api);
  return <><button type="button" className="x-archive-media-tile" aria-label={`${label(post.presentation?.displayName||post.author)||"보관"} ${isVideo?"영상":"이미지"} 열기`} onClick={onOpen} onContextMenu={onContextMenu}>{src?<img src={src} alt={alt} loading="lazy"/>:null}{isVideo?<span className="x-archive-tile-video" aria-hidden="true">▶</span>:null}</button>{contextMenu}</>;
}
export function ArchiveMediaTiles({record,api,onOpen}:{record:XArchiveRecord;api:XArchiveApi;onOpen:(postId:string)=>void}){
  const {local}=useArchiveMedia(record,api);
  const videoPosters=new Set(record.assets.filter(asset=>["video","video_hls"].includes(asset.kind)).flatMap(asset=>{
    const poster=record.document.records.find(post=>post.id===asset.postId)?.media.find(media=>media.url===asset.sourceUrl)?.poster;
    return poster?[`${asset.postId}\0${poster}`]:[];
  }));
  const media=record.assets.filter(asset=>["image","video","video_hls"].includes(asset.kind)||asset.kind==="poster"&&!videoPosters.has(`${asset.postId}\0${asset.sourceUrl}`));
  return <>{media.map(asset=>{
    const post=record.document.records.find(item=>item.id===asset.postId);if(!post)return null;
    const isVideo=["video","video_hls"].includes(asset.kind),source=post.media.find(item=>item.url===asset.sourceUrl),poster=record.assets.find(item=>item.postId===post.id&&item.kind==="poster"&&item.sourceUrl===source?.poster);
    if(!local(asset))return null;
    const src=local(isVideo?poster:asset);
    return <ArchiveMediaTile key={asset.name} record={record} post={post} api={api} src={src} alt={source?.alt||""} isVideo={isVideo} onOpen={()=>onOpen(post.id)}/>;
  })}</>;
}
