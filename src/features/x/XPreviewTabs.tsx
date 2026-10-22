import { useLayoutEffect, useRef, type KeyboardEvent } from "react";
import type { XPreviewTab } from "./api";

/** This strip selects retained native documents; it does not own page content. */
export function XPreviewTabs({tabs,activeId,disabled,panelId,onSelect,onClose}:{
  tabs:XPreviewTab[];activeId:string|null;disabled:boolean;panelId:string;
  onSelect:(id:string)=>void;onClose:(id:string)=>void;
}){
  const list=useRef<HTMLDivElement>(null);
  const focused=useRef<{id:string;node:HTMLElement}|null>(null);
  const previousActive=useRef<string|null|undefined>(undefined);
  const ids=tabs.map(tab=>tab.id).join("\u0000");
  useLayoutEffect(()=>{
    const active=list.current?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
    if(previousActive.current!==activeId)active?.scrollIntoView?.({block:"nearest",inline:"nearest",behavior:"instant"});
    previousActive.current=activeId;
    // Closing the focused tab should not strand keyboard focus on body.
    // A new tab opened from a native page never moves focus into React.
    const old=focused.current;
    if(old&&!old.node.isConnected&&!tabs.some(tab=>tab.id===old.id)
      &&(!document.activeElement||document.activeElement===document.body)){
      active?.focus({preventScroll:true});focused.current=null;
    }
  },[activeId,ids]); // State/title updates must not keep forcing the user's scroll.
  const navigate=(event:KeyboardEvent<HTMLButtonElement>,id:string)=>{
    if(event.altKey||event.ctrlKey||event.metaKey||event.shiftKey||event.nativeEvent.isComposing)return;
    const index=tabs.findIndex(tab=>tab.id===id);
    const target=event.key==="ArrowRight"?(index+1)%tabs.length:event.key==="ArrowLeft"?(index+tabs.length-1)%tabs.length:
      event.key==="Home"?0:event.key==="End"?tabs.length-1:-1;
    if(target<0)return;
    event.preventDefault();event.stopPropagation();
    list.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[target]?.focus();
  };
  return <header className="x-preview-tabs-heading">
    <div className="x-preview-tabs" role="tablist" aria-label="X 미리보기 탭" aria-orientation="horizontal" ref={list}>
      {tabs.map((tab,index)=><div className={"x-preview-tab"+(tab.id===activeId?" is-active":"")} role="presentation" key={tab.id}>
        <button type="button" role="tab" id={`${panelId}-tab-${index}`} aria-controls={panelId}
          aria-selected={tab.id===activeId} aria-busy={tab.state==="loading"}
          tabIndex={tab.id===activeId||activeId===null&&index===0?0:-1} title={tab.title} disabled={disabled}
          onFocus={event=>{focused.current={id:tab.id,node:event.currentTarget};}}
          onKeyDown={event=>navigate(event,tab.id)} onClick={()=>onSelect(tab.id)}>
          <span>{tab.title}</span>
        </button>
        <button className="x-icon-button x-preview-tab-close" type="button" aria-label={`${tab.title} 닫기`} title="닫기"
          disabled={disabled} onFocus={event=>{focused.current={id:tab.id,node:event.currentTarget};}}
          onClick={()=>onClose(tab.id)}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg>
        </button>
      </div>)}
    </div>
  </header>;
}
