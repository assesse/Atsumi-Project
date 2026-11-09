import { afterEach, beforeEach, expect, it, vi } from "vitest";
import script from "../../../src-tauri/src/x_surface_collection.js?raw";
import css from "../../../src-tauri/src/x_surface_collection.css?raw";
const post=(id:string,body="본문",media:string[]=['a'])=>'<article data-testid="tweet"><div data-testid="User-Name"><a href="/artist">작가</a><a href="/artist/status/'+id+'"><time datetime="2026-11-09T01:00:00Z">오늘</time></a></div><div data-testid="tweetText">'+body+'</div>'+media.map((m,i)=>'<a href="/artist/status/'+id+'/photo/'+(i+1)+'"><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/'+m+'.jpg?name=small"></div></a>').join('')+'</article>';
type Adapter={mode:(view:string)=>void;update:(ready:boolean,reset?:boolean)=>void};
function setup(html=post("123456"),feed="bookmarks"){
  const doc=new DOMParser().parseFromString('<body><main data-testid="primaryColumn">'+html+'</main></body>','text/html');
  Object.defineProperty(doc,'hidden',{value:false,configurable:true});
  Object.defineProperty(doc,'scrollingElement',{value:doc.documentElement});
  Object.defineProperty(doc.documentElement,'scrollHeight',{value:6000,configurable:true});
  Object.defineProperty(doc.documentElement,'clientHeight',{value:600,configurable:true});
  const calls=new WeakMap<Element,Record<string,EventListener>>();
  const create=doc.createElement.bind(doc);
  vi.spyOn(doc,'createElement').mockImplementation(((...args:Parameters<typeof create>)=>{
    const node=create(...args);const add=node.addEventListener.bind(node);
    vi.spyOn(node,'addEventListener').mockImplementation(((type:string,listener:EventListener,options?:boolean)=>{calls.set(node,{...calls.get(node),[type]:listener});add(type,listener,options);}) as typeof node.addEventListener);
    return node;
  }) as typeof doc.createElement);
  const open=vi.fn(),pause=vi.fn();
  const win={top:null as unknown,__atsumiXCollection:undefined as Adapter|undefined,__atsumiXSurface:{collectionOpen:open,pause},__atsumiXAds:{blocked:(node:Element)=>node.hasAttribute('data-ad')}};
  win.top=win;
  const loc={protocol:"https:",hostname:"x.com",origin:"https://x.com",pathname:feed==="likes"?"/alice/likes":"/i/bookmarks"};
  new Function("window","document","location",script.replace("__ATSUMI_X_COLLECTION__",JSON.stringify({feed,owner:"alice"})).replace("__ATSUMI_X_COLLECTION_CSS__",JSON.stringify(css)))(win,doc,loc);
  return {doc,win,loc,open,pause,adapter:win.__atsumiXCollection!,
    click:(node:Element)=>calls.get(node)?.click?.({isTrusted:true,preventDefault:vi.fn(),stopPropagation:vi.fn()} as unknown as Event),
    cards:()=>[...doc.querySelectorAll<HTMLButtonElement>(".atsumi-x-collection-card")]};
}
beforeEach(()=>vi.useFakeTimers());
afterEach(()=>{vi.clearAllTimers();vi.useRealTimers();vi.restoreAllMocks();});
it("keeps the original collection untouched until card mode is selected",()=>{
 const a=setup();a.adapter.update(true);
 expect(a.doc.querySelector(".atsumi-x-collection")).toBeNull();
 expect(a.doc.querySelector("article")?.hasAttribute("style")).toBe(false);
 expect(a.pause).not.toHaveBeenCalled();
});
it("renders post covers and text-only cards, and per-media cards from the same posts",()=>{
 const a=setup(post("123456","첫 글",["a","b"])+post("234567","텍스트만",[]));
 a.adapter.mode("posts");a.adapter.update(true);
 expect(a.cards()).toHaveLength(2);expect(a.doc.querySelector(".is-text")?.textContent).toContain("텍스트만");
 expect(a.cards()[0]?.querySelector("strong")?.textContent).toContain("첫 글");
 a.adapter.mode("media");expect(a.cards()).toHaveLength(2);
 expect(a.cards().map(n=>n.querySelector(".atsumi-x-collection-count")?.textContent)).toEqual(["1/2","2/2"]);
 a.click(a.cards()[1]!);expect(a.open).toHaveBeenLastCalledWith(expect.anything(),"/artist/status/123456/photo/2");
 a.adapter.mode("posts");a.click(a.cards()[0]!);expect(a.open).toHaveBeenLastCalledWith(expect.anything(),"/artist/status/123456");
});
it("retains decoded nodes and source ordering when native virtualized rows are recycled",()=>{
 const a=setup(post("123456")+post("234567"));a.adapter.mode("posts");a.adapter.update(true);
 const first=a.cards()[0]!,image=first.querySelector("img"),primary=a.doc.querySelector("main")!;
 primary.innerHTML=post("234567")+post("345678");
 a.adapter.update(true);
 expect(a.cards().map(n=>n.dataset.postId)).toEqual(["123456","234567","345678"]);
 expect(a.cards()[0]).toBe(first);expect(a.cards()[0]!.querySelector("img")).toBe(image);
 a.adapter.update(true);expect(a.cards()).toHaveLength(3);
});
it("does not confuse a quote or ad with its parent, or fetch foreign/invented media",()=>{
 const quote='<div data-testid="quoteTweet">'+post("999999","인용",["quote"])+'</div>';
 const a=setup(post("123456","부모",[]).replace("</article>",quote+"</article>")+post("234567").replace('data-testid="tweet"','data-testid="tweet" data-ad')+post("345678").replace("https://pbs.twimg.com/","https://untrusted.example/"));
 a.adapter.mode("posts");a.adapter.update(true);
 expect(a.cards().map(n=>n.dataset.postId)).toEqual(["123456","345678"]);
 expect(a.doc.querySelectorAll(".atsumi-x-collection img")).toHaveLength(0);
});
it("keeps native scroll, independent card scroll, preview and source nodes across mode changes",()=>{
 const a=setup();const source=a.doc.querySelector("article");
 a.doc.documentElement.scrollTop=120;
 a.adapter.mode("posts");a.adapter.update(true);
 const root=a.doc.querySelector<HTMLElement>(".atsumi-x-collection")!;
 root.scrollTop=230;a.doc.documentElement.scrollTop=1700;
 a.adapter.mode("original");
 expect(a.doc.documentElement.scrollTop).toBe(120);expect(root.hidden).toBe(true);
 expect(a.doc.querySelector("article")).toBe(source);
 expect(a.doc.querySelector("[data-atsumi-x-collection-source]")).toBeNull();
 a.adapter.mode("posts");expect(a.doc.documentElement.scrollTop).toBe(1700);expect(root.scrollTop).toBe(230);
 a.adapter.mode("media");expect(root.scrollTop).toBe(0);
 a.adapter.mode("posts");expect(root.scrollTop).toBe(230);
});
it("hides cards while private/inactive and discards previous identity on account reset",()=>{
 const a=setup();a.adapter.mode("posts");a.adapter.update(true);
 const root=a.doc.querySelector<HTMLElement>(".atsumi-x-collection")!;
 a.adapter.update(false);expect(root.hidden).toBe(true);
 a.adapter.update(true);expect(a.cards()).toHaveLength(1);
 a.adapter.update(false,true);expect(a.cards()).toHaveLength(0);
 a.doc.querySelector("main")!.innerHTML=post("345678");
 a.adapter.update(true);expect(a.cards().map(n=>n.dataset.postId)).toEqual(["345678"]);
 a.loc.pathname="/someone/likes";a.adapter.update(true);expect(root.hidden).toBe(true);
});
it("advances the official scroller once at a time, preserves items on timeout and allows retry",()=>{
 const a=setup();a.adapter.mode("posts");a.adapter.update(true);
 const button=a.doc.querySelector(".atsumi-x-collection-more")!;
 a.click(button);a.click(button);vi.advanceTimersByTime(1000);
 expect(a.doc.documentElement.scrollTop).toBeGreaterThan(0);
 vi.advanceTimersByTime(13000);
 expect(a.doc.querySelector(".atsumi-x-collection-status")?.textContent).toContain("불러오지 못했습니다");
 expect(a.cards()).toHaveLength(1);expect(button.textContent).toContain("다시 시도");
 a.click(button);a.doc.querySelector("main")!.insertAdjacentHTML("beforeend",post("234567"));
 vi.advanceTimersByTime(1000);
 expect(a.cards()).toHaveLength(2);expect(a.doc.querySelector(".atsumi-x-collection")?.getAttribute("aria-busy")).toBe("false");
});
it("ignores untrusted clicks and never installs into home or preview",()=>{
 const a=setup();a.adapter.mode("posts");a.adapter.update(true);a.cards()[0]!.click();expect(a.open).not.toHaveBeenCalled();
 for(const feed of ["foryou","preview"])expect(setup(post("123456"),feed).adapter).toBeUndefined();
});

