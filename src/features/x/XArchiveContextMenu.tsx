import {useLayoutEffect,useRef,useState,type MouseEvent as ReactMouseEvent} from "react";
import {createPortal} from "react-dom";
import {xError} from "./api";
import {validXArchiveId,type XArchiveApi,type XArchiveRecord,type XCapturePost} from "./xArchiveApi";

type MenuBinding={record:XArchiveRecord;post:XCapturePost;api:XArchiveApi;anchor:HTMLElement;restore:HTMLElement|null;x:number;y:number};
const visible=(node:HTMLElement)=>{
  if(!node.isConnected||document.hidden)return false;
  for(let current:HTMLElement|null=node;current;current=current.parentElement){
    if(current.hidden||current.hasAttribute("inert")||current.getAttribute("aria-hidden")==="true")return false;
    const style=getComputedStyle(current);
    if(style.display==="none"||style.visibility==="hidden"||style.visibility==="collapse"||style.opacity==="0")return false;
  }
  return true;
};
const validPost=(record:XArchiveRecord,post:XCapturePost)=>{
  const matched=/^https:\/\/x\.com\/[A-Za-z0-9_]{1,15}\/status\/([0-9]{1,24})$/.exec(post.url);
  return validXArchiveId(record.id)&&record.version===1&&record.document.coverage==="loaded_dom_only"
    &&record.document.records.filter(item=>item.id===post.id).length===1
    &&record.document.records.includes(post)&&matched?.[1]===post.id;
};

/** An explicit local action: no remote reads and no live-feed cache identity. */
export function useArchiveContextMenu(record:XArchiveRecord,post:XCapturePost,api:XArchiveApi){
  const [opened,setOpened]=useState<MenuBinding|null>(null),[error,setError]=useState("");
  const live=useRef({record,post,api});live.current={record,post,api};
  const current=useRef<MenuBinding|null>(null),menu=useRef<HTMLDivElement>(null),button=useRef<HTMLButtonElement>(null);
  const mounted=useRef(false),revision=useRef(0);
  const owns=(value:MenuBinding)=>mounted.current&&live.current.record===value.record&&live.current.post===value.post
    &&live.current.api===value.api&&visible(value.anchor)&&validPost(value.record,value.post);
  const dismiss=(restore=false)=>{
    const previous=current.current;current.current=null;revision.current++;setOpened(null);
    if(restore&&previous?.restore&&visible(previous.restore))previous.restore.focus({preventScroll:true});
  };
  useLayoutEffect(()=>{
    mounted.current=true;current.current=null;revision.current++;setOpened(null);setError("");
    return()=>{mounted.current=false;current.current=null;revision.current++;};
  },[record,post,api]);
  const shown=opened&&opened.record===record&&opened.post===post&&opened.api===api?opened:null;
  useLayoutEffect(()=>{
    if(!shown)return;
    const element=menu.current;if(!element||!owns(shown)){dismiss();return;}
    const width=element.offsetWidth||232,height=element.offsetHeight||50;
    element.style.left=`${Math.max(8,Math.min(shown.x-width+12,innerWidth-width-8))}px`;
    element.style.top=`${Math.max(8,Math.min(shown.y,innerHeight-height-8))}px`;
    button.current?.focus({preventScroll:true});
    const outside=(event:Event)=>{if(!(event.target instanceof Node)||!element.contains(event.target))dismiss();};
    const close=()=>dismiss();
    const key=(event:KeyboardEvent)=>{
      if(event.isComposing||event.ctrlKey||event.metaKey||event.altKey)return;
      if(event.key==="Escape"){event.preventDefault();event.stopImmediatePropagation();dismiss(true);}
      else if(event.key==="Tab")dismiss(true);
      else if(["ArrowUp","ArrowDown","ArrowLeft","ArrowRight","Home","End","a","A","d","D"].includes(event.key)){
        // While this menu owns focus, do not move the underlying local photo.
        event.preventDefault();event.stopImmediatePropagation();button.current?.focus({preventScroll:true});
      }
    };
    window.addEventListener("pointerdown",outside,true);window.addEventListener("contextmenu",outside,true);
    window.addEventListener("scroll",outside,true);window.addEventListener("resize",close);window.addEventListener("blur",close);
    window.addEventListener("keydown",key,true);document.addEventListener("visibilitychange",close);
    return()=>{
      window.removeEventListener("pointerdown",outside,true);window.removeEventListener("contextmenu",outside,true);
      window.removeEventListener("scroll",outside,true);window.removeEventListener("resize",close);window.removeEventListener("blur",close);
      window.removeEventListener("keydown",key,true);document.removeEventListener("visibilitychange",close);
    };
  },[shown]);
  const onContextMenu=(event:ReactMouseEvent<HTMLElement>)=>{
    event.preventDefault();event.stopPropagation();
    dismiss();setError("");
    const target=event.target instanceof HTMLElement?event.target:event.currentTarget;
    if(!visible(target)||!validPost(record,post))return;
    const focus=document.activeElement instanceof HTMLElement&&document.activeElement!==document.body?document.activeElement:null;
    const anchor=event.currentTarget,rect=(focus&&anchor.contains(focus)?focus:anchor).getBoundingClientRect();
    const next={record,post,api,anchor:target,restore:focus,x:event.clientX||rect.left+12,y:event.clientY||rect.top+Math.min(rect.height,24)};
    current.current=next;setOpened(next);
  };
  const openSource=()=>{
    const value=current.current;if(!value||!owns(value)){dismiss();return;}
    dismiss();const request=revision.current;
    try{
      void value.api.openSource(value.record.id,value.post.id).catch(cause=>{
        if(request===revision.current&&owns(value))setError(xError(cause));
      });
    }catch(cause){if(request===revision.current&&owns(value))setError(xError(cause));}
  };
  const contextMenu=<>
    {shown?createPortal(<div className="x-archive-context-menu" role="menu" aria-label="보관 게시물 메뉴" ref={menu}
      onContextMenu={event=>{event.preventDefault();event.stopPropagation();}} onClick={event=>event.stopPropagation()}>
      <button ref={button} type="button" role="menuitem" onClick={openSource}>외부 브라우저에서 열기</button>
    </div>,document.body):null}
    {error?<p className="x-archive-error" role="alert">{error}</p>:null}
  </>;
  return{onContextMenu,contextMenu};
}
