import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

const storageKey="atsumi.x.feed-preview-ratio.v1",minimum=.22,maximum=.78;
const clamp=(value:number)=>Math.max(minimum,Math.min(maximum,value));
function saved(){try{const raw=localStorage.getItem(storageKey);if(raw===null)return null;const value=Number(raw);return Number.isFinite(value)&&value>=minimum&&value<=maximum?value:null;}catch{return null;}}
export function XSplitPane({children}:{children:ReactNode[]}){
  const [initial]=useState(saved),[ratio,setRatio]=useState(initial??.48),[dragging,setDragging]=useState(false);
  const element=useRef<HTMLDivElement>(null),handle=useRef<HTMLDivElement>(null),current=useRef(ratio);
  const drag=useRef<{pointer:number;left:number;width:number;offset:number}|null>(null);
  const remember=(value:number)=>{try{localStorage.setItem(storageKey,String(value));}catch{}};
  const apply=(value:number)=>{const next=clamp(value);current.current=next;setRatio(next);return next;};
  useLayoutEffect(()=>{if(initial!==null)return;const width=element.current?.getBoundingClientRect().width??0;if(width>8)apply(Math.min(600,(width-8)*.48)/(width-8));},[initial]);
  useEffect(()=>{
    const finish=(event?:PointerEvent)=>{if(!drag.current||event&&event.pointerId!==drag.current.pointer)return;const pointer=drag.current.pointer;drag.current=null;setDragging(false);remember(current.current);try{handle.current?.releasePointerCapture(pointer);}catch{}};
    const move=(event:PointerEvent)=>{const active=drag.current;if(!active||event.pointerId!==active.pointer)return;if((event.buttons&1)===0){finish(event);return;}event.preventDefault();apply((event.clientX-active.left-active.offset)/active.width);};
    const blur=()=>finish();
    window.addEventListener("pointermove",move,{passive:false});window.addEventListener("pointerup",finish);window.addEventListener("pointercancel",finish);window.addEventListener("blur",blur);
    return()=>{drag.current=null;window.removeEventListener("pointermove",move);window.removeEventListener("pointerup",finish);window.removeEventListener("pointercancel",finish);window.removeEventListener("blur",blur);};
  },[]);
  return <div className={`x-body x-resizable-body${dragging?" is-resizing":""}`} ref={element} style={{gridTemplateColumns:`minmax(0,${ratio}fr) 8px minmax(0,${1-ratio}fr)`}}>
    {children[0]}
    <div className="x-pane-divider" role="separator" aria-label="피드와 프리뷰 너비 조절" aria-orientation="vertical" aria-valuemin={minimum*100} aria-valuemax={maximum*100} aria-valuenow={Math.round(ratio*100)} tabIndex={0} ref={handle} title="드래그하여 너비 조절 · 두 번 클릭하여 초기화"
      onPointerDown={event=>{if(event.button!==0||event.isPrimary===false)return;const bounds=element.current?.getBoundingClientRect(),point=event.currentTarget.getBoundingClientRect();if(!bounds||bounds.width<=8)return;event.preventDefault();drag.current={pointer:event.pointerId,left:bounds.left,width:bounds.width-8,offset:Math.max(0,Math.min(8,event.clientX-point.left))};setDragging(true);try{event.currentTarget.setPointerCapture(event.pointerId);}catch{}}}
      onLostPointerCapture={event=>{if(drag.current?.pointer===event.pointerId){drag.current=null;setDragging(false);remember(current.current);}}}
      onDoubleClick={()=>{try{localStorage.removeItem(storageKey);}catch{}const width=element.current?.getBoundingClientRect().width??0;apply(width>8?Math.min(600,(width-8)*.48)/(width-8):.48);}}
      onKeyDown={event=>{if(event.altKey||event.ctrlKey||event.metaKey)return;const value=event.key==="Home"?minimum:event.key==="End"?maximum:event.key==="ArrowLeft"?current.current-(event.shiftKey?.05:.02):event.key==="ArrowRight"?current.current+(event.shiftKey?.05:.02):null;if(value!==null){event.preventDefault();event.stopPropagation();remember(apply(value));}}}/>
    {children[1]}
  </div>;
}
