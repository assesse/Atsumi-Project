import {useEffect,useRef} from "react";
import type {XApi,XPost} from "./api";
import {useXImage} from "./XMedia";

export type XImageSelection={post:XPost;index:number;generation:number;quotePreview?:boolean};
export function XImageViewer({api,selection,onChange,onClose}:{api:XApi;selection:XImageSelection;onChange:(index:number)=>void;onClose:()=>void}){
  const {post,index,generation,quotePreview}=selection;
  const image=useXImage<HTMLDivElement>({api,post,generation,active:true,quotePreview},index);
  const close=useRef<HTMLButtonElement>(null),dialog=useRef<HTMLDivElement>(null);
  const indices=post.media.flatMap((url,i)=>quotePreview||!post.video||!/_video_thumb\//.test(url)?[i]:[]);
  const position=indices.indexOf(index),move=(offset:number)=>{const next=indices[(position+offset+indices.length)%indices.length];if(next!==undefined)onChange(next);};
  useEffect(()=>{const previous=document.activeElement;close.current?.focus();return()=>{if(previous instanceof HTMLElement&&previous.isConnected)previous.focus();};},[]);
  return <div className="x-image-backdrop" onClick={e=>{if(e.target===e.currentTarget)onClose();}}>
    <div className="x-image-viewer" ref={dialog} role="dialog" aria-modal="true" aria-label="이미지 크게 보기" onKeyDown={e=>{
      if(e.key==='Escape'){e.preventDefault();e.stopPropagation();onClose();}
      if(e.key==='ArrowLeft'&&indices.length>1){e.preventDefault();move(-1);}
      if(e.key==='ArrowRight'&&indices.length>1){e.preventDefault();move(1);}
      if(e.key==='Tab'){
        const buttons=[...dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')??[]];
        if(e.shiftKey&&document.activeElement===buttons[0]){e.preventDefault();buttons.at(-1)?.focus();}
        else if(!e.shiftKey&&document.activeElement===buttons.at(-1)){e.preventDefault();buttons[0]?.focus();}
      }
    }}>
      <header><span>{position+1} / {indices.length}</span><button ref={close} aria-label="이미지 닫기" onClick={onClose}>×</button></header>
      <div className="x-full-image" ref={image.ref}>{image.data?<img src={image.data} alt={`${post.name} 이미지 ${index+1}`} onError={image.onError}/>:<p role={image.error?'alert':'status'}>{image.error??'이미지 불러오는 중'}{image.error?<button onClick={image.retry}>다시 시도</button>:null}</p>}</div>
      {indices.length>1?<footer><button onClick={()=>move(-1)} aria-label="이전 이미지">‹</button><button onClick={()=>move(1)} aria-label="다음 이미지">›</button></footer>:null}
    </div>
  </div>;
}
