import { useEffect, useRef, useState } from "react";
import { xError, type XCollectionView, type XSurfaceApi } from "./api";

type Collection = "likes" | "bookmarks";
type Preference = { cards: boolean; basis: "posts" | "media" };
const storageKey=(feed:Collection)=>"atsumi.x.collection-view.v1."+feed;
function read(feed:Collection):Preference {
  try {
    const value:unknown=JSON.parse(localStorage.getItem(storageKey(feed))??"null");
    if(value&&typeof value==="object"&&"cards" in value&&"basis" in value
      &&typeof value.cards==="boolean"&&(value.basis==="posts"||value.basis==="media"))
      return {cards:value.cards,basis:value.basis};
  } catch { /* View preferences must not block the collection. */ }
  return {cards:false,basis:"posts"};
}
export function XCollectionControls({api,feed,generation,enabled,onError}:{
  api:XSurfaceApi;feed:Collection;generation:number|null;enabled:boolean;onError:(message:string)=>void;
}){
  const [preference,setPreference]=useState(()=>read(feed));
  const report=useRef(onError);report.current=onError;
  const mode:XCollectionView=preference.cards?preference.basis:"original";
  const save=(next:Preference)=>{setPreference(next);try{localStorage.setItem(storageKey(feed),JSON.stringify(next));}catch{}};
  useEffect(()=>{
    if(!enabled||generation===null||!api.collectionView)return;
    let cancelled=false;
    const order=nextCollectionOrder();
    void api.collectionView(generation,feed,mode,order).catch(cause=>{
      if(cancelled)return;
      report.current(xError(cause));
      if(mode!=="original")setPreference(previous=>({...previous,cards:false}));
    });
    return()=>{cancelled=true;};
  },[api,feed,generation,enabled,mode]);
  const disabled=!enabled||!api.collectionView;
  return <div className="x-collection-view-controls" aria-label="모음 보기 방식">
    <div role="group" aria-label="보기 방식">
      <button type="button" aria-pressed={!preference.cards} disabled={disabled} onClick={()=>save({...preference,cards:false})}>기본 보기</button>
      <button type="button" aria-pressed={preference.cards} disabled={disabled} onClick={()=>save({...preference,cards:true})}>카드 보기</button>
    </div>
    <select aria-label="카드 기준" value={preference.basis} disabled={disabled||!preference.cards}
      onChange={event=>save({cards:true,basis:event.target.value==="media"?"media":"posts"})}>
      <option value="posts">게시물</option><option value="media">미디어</option>
    </select>
  </div>;
}
let collectionOrder=0;
function nextCollectionOrder(){return collectionOrder=Math.max(Date.now(),collectionOrder+1);}