it("fills a short viewport and stops repeated empty media pages without claiming completion",()=>{
 const a=setup(post("123456","텍스트",[]));a.adapter.mode("media");a.adapter.update(true);
 const root=a.doc.querySelector<HTMLElement>(".atsumi-x-collection")!;
 Object.defineProperty(root,"clientHeight",{value:700});Object.defineProperty(root,"scrollHeight",{value:100});
 a.adapter.update(true);vi.advanceTimersByTime(200);
 expect(root.getAttribute("aria-busy")).toBe("true");
 for(const id of [234567,345678,456789]){
   a.doc.querySelector("main")!.insertAdjacentHTML("beforeend",post(String(id),"더 불러온 글",[]));vi.advanceTimersByTime(1100);
 }
 expect(a.doc.querySelector(".atsumi-x-collection-status")?.textContent).toContain("다시 시도");
 expect(root.getAttribute("aria-busy")).toBe("false");expect(a.cards()).toHaveLength(0);
 a.adapter.update(false);const previous=a.doc.documentElement.scrollTop;vi.advanceTimersByTime(15000);expect(a.doc.documentElement.scrollTop).toBe(previous);
});
it("retains a deep local card window when switching its presentation",()=>{
 const a=setup(Array.from({length:75},(_,i)=>post(String(123456+i))).join(''));a.adapter.mode("posts");a.adapter.update(true);
 expect(a.cards()).toHaveLength(60);a.click(a.doc.querySelector(".atsumi-x-collection-more")!);expect(a.cards()).toHaveLength(75);
 const root=a.doc.querySelector<HTMLElement>(".atsumi-x-collection")!;root.scrollTop=1200;
 a.adapter.mode("media");a.adapter.mode("posts");expect(a.cards()).toHaveLength(75);expect(root.scrollTop).toBe(1200);
});

it("does not count a video's poster and its alternate-resolution image twice",()=>{
 const html=post("123456","동영상",[]).replace('</article>','<div data-testid="videoPlayer"><img src="https://pbs.twimg.com/ext_tw_video_thumb/poster.jpg?name=small"><video poster="https://pbs.twimg.com/ext_tw_video_thumb/poster.jpg?name=large"></video></div></article>');
 const a=setup(html);a.adapter.mode("media");a.adapter.update(true);expect(a.cards()).toHaveLength(1);
 expect(a.cards()[0]!.querySelector("img")!.getAttribute("src")).toBe("https://pbs.twimg.com/ext_tw_video_thumb/poster.jpg?name=small");
 a.click(a.cards()[0]!);expect(a.open).toHaveBeenCalledWith(expect.anything(),"/artist/status/123456/video/1");
});
