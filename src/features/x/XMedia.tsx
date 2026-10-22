import { useEffect, useRef, useState } from "react";
import { xError, type XApi, type XPost } from "./api";
import { scheduleXImage } from "./imageQueue";

type Props = {api:XApi;post:XPost;generation:number;active:boolean;quotePreview?:boolean};
export function useXImage<T extends HTMLElement=HTMLSpanElement>({api,post,generation,active,quotePreview=false}:Props,index:number) {
  const ref=useRef<T>(null),[visible,setVisible]=useState(false),[data,setData]=useState<string|null>(null),[error,setError]=useState<string|null>(null),[attempt,retry]=useState(0);
  useEffect(()=>{
    if(!active){setVisible(false);return;}
    if(typeof IntersectionObserver==='undefined'){setVisible(true);return;}
    const observer=new IntersectionObserver(entries=>setVisible(entries.some(entry=>entry.isIntersecting)),{rootMargin:'160px'});
    if(ref.current)observer.observe(ref.current);
    return()=>observer.disconnect();
  },[active]);
  useEffect(()=>{
    setData(null);setError(null);
    if(!active||!visible||(index===-1&&!post.avatar)||(index>=0&&!post.media[index]))return;
    let stopped=false;
    const cancel=scheduleXImage(async()=>{
      if(stopped)return;
      try{
        const value=await api.image(post.id,quotePreview?(index===-1?-2:index+4):index,generation);
        if(!/^data:image\/jpeg;base64,[A-Za-z0-9+/=]+$/.test(value)||value.length>1_500_000)throw new Error('이미지 응답 형식 오류 (IMAGE_RESPONSE)');
        if(!stopped)setData(value);
      }catch(cause){if(!stopped)setError(xError(cause));}
    });
    if(!cancel)setError('이미지 요청이 많습니다. 다시 시도해 주세요.');
    return()=>{stopped=true;cancel?.();};
  },[api,post.id,post.avatar,post.media[index],index,generation,active,visible,attempt,quotePreview]);
  return {ref,data,error,retry:()=>retry(value=>value+1),onError:()=>{setData(null);setError('화면에서 이미지를 표시하지 못했습니다. (IMAGE_RENDER)');}};
}
export function Photo(props:Props&{index:number;onOpen?:()=>void}) {
  const image=useXImage(props,props.index);
  return <span ref={image.ref} className="x-photo-frame">
    <button type="button" className="x-photo" onClick={props.onOpen} aria-label={`이미지 ${props.index+1} 크게 보기`} disabled={!props.onOpen}>
      {image.data?<img src={image.data} alt="" decoding="async" onError={image.onError}/>:<span>{image.error?'이미지를 불러오지 못했습니다':'이미지 불러오는 중'}</span>}
    </button>
    {image.error?<button type="button" className="x-image-retry" title={image.error} onClick={image.retry}>다시 시도</button>:null}
  </span>;
}
export function Author(props:Props) {
  const {post}=props,image=useXImage(props,-1);
  const date=post.time&&Number.isFinite(Date.parse(post.time))?new Date(post.time):null;
  return <header><span ref={image.ref} className="x-avatar" aria-hidden="true" title={image.error??undefined}>{image.data?<img src={image.data} alt="" onError={image.onError}/>:post.name.slice(0,1)}</span><div><strong>{post.name}</strong><span>@{post.handle}</span></div><time dateTime={post.time} title={date?.toLocaleString()}>{date?.toLocaleDateString(undefined,{month:'short',day:'numeric'})??''}</time></header>;
}
