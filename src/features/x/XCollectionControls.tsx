import { useEffect, useRef, useState } from "react";
import { xError, type XCollectionView, type XSurfaceApi } from "./api";

import { loadXCollectionPreference as read, saveXCollectionPreference, useXCollectionCardWidth, type XCollection as Collection, type XCollectionPreference as Preference } from "./collectionPreferences";

export function XCollectionControls({api,feed,generation,enabled,onError}:{
  api:XSurfaceApi;feed:Collection;generation:number|null;enabled:boolean;onError:(message:string)=>void;
}){
  const [preference,setPreference]=useState(()=>read(feed));
  const width=useXCollectionCardWidth();
  const report=useRef(onError);report.current=onError;
  const mode:XCollectionView=preference.cards?preference.basis:"original";
  const save=(next:Preference)=>{const current={...next,width};setPreference(current);saveXCollectionPreference(feed,current);};
  useEffect(()=>{
    if(!enabled||generation===null||!api.collectionView)return;
    let cancelled=false;
    const order=nextCollectionOrder();
    void api.collectionView(generation,feed,mode,order,width).catch(cause=>{
      if(cancelled)return;
      report.current(xError(cause));
      if(mode!=="original")setPreference(previous=>({...previous,cards:false}));
    });
    return()=>{cancelled=true;};
  },[api,feed,generation,enabled,mode,width]);
  const disabled=!enabled||!api.collectionView;
  return <div className="x-collection-view-controls" aria-label="모음 보기 방식">
    <div role="group" aria-label="보기 방식">
      <button type="button" aria-pressed={!preference.cards} disabled={disabled} onClick={()=>save({...preference,cards:false})}>기본 보기</button>
      <button type="button" aria-pressed={preference.cards} disabled={disabled} onClick={()=>save({...preference,cards:true})}>카드 보기</button>
    </div>
    <select aria-label="카드 기준" value={preference.basis} disabled={disabled||!preference.cards}
      onChange={event=>save({...preference,cards:true,basis:event.target.value==="media"?"media":"posts"})}>
      <option value="posts">게시물</option><option value="media">미디어</option>
    </select>
  </div>;
}
let collectionOrder=0;
function nextCollectionOrder(){return collectionOrder=Math.max(Date.now(),collectionOrder+1);}
