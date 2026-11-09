import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { XCollectionControls } from "./XCollectionControls";
import type { XSurfaceApi } from "./api";
import { auditLayouts, layoutBrowser } from "../../test/offlineLayoutBrowser";
import css from "./XWorkspace.css?raw";
import cardCss from "../../../src-tauri/src/x_surface_collection.css?raw";
import script from "../../../src-tauri/src/x_surface_collection.js?raw";
const api={collectionView:async()=>{}} as unknown as XSurfaceApi;
const picture='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="400" height="600"><defs><linearGradient id="g"><stop stop-color="#2b5266"/><stop offset="1" stop-color="#a07893"/></linearGradient></defs><path fill="url(#g)" d="M0 0h400v600H0z"/><circle cx="200" cy="200" r="105" fill="#d6c2b5"/><path fill="#314456" d="M60 520q140-320 280 0"/></svg>');
function cards(){
 const post=(i:number)=>'<article data-testid="tweet"><div data-testid="User-Name"><a href="/author">창작자 이름</a><a href="/author/status/'+(123456+i)+'"><time datetime="2026-11-09T03:00:00Z">오늘</time></a></div><div data-testid="tweetText">'+(i===0?'텍스트만 있는 게시물도 내용을 보며 탐색합니다. 카드 하단에는 작성자와 날짜가 표시됩니다.':'새 작품 소식 · 긴 제목과 설명이 있는 이미지 카드')+'</div>'+(i?'<a href="/author/status/'+(123456+i)+'/photo/1"><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/fixture.jpg?name=small"/></div></a>':'')+'</article>';
 const doc=new DOMParser().parseFromString('<main data-testid="primaryColumn">'+[0,1,2,3,4,5,6,7].map(post).join('')+'</main>','text/html');
 Object.defineProperty(doc,'hidden',{value:false});
 const win:Record<string,any>={top:null,__atsumiXSurface:{pause(){}}};win.top=win;
 new Function('window','document','location',script.replace('__ATSUMI_X_COLLECTION__',JSON.stringify({feed:'bookmarks',owner:'alice'})).replace('__ATSUMI_X_COLLECTION_CSS__',JSON.stringify(cardCss)))(win,doc,{protocol:'https:',hostname:'x.com',origin:'https://x.com',pathname:'/i/bookmarks'});
 win.__atsumiXCollection.mode('posts');win.__atsumiXCollection.update(true);
 const root=doc.querySelector<HTMLElement>('.atsumi-x-collection')!;
 root.setAttribute('data-layout-probe','cards');
 for(const image of root.querySelectorAll('img'))image.setAttribute('src',picture);
 const result=root.outerHTML;win.__atsumiXCollection.update(false);return result;
}
describe.skipIf(!layoutBrowser)('X collection card layout in isolated document fixtures',()=>{
 it.each([160,320,600,1000])('keeps controls and cards reachable in a %ipx feed pane',async width=>{
  localStorage.setItem('atsumi.x.collection-view.v1.bookmarks',JSON.stringify({cards:true,basis:"posts"}));
  const toolbar=renderToStaticMarkup(<XCollectionControls api={api} feed="bookmarks" generation={1} enabled onError={()=>{}}/>);
  const html='<section class="x-workspace x-official-workspace"><div class="x-feed-pane" style="height:100vh">'+toolbar+'<div class="x-native-feed" style="flex:1">'+cards()+'</div></div></section>';
  localStorage.removeItem('atsumi.x.collection-view.v1.bookmarks');
  const originalToolbar=renderToStaticMarkup(<XCollectionControls api={api} feed="bookmarks" generation={1} enabled onError={()=>{}}/>);
  const scenes=[{name:'post-cards',html},{name:'original-collection',html:html.replace(toolbar,originalToolbar).replace(/<section class="atsumi-x-collection"[\s\S]*?<\/section>/,'')}];
  const results=await auditLayouts(scenes,'html,body{margin:0;font:14px sans-serif}*{box-sizing:border-box}'+css+cardCss+'.x-native-feed>.atsumi-x-collection{position:absolute}',width,800,'x-collection');
  expect(results.flatMap(r=>r.issues.map(i=>r.name+': '+i))).toEqual([]);
  const probe=(results[0]!.geometry as {probes:Record<string,{width:number;clientWidth:number;scrollWidth:number;height:number}>}).probes.cards!;
  expect(probe.width).toBeGreaterThan(130);expect(probe.height).toBeGreaterThan(500);expect(probe.scrollWidth).toBeLessThanOrEqual(probe.clientWidth+1);
 },60000);
});

// Render the surrounding split, not just the cards: native X refuses an
// occluded pane even when the clipped strip is only a fraction of a CSS pixel.
describe.skipIf(!layoutBrowser)('X collection native feed bounds',()=>{
 it.each([[1280,720],[1920,1080],[2950,1356]])('keeps native feed geometry visible at %ix%i',async(width,height)=>{
  const scenes=[.22,.333,.45,.48,.78].flatMap(ratio=>[false,true].map(collection=>({name:ratio+'-'+collection,html:
    '<div class="test-shell"><section class="x-workspace x-official-workspace"><div class="x-body" style="grid-template-columns:minmax(0,'+ratio+'fr) 8px minmax(0,'+(1-ratio)+'fr)"><div class="x-feed-pane">'+
    (collection?'<div class="x-collection-view-controls">기본 보기 / 카드 보기</div>':'')+
    '<div class="x-native-feed" id="player"></div></div><div class="x-pane-divider"></div><aside class="x-preview"><div class="x-native-preview"></div></aside></div></section></div>'})));
  const results=await auditLayouts(scenes,'*{box-sizing:border-box}html,body{margin:0;width:100%;height:100%;overflow:hidden}.test-shell{position:absolute;left:214px;right:0;top:0;bottom:0}'+css,width,height,'x-native-collection-bounds',true);
  for(const result of results){
   const g=result.geometry as {visible:boolean;player:{width:number;height:number};occlusions:unknown[]};
   expect(g.visible,result.name).toBe(true);expect(g.player.height,result.name).toBeGreaterThan(500);
   expect(g.occlusions,result.name+': native pane must not be falsely hidden').toEqual([]);
  }
 },60000);
});
