import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_surface.js?raw';
import adScript from '../../../src-tauri/src/x_surface_ads.js?raw';
const identity='<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
const post='<article data-testid="tweet"><div data-testid="User-Name"><a href="/creator/status/123456"><time>today</time></a></div><div data-testid="socialContext">Alex reposted</div><p data-testid="tweetText">Landscape</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Fixture"></div><video></video><button data-testid="like">Like</button><button data-testid="reply">Reply</button><button data-testid="retweet">Repost</button></article>';
function adapter(html=identity+'<main role="main"><div data-testid="primaryColumn">'+post+'</div></main>',feed='foryou',host='x.com',diagnostics=false,historyEnabled=false,withAds=false) {
  const doc=new DOMParser().parseFromString(html,'text/html'),timers:Array<()=>void>=[];
  const handlers:Record<string,Array<(e:unknown)=>void>>={};
  const original=doc.addEventListener.bind(doc);
  vi.spyOn(doc,'addEventListener').mockImplementation(((name:string,handler:EventListener)=>{(handlers[name]??=[]).push(handler as (e:unknown)=>void);original(name,handler);}) as typeof doc.addEventListener);
  const message=vi.fn();
  const location={protocol:'https:',hostname:host,origin:'https://'+host,pathname:'/home',href:'https://'+host+'/home',assign:vi.fn()};
  let historyState:unknown=null;
  const changeHistory=(data:unknown,_unused:string,url?:string|URL|null)=>{historyState=data;if(url!=null){const next=new URL(String(url),location.href);location.pathname=next.pathname;location.href=next.href;}};
  const sourcePush=vi.fn(changeHistory),sourceReplace=vi.fn(changeHistory);
  const windowHandlers:Record<string,Array<(event:Event)=>void>>={};
  const win={chrome:{webview:{postMessage:message}},history:historyEnabled?{get state(){return historyState;},pushState:sourcePush,replaceState:sourceReplace}:undefined,navigator:{userActivation:{isActive:false}},top:null as unknown,
    addEventListener:vi.fn((name:string,handler:(event:Event)=>void)=>{(windowHandlers[name]??=[]).push(handler);}),
    dispatchEvent:vi.fn((event:Event)=>{for(const handler of windowHandlers[event.type]??[])handler(event);return true;}),
    getSelection:()=>({toString:()=>''}),__atsumiXSurface:undefined as undefined|{visibility(v:boolean):void;refresh():void;pause():void;playback(v:boolean):void;archivePhotoOwner():string;dismissMenu(cancelRouting?:boolean):boolean;dismissMenus(cancelRouting?:boolean):boolean}};win.top=win;
  const observations:Array<{target:Node|null;options?:MutationObserverInit;active:boolean;notify:()=>void}>=[];
  class Observer {
    entry:{target:Node|null;options?:MutationObserverInit;active:boolean;notify:()=>void};
    constructor(notify:()=>void){this.entry={target:null,active:false,notify};observations.push(this.entry);}
    observe(target:Node,options:MutationObserverInit){Object.assign(this.entry,{target,options,active:true});}
    disconnect(){this.entry.active=false;}
  }
  if(withAds){
    Object.defineProperty(doc,'hidden',{value:false,configurable:true});
    new Function('window','document','location','URL','Element','getComputedStyle','MutationObserver','setTimeout',
      adScript.replace('__ATSUMI_X_ADS__',JSON.stringify({owner:'alice',diagnostics})))(win,doc,location,URL,Element,
      (node:HTMLElement)=>({display:node.getAttribute('data-atsumi-x-ad-hidden')==='promoted-v1'?'none':node.style.display||'block',visibility:node.style.visibility||'visible',opacity:node.style.opacity||'1'}),
      Observer,(f:()=>void)=>{timers.push(f);return timers.length;});
  }
  new Function('window','document','location','URL','Element','HTMLMediaElement','MutationObserver','setTimeout','innerWidth','innerHeight',script.replace('__ATSUMI_X_SURFACE__',JSON.stringify({nonce:'fixture',feed,owner:'alice',diagnostics})))(win,doc,location,URL,Element,HTMLMediaElement,Observer,(f:()=>void)=>{timers.push(f);return timers.length;},600,900);
  const flush=()=>{const task=timers.shift();task?.();};
  const update=()=>{win.__atsumiXSurface?.refresh();flush();};
  const status=()=>JSON.parse(new URL(message.mock.calls.at(-1)![0].slice('ATSUMI_X_SURFACE_V1:'.length)).searchParams.get('value')!);
  return {doc,win,location,flush,update,status,handlers,windowHandlers,message,observations,timers,sourcePush,sourceReplace};
}
afterEach(()=>vi.restoreAllMocks());
it.each(['foryou','preview'])('revalidates ad suppression synchronously before the %s pane resumes',feed=>{
  const ad='<div data-testid="cellInnerDiv"><article data-testid="tweet"><div><div data-testid="User-Name"><a href="/creator">Creator</a></div><span class="badge">Ad</span><button data-testid="caret">More</button></div><p data-testid="tweetText">Body</p></article><div data-testid="placementTracking"></div></div>';
  const a=adapter(identity+'<main data-testid="primaryColumn">'+ad+'</main>',feed,'x.com',false,false,true);
  a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
  const cell=a.doc.querySelector('[data-testid="cellInnerDiv"]')!;
  expect(cell.getAttribute('data-atsumi-x-ad-hidden')).toBe('promoted-v1');
  a.win.__atsumiXSurface!.visibility(false);expect(cell.hasAttribute('data-atsumi-x-ad-hidden')).toBe(false);
  a.win.__atsumiXSurface!.visibility(true);
  // No 180ms readiness/update task is needed to restore the known ad marker.
  expect(cell.getAttribute('data-atsumi-x-ad-hidden')).toBe('promoted-v1');
  for(let i=0;i<8;i++){a.flush();a.win.__atsumiXSurface!.visibility(true);expect(cell.getAttribute('data-atsumi-x-ad-hidden')).toBe('promoted-v1');}
  a.win.__atsumiXSurface!.visibility(false);a.doc.querySelector('.badge')!.textContent='Ordinary';
  a.win.__atsumiXSurface!.visibility(true);expect(cell.hasAttribute('data-atsumi-x-ad-hidden')).toBe(false);
});
it('does not reuse a previous ad proof after account ownership changes while concealed',()=>{
  const html=identity+'<main data-testid="primaryColumn"><div data-testid="placementTracking"><article data-testid="tweet"><div><div data-testid="User-Name">Creator</div><span>Ad</span><button data-testid="caret">More</button></div></article></div></main>';
  const a=adapter(html,'preview','x.com',false,false,true);a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
  a.win.__atsumiXSurface!.visibility(false);a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
  a.win.__atsumiXSurface!.visibility(true);expect(a.doc.querySelector('[data-atsumi-x-ad-hidden]')).toBeNull();
});
it.each(['foryou','preview'])('trusted wheel claims only the visible %s pane without scrolling, focusing, or unmuting source media',feed=>{
  const a=adapter(undefined,feed);Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
  const target=a.doc.querySelector('[data-testid="tweetText"]')!,video=a.doc.querySelector('video')!;
  const focus=vi.spyOn(target as HTMLElement,'focus'),play=vi.spyOn(video,'play').mockResolvedValue(),before={muted:video.muted,volume:video.volume,src:video.src};
  const event={isTrusted:true,target,deltaX:0,deltaY:120,ctrlKey:false,metaKey:false,altKey:false,preventDefault:vi.fn(),stopPropagation:vi.fn()};
  for(let i=0;i<10;i++)a.handlers.wheel!.forEach(handler=>handler(event));
  const packets=a.message.mock.calls.map(([raw])=>new URL(raw.slice('ATSUMI_X_SURFACE_V1:'.length)));
  expect(packets).toHaveLength(1);expect(packets[0]!.searchParams.get('kind')).toBe('wheel');expect(packets[0]!.searchParams.get('value')).toBe('');
  expect(event.preventDefault).not.toHaveBeenCalled();expect(event.stopPropagation).not.toHaveBeenCalled();expect(focus).not.toHaveBeenCalled();expect(play).not.toHaveBeenCalled();
  expect({muted:video.muted,volume:video.volume,src:video.src}).toEqual(before);
  expect(vi.mocked(a.doc.addEventListener).mock.calls.find(([name])=>name==='wheel')?.[2]).toEqual({capture:true,passive:true});
});
it.each(['synthetic','hidden-pane','hidden-document','ctrl','meta','alt','zero','nonfinite','foreign','detached','editor','menu','hidden-node','outside'])('rejects a %s wheel ownership claim',kind=>{
  const a=adapter();Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
  let target:Element=a.doc.querySelector('[data-testid="tweetText"]')!;
  const event={isTrusted:kind!=='synthetic',target,deltaX:0,deltaY:kind==='zero'?0:kind==='nonfinite'?NaN:120,ctrlKey:kind==='ctrl',metaKey:kind==='meta',altKey:kind==='alt'};
  if(kind==='hidden-pane')a.win.__atsumiXSurface!.visibility(false);
  if(kind==='hidden-document')Object.defineProperty(a.doc,'hidden',{value:true});
  if(kind==='foreign')target=document.body;
  if(kind==='detached')target.remove();
  if(kind==='editor'||kind==='menu'){target=a.doc.createElement(kind==='editor'?'textarea':'div');target.className=kind==='menu'?'atsumi-x-download-menu':'';a.doc.querySelector('main')!.append(target);}
  if(kind==='hidden-node')target.setAttribute('hidden','');
  if(kind==='outside')target=a.doc.body;
  event.target=target;a.message.mockClear();a.handlers.wheel!.forEach(handler=>handler(event));expect(a.message).not.toHaveBeenCalled();
});
it('does not claim wheel ownership from an ad or from a programmatic scroll event',()=>{
  const a=adapter();Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
  Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:()=>true}});a.message.mockClear();
  a.handlers.wheel!.forEach(handler=>handler({isTrusted:true,target:a.doc.querySelector('article'),deltaX:0,deltaY:80}));
  a.doc.dispatchEvent(new Event('scroll',{bubbles:true}));expect(a.message).not.toHaveBeenCalled();
});
function wheelPlaybackAdapter(feed='foryou') {
  const a=adapter(undefined,feed,'x.com',false,true);
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
  const video=a.doc.querySelector('video')!,target=a.doc.querySelector('[data-testid="tweetText"]')!;
  const pause=vi.spyOn(video,'pause').mockImplementation(()=>{}),play=vi.spyOn(video,'play').mockResolvedValue();
  const box={x:10,y:10,left:10,top:10,right:310,bottom:210,width:300,height:200,toJSON:()=>({})};
  vi.spyOn(video,'getBoundingClientRect').mockReturnValue(box);
  a.win.__atsumiXSurface!.playback(false);pause.mockClear();a.message.mockClear();
  const wheel=()=>{
    const event={isTrusted:true,target,deltaX:0,deltaY:80};
    a.windowHandlers.wheel!.forEach(handler=>handler(event as unknown as Event));
    a.handlers.wheel!.forEach(handler=>handler(event));
  };
  return {...a,video,target,pause,play,wheel,box};
}
it.each(['foryou','preview'])('lets the first source %s play after a real wheel reach the native claim before its asynchronous grant',feed=>{
  const a=wheelPlaybackAdapter(feed),focus=vi.spyOn(a.video,'focus'),before={muted:a.video.muted,volume:a.video.volume,src:a.video.src};
  let now=1000;vi.spyOn(performance,'now').mockImplementation(()=>now);
  a.wheel();now+=20;a.wheel(); // A second tick must not erase the pending first-play intent.
  a.message.mockClear();a.handlers.play![0]!({isTrusted:true,target:a.video});
  expect(bridgeValues(a)).toEqual(['manual']);expect(a.pause).not.toHaveBeenCalled();
  expect(a.play).not.toHaveBeenCalled();expect(focus).not.toHaveBeenCalled();
  expect({muted:a.video.muted,volume:a.video.volume,src:a.video.src}).toEqual(before);
  a.message.mockClear();a.handlers.play![0]!({isTrusted:true,target:a.video});
  expect(a.pause).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();
});
it.each(['expired','owner','login','route','same-history','roundtrip-history','popstate','hidden-pane','hidden-roundtrip','hidden-document','document-roundtrip','typing','keydown','pointer','touch','cancel-routing','detached-root','replaced-root','detached-media','other-dialog','outside-viewport','css-hidden','ancestor-hidden','ancestor-clipped'])('rejects a %s wheel-to-play intent instead of borrowing an old source claim',change=>{
  const a=wheelPlaybackAdapter();let now=1000;vi.spyOn(performance,'now').mockImplementation(()=>now);a.wheel();
  if(change==='expired')now+=351;
  if(change==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
  if(change==='login')a.doc.body.append(a.doc.createElement('input'));if(change==='login')a.doc.querySelector('input')!.type='password';
  if(change==='route'){a.location.href='https://x.com/bookmarks';a.location.pathname='/bookmarks';}
  if(change==='same-history')a.win.history!.replaceState({},'','/home');
  if(change==='roundtrip-history'){a.win.history!.pushState({},'','/bookmarks');a.win.history!.pushState({},'','/home');}
  if(change==='popstate')a.windowHandlers.popstate![0]!(new PopStateEvent('popstate'));
  if(change==='hidden-pane'||change==='hidden-roundtrip')a.win.__atsumiXSurface!.visibility(false);
  if(change==='hidden-roundtrip')a.win.__atsumiXSurface!.visibility(true);
  if(change==='hidden-document'||change==='document-roundtrip')Object.defineProperty(a.doc,'hidden',{value:true,configurable:true});
  if(change==='document-roundtrip'){a.handlers.visibilitychange!.forEach(handler=>handler({}));Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});}
  if(change==='typing'){const input=a.doc.createElement('textarea');a.doc.body.append(input);Object.defineProperty(a.doc,'activeElement',{value:input,configurable:true});}
  if(change==='keydown')a.handlers.keydown![0]!({isTrusted:true,key:'a',target:a.doc.body});
  if(change==='pointer'||change==='touch')a.windowHandlers[change==='pointer'?'pointerdown':'touchstart']![0]!({isTrusted:true} as Event);
  if(change==='cancel-routing')a.win.__atsumiXSurface!.dismissMenus(true);
  if(change==='detached-root')a.doc.querySelector('main')!.remove();
  if(change==='replaced-root'){const root=a.doc.querySelector('[data-testid="primaryColumn"]')!,next=root.cloneNode(false);root.replaceWith(next);next.appendChild(a.video);}
  if(change==='detached-media')a.video.remove();
  if(change==='other-dialog'){const dialog=a.doc.createElement('div');dialog.setAttribute('role','dialog');a.doc.body.append(dialog);dialog.append(a.video);}
  if(change==='outside-viewport')vi.mocked(a.video.getBoundingClientRect).mockReturnValue({...a.box,top:1000,bottom:1200});
  if(change==='css-hidden')a.video.style.visibility='hidden';
  if(change==='ancestor-hidden')a.video.parentElement!.style.opacity='0';
  if(change==='ancestor-clipped'){a.video.parentElement!.style.overflowX='hidden';a.video.parentElement!.style.overflowY='hidden';vi.spyOn(a.video.parentElement!,'getBoundingClientRect').mockReturnValue({...a.box,left:400,right:500});}
  a.pause.mockClear();a.message.mockClear();a.handlers.play![0]!({isTrusted:true,target:a.video});
  expect(a.pause).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();expect(a.play).not.toHaveBeenCalled();
});
it('does not arm a wheel play intent while editing, unverified, or on a hidden source target',()=>{
  for(const change of ['typing','owner','hidden']){
    const a=wheelPlaybackAdapter();
    if(change==='typing'){const editor=a.doc.createElement('textarea');a.doc.body.append(editor);Object.defineProperty(a.doc,'activeElement',{value:editor,configurable:true});}
    if(change==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
    if(change==='hidden')a.target.parentElement!.style.visibility='hidden';
    a.wheel();a.handlers.play![0]!({isTrusted:true,target:a.video});
    expect(a.pause).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();
  }
});
it('cannot turn a wheel intent into ad playback or transfer it after that rejected event',()=>{
  const a=wheelPlaybackAdapter();let ad=false;
  Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:()=>ad}});
  a.wheel();a.message.mockClear();ad=true;a.handlers.play![0]!({isTrusted:true,target:a.video});
  ad=false;a.handlers.play![0]!({isTrusted:true,target:a.video});
  expect(a.pause).toHaveBeenCalledTimes(2);expect(a.message).not.toHaveBeenCalled();
});
it('ignores a synthetic play without granting or consuming a real wheel intent',()=>{
  const a=wheelPlaybackAdapter();a.wheel();a.message.mockClear();
  a.handlers.play![0]!({isTrusted:false,target:a.video});expect(a.message).not.toHaveBeenCalled();
  a.handlers.play![0]!({isTrusted:true,target:a.video});expect(bridgeValues(a)).toEqual(['manual']);expect(a.pause).not.toHaveBeenCalled();
});
it('samples a visible ready photo only four times through read-only probes without forcing status repaint',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';
  a.win.__atsumiXSurface!.visibility(true);a.flush();
  const before=a.doc.documentElement.outerHTML;a.message.mockClear();
  for(let i=0;i<4;i++)a.flush();
  const packets=a.message.mock.calls.map(([raw])=>{const url=new URL(raw.slice('ATSUMI_X_SURFACE_V1:'.length));return{kind:url.searchParams.get('kind'),data:JSON.parse(url.searchParams.get('value')!)};});
  expect(packets.map(p=>p.kind)).toEqual(['photoProbe','photoProbe','photoProbe','photoProbe']);
  expect(packets.map(p=>p.data.sample)).toEqual([1,2,3,4]);
  expect(JSON.stringify(packets)).not.toMatch(/creator|123456|PhotoFixture|https|pbs\.twimg/);
  expect(a.doc.documentElement.outerHTML).toBe(before);expect(a.timers).toHaveLength(0);
  a.message.mockClear();
  for(let i=0;i<5;i++){a.win.__atsumiXSurface!.visibility(true);a.flush();}
  expect(a.message).not.toHaveBeenCalled();expect(a.timers).toHaveLength(0);
});
it('does not include action probe counters in normal readiness or repaint when loaded/hover counters change',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();
  let loaded=0,hovered=false;
  const diagnostic=vi.fn(()=>({reason:'ready',imageLoaded:loaded,controlHovered:hovered}));
  Object.defineProperty(a.win,'__atsumiXActions',{value:{visibility:vi.fn(),update:vi.fn(),photoDiagnostic:diagnostic}});
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';
  a.win.__atsumiXSurface!.visibility(true);a.flush();
  expect(a.status().proof.photoShape).not.toHaveProperty('actionShape');
  expect(diagnostic).not.toHaveBeenCalled();
  for(let i=0;i<4;i++)a.flush();
  expect(diagnostic).toHaveBeenCalledTimes(4);
  diagnostic.mockClear();a.message.mockClear();loaded=1;hovered=true;
  for(let i=0;i<3;i++){a.win.__atsumiXSurface!.visibility(true);a.flush();}
  expect(diagnostic).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('does not emit late photo probes after route, visibility or document visibility changes',()=>{
  for(const change of ['route','pane','document']){
    const a=adapter(undefined,'foryou','x.com',true);a.flush();
    Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
    a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';
    a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
    if(change==='route')a.location.pathname='/creator/status/123456/photo/2';
    else if(change==='pane')a.win.__atsumiXSurface!.visibility(false);
    else Object.defineProperty(a.doc,'hidden',{value:true,configurable:true});
    for(let i=0;i<4;i++)a.flush();
    expect(a.message).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
  }
});
it('does not rearm four photo probes after temporary readiness loss within the same visible route visit',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';
  a.win.__atsumiXSurface!.visibility(true);a.flush();
  for(let i=0;i<4;i++)a.flush();
  a.message.mockClear();a.doc.body.innerHTML='';
  a.win.__atsumiXSurface!.visibility(true);a.flush();expect(a.status().state).toBe('loading');
  a.doc.body.innerHTML=nativePagePhoto;
  a.win.__atsumiXSurface!.visibility(true);a.flush();expect(a.status().state).toBe('ready');
  expect(a.timers).toHaveLength(0);
  expect(a.message.mock.calls.map(([raw])=>new URL(raw.slice('ATSUMI_X_SURFACE_V1:'.length)).searchParams.get('kind'))).toEqual(['status','status']);
});
it('never schedules photo probes for release or an unproved photo document',()=>{
  for(const diagnostics of [false,true]){
    const a=adapter(nativePagePhoto,'foryou','x.com',diagnostics);
    Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
    a.location.pathname='/creator/status/123456/photo/1';a.win.__atsumiXSurface!.visibility(true);a.flush();
    expect(a.timers).toHaveLength(0);
    expect(a.status().state).toBe('loading');
  }
  const a=adapter();a.flush();
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';
  a.win.__atsumiXSurface!.visibility(true);a.flush();
  expect(a.status().state).toBe('ready');expect(a.timers).toHaveLength(0);
});
it('sends only a native message envelope without navigating or falling back to navigation',()=>{
  const a=adapter();a.flush();expect(a.message).toHaveBeenCalledOnce();
  expect(a.message.mock.calls[0]![0]).toMatch(/^ATSUMI_X_SURFACE_V1:https:\/\/x\.com\/__atsumi_surface__\/fixture\?kind=status&value=/);
  expect(a.location.assign).not.toHaveBeenCalled();
  a.message.mockClear();Reflect.deleteProperty(a.win,'chrome');a.update();
  expect(a.message).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
});
it('does not resend unchanged readiness merely because the native pane is repainted',()=>{
  const a=adapter();a.flush();a.message.mockClear();
  for(let i=0;i<5;i++){a.win.__atsumiXSurface!.visibility(true);a.flush();}
  expect(a.message).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
});
it('completes identity proof while hidden, retaining the entire official DOM',()=>{
  const a=adapter();a.flush();expect(a.status()).toMatchObject({state:'ready',owner:'alice'});
  expect(a.doc.querySelectorAll('img,video')).toHaveLength(2);expect(a.doc.querySelector('[data-testid=socialContext]')?.textContent).toContain('reposted');
  expect(a.doc.querySelectorAll('[data-testid=reply],[data-testid=retweet],[data-testid=like]')).toHaveLength(3);
});
it('does not infer ownership from post authors and rejects another account',()=>{
  const a=adapter('<div data-testid="primaryColumn">'+post+'</div>');a.flush();expect(a.status()).toMatchObject({state:'loading',owner:null,proof:{profile:false}});
  const b=adapter(identity.replace('/alice','/bob')+'<div data-testid="primaryColumn"></div>');b.flush();expect(b.status().state).toBe('ownerChanged');
});
it('does not run on a foreign origin',()=>{const a=adapter('', 'foryou','x.com.evil.test');a.flush();expect(a.win.__atsumiXSurface).toBeUndefined();expect(a.message).not.toHaveBeenCalled();});
it('keeps document proof across responsive navigation removal, but clears it for login',()=>{
  const a=adapter();a.flush();a.doc.querySelector('[data-testid=AppTabBar_Profile_Link]')!.remove();a.doc.querySelector('[data-testid=SideNav_AccountSwitcher_Button]')!.remove();a.update();expect(a.status().state).toBe('ready');
  a.location.pathname='/i/flow/login';a.update();expect(a.status().state).toBe('loginRequired');a.location.pathname='/home';a.update();expect(a.status().state).toBe('loading');
});
it('revokes cached owner proof on an explicit mismatching profile even without the account control',()=>{
  const a=adapter();a.flush();
  a.doc.querySelector('[data-testid=SideNav_AccountSwitcher_Button]')!.remove();
  a.doc.querySelector('[data-testid=AppTabBar_Profile_Link]')!.setAttribute('href','/bob');a.update();
  expect(a.status()).toMatchObject({state:'ownerChanged',owner:null,proof:{profile:true,account:false}});
  a.doc.querySelector('[data-testid=AppTabBar_Profile_Link]')!.remove();a.update();
  expect(a.status()).toMatchObject({state:'loading',owner:null});
});
it('requires fresh full proof after a mismatch and keeps login above stale account evidence',()=>{
  const a=adapter();a.flush();const profile=a.doc.querySelector('[data-testid=AppTabBar_Profile_Link]')!;
  const account=a.doc.querySelector('[data-testid=SideNav_AccountSwitcher_Button]')!;account.remove();
  profile.setAttribute('href','/bob');a.update();expect(a.status().state).toBe('ownerChanged');
  profile.setAttribute('href','/alice');a.update();expect(a.status().state).toBe('loading');
  a.doc.body.append(account);a.update();expect(a.status()).toMatchObject({state:'ready',owner:'alice'});
  a.location.pathname='/i/flow/login';profile.setAttribute('href','/bob');a.update();expect(a.status().state).toBe('loginRequired');
});
const nativePhoto='<div role="dialog"><button aria-label="Close">Close</button><img src="https://pbs.twimg.com/media/PhotoFixture?format=jpg&amp;name=large"></div>';
it('keeps an exact native photo dialog visible after the proven owner layout loses primaryColumn',()=>{
  const a=adapter();a.flush();a.doc.body.innerHTML=nativePhoto;
  a.location.pathname='/creator/status/123456/photo/1';a.update();
  expect(a.status()).toMatchObject({state:'ready',owner:'alice',proof:{primary:false,photo:true}});
  expect(a.doc.querySelector('[role=dialog]')?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  a.location.pathname='/home';a.update();expect(a.status().state).toBe('loading');
});
it('does not treat unknown dialogs or image routes as owner or photo readiness proof',()=>{
  const missing=adapter(nativePhoto);missing.location.pathname='/creator/status/123456/photo/1';missing.flush();
  expect(missing.status()).toMatchObject({state:'loading',owner:null});
  for(const path of ['/home','/creator/status/123456','/creator/status/123456/photo/0','/creator/status/123456/photo/5','/creator/status/123456/photo/1/extra','/creator/status/123456/video/1']){
    const a=adapter();a.flush();a.doc.body.innerHTML=nativePhoto;a.location.pathname=path;a.update();expect(a.status().state).toBe('loading');
  }
  for(const html of [nativePhoto.replace('https://pbs.twimg.com/media/PhotoFixture','https://pbs.twimg.com.evil.test/media/PhotoFixture'),nativePhoto.replace('/media/PhotoFixture','/profile_images/PhotoFixture'),nativePhoto.replace('<div role="dialog">','<div role="dialog" aria-hidden="true">'),nativePhoto.replace('<img','<input type="password"><img'),nativePhoto.replace('<img','<a href="/i/flow/login">Log in</a><img')]){
    const a=adapter();a.flush();a.doc.body.innerHTML=html;a.location.pathname='/creator/status/123456/photo/1';a.update();expect(a.status().state).not.toBe('ready');
  }
});
it('never keeps photo readiness across an explicit account conflict or authentication flow',()=>{
  const a=adapter();a.flush();a.doc.body.innerHTML=identity.replace('/alice','/bob')+nativePhoto;
  a.location.pathname='/creator/status/123456/photo/1';a.update();expect(a.status().state).toBe('ownerChanged');
  a.doc.body.innerHTML=nativePhoto;a.update();expect(a.status().state).toBe('loading');
  const b=adapter();b.flush();b.doc.body.innerHTML=nativePhoto;b.location.pathname='/i/flow/login';b.update();expect(b.status().state).toBe('loginRequired');
});
const nativePagePhoto='<img src="https://pbs.twimg.com/media/PhotoFixture?format=jpg&amp;name=large">';
it('admits the observed full-page photo layout only after this document proved the same owner',()=>{
  for(const html of [nativePagePhoto,nativePhoto.replace('role="dialog"','role="region"')]){
    const a=adapter();a.flush();a.doc.body.innerHTML=html;a.location.pathname='/creator/status/123456/photo/1';a.update();
    expect(a.status()).toMatchObject({state:'ready',owner:'alice',proof:{profile:false,account:false,primary:false,photo:true}});
    a.location.pathname='/home';a.update();expect(a.status().state).toBe('loading');
  }
  const b=adapter(nativePagePhoto);b.location.pathname='/creator/status/123456/photo/1';b.flush();expect(b.status()).toMatchObject({state:'loading',owner:null});
});
it('does not use full-page image fallback across hidden media, unrelated dialogs or authentication gates',()=>{
  for(const html of ['<div hidden>'+nativePagePhoto+'</div>','<div aria-hidden="true">'+nativePagePhoto+'</div>','<div inert>'+nativePagePhoto+'</div>',nativePagePhoto.replace('pbs.twimg.com','pbs.twimg.com.evil.test'),nativePagePhoto.replace('/media/','/profile_images/'),nativePagePhoto+'<div role="dialog"><p>Unrelated dialog</p></div>',nativePagePhoto+'<input type="password">',nativePagePhoto+'<input autocomplete="one-time-code">',nativePagePhoto+'<input data-testid="ocfEnterTextTextInput">',nativePagePhoto+'<a href="/i/flow/login">Login</a>','']){
    const a=adapter();a.flush();a.doc.body.innerHTML=html;a.location.pathname='/creator/status/123456/photo/1';a.update();expect(a.status().state).not.toBe('ready');
  }
});
it('revokes full-page photo identity on conflicting account evidence and does not reuse it after login',()=>{
  const a=adapter();a.flush();a.doc.body.innerHTML=identity.replace('/alice','/bob')+nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';a.update();expect(a.status().state).toBe('ownerChanged');
  a.doc.body.innerHTML=nativePagePhoto;a.update();expect(a.status().state).toBe('loading');
  const b=adapter();b.flush();b.doc.body.innerHTML=nativePagePhoto;b.location.pathname='/i/flow/login';b.update();expect(b.status().state).toBe('loginRequired');b.location.pathname='/creator/status/123456/photo/1';b.update();expect(b.status().state).toBe('loading');
});
it('returns read-only photo archive proof only for the visible already-proven selected owner',()=>{
  const a=adapter();a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/creator/status/123456/photo/1';a.update();a.message.mockClear();
  const before=a.doc.documentElement.outerHTML;
  expect(Object.isFrozen(a.win.__atsumiXSurface)).toBe(true);
  expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('alice');
  expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('alice');
  expect(a.doc.documentElement.outerHTML).toBe(before);expect(a.message).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
  a.win.__atsumiXSurface!.visibility(false);expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('');
});
it('does not infer photo archive ownership or admit noncanonical routes',()=>{
  const missing=adapter(nativePagePhoto);missing.location.pathname='/creator/status/123456/photo/1';missing.flush();missing.win.__atsumiXSurface!.visibility(true);missing.flush();expect(missing.win.__atsumiXSurface!.archivePhotoOwner()).toBe('');
  for(const path of ['/home','/creator/status/123456','/creator/status/123456/video/1','/creator/status/123456/photo/0','/creator/status/123456/photo/5','/creator/status/123456/photo/1/extra','/i/web/status/123456/photo/1','/creator/status/012345/photo/1']){
    const a=adapter();a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.doc.body.innerHTML=nativePagePhoto;a.location.pathname=path;expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('');
  }
});
it('rejects photo archive proof on current unobserved account conflict or authentication gates',()=>{
  for(const extra of ['<a data-testid="AppTabBar_Profile_Link" href="/bob"></a>','<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><a data-testid="AppTabBar_Profile_Link" href="/bob"></a>','<a data-testid="AppTabBar_Profile_Link" href="https://evil.test/alice"></a>','<a data-testid="AppTabBar_Profile_Link" href="/alice/other"></a>','<a data-testid="AppTabBar_Profile_Link"></a>','<input type="password">','<input autocomplete="one-time-code">','<input data-testid="ocfEnterTextTextInput">','<button data-testid="LoginForm_Login_Button">Login</button>','<a href="/i/flow/login">Login</a>','<a href="/login">Login</a>']){
    const a=adapter();a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.doc.body.innerHTML=nativePagePhoto+extra;a.location.pathname='/creator/status/123456/photo/1';a.message.mockClear();
    expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('');expect(a.message).not.toHaveBeenCalled();
  }
});
it('rejects photo archive proof without a permitted exposed source image',()=>{
  for(const html of ['', '<div hidden>'+nativePagePhoto+'</div>','<div aria-hidden="true">'+nativePagePhoto+'</div>','<div inert>'+nativePagePhoto+'</div>',nativePagePhoto.replace('pbs.twimg.com','pbs.twimg.com.evil.test'),nativePagePhoto.replace('/media/','/profile_images/'),nativePagePhoto+'<div role="dialog">Unrelated dialog</div>']){
    const a=adapter();a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.doc.body.innerHTML=html;a.location.pathname='/creator/status/123456/photo/1';expect(a.win.__atsumiXSurface!.archivePhotoOwner()).toBe('');
  }
});
it('never clicks reply, like, repost, or bookmark buttons automatically',()=>{
  const a=adapter(),buttons=[...a.doc.querySelectorAll('button')].map(b=>vi.spyOn(b,'click'));a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();expect(buttons.every(b=>b.mock.calls.length===0)).toBe(true);
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql|__TAURI/);
});
it('selects only the requested ordinary official home tab once',()=>{
  const a=adapter(identity+'<div data-testid="primaryColumn"><div role="tablist"><button role="tab" aria-selected="true">For you</button><button role="tab" aria-selected="false">Following</button></div></div>','following');
  const button=a.doc.querySelectorAll<HTMLButtonElement>('[role=tab]')[1]!,click=vi.spyOn(button,'click');a.flush();a.update();expect(click).toHaveBeenCalledOnce();
});
it('hides only the duplicate two-item collection navigation on its matching collection route',()=>{
  const pair='<div role="tablist" id="collection-tabs"><a role="tab" href="/i/bookmarks">북마크</a><a role="tab" href="/i/history/likes">마음에 들어요</a></div>';
  for(const [feed,path] of [['bookmarks','/i/bookmarks'],['bookmarks','/i/history/bookmarks'],['bookmarks','/i/history'],['likes','/i/history/likes'],['likes','/alice/likes']]){
    const a=adapter(identity+'<div data-testid="primaryColumn"><h1>History</h1><input placeholder="Search">'+pair+post+'<div role="tablist" id="other-tabs"><button role="tab">Latest</button><button role="tab">Media</button></div></div>',feed);
    a.location.pathname=path!;a.flush();expect(a.doc.querySelector('#collection-tabs')?.getAttribute('data-atsumi-x-collection-tabs')).toBe('true');
    for(const node of a.doc.querySelectorAll('h1,input,article,#other-tabs,[data-testid=primaryColumn]'))expect(node.hasAttribute('data-atsumi-x-collection-tabs')).toBe(false);
    a.location.pathname='/home';a.update();expect(a.doc.querySelector('[data-atsumi-x-collection-tabs]')).toBeNull();
  }
});
it('recognizes the exact source collection labels when the two native tabs have no links',()=>{
  for(const [bookmarks,likes] of [['Bookmarks','Likes'],['북마크','마음에 들어요'],['ブックマーク','いいね']]){
    const a=adapter(identity+'<div data-testid="primaryColumn"><div role="tablist"><button role="tab">'+bookmarks+'</button><button role="tab">'+likes+'</button></div></div>','bookmarks');
    const clicks=[...a.doc.querySelectorAll('button')].map(button=>vi.spyOn(button,'click'));a.location.pathname='/i/bookmarks';a.flush();
    expect(a.doc.querySelector('[data-atsumi-x-collection-tabs]')).not.toBeNull();expect(clicks.every(click=>click.mock.calls.length===0)).toBe(true);
  }
});
it('preserves unrelated, ambiguous, detail, foreign-account and nonmatching collection navigation',()=>{
  for(const html of ['<div role="tablist"><button role="tab">Bookmarks</button><button role="tab">Likes</button><button role="tab">Replies</button></div>','<div role="tablist"><button role="tab">Bookmarks</button><button role="tab">Bookmarks</button></div>','<div role="tablist"><a role="tab" href="/i/bookmarks">Bookmarks</a><a role="tab" href="/bob/likes">Likes</a></div>','<article><div role="tablist"><button role="tab">Bookmarks</button><button role="tab">Likes</button></div></article>','<div role="dialog"><div role="tablist"><button role="tab">Bookmarks</button><button role="tab">Likes</button></div></div>']){
    const a=adapter(identity+'<div data-testid="primaryColumn">'+html+'</div>','bookmarks');a.location.pathname='/i/bookmarks';a.flush();expect(a.doc.querySelector('[data-atsumi-x-collection-tabs]')).toBeNull();
  }
  for(const [feed,path] of [['preview','/i/bookmarks'],['foryou','/i/bookmarks'],['likes','/i/bookmarks'],['bookmarks','/i/history/likes'],['likes','/bob/likes']]){
    const a=adapter(identity+'<div data-testid="primaryColumn"><div role="tablist"><button role="tab">Bookmarks</button><button role="tab">Likes</button></div></div>',feed);a.location.pathname=path!;a.flush();expect(a.doc.querySelector('[data-atsumi-x-collection-tabs]')).toBeNull();
  }
});
const paneBack='<button data-testid="app-bar-back" id="pane-back"><svg><path></path></svg></button>';
it('removes only the exact primary-column Back in a scoped preview or collection root',()=>{
  for(const [feed,path] of [['preview','/creator/status/123456'],['preview','/creator'],['likes','/alice/likes'],['likes','/i/history/likes'],['bookmarks','/i/bookmarks'],['bookmarks','/i/history/bookmarks'],['bookmarks','/i/history']]){
    const a=adapter(identity+'<div data-testid="primaryColumn">'+paneBack+'<h1>Native title</h1>'+post+'</div>',feed);
    a.location.pathname=path!;a.flush();
    expect(a.doc.querySelector('#pane-back')?.getAttribute('data-atsumi-x-pane-back')).toBe('true');
    expect(a.doc.querySelector('style')?.textContent).toContain('[data-atsumi-x-pane-back="true"]');
    for(const node of a.doc.querySelectorAll('h1,article,video,[data-testid=primaryColumn]'))expect(node.hasAttribute('data-atsumi-x-pane-back')).toBe(false);
  }
});
it('blocks scoped Back during its first visible frame without invoking native history or changing accounts',()=>{
  for(const [feed,path] of [['preview','/creator'],['likes','/alice/likes'],['bookmarks','/i/bookmarks']]){
    const a=adapter(identity+'<div data-testid="primaryColumn">'+paneBack+post+'</div>',feed);
    a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.location.pathname=path!;a.message.mockClear();
    const event={target:a.doc.querySelector('#pane-back path')!,isTrusted:true,button:0,defaultPrevented:false,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};
    a.handlers.click![0]!(event);
    expect(event.preventDefault).toHaveBeenCalledOnce();expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
    expect(a.message).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
  }
});
it('preserves photo/modal/media Back and ignores arbitrary labels or unrelated controls',()=>{
  const cases=[
    {feed:'preview',path:'/creator/status/123456/photo/1',html:paneBack},
    {feed:'preview',path:'/creator/status/123456/video/1',html:paneBack},
    {feed:'preview',path:'/creator',html:'<div role="dialog">'+paneBack+'</div>'},
    {feed:'preview',path:'/creator',html:'<div aria-modal="true">'+paneBack+'</div>'},
    {feed:'preview',path:'/creator',html:'<div data-testid="videoPlayer">'+paneBack+'</div>'},
    {feed:'preview',path:'/creator',html:'<article>'+paneBack+'</article>'},
    {feed:'preview',path:'/creator',html:'<button aria-label="Back" id="pane-back"><svg><path></path></svg></button>'},
    {feed:'foryou',path:'/home',html:paneBack},
    {feed:'likes',path:'/bob/likes',html:paneBack},
    {feed:'bookmarks',path:'/alice/likes',html:paneBack},
  ];
  for(const {feed,path,html} of cases){
    const a=adapter(identity+'<div data-testid="primaryColumn">'+html+post+'</div>',feed);
    a.location.pathname=path;a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
    expect(a.doc.querySelector('[data-atsumi-x-pane-back]')).toBeNull();
    const event={target:a.doc.querySelector('#pane-back path')!,isTrusted:true,button:0,defaultPrevented:false,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};
    a.handlers.click![0]!(event);expect(event.preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('restores a previously trimmed Back when X enters its own full-page photo route',()=>{
  const a=adapter(identity+'<div data-testid="primaryColumn">'+paneBack+post+'</div>','preview');
  a.location.pathname='/creator/status/123456';a.flush();expect(a.doc.querySelector('[data-atsumi-x-pane-back]')).not.toBeNull();
  a.location.pathname='/creator/status/123456/photo/1';a.update();expect(a.doc.querySelector('[data-atsumi-x-pane-back]')).toBeNull();
});
it('does not trim a Back outside the primary column, during fullscreen, or before owner verification',()=>{
  const outside=adapter(identity+paneBack+'<div data-testid="primaryColumn">'+post+'</div>','preview');
  outside.location.pathname='/creator';outside.flush();expect(outside.doc.querySelector('[data-atsumi-x-pane-back]')).toBeNull();
  const fullscreen=adapter(identity+'<div data-testid="primaryColumn">'+paneBack+post+'</div>','preview');
  fullscreen.location.pathname='/creator';Object.defineProperty(fullscreen.doc,'fullscreenElement',{value:fullscreen.doc.querySelector('video')});fullscreen.flush();expect(fullscreen.doc.querySelector('[data-atsumi-x-pane-back]')).toBeNull();
  for(const prefix of ['',identity.replace('/alice','/bob')]){
    const a=adapter(prefix+'<div data-testid="primaryColumn">'+paneBack+'</div>','preview');
    a.location.pathname='/creator';a.flush();expect(a.doc.querySelector('[data-atsumi-x-pane-back]')).toBeNull();
  }
});
it('reveals only a visible ordinary warning, never an age or authentication gate',()=>{
  const warning=(text:string)=>'<div data-testid="sensitiveMediaWarning">'+text+'<button>Show</button></div>';
  const a=adapter(identity+'<div data-testid="primaryColumn"><article data-testid="tweet">'+warning('Sensitive content')+warning('Sensitive content verify age 18')+'</article></div>');
  const buttons=[...a.doc.querySelectorAll<HTMLButtonElement>('article button')].map(b=>vi.spyOn(b,'click'));
  a.flush();expect(buttons[0]).not.toHaveBeenCalled();a.win.__atsumiXSurface!.visibility(true);a.flush();a.update();
  expect(buttons[0]).toHaveBeenCalledOnce();expect(buttons[1]).not.toHaveBeenCalled();
});
it('pauses hidden media and blocks background play events',()=>{
  const a=adapter(),video=a.doc.querySelector('video')!,pause=vi.spyOn(video,'pause').mockImplementation(()=>{});a.flush();a.win.__atsumiXSurface!.visibility(false);expect(pause).toHaveBeenCalled();
  a.handlers.play![0]!({target:video});expect(pause).toHaveBeenCalledTimes(2);
});
it('routes trusted plain post clicks to preview but leaves native social actions and video alone',()=>{
  const a=adapter();a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
  const click=(target:Element,isTrusted=true)=>{const event={target,isTrusted,button:0,defaultPrevented:false,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.click![0]!(event);return event;};
  expect(click(a.doc.querySelector('[data-testid=tweetText]')!).preventDefault).toHaveBeenCalled();expect(a.message).toHaveBeenLastCalledWith(expect.stringContaining('kind=preview'));
  a.message.mockClear();for(const target of a.doc.querySelectorAll('button,video'))expect(click(target).preventDefault).not.toHaveBeenCalled();
  click(a.doc.querySelector('[data-testid=tweetText]')!,false);expect(a.message).not.toHaveBeenCalled();
});
it('does not hijack preview reply typing or native dialog Escape',()=>{
  const a=adapter(undefined,'preview');a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
  const input=a.doc.createElement('textarea');a.doc.body.append(input);
  const key={target:input,key:'Escape',isTrusted:true,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.keydown![0]!(key);expect(key.preventDefault).not.toHaveBeenCalled();
  const dialog=a.doc.createElement('div');dialog.setAttribute('role','dialog');a.doc.body.append(dialog);a.handlers.keydown![0]!({...key,target:dialog});expect(a.message).not.toHaveBeenCalled();
});
it('leaves Escape to native full-page photos on exact photo routes without a dialog',()=>{
  for(const owner of ['creator','i/web'])for(const index of [1,2,3,4]){
    const a=adapter(undefined,'preview');a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();
    a.doc.body.innerHTML=nativePagePhoto;a.location.pathname='/'+owner+'/status/123456/photo/'+index;a.update();expect(a.status().state).toBe('ready');a.message.mockClear();
    const key={target:a.doc.body,key:'Escape',isTrusted:true,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.keydown![0]!(key);
    expect(key.preventDefault).not.toHaveBeenCalled();expect(key.stopImmediatePropagation).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('keeps ordinary preview Escape closing without exempting arbitrary photo-looking routes',()=>{
  for(const path of ['/creator/status/123456','/creator','/creator/status/123456/photo/0','/creator/status/123456/photo/5','/creator/status/123456/photo/1/extra','/creator/photo/1','/creator/status/012345/photo/1']){
    const a=adapter(undefined,'preview');a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.location.pathname=path;a.message.mockClear();
    const key={target:a.doc.body,key:'Escape',isTrusted:true,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.keydown![0]!(key);
    expect(key.preventDefault).toHaveBeenCalledOnce();expect(a.message).toHaveBeenCalledExactlyOnceWith(expect.stringContaining('kind=close&value='));
  }
});
const visibleAdapter=(html?:string,feed='foryou')=>{const a=adapter(html,feed);a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();return a;};
const clickTarget=(a:ReturnType<typeof adapter>,target:Element,extra={})=>{const e={target,isTrusted:true,button:0,defaultPrevented:false,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...extra};a.handlers.click![0]!(e);return e;};
const bridgeValues=(a:ReturnType<typeof adapter>)=>a.message.mock.calls.map(([url])=>new URL(url.slice('ATSUMI_X_SURFACE_V1:'.length)).searchParams.get('value'));
it.each(['foryou','preview'])('opens an explicit external %s link without changing either source document',feed=>{
  const a=visibleAdapter(undefined,feed);Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  const link=a.doc.createElement('a');link.setAttribute('href','https://t.co/ObservedLink?ref=one%20two');link.innerHTML='<span>External card</span>';a.doc.querySelector('article')!.append(link);
  const before={href:a.location.href,path:a.location.pathname,html:a.doc.querySelector('main')!.innerHTML};
  for(const extra of [{},{ctrlKey:true},{shiftKey:true},{metaKey:true}]){
    a.message.mockClear();const event=clickTarget(a,link.firstElementChild!,extra);
    expect(event.preventDefault).toHaveBeenCalledOnce();expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
    expect(bridgeEntries(a)).toEqual([['externalLink','https://t.co/ObservedLink?ref=one%20two']]);
  }
  a.message.mockClear();const event={isTrusted:true,target:link,button:1,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.auxclick![0]!(event);
  expect(bridgeEntries(a)).toEqual([['externalLink','https://t.co/ObservedLink?ref=one%20two']]);expect(event.preventDefault).toHaveBeenCalledOnce();
  expect({href:a.location.href,path:a.location.pathname,html:a.doc.querySelector('main')!.innerHTML}).toEqual(before);
  expect(a.location.assign).not.toHaveBeenCalled();expect(a.sourcePush).not.toHaveBeenCalled();expect(a.sourceReplace).not.toHaveBeenCalled();
});
it.each(['synthetic','hidden-pane','hidden-document','owner','login','detached','foreign','editor','button','hidden-node','ad','download','selection','prevented','alt','secondary'])('does not open a browser from a %s external activation',mode=>{
  const a=visibleAdapter();Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  const link=a.doc.createElement('a');link.setAttribute('href','https://example.org/article');link.textContent='External';a.doc.querySelector('article')!.append(link);
  let target:Element=link;
  if(mode==='hidden-pane')a.win.__atsumiXSurface!.visibility(false);
  if(mode==='hidden-document')Object.defineProperty(a.doc,'hidden',{value:true});
  if(mode==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/other');
  if(mode==='login'){const input=a.doc.createElement('input');input.type='password';a.doc.body.append(input);}
  if(mode==='detached')link.remove();
  if(mode==='foreign'){target=document.createElement('a');target.setAttribute('href','https://example.org/article');document.body.append(target);}
  if(mode==='editor'||mode==='button'){const child=a.doc.createElement(mode==='editor'?'textarea':'button');link.append(child);target=child;}
  if(mode==='hidden-node')link.setAttribute('hidden','');
  if(mode==='ad')Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:()=>true}});
  if(mode==='download')link.setAttribute('download','file');
  if(mode==='selection')a.win.getSelection=()=>({toString:()=> 'selected text'});
  a.message.mockClear();clickTarget(a,target,{isTrusted:mode!=='synthetic',defaultPrevented:mode==='prevented',altKey:mode==='alt',button:mode==='secondary'?2:0});
  expect(bridgeEntries(a).filter(([kind])=>kind==='externalLink')).toEqual([]);
  if(mode==='foreign')target.remove();
});
it.each(['javascript:alert(1)','data:text/html,hello','file:///C:/private','ms-settings:','blob:https://example.org/id','https://user:secret@example.org/','https://example.org/\\unsafe','https://example.org/a\nb','https://example.org/a b','https://x.com./creator','https://www.x.com./creator','https://example.org/'+ 'a'.repeat(2048)])('rejects unsafe or oversized external href %s',href=>{
  const a=visibleAdapter();Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  const link=a.doc.createElement('a');link.setAttribute('href',href);a.doc.querySelector('article')!.append(link);
  clickTarget(a,link);expect(bridgeEntries(a).filter(([kind])=>kind==='externalLink')).toEqual([]);
});
it('opens a validated external context target only after a fresh trusted unchanged action',()=>{
  for(const mode of ['unchanged','changed','owner','hidden','source-hidden','removed','synthetic']){
    const a=visibleAdapter();Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
    const link=a.doc.createElement('a');link.href='https://example.org/observed';link.innerHTML='<span>External</span>';a.doc.querySelector('article')!.append(link);
    let handler:EventListener|undefined;const original=Element.prototype.addEventListener;
    const spy=vi.spyOn(Element.prototype,'addEventListener').mockImplementation(function(this:Element,type,listener,options){if(type==='click'&&this.getAttribute('role')==='menuitem'&&typeof listener==='function')handler=listener;return original.call(this,type,listener,options);});
    a.handlers.contextmenu![0]!({target:link.firstElementChild,isTrusted:true,clientX:20,clientY:20,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()});
    const menu=a.doc.querySelector<HTMLElement>('.atsumi-x-link-menu')!;expect(menu.style.background).toBe('rgb(0, 0, 0)');expect(a.message).not.toHaveBeenCalled();
    if(mode==='changed')link.href='https://example.org/other';
    if(mode==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/other');
    if(mode==='hidden')a.win.__atsumiXSurface!.visibility(false);
    if(mode==='source-hidden')link.setAttribute('hidden','');
    if(mode==='removed')link.remove();
    handler!.call(menu.querySelector('button')!,{isTrusted:mode!=='synthetic',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()} as unknown as Event);
    expect(bridgeEntries(a)).toEqual(mode==='unchanged'?[['externalLink','https://example.org/observed']]:[]);spy.mockRestore();
  }
});
it('routes an observed author or repost actor profile into the independent preview',()=>{
  const a=visibleAdapter(identity+'<main role="main"><div data-testid="primaryColumn">'+post.replace('Landscape','<a href="/creator">Creator</a><a href="https://www.x.com/reposter">Reposter</a>')+'</div></main>');
  for(const link of a.doc.querySelectorAll('[data-testid=tweetText] a'))expect(clickTarget(a,link).preventDefault).toHaveBeenCalled();
  expect(bridgeValues(a)).toEqual(['/creator','/reposter']);
});
it('never turns source navigation or malformed profile paths into profiles',()=>{
  const a=visibleAdapter();
  for(const href of ['/home','/i','/settings','/compose/post','/explore','/search','/login','/logout','/notifications','/messages','/grok','//evil.test/alice','https://user@x.com/alice','https://x.com:444/alice','/alice/extra','/alice%2fbob']){
    const link=a.doc.createElement('a');link.href=href;a.doc.body.append(link);expect(clickTarget(a,link).preventDefault).not.toHaveBeenCalled();
  }
  expect(a.message).not.toHaveBeenCalled();
});
it('routes an exact nested quote permalink without guessing the enclosing original',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><a href="/quoted/status/654321"><time>quoted</time></a></div><p data-testid="tweetText" id="quoted-body">Quote</p></div>';
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>');
  clickTarget(a,a.doc.querySelector('#quoted-body')!);expect(bridgeValues(a)).toEqual(['/quoted/status/654321']);
});
it('routes button-role quotes only from their own exact observed permalink',()=>{
  const quote='<div role="button" tabindex="0"><div data-testid="User-Name"><a href="/quoted">Quoted</a><a href="/quoted/status/654321"><time>quoted</time></a></div><p id="quoted-body">Quote</p><button data-testid="like">Like</button></div>';
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>');
  expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).toHaveBeenCalled();expect(bridgeValues(a)).toEqual(['/quoted/status/654321']);
  a.message.mockClear();const author=a.doc.querySelector('#quoted-body')!.parentElement!.querySelector('a')!;
  expect(clickTarget(a,author).preventDefault).toHaveBeenCalled();expect(bridgeValues(a)).toEqual(['/quoted']);
  a.message.mockClear();for(const button of a.doc.querySelectorAll('button'))expect(clickTarget(a,button).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('never borrows a parent permalink for a linkless or ambiguous button-role card',()=>{
  for(const content of ['<p id="quoted-body">Card</p>','<div data-testid="User-Name"><a href="/quoted">Quoted</a><time>date</time></div><p id="quoted-body">Quote</p>','<div data-testid="User-Name"><a href="/quoted/status/654321"><time>date</time></a><a href="/other/status/654322"><time>date</time></a></div><p id="quoted-body">Quote</p>']){
    const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>','<div role="button">'+content+'</div></article>')+'</div>');
    expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('routes quote body from its unique own photo permalink and matching own author',()=>{
  for(const role of ['link','button']){
    const quote='<div role="'+role+'"><div data-testid="User-Name"><a href="/quoted">Quoted</a><time>date</time></div><p id="quoted-body">Quote</p><a href="/quoted/status/654321/photo/1" id="photo"><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Fixture"></div></a></div>';
    const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>');
    expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).toHaveBeenCalled();expect(bridgeValues(a)).toEqual(['/quoted/status/654321']);
    a.message.mockClear();for(const target of a.doc.querySelectorAll('#photo,#photo img,[data-testid=tweetPhoto]'))expect(clickTarget(a,target).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('routes the observed quote shape with an explicit own handle but no author or time link',()=>{
  for(const author of ['<span>@quoted</span>','<span><span>@</span><span>Quoted</span></span>']){
    const quote='<div role="link"><div data-testid="User-Name"><span>Quoted name</span>'+author+'<time>date</time></div><p id="quoted-body">Quote</p><a href="/quoted/status/654321/photo/1" id="photo"><img src="https://pbs.twimg.com/media/Fixture"></a></div>';
    const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>');
    expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).toHaveBeenCalledOnce();expect(bridgeValues(a)).toEqual(['/quoted/status/654321']);
    a.message.mockClear();for(const target of a.doc.querySelectorAll('#photo,#photo img'))expect(clickTarget(a,target).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('does not borrow, guess or override missing or conflicting rendered quote authors',()=>{
  for(const author of ['', '<span>quoted</span>', '<span>@other</span>', '<span>@quoted</span><span>@other</span>', '<a href="/other">Other</a><span>@quoted</span>', '<a href="/quoted/status/654321">Unknown name link</a><span>@quoted</span>', '<div hidden><span>@quoted</span></div>', '<div aria-hidden="true"><span>@quoted</span></div>', '<div inert><span>@quoted</span></div>', '<div role="link"><span>@quoted</span></div>', '<span>Contact @quoted</span>', '<span>@quotedtoolonghandle</span>']){
    const quote='<div role="link"><div data-testid="User-Name"><span>Quoted name</span>'+author+'<time>date</time></div><p id="quoted-body">Quote</p><a href="/quoted/status/654321/photo/1"><img></a></div>';
    const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('Landscape','<span>@quoted</span>').replace('</article>',quote+'</article>')+'</div>');
    expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
});
it('rejects conflicting or borrowed quote media identities and never guesses an outer media-only post',()=>{
  const own='<div data-testid="User-Name"><a href="/quoted">Quoted</a><time>date</time></div><p id="quoted-body">Quote</p>';
  for(const media of ['<a href="/other/status/654321/photo/1"><img></a>','<a href="/quoted/status/654321/photo/1"><img></a><a href="/quoted/status/654322/photo/1"><img></a>','<div role="link"><a href="/quoted/status/654321/photo/1"><img></a></div>','<a href="/quoted/status/654321/photo/1">Link without image</a>']){
    const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>','<div role="link">'+own+media+'</div></article>')+'</div>');
    expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
  }
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn"><article data-testid="tweet">'+own+'<a href="/quoted/status/654321/photo/1"><img></a></article></div>');
  expect(clickTarget(a,a.doc.querySelector('#quoted-body')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('leaves linkless quotes and card navigation native instead of routing the parent',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><a href="/quoted">Quoted</a><time>date</time></div><p id="quoted-body">Quote</p></div><div role="link" id="card">Card</div>';
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>');
  for(const target of a.doc.querySelectorAll('#quoted-body,#card'))expect(clickTarget(a,target).preventDefault).not.toHaveBeenCalled();
  expect(a.message).not.toHaveBeenCalled();
});
it('does not infer a parent identity from a quoted author when its own permalink is absent',()=>{
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn"><article data-testid="tweet"><p id="outer">Outer</p><div role="link"><div data-testid="User-Name"><a href="/quoted/status/654321"><time>date</time></a></div></div></article></div>');
  expect(clickTarget(a,a.doc.querySelector('#outer')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('preserves native photo/video links, image viewer dialogs and modified clicks',()=>{
  const a=visibleAdapter();
  for(const target of a.doc.querySelectorAll('[data-testid=tweetPhoto],img,video'))expect(clickTarget(a,target).preventDefault).not.toHaveBeenCalled();
  for(const href of ['/creator/status/123456/photo/1','/creator/status/123456/video/1']){const link=a.doc.createElement('a');link.href=href;a.doc.body.append(link);expect(clickTarget(a,link).preventDefault).not.toHaveBeenCalled();}
  const dialog=a.doc.createElement('div');dialog.setAttribute('role','dialog');dialog.innerHTML='<a href="/creator/status/123456">Post</a>';a.doc.body.append(dialog);expect(clickTarget(a,dialog.firstElementChild!).preventDefault).not.toHaveBeenCalled();
  for(const extra of [{metaKey:true},{altKey:true},{shiftKey:true},{ctrlKey:true,shiftKey:true}])expect(clickTarget(a,a.doc.querySelector('[data-testid=tweetText]')!,extra).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('keeps all detail/profile navigation in the official right document',()=>{
  const a=visibleAdapter(undefined,'preview');expect(clickTarget(a,a.doc.querySelector('[data-testid=tweetText]')!).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('uses the exact context link, including a profile, without falling back from a linkless quote',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><time>date</time></div><p id="quote">Quote</p></div>';
  const a=visibleAdapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>','<a href="/quoted/status/654321" id="exact">Quoted post</a><a href="/creator" id="profile">Creator</a>'+quote+'</article>')+'</div>');
  const context=(id:string)=>{const e={target:a.doc.querySelector(id),isTrusted:true,clientX:20,clientY:20,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};a.handlers.contextmenu![0]!(e);return e;};
  expect(context('#exact').preventDefault).toHaveBeenCalled();const menu=a.doc.querySelector('[role=menu]')!;
  const source=menu.querySelector('button')!;source.dispatchEvent(new MouseEvent('click'));expect(a.message).not.toHaveBeenCalled(); // Synthetic activation cannot open externally.
  menu.remove();expect(context('#profile').preventDefault).toHaveBeenCalled();a.doc.querySelector('[role=menu]')!.remove();expect(context('#quote').preventDefault).not.toHaveBeenCalled();
});
function openLinkMenu(a:ReturnType<typeof adapter>){
  const link=a.doc.createElement('a');link.href='/creator';a.doc.body.append(link);
  const event={target:link,isTrusted:true,clientX:590,clientY:895,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};
  a.handlers.contextmenu![0]!(event);
  return a.doc.querySelector<HTMLElement>('.atsumi-x-link-menu')!;
}
it('uses the same black surface, spacing and focus treatment for a plain-link context menu',()=>{
  const a=visibleAdapter(),menu=openLinkMenu(a),button=menu.querySelector<HTMLButtonElement>('button')!;
  expect(menu.getAttribute('aria-label')).toBe('링크 메뉴');
  expect(menu.style).toMatchObject({background:'rgb(0, 0, 0)',color:'rgb(231, 233, 234)',borderRadius:'12px',padding:'4px',width:'232px',overflow:'auto'});
  expect(button.style).toMatchObject({display:'block',width:'100%',padding:'12px',textAlign:'left',boxSizing:'border-box'});
  expect(button.textContent).toBe('외부 브라우저에서 열기');
  const css=a.doc.querySelector('style[data-atsumi-x="link-menu"]')!.textContent;
  expect(css).toContain('#16181c');expect(css).toContain('outline:2px solid #1d9bf0');
  expect(menu.style.left).toBe('360px');expect(menu.style.top).toBe('892px');
});
it('revalidates a link menu target after ad reclassification, replacement or disconnection',()=>{
  for(const mode of ['ad','changed','removed','unchanged']){
    const a=visibleAdapter();let handler:EventListener|undefined;
    const original=Element.prototype.addEventListener;
    const spy=vi.spyOn(Element.prototype,'addEventListener').mockImplementation(function(this:Element,type,listener,options){
      if(type==='click'&&this.getAttribute('role')==='menuitem'&&typeof listener==='function')handler=listener;
      return original.call(this,type,listener,options);
    });
    const menu=openLinkMenu(a),link=a.doc.querySelector('a[href="/creator"]')!;
    Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:(node:Element)=>mode==='ad'&&node===link}});
    if(mode==='changed')link.setAttribute('href','/other');
    if(mode==='removed')link.remove();
    handler!.call(menu.querySelector('button')!,{isTrusted:true,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()} as unknown as Event);
    expect(bridgeEntries(a)).toEqual(mode==='unchanged'?[['external','/creator']]:[]);
    spy.mockRestore();
  }
});
it('dismisses both owned menus idempotently through either hook without synthetic clicks or focus changes',()=>{
  const a=visibleAdapter();let actionsOpen=true;
  const dismiss=vi.fn(()=>{const had=actionsOpen;actionsOpen=false;return had;});
  Object.defineProperty(a.win,'__atsumiXActions',{value:{dismissMenus:dismiss}});
  openLinkMenu(a);expect(dismiss).toHaveBeenCalledOnce();
  actionsOpen=true;dismiss.mockClear();a.message.mockClear();
  const clicks=vi.spyOn(a.doc,'dispatchEvent'),active=a.doc.activeElement;
  expect(a.win.__atsumiXSurface!.dismissMenus()).toBe(true);
  expect(dismiss).toHaveBeenCalledOnce();expect(a.doc.querySelector('[role=menu]')).toBeNull();
  expect(a.win.__atsumiXSurface!.dismissMenus()).toBe(false);
  actionsOpen=true;expect(a.win.__atsumiXSurface!.dismissMenu()).toBe(true);
  expect(a.win.__atsumiXSurface!.dismissMenu()).toBe(false);
  expect(a.doc.activeElement).toBe(active);expect(clicks).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('keeps an owned menu across repeated visible paints but closes it when concealed',()=>{
  const a=visibleAdapter(),menu=openLinkMenu(a),dismiss=vi.fn(()=>false);
  Object.defineProperty(a.win,'__atsumiXActions',{value:{visibility:vi.fn(),update:vi.fn(),dismissMenus:dismiss}});
  for(let i=0;i<4;i++){a.win.__atsumiXSurface!.visibility(true);a.flush();}
  expect(menu.isConnected).toBe(true);expect(dismiss).not.toHaveBeenCalled();
  vi.spyOn(a.doc.querySelector('video')!,'pause').mockImplementation(()=>{});
  a.win.__atsumiXSurface!.visibility(false);
  expect(menu.isConnected).toBe(false);expect(dismiss).toHaveBeenCalledOnce();
  a.win.__atsumiXSurface!.visibility(false);expect(dismiss).toHaveBeenCalledTimes(2);
});
it('does not let an untrusted outside pointer dismiss the owned menu',()=>{
  const a=visibleAdapter(),menu=openLinkMenu(a);
  a.handlers.pointerdown![0]!({target:a.doc.body,isTrusted:false,button:0});
  expect(menu.isConnected).toBe(true);
  a.handlers.pointerdown![0]!({target:a.doc.body,isTrusted:true,button:0});
  expect(menu.isConnected).toBe(false);
});
const linklessQuote='<div role="link" id="linkless"><div data-testid="User-Name"><span>Quoted</span><span>@quoted</span><time>date</time></div><p data-testid="tweetText" id="linkless-body">Quoted text</p></div>';
function sourceRouteAdapter(quote=linklessQuote,feed='foryou'){
  const a=adapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>',feed,'x.com',false,true);
  Object.defineProperty(a.doc,'hidden',{value:false,configurable:true});
  Object.defineProperty(a.doc,'scrollingElement',{value:a.doc.documentElement,configurable:true});
  a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();return a;
}
const bridgeEntries=(a:ReturnType<typeof adapter>)=>a.message.mock.calls.map(([raw])=>{const url=new URL(raw.slice('ATSUMI_X_SURFACE_V1:'.length));return [url.searchParams.get('kind'),url.searchParams.get('value')];});
function flushRouteTasks(a:ReturnType<typeof adapter>){for(let i=0;i<20&&a.timers.length;i++)a.flush();expect(a.timers).toHaveLength(0);}
it('uses only the exact navigation requested by a trusted linkless quote activation, once',()=>{
  for(const method of ['pushState','replaceState'] as const){
    const a=sourceRouteAdapter(),before=a.doc.documentElement.outerHTML;
    a.doc.documentElement.scrollTop=240;
    const event=clickTarget(a,a.doc.querySelector('#linkless-body')!);
    expect(event.preventDefault).not.toHaveBeenCalled(); // No replay or fabricated quote href.
    expect(bridgeEntries(a)).toEqual([['routeIntent','arm']]);
    a.win.history![method]({source:'original'},'', '/quoted/status/987654');
    expect(bridgeEntries(a)).toEqual([['routeIntent','arm'],['routeIntent','cancel'],['preview','/quoted/status/987654']]);
    expect(a.location.pathname).toBe('/home');expect(a.location.href).toBe('https://x.com/home');
    expect(a.doc.documentElement.outerHTML).toBe(before);expect(a.doc.documentElement.scrollTop).toBe(240);
    expect(a.sourcePush).not.toHaveBeenCalled();expect(a.sourceReplace).not.toHaveBeenCalled();
    a.win.history![method](null,'','/quoted/status/987655');
    expect(a.location.pathname).toBe('/quoted/status/987655');
    expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toHaveLength(1);
  }
});
it('resynchronizes a source subscriber that mutates its own DOM after history returns, without changing browser history',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector('[data-testid="primaryColumn"]')!,original=primary.innerHTML;
  const state={key:'original',opaque:{mustNotBeSent:'private fixture'}};
  a.win.history!.replaceState(state,'','/home');a.sourceReplace.mockClear();
  a.doc.documentElement.scrollTop=730;a.doc.documentElement.scrollLeft=12;
  const sourcePop=vi.fn((event:Event)=>{
    expect((event as PopStateEvent).state).toBe(state);
    expect(a.location.href).toBe('https://x.com/home');
    primary.innerHTML=original; // Source-owned rendering, not adapter DOM repair.
  });
  a.win.addEventListener('popstate',sourcePop);
  clickTarget(a,a.doc.querySelector('#linkless-body')!);
  const sourceRouter=(path:string)=>{
    a.win.history!.pushState(null,'',path);
    primary.textContent='Independently routed detail';
    a.doc.documentElement.scrollTop=0;a.doc.documentElement.scrollLeft=0;
  };
  sourceRouter('/quoted/status/987654');
  expect(a.location.pathname).toBe('/home');
  expect(bridgeEntries(a).at(-1)).toEqual(['preview','/quoted/status/987654']);
  expect(a.doc.querySelector('article')).toBeNull(); // URL retention alone is insufficient.
  expect(sourcePop).not.toHaveBeenCalled(); // Never dispatch in the original caller's stack.
  flushRouteTasks(a);
  expect(sourcePop).toHaveBeenCalledOnce();expect(a.win.dispatchEvent).toHaveBeenCalledOnce();
  expect(a.doc.querySelector('article')).not.toBeNull();expect(primary.innerHTML).toBe(original);
  expect(a.doc.documentElement.scrollTop).toBe(730);expect(a.doc.documentElement.scrollLeft).toBe(12);
  expect(a.win.history!.state).toBe(state);expect(a.location.href).toBe('https://x.com/home');
  expect(a.sourcePush).not.toHaveBeenCalled();expect(a.sourceReplace).not.toHaveBeenCalled();
  expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([['preview','/quoted/status/987654']]);
  expect(JSON.stringify(a.message.mock.calls)).not.toContain('private fixture');
  expect(a.location.assign).not.toHaveBeenCalled();
});
it('never loops, fabricates DOM, or scrolls detail when the source ignores the one resynchronization event',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector('[data-testid="primaryColumn"]')!;
  a.doc.documentElement.scrollTop=730;
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  primary.textContent='Source did not accept popstate';a.doc.documentElement.scrollTop=0;
  flushRouteTasks(a);
  expect(a.win.dispatchEvent).toHaveBeenCalledOnce();
  expect(primary.textContent).toBe('Source did not accept popstate');expect(a.doc.documentElement.scrollTop).toBe(0);
  expect(a.sourcePush).not.toHaveBeenCalled();expect(a.sourceReplace).not.toHaveBeenCalled();
  expect(a.location.assign).not.toHaveBeenCalled();
});
it('restores the same nested feed scroller only after the source restores its original outer article',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector<HTMLElement>('[data-testid="primaryColumn"]')!;
  primary.style.overflowY='auto';Object.defineProperty(primary,'scrollHeight',{value:2000});Object.defineProperty(primary,'clientHeight',{value:500});
  primary.scrollTop=455;a.doc.documentElement.scrollTop=230;
  const original=primary.innerHTML;
  a.win.addEventListener('popstate',()=>{
    // Emulate an asynchronous source render after its listener returns.
    a.timers.push(()=>{primary.innerHTML=original;});
  });
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  primary.textContent='Pending detail';primary.scrollTop=0;a.doc.documentElement.scrollTop=0;
  flushRouteTasks(a);
  expect(primary.scrollTop).toBe(455);expect(a.doc.documentElement.scrollTop).toBe(230);
  expect(a.win.dispatchEvent).toHaveBeenCalledOnce();
});
it('does not copy saved positions to a replacement scroll container',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector<HTMLElement>('[data-testid="primaryColumn"]')!;
  primary.style.overflowY='auto';Object.defineProperty(primary,'scrollHeight',{value:2000});Object.defineProperty(primary,'clientHeight',{value:500});primary.scrollTop=455;
  const original=primary.outerHTML;
  a.win.addEventListener('popstate',()=>{primary.outerHTML=original;});
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  primary.textContent='Detail';
  flushRouteTasks(a);
  expect(primary.isConnected).toBe(false);
  expect(a.doc.querySelector<HTMLElement>('[data-testid="primaryColumn"]')!.scrollTop).toBe(0);
});
it('does not mistake a hidden or nested copy of the previous article for a restored source feed',()=>{
  for(const wrap of [(html:string)=>'<div hidden>'+html+'</div>',(html:string)=>'<div style="opacity:0">'+html+'</div>',(html:string)=>'<article data-testid="tweet"><p>Destination</p>'+html+'</article>']){
    const a=sourceRouteAdapter(),primary=a.doc.querySelector('[data-testid="primaryColumn"]')!,original=primary.innerHTML;
    a.doc.documentElement.scrollTop=730;
    a.win.addEventListener('popstate',()=>{primary.innerHTML=wrap(original);});
    clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
    primary.textContent='Detail';a.doc.documentElement.scrollTop=0;
    flushRouteTasks(a);
    expect(a.win.dispatchEvent).toHaveBeenCalledOnce();expect(a.doc.documentElement.scrollTop).toBe(0);
  }
});
it('cancels recovery on explicit shell/tab dismissal but preserves it for source-preview reuse menu dismissal',()=>{
  for(const cancel of [true,false,undefined]){
    const a=sourceRouteAdapter();
    clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(a.win.__atsumiXSurface!.dismissMenus(cancel)).toBe(false);
    flushRouteTasks(a);expect(a.win.dispatchEvent).toHaveBeenCalledTimes(cancel===true?0:1);
  }
  const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!);
  a.win.__atsumiXSurface!.dismissMenus(true);a.win.history!.pushState(null,'','/quoted/status/987654');
  expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([]);
  expect(a.sourcePush).toHaveBeenCalledOnce();
});
it('cancels queued resynchronization for newer real input, history, account, visibility or route changes',()=>{
  for(const change of ['pointerdown','keydown','wheel','touchstart','click','history','same-history','popstate','owner','login','hidden','document-hidden','route','expiry']){
    const a=sourceRouteAdapter();let now=1000;vi.spyOn(performance,'now').mockImplementation(()=>now);
    clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
    if(['pointerdown','keydown','wheel','touchstart'].includes(change))a.windowHandlers[change]![0]!({isTrusted:true} as Event);
    if(change==='click')clickTarget(a,a.doc.body);
    if(change==='history')a.win.history!.pushState(null,'','/other/status/222222');
    if(change==='same-history')a.win.history!.replaceState({key:'newer'},'','/home');
    if(change==='popstate')a.windowHandlers.popstate![0]!(new PopStateEvent('popstate'));
    if(change==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
    if(change==='login')a.doc.body.insertAdjacentHTML('beforeend','<input type="password">');
    if(change==='hidden'){vi.spyOn(a.doc.querySelector('video')!,'pause').mockImplementation(()=>{});a.win.__atsumiXSurface!.visibility(false);}
    if(change==='document-hidden')Object.defineProperty(a.doc,'hidden',{value:true,configurable:true});
    if(change==='route'){a.location.pathname='/explore';a.location.href='https://x.com/explore';}
    if(change==='expiry')now=2001;
    flushRouteTasks(a);expect(a.win.dispatchEvent).not.toHaveBeenCalled();vi.restoreAllMocks();
  }
});
it('cancels deferred scroll restoration if real input arrives after source resynchronization',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector('[data-testid="primaryColumn"]')!,original=primary.innerHTML;
  a.doc.documentElement.scrollTop=730;
  a.win.addEventListener('popstate',()=>{
    primary.innerHTML=original;
    a.windowHandlers.wheel![0]!({isTrusted:true} as Event);
    a.doc.documentElement.scrollTop=95;
  });
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  primary.textContent='Detail';a.doc.documentElement.scrollTop=0;
  flushRouteTasks(a);
  expect(a.win.dispatchEvent).toHaveBeenCalledOnce();expect(a.doc.documentElement.scrollTop).toBe(95);
});
it('does not restore positions after a later source render has advanced history during its subscriber',()=>{
  const a=sourceRouteAdapter(),primary=a.doc.querySelector('[data-testid="primaryColumn"]')!,original=primary.innerHTML;
  a.doc.documentElement.scrollTop=730;
  a.win.addEventListener('popstate',()=>{
    primary.innerHTML=original;
    a.win.history!.replaceState({newer:'source-owned'},'','/home');
    a.doc.documentElement.scrollTop=80;
  });
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  primary.textContent='Detail';a.doc.documentElement.scrollTop=0;
  flushRouteTasks(a);
  expect(a.win.dispatchEvent).toHaveBeenCalledOnce();expect(a.doc.documentElement.scrollTop).toBe(80);
  expect(a.win.history!.state).toEqual({newer:'source-owned'});
});
it('ignores untrusted input for recovery fencing and never invokes native source action buttons',()=>{
  const a=sourceRouteAdapter(),buttons=[...a.doc.querySelectorAll('button')].map(button=>vi.spyOn(button,'click'));
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  a.windowHandlers.pointerdown![0]!({isTrusted:false} as Event);
  flushRouteTasks(a);
  expect(a.win.dispatchEvent).toHaveBeenCalledOnce();expect(buttons.every(button=>button.mock.calls.length===0)).toBe(true);
});
it('also observes button-role quote and actual source profile destinations without guessing from visible text',()=>{
  for(const role of ['link','button']){
    const a=sourceRouteAdapter(linklessQuote.replace('role="link"','role="'+role+'"'));
    clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted');
    expect(bridgeEntries(a).at(-1)).toEqual(['preview','/quoted']);
    expect(a.location.pathname).toBe('/home');
  }
});
it('lets source same-location bookkeeping run before its next exact destination',()=>{
  const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!);
  a.win.history!.replaceState({key:'source'},'','/home');
  expect(a.sourceReplace).toHaveBeenCalledOnce();expect(bridgeEntries(a)).toEqual([['routeIntent','arm']]);
  a.win.history!.pushState(null,'','/quoted/status/987654');
  expect(bridgeEntries(a).at(-1)).toEqual(['preview','/quoted/status/987654']);
  expect(a.location.pathname).toBe('/home');
});
it('does not arm routing from synthetic, other modified, secondary, selected-text or prevented activation',()=>{
  for(const extra of [{isTrusted:false},{ctrlKey:true,shiftKey:true},{metaKey:true},{altKey:true},{shiftKey:true},{button:1},{button:2},{defaultPrevented:true}]){
    const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!,extra);
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(a.message).not.toHaveBeenCalled();expect(a.sourcePush).toHaveBeenCalledOnce();
  }
  const a=sourceRouteAdapter();a.win.getSelection=()=>({toString:()=> 'selected text'});
  clickTarget(a,a.doc.querySelector('#linkless-body')!);a.win.history!.pushState(null,'','/quoted/status/987654');
  expect(a.message).not.toHaveBeenCalled();expect(a.sourcePush).toHaveBeenCalledOnce();
});
it('does not arm native controls, media, dialogs, missing or ambiguous quote author scopes',()=>{
  for(const quote of [
    linklessQuote.replace('<p data-testid="tweetText"','<p role="button" data-testid="like"'),
    linklessQuote.replace('<p data-testid="tweetText"','<p data-testid="videoPlayer"'),
    linklessQuote.replace('<p data-testid="tweetText"','<p data-testid="tweetPhoto"'),
    linklessQuote.replace('<p data-testid="tweetText"','<p contenteditable="true" data-testid="tweetText"'),
    '<div role="dialog">'+linklessQuote+'</div>',
    linklessQuote.replace('<span>@quoted</span>',''),
    linklessQuote.replace('<span>@quoted</span>','<span>@quoted</span><span>@other</span>'),
    linklessQuote.replace('<p data-testid="tweetText"','<div data-testid="User-Name"><span>@other</span></div><p data-testid="tweetText"'),
    linklessQuote.replace('id="linkless"','id="linkless" hidden'),
    linklessQuote.replace('id="linkless"','id="linkless" style="opacity:0"'),
  ]){
    const a=sourceRouteAdapter(quote);clickTarget(a,a.doc.querySelector('#linkless-body')!);
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(bridgeEntries(a).filter(([kind])=>kind==='routeIntent'||kind==='preview')).toEqual([]);
    expect(a.sourcePush).toHaveBeenCalledOnce();
  }
});
it('keeps authentication, foreign, reserved, photo/video and noncanonical navigation source-owned',()=>{
  for(const path of ['/login','/i/flow/login','/home?x=1','https://evil.test/quoted/status/987654','https://user@x.com/quoted/status/987654','https://x.com:444/quoted/status/987654','/quoted/status/987654/photo/1','/quoted/status/987654/video/1','/quoted/status/987654?source=feed','/quoted/status/987654#reply','/quoted/status/012345','/settings','/quoted/extra']){
    const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!);
    a.win.history!.pushState(null,'',path);
    expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([]);
    expect(a.sourcePush).toHaveBeenCalledExactlyOnceWith(null,'',path);
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([]);
  }
});
it('revokes route observation on expiry, detach, source route, account or visibility changes',()=>{
  for(const change of ['expiry','timeout','detach','route','owner','login','hidden','author']){
    const a=sourceRouteAdapter();let now=1000;vi.spyOn(performance,'now').mockImplementation(()=>now);
    clickTarget(a,a.doc.querySelector('#linkless-body')!);
    if(change==='expiry')now=2001;
    if(change==='timeout')a.flush();
    if(change==='detach')a.doc.querySelector('#linkless')!.remove();
    if(change==='route')a.location.pathname='/explore';
    if(change==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
    if(change==='login')a.doc.body.insertAdjacentHTML('beforeend','<input type="password">');
    if(change==='hidden'){vi.spyOn(a.doc.querySelector('video')!,'pause').mockImplementation(()=>{});a.win.__atsumiXSurface!.visibility(false);}
    if(change==='author')a.doc.querySelector('#linkless [data-testid="User-Name"]')!.innerHTML='<span>@other</span>';
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([]);
    expect(a.sourcePush).toHaveBeenCalledOnce();vi.restoreAllMocks();
  }
});
it('cancels a pending route intent on a new gesture and leaves preview history untouched',()=>{
  for(const action of ['pointer','click','context','escape']){
    const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!);
    if(action==='pointer')a.handlers.pointerdown![0]!({target:a.doc.body,isTrusted:true,button:0});
    if(action==='click')clickTarget(a,a.doc.body,{ctrlKey:true});
    if(action==='context')a.handlers.contextmenu![0]!({target:a.doc.body,isTrusted:true,clientX:10,clientY:10});
    if(action==='escape')a.handlers.keydown![0]!({target:a.doc.body,key:'Escape',isTrusted:true});
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(bridgeEntries(a).filter(([kind])=>kind==='preview')).toEqual([]);expect(a.sourcePush).toHaveBeenCalledOnce();
  }
  const a=sourceRouteAdapter(linklessQuote,'preview');clickTarget(a,a.doc.querySelector('#linkless-body')!);
  a.win.history!.pushState(null,'','/quoted/status/987654');
  expect(a.message).not.toHaveBeenCalled();expect(a.sourcePush).toHaveBeenCalledOnce();
});
it('opens an exact Ctrl-clicked post or profile in a background preview from either selected pane',()=>{
  for(const feed of ['foryou','likes','bookmarks','preview']){
    const a=visibleAdapter(undefined,feed);
    const link=a.doc.createElement('a');link.href='/creator';a.doc.querySelector('article')!.append(link);
    for(const [target,path] of [[a.doc.querySelector('[data-testid="tweetText"]')!,'/creator/status/123456'],[link,'/creator']] as const){
      a.message.mockClear();const before=a.doc.documentElement.outerHTML;
      expect(clickTarget(a,target,{ctrlKey:true}).preventDefault).toHaveBeenCalledOnce();
      expect(bridgeEntries(a)).toEqual([['focus',''],['previewBackground',path]]);
      expect(a.location.pathname).toBe('/home');expect(a.doc.documentElement.outerHTML).toBe(before);
    }
  }
});
it('binds linkless Ctrl activation only to an observed source-requested exact background destination',()=>{
  for(const feed of ['foryou','preview']){
    const a=sourceRouteAdapter(linklessQuote,feed);
    expect(clickTarget(a,a.doc.querySelector('#linkless-body')!,{ctrlKey:true}).preventDefault).not.toHaveBeenCalled();
    expect(bridgeEntries(a)).toEqual([['focus',''],['routeIntent','armBackground']]);
    a.win.history!.pushState(null,'','/quoted/status/987654');
    expect(bridgeEntries(a)).toEqual([['focus',''],['routeIntent','armBackground'],['routeIntent','cancel'],['previewBackground','/quoted/status/987654']]);
    expect(a.location.pathname).toBe('/home');expect(a.sourcePush).not.toHaveBeenCalled();
  }
});
it('does not synthesize a background destination for a linkless Ctrl click whose source does not navigate',()=>{
  const a=sourceRouteAdapter();clickTarget(a,a.doc.querySelector('#linkless-body')!,{ctrlKey:true});a.flush();
  expect(bridgeEntries(a)).toEqual([['focus',''],['routeIntent','armBackground'],['routeIntent','cancel']]);
  expect(a.location.pathname).toBe('/home');expect(a.sourcePush).not.toHaveBeenCalled();expect(a.location.assign).not.toHaveBeenCalled();
});
it('preserves Ctrl native photo/video/social actions and does not arm unresolved non-quote cards',()=>{
  const a=sourceRouteAdapter();
  for(const target of a.doc.querySelectorAll('button,img,video,[data-testid="tweetPhoto"]'))expect(clickTarget(a,target,{ctrlKey:true}).preventDefault).not.toHaveBeenCalled();
  for(const path of ['/creator/status/123456/photo/1','/creator/status/123456/video/1']){
    const link=a.doc.createElement('a');link.href=path;a.doc.body.append(link);
    expect(clickTarget(a,link,{ctrlKey:true}).preventDefault).not.toHaveBeenCalled();
  }
  const empty=a.doc.createElement('div');empty.setAttribute('role','link');a.doc.querySelector('article')!.append(empty);
  expect(clickTarget(a,empty,{ctrlKey:true}).preventDefault).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('reports real visible native playback and can pause without hiding or blocking the pane',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!,pause=vi.spyOn(video,'pause').mockImplementation(()=>{});
  a.handlers.play![0]!({target:video,isTrusted:true});expect(a.message).toHaveBeenLastCalledWith(expect.stringContaining('kind=play&value=auto'));
  a.message.mockClear();a.win.__atsumiXSurface!.pause();expect(pause).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();
  a.handlers.play![0]!({target:video,isTrusted:true});expect(a.message).toHaveBeenCalledOnce(); // A user can reclaim feed playback while detail remains open.
});
it('rechecks ad ownership before an ad can claim playback while preserving ordinary video intent',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!;
  const ad=a.doc.createElement('video');a.doc.querySelector('article')!.append(ad);
  const pause=vi.spyOn(ad,'pause').mockImplementation(()=>{});
  const ordinaryPause=vi.spyOn(video,'pause').mockImplementation(()=>{});
  Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:(node:Element)=>node===ad,clear:vi.fn(),update:vi.fn()}});
  a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:true,button:0});
  a.handlers.play![0]!({target:ad,isTrusted:true});
  expect(pause).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();
  a.handlers.play![0]!({target:video,isTrusted:true});
  expect(bridgeValues(a)).toEqual(['manual']);expect(ordinaryPause).not.toHaveBeenCalled();
});
it('enables the ad adapter only for a ready visible owner and clears it on concealment or owner loss',()=>{
  const a=adapter(),update=vi.fn(),clear=vi.fn();
  Object.defineProperty(a.win,'__atsumiXAds',{value:{blocked:()=>false,update,clear}});
  a.flush();expect(update).toHaveBeenLastCalledWith(false);
  a.win.__atsumiXSurface!.visibility(true);a.flush();expect(update).toHaveBeenLastCalledWith(true);
  a.win.__atsumiXSurface!.visibility(false);expect(clear).toHaveBeenCalledOnce();
  a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
  a.win.__atsumiXSurface!.visibility(true);a.flush();expect(clear).toHaveBeenCalledTimes(2);
  expect(a.status().state).toBe('ownerChanged');
});
it('a native media gesture can reclaim playback but repeated autoplay cannot ping-pong',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!,pause=vi.spyOn(video,'pause').mockImplementation(()=>{});
  a.win.__atsumiXSurface!.playback(false);a.message.mockClear();
  for(let i=0;i<3;i++)a.handlers.play![0]!({target:video,isTrusted:true});expect(a.message).not.toHaveBeenCalled();expect(pause).toHaveBeenCalledTimes(4);
  a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:true,button:0});
  a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['manual']);
  a.message.mockClear();a.handlers.play![0]!({target:video,isTrusted:true});expect(a.message).not.toHaveBeenCalled(); // Intent is consumed once.
});
it('only recent trusted input on the same native player counts as manual playback',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!;vi.spyOn(video,'pause').mockImplementation(()=>{});
  let now=1000;vi.spyOn(performance,'now').mockImplementation(()=>now);
  a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:false,button:0});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['auto']);
  a.message.mockClear();a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:true,button:0});now+=1501;a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['auto']);
  a.message.mockClear();a.handlers.pointerdown![0]!({type:'pointerdown',target:a.doc.querySelector('[data-testid=tweetText]'),isTrusted:true,button:0});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['auto']);
  a.message.mockClear();a.handlers.keydown![0]!({type:'keydown',target:video,key:' ',isTrusted:true});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['manual']);
});
it('does not treat a context click or the trailing click of one gesture as another manual play',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!;vi.spyOn(video,'pause').mockImplementation(()=>{});
  a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:true,button:2});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['auto']);
  a.message.mockClear();a.handlers.pointerdown![0]!({type:'pointerdown',target:video,isTrusted:true,button:0});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['manual']);
  a.message.mockClear();clickTarget(a,video,{type:'click'});a.handlers.play![0]!({target:video,isTrusted:true});expect(bridgeValues(a)).toEqual(['auto']);
});
it('ignores synthetic play claims and never grants hidden playback',()=>{
  const a=visibleAdapter(),video=a.doc.querySelector('video')!,pause=vi.spyOn(video,'pause').mockImplementation(()=>{});
  a.handlers.play![0]!({target:video,isTrusted:false});expect(a.message).not.toHaveBeenCalled();
  a.win.__atsumiXSurface!.visibility(false);a.handlers.play![0]!({target:video,isTrusted:true});expect(pause).toHaveBeenCalledTimes(2);expect(a.message).not.toHaveBeenCalled();
});
function frameAdapter(src='https://www.youtube.com/embed/abcdefghijk'){
  const a=visibleAdapter();
  const frame=a.doc.createElement('iframe');frame.src=src;a.doc.querySelector('article')!.append(frame);
  Object.defineProperty(a.doc,'activeElement',{configurable:true,value:frame});
  Object.defineProperty(a.doc,'hidden',{configurable:true,value:false});
  vi.spyOn(a.doc,'hasFocus').mockReturnValue(true);
  vi.spyOn(frame,'getBoundingClientRect').mockReturnValue({x:10,y:10,left:10,top:10,right:410,bottom:310,width:400,height:300,toJSON:()=>({})});
  frame.style.visibility='visible';frame.style.opacity='1';
  a.win.navigator.userActivation.isActive=true;
  const blur=a.win.addEventListener.mock.calls.find(([name])=>name==='blur')![1] as (e:{isTrusted:boolean})=>void;
  return {...a,frame,blur};
}
it('grants a visible focused embedded player after a genuine in-frame gesture without changing its settings',()=>{
  for(const src of ['https://www.youtube.com/embed/abcdefghijk','https://www.youtube-nocookie.com/embed/abcdefghijk','https://cards-frame.x.com/cards/tfw/v1/123456','https://cards-frame.twitter.com/cards/tfw/v1/123456']){
    const a=frameAdapter(src),before=a.frame.outerHTML;
    a.blur({isTrusted:true});a.flush();
    expect(a.message.mock.calls.map(([body])=>new URL(body.slice('ATSUMI_X_SURFACE_V1:'.length)).searchParams.get('kind'))).toEqual(['focus','play']);
    expect(a.message).toHaveBeenLastCalledWith(expect.stringContaining('kind=play&value=manual'));
    expect(a.frame.outerHTML).toBe(before);
  }
});
it('rejects synthetic, background, stale or non-gesture frame focus',()=>{
  for(const reason of ['synthetic','activation','background','hidden','privacy','changed','detached','srcdoc','outside','tiny']){
    const a=frameAdapter();
    if(reason==='activation')a.win.navigator.userActivation.isActive=false;
    if(reason==='background')vi.mocked(a.doc.hasFocus).mockReturnValue(false);
    if(reason==='hidden')Object.defineProperty(a.doc,'hidden',{value:true});
    if(reason==='srcdoc')a.frame.setAttribute('srcdoc','<video autoplay></video>');
    if(reason==='outside')a.doc.body.append(a.frame);
    if(reason==='tiny')vi.mocked(a.frame.getBoundingClientRect).mockReturnValue({left:0,top:0,right:1,bottom:1,width:1,height:1} as DOMRect);
    a.blur({isTrusted:reason!=='synthetic'});
    if(reason==='privacy'||reason==='changed')a.win.__atsumiXSurface!.visibility(false);
    if(reason==='changed')a.win.__atsumiXSurface!.visibility(true);
    if(reason==='detached')a.frame.remove();
    a.flush();expect(a.message.mock.calls.filter(([body])=>body.includes('kind=play'))).toHaveLength(0);
  }
});
it('does not claim arbitrary iframe origins, non-player routes or hidden frames',()=>{
  for(const src of ['https://evil.test/embed/abcdefghijk','https://www.youtube.com.evil.test/embed/abcdefghijk','http://www.youtube.com/embed/abcdefghijk','https://user@www.youtube.com/embed/abcdefghijk','https://www.youtube.com:444/embed/abcdefghijk','https://www.youtube.com/watch?v=abcdefghijk','https://cards-frame.x.com/login','about:blank']){
    const a=frameAdapter(src);a.blur({isTrusted:true});a.flush();expect(a.message).not.toHaveBeenCalled();
  }
  const a=frameAdapter();a.frame.setAttribute('hidden','');a.blur({isTrusted:true});a.flush();expect(a.message).not.toHaveBeenCalled();
});
it('does not claim a player hidden or completely clipped by an ancestor',()=>{
  // jsdom does not expand overflow shorthand into its computed axis values.
  for(const style of ['opacity:0','display:none','visibility:hidden','overflow-x:hidden;overflow-y:hidden']){
    const a=frameAdapter(),wrapper=a.doc.createElement('div');a.frame.replaceWith(wrapper);wrapper.append(a.frame);wrapper.style.cssText=style;
    vi.spyOn(wrapper,'getBoundingClientRect').mockReturnValue({left:500,top:500,right:550,bottom:550,width:50,height:50} as DOMRect);
    a.blur({isTrusted:true});a.flush();expect(a.message,style).not.toHaveBeenCalled();
  }
});
it('hides only a home global composer while preserving account home tabs, thread replies and dialogs',()=>{
  const composer='<div id="composer"><div data-testid="tweetTextarea_0" contenteditable="true"></div><button data-testid="tweetButtonInline">Post</button></div>';
  const a=adapter(identity+'<div data-testid="primaryColumn"><div role="tablist"><button role="tab">For you</button><button role="tab">Community A</button></div>'+composer+'<article data-testid="tweet">'+composer.replace('id="composer"','id="reply"')+'</article><div role="dialog">'+composer.replace('id="composer"','id="dialog-composer"')+'</div></div>');
  a.flush();expect(a.doc.querySelector('#composer')?.getAttribute('data-atsumi-x-composer')).toBe('true');
  for(const selector of ['#reply','#dialog-composer','[role=tablist]'])expect(a.doc.querySelector(selector)?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  a.location.pathname='/creator/status/123456';a.update();expect(a.doc.querySelector('#composer')?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  const detail=adapter(identity+'<div data-testid="primaryColumn">'+composer+'</div>','preview');detail.flush();expect(detail.doc.querySelector('#composer')?.hasAttribute('data-atsumi-x-composer')).toBe(false);
});
it('recognizes a standalone home editor with either submit variant or toolbar without needing a form',()=>{
  for(const controls of ['<button data-testid="tweetButton">Post</button>','<button data-testid="tweetButtonInline">Post</button>','<div role="toolbar"><button>Add photo</button></div>']) {
    const a=adapter(identity+'<main role="main"><div data-testid="primaryColumn"><div role="tablist"><button role="tab">For you</button></div><div id="composer"><a href="/alice">Avatar</a><div><div data-testid="tweetTextarea_0" contenteditable="true"></div><div>'+controls+'</div></div></div><section role="region">'+post+'</section></div></main>');
    a.flush();expect(a.doc.querySelector('#composer')?.getAttribute('data-atsumi-x-composer')).toBe('true');
    for(const selector of ['[data-testid=primaryColumn]','[role=tablist]','[role=region]','article'])expect(a.doc.querySelector(selector)?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  }
});
it('does not hide a timeline, heading or unidentifiable editor-only ancestor as a composer',()=>{
  for(const wrapper of ['<div role="region">','<section>']) {
    const a=adapter(identity+'<div data-testid="primaryColumn">'+wrapper+'<h2>Timeline</h2><div data-testid="tweetTextarea_0"></div><div role="toolbar">Toolbar</div>'+post+'</'+(wrapper.startsWith('<section')?'section':'div')+'></div>');a.flush();
    expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
  }
  const a=adapter(identity+'<div data-testid="primaryColumn"><div data-testid="tweetTextarea_0"></div></div>');a.flush();expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
});
it('accepts button-based composer tool tablists without hiding the real navigation tab strip',()=>{
  const tools='<div id="tools" role="tablist"><button aria-label="Add photos">Photo</button><div role="button" tabindex="0">GIF</div></div>';
  const composer='<div id="composer"><div data-testid="tweetTextarea_0" contenteditable="true"></div>'+tools+'<button data-testid="tweetButtonInline">Post</button></div>';
  const a=adapter(identity+'<div data-testid="primaryColumn"><div id="navigation"><div id="tabs" role="tablist"><button role="tab" aria-selected="true">For you</button><button role="tab">Following</button></div>'+composer+'</div>'+post+'</div>');
  const controls=[...a.doc.querySelectorAll('button,[role=button]')].map(button=>vi.spyOn(button as HTMLElement,'click'));
  a.flush();expect(a.doc.querySelector('#composer')?.getAttribute('data-atsumi-x-composer')).toBe('true');
  for(const selector of ['#navigation','#tabs','[role=tab]','article','[data-testid=primaryColumn]'])expect(a.doc.querySelector(selector)?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  expect(controls.every(click=>click.mock.calls.length===0)).toBe(true);
});
it('rejects empty, navigation-link, tab-panel and actual-tab containers as composer toolbars',()=>{
  for(const content of ['', '<a href="/home">Home</a><button>More</button>', '<div role="link">Home</div><button>More</button>', '<button role="tab">For you</button>', '<button>More</button><div role="tabpanel">Panel</div>']) {
    const a=adapter(identity+'<div data-testid="primaryColumn"><div id="ambiguous"><div data-testid="tweetTextarea_0"></div><div role="tablist">'+content+'</div><button data-testid="tweetButtonInline">Post</button></div></div>');
    a.flush();expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
  }
});
it('accepts a button-only media navigation wrapper only inside an identified home composer',()=>{
  for(const nav of ['nav','div role="navigation"']) {
    const tools='<'+nav+' id="tools"><div role="tablist"><button aria-label="Add photos">Photo</button><button>GIF</button></div></'+nav.split(' ')[0]+'>';
    const composer='<div id="composer"><div data-testid="tweetTextarea_0"></div>'+tools+'<button data-testid="tweetButtonInline">Post</button></div>';
    const a=adapter(identity+'<div data-testid="primaryColumn"><div id="outer"><nav id="source-nav"><a href="/home">Home</a><a href="/i/bookmarks">Bookmarks</a></nav><div role="tablist"><button role="tab" aria-selected="true">For you</button></div>'+composer+'</div>'+post+'</div>');
    a.flush();expect(a.doc.querySelector('#composer')?.getAttribute('data-atsumi-x-composer')).toBe('true');
    for(const selector of ['#outer','#source-nav','[role=tab]','article'])expect(a.doc.querySelector(selector)?.hasAttribute('data-atsumi-x-composer')).toBe(false);
  }
});
it('never absorbs real or ambiguous navigation inside a purported media-tools wrapper',()=>{
  for(const content of ['', '<button role="tab">Following</button>', '<button>Photo</button><a href="/home">Home</a>', '<button>Photo</button><div role="link">Profile</div>', '<button>Photo</button><div data-testid="tweetTextarea_0"></div>', '<button>Photo</button><h2>For you</h2>', '<button>Photo</button><div role="tabpanel">Panel</div>']) {
    const a=adapter(identity+'<div data-testid="primaryColumn"><div id="ambiguous"><div data-testid="tweetTextarea_0"></div><nav>'+content+'</nav><button data-testid="tweetButtonInline">Post</button></div></div>');
    a.flush();expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
  }
});
it('keeps native navigation containers and non-home media-tool editors out of composer trimming',()=>{
  const editor='<div data-testid="tweetTextarea_0"></div><div role="tablist"><button>Photo</button><button>GIF</button></div><button data-testid="tweetButtonInline">Post</button>';
  for(const wrapper of ['nav','div role="navigation"','div role="tablist"','div role="tab"']) {
    const a=adapter(identity+'<div data-testid="primaryColumn"><'+wrapper+'>'+editor+'</'+wrapper.split(' ')[0]+'></div>');
    a.flush();expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
  }
  const a=adapter(identity+'<div data-testid="primaryColumn"><div id="composer">'+editor+'</div></div>');a.flush();
  expect(a.doc.querySelector('#composer')?.hasAttribute('data-atsumi-x-composer')).toBe(true);
  a.location.pathname='/creator/status/123456';a.update();expect(a.doc.querySelector('[data-atsumi-x-composer]')).toBeNull();
});
it('bounds the native X column without replacing native media or changing its ratio',()=>{
  const a=adapter();a.flush();const css=a.doc.querySelector('style')!.textContent!;
  expect(css).toContain('[data-testid="primaryColumn"]{width:100%!important;max-width:600px!important;min-width:0!important;margin:0 auto!important');
  expect(css).not.toMatch(/video\s*\{|object-fit|aspect-ratio|height:\d+px/);
});
it('reports only bounded readiness booleans separately from source content',()=>{
  const a=adapter();a.flush();expect(a.status().proof).toMatchObject({profile:true,account:true,primary:true,photo:false,login:false});
  a.doc.querySelector('[data-testid=primaryColumn]')!.remove();a.update();expect(a.status()).toMatchObject({state:'loading',proof:{primary:false}});
});
it('diagnoses exact and alternate photo routes without exposing path or source content',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();a.doc.body.innerHTML=nativePhoto+'<p>Private fixture body</p>';
  a.location.pathname='/creator/status/123456/photo/1';a.update();
  expect(a.status().proof.photoShape).toEqual({route:'photo',segments:5,dialogs:1,dialogImages:1,photoCandidates:0,dialogImageHost:true,dialogImageMedia:true,pageImageMedia:true,hiddenDialog:false,securityInput:false});
  const shape=JSON.stringify(a.status().proof.photoShape);expect(shape).not.toMatch(/creator|123456|PhotoFixture|Private|https|pbs\.twimg/);
  a.location.pathname='/creator/status/123456/photo/9';a.update();expect(a.status()).toMatchObject({state:'loading',proof:{photoShape:{route:'photoOther'}}});
  a.location.pathname='/creator/status/123456/photo/1';a.doc.body.innerHTML='';a.update();expect(a.status()).toMatchObject({state:'loading',proof:{photoShape:{route:'photo',dialogs:0,dialogImages:0,dialogImageHost:false,pageImageMedia:false}}});
});
it('bounds diagnostic counts and distinguishes host, image path, hidden and security shapes',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();a.doc.body.innerHTML='<div role="dialog" aria-hidden="true"><input type="password"><img src="https://pbs.twimg.com/profile_images/Fixture">'+nativePhoto.repeat(40)+'</div>';
  a.location.pathname='/creator/status/123456/photo/1';a.update();
  const shape=a.status().proof.photoShape;expect(shape).toMatchObject({dialogs:8,dialogImages:32,dialogImageHost:true,dialogImageMedia:true,pageImageMedia:true,hiddenDialog:true,securityInput:true});
  expect(a.status().state).toBe('loading');
});
it('omits photo shape in release configuration without changing the readiness decision',()=>{
  for(const diagnostics of [true,false]){
    const a=adapter(undefined,'foryou','x.com',diagnostics);a.flush();a.doc.body.innerHTML=nativePhoto;a.location.pathname='/creator/status/123456/photo/1';a.update();
    expect(a.status().state).toBe('ready');expect(!!a.status().proof.photoShape).toBe(diagnostics);
  }
});
it('diagnoses only bounded own-switcher structure without putting labels, names or URLs in debug shape',()=>{
  const account='<header role="banner" style="display:none"><button data-testid="SideNav_AccountSwitcher_Button" aria-label="Private account label"><div data-testid="UserAvatar-Container-alice" style="background-image:url(https://pbs.twimg.com/profile_images/123456/PrivateAvatar.jpg)"><img src="https://pbs.twimg.com/profile_images/123456/PrivateAvatar.jpg"></div><span dir="auto">Private Owner Name</span><span dir="auto">@alice</span></button></header>';
  const a=adapter(identity.replace('<button data-testid="SideNav_AccountSwitcher_Button"></button>',account)+'<main><div data-testid="primaryColumn">'+post+'</div></main>','foryou','x.com',true);
  a.flush();
  const shape=a.status().proof.accountShape;
  expect(shape).toMatchObject({images:1,imageSources:1,loadedImages:0,backgrounds:1,autoTextNodes:2,ariaLabel:true,avatarContainers:1,hidden:true,hasBox:false,narrow:true});
  expect(shape.textLength).toBe('Private Owner Name@alice'.length);
  expect(Object.values(shape).every(value=>typeof value==='number'||typeof value==='boolean')).toBe(true);
  expect(JSON.stringify(shape)).not.toMatch(/Private|alice|123456|https|pbs\.twimg|jpg/);
  // Presentation still has its separately validated identity path; shape is
  // never a replacement name/avatar source or readiness proof.
  expect(a.status()).toMatchObject({state:'ready',accountPresentation:{handle:'alice',name:'Private Owner Name',avatar:'https://pbs.twimg.com/profile_images/123456/PrivateAvatar.jpg'}});
});
it('caps switcher scans and does not borrow feed-author images for an empty account control',()=>{
  const a=adapter(undefined,'foryou','x.com',true);a.flush();
  expect(a.status().proof.accountShape).toMatchObject({nodes:1,images:0,imageSources:0,backgrounds:0,textLength:0,autoTextNodes:0,ariaLabel:false,avatarContainers:0});
  const account=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')!;
  account.innerHTML=('<div dir="auto" data-testid="UserAvatar-Container-fixture" style="background-image:url(https://example.invalid/Private)"><img src="https://example.invalid/Private">'+('x'.repeat(50))+'</div>').repeat(150);
  a.update();
  expect(a.status().proof.accountShape).toMatchObject({nodes:128,images:32,imageSources:32,backgrounds:32,textLength:512,autoTextNodes:32,avatarContainers:8});
  account.remove();a.update();
  expect(a.status().proof.accountShape).toMatchObject({nodes:0,images:0,imageSources:0,loadedImages:0,backgrounds:0,textLength:0,autoTextNodes:0,ariaLabel:false,avatarContainers:0,hidden:false,hasBox:false});
});
it('does not compute or emit account structure diagnostics in release configuration',()=>{
  const a=adapter(undefined,'foryou','x.com',false);
  const walk=vi.spyOn(a.doc,'createTreeWalker');
  a.flush();
  expect(a.status().proof.accountShape).toBeUndefined();
  expect(walk.mock.calls.filter(([,whatToShow])=>whatToShow===1)).toHaveLength(0);
  expect(a.status().state).toBe('ready');
});
it('re-observes late own-avatar attributes and name text without navigation, polling or duplicate status',()=>{
  const html=identity.replace('SideNav_AccountSwitcher_Button"></button>','SideNav_AccountSwitcher_Button"><img><span dir="auto"></span></button>')+'<div data-testid="primaryColumn">'+post+'</div>';
  const a=adapter(html);a.flush();
  const account=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')!;
  const observer=a.observations.find(value=>value.target===account)!;
  expect(observer.options).toEqual({subtree:true,attributes:true,attributeFilter:['src','srcset','sizes','dir'],characterData:true});
  expect(a.status().accountPresentation).toEqual({handle:'alice',name:'',avatar:null});
  a.message.mockClear();
  account.querySelector('img')!.setAttribute('src','https://pbs.twimg.com/profile_images/123456/Actual.jpg');
  account.querySelector('span')!.textContent='Observed name';
  for(let i=0;i<10;i++)observer.notify();
  expect(a.timers).toHaveLength(1);a.flush();
  expect(a.status().accountPresentation).toEqual({handle:'alice',name:'Observed name',avatar:'https://pbs.twimg.com/profile_images/123456/Actual.jpg',avatarCandidates:[{url:'https://pbs.twimg.com/profile_images/123456/Actual.jpg',width:0}]});
  expect(a.message).toHaveBeenCalledOnce();expect(a.timers).toHaveLength(0);expect(a.location.assign).not.toHaveBeenCalled();
  observer.notify();a.flush();expect(a.message).toHaveBeenCalledOnce();
});
it('handles own-image load/currentSrc completion but not feed image loads or injected feed controls',()=>{
  const html=identity.replace('SideNav_AccountSwitcher_Button"></button>','SideNav_AccountSwitcher_Button"><img></button>')+'<div data-testid="primaryColumn">'+post+'</div>';
  const a=adapter(html);a.flush();a.message.mockClear();
  const account=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')!,image=account.querySelector('img')!;
  a.doc.querySelector('article img')!.dispatchEvent(new Event('load'));
  expect(a.timers).toHaveLength(0);
  expect(a.observations.filter(value=>value.options?.attributes).map(value=>value.target)).toEqual([account]);
  vi.spyOn(image,'currentSrc','get').mockReturnValue('https://pbs.twimg.com/profile_images/123456/Loaded.jpg');
  image.dispatchEvent(new Event('load'));expect(a.timers).toHaveLength(1);a.flush();
  expect(a.status().accountPresentation.avatar).toBe('https://pbs.twimg.com/profile_images/123456/Loaded.jpg');
  expect(a.message).toHaveBeenCalledOnce();expect(a.timers).toHaveLength(0);
});
it('collects bounded existing own-avatar renditions without synthesizing larger URLs',()=>{
  const mini='https://pbs.twimg.com/profile_images/123456/Owner_mini.jpg',large='https://pbs.twimg.com/profile_images/123456/Owner_400x400.jpg';
  const a=adapter(identity.replace('SideNav_AccountSwitcher_Button"></button>',`SideNav_AccountSwitcher_Button"><img src="${mini}" srcset="${mini} 24w, ${large} 400w"></button>`));
  a.flush();
  expect(a.status().accountPresentation).toMatchObject({avatar:mini,avatarCandidates:[{url:mini,width:24},{url:large,width:400}]});
  const img=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"] img')!;
  img.removeAttribute('srcset');a.update();
  expect(a.status().accountPresentation.avatarCandidates).toEqual([{url:mini,width:0}]);
  expect(JSON.stringify(a.status().accountPresentation)).not.toContain('_400x400');
  expect(a.location.assign).not.toHaveBeenCalled();
});
it('keeps currentSrc and src as separate observed candidates and includes decoded dimensions only for the current image',()=>{
  const a=adapter(identity.replace('SideNav_AccountSwitcher_Button"></button>','SideNav_AccountSwitcher_Button"><img src="https://pbs.twimg.com/profile_images/123456/Owner_normal.jpg" srcset="https://pbs.twimg.com/profile_images/123456/Owner_400x400.jpg 2x"></button>'));
  const img=a.doc.querySelector('img')!;
  vi.spyOn(img,'currentSrc','get').mockReturnValue('https://pbs.twimg.com/profile_images/123456/Owner_mini.jpg');
  vi.spyOn(img,'naturalWidth','get').mockReturnValue(24);vi.spyOn(img,'naturalHeight','get').mockReturnValue(24);
  a.flush();
  expect(a.status().accountPresentation.avatarCandidates).toEqual([
    {url:'https://pbs.twimg.com/profile_images/123456/Owner_mini.jpg',width:24},
    {url:'https://pbs.twimg.com/profile_images/123456/Owner_normal.jpg',width:0},
    {url:'https://pbs.twimg.com/profile_images/123456/Owner_400x400.jpg',width:0},
  ]);
});
it('does not borrow feed avatars and bounds malformed or excessive account srcsets',()=>{
  const a=adapter(identity.replace('SideNav_AccountSwitcher_Button"></button>','SideNav_AccountSwitcher_Button"><img src="https://pbs.twimg.com/profile_images/123456/Owner_mini.jpg"></button>')+post);
  const img=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"] img')!;
  img.setAttribute('srcset',Array.from({length:20},(_,i)=>`https://pbs.twimg.com/profile_images/123456/Owner_${i}.jpg ${i+1}w`).join(','));a.flush();
  expect(a.status().accountPresentation.avatarCandidates).toHaveLength(8);
  expect(JSON.stringify(a.status().accountPresentation)).not.toContain('/media/Fixture');
  img.setAttribute('srcset','https://pbs.twimg.com/profile_images/123456/Owner_400x400.jpg Infinityw, javascript:evil 400w, https://pbs.twimg.com/profile_images/123456/Owner_400x400.jpg 99999w');a.update();
  expect(a.status().accountPresentation.avatarCandidates).toHaveLength(1);
  img.setAttribute('srcset','x'.repeat(4097));a.update();expect(a.status().accountPresentation.avatarCandidates).toHaveLength(1);
  a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');a.update();
  expect(a.status().accountPresentation).toBeNull();
});
it('trims only optional avatar candidates to preserve the native status-envelope limit',()=>{
  const mini='https://pbs.twimg.com/profile_images/123456/Owner_mini.jpg';
  const a=adapter(identity.replace('SideNav_AccountSwitcher_Button"></button>',`SideNav_AccountSwitcher_Button"><img src="${mini}"></button>`)+`<main data-testid="primaryColumn">${post}</main>`,'foryou','x.com',true);
  const img=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"] img')!;
  img.setAttribute('srcset',Array.from({length:8},(_,i)=>`https://pbs.twimg.com/profile_images/123456/${'A'.repeat(400)}${i}.jpg ${100+i}w`).join(','));
  a.flush();
  const packet=a.message.mock.calls.at(-1)![0] as string;
  expect(packet.slice('ATSUMI_X_SURFACE_V1:'.length).length).toBeLessThanOrEqual(4096);
  expect(a.status()).toMatchObject({state:'ready',accountPresentation:{handle:'alice',avatar:mini},proof:{profile:true,account:true}});
  expect(a.status().accountPresentation.avatarCandidates.length).toBeLessThan(8);
  expect(a.status().accountPresentation.avatarCandidates.length).toBeGreaterThan(0);
  a.update();a.message.mockClear();
  a.observations.find(value=>value.target===img.parentElement)!.notify();a.flush();expect(a.message).not.toHaveBeenCalled();
});
it('revokes the own-account observer for owner conflicts, login, or replacement and ignores detached old callbacks',()=>{
  for(const changed of ['owner','login','replacement']){
    const html=identity.replace('SideNav_AccountSwitcher_Button"></button>','SideNav_AccountSwitcher_Button"><img></button>')+'<div data-testid="primaryColumn"></div>';
    const a=adapter(html);a.flush();
    const account=a.doc.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')!;
    const observer=a.observations.find(value=>value.target===account)!;
    if(changed==='owner')a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
    else if(changed==='login')a.location.pathname='/i/flow/login';
    else account.replaceWith(account.cloneNode(true));
    a.update();expect(observer.active).toBe(false);a.message.mockClear();
    observer.notify();account.querySelector('img')!.dispatchEvent(new Event('load'));
    expect(a.timers).toHaveLength(0);expect(a.message).not.toHaveBeenCalled();
    if(changed==='replacement')expect(a.observations.filter(value=>value.active&&value.options?.attributes)).toHaveLength(1);
    else expect(a.observations.filter(value=>value.active&&value.options?.attributes)).toHaveLength(0);
  }
});
it('does not observe account-looking controls unless the current profile proof matches',()=>{
  for(const html of [identity.replace('/alice','/bob'),identity.replace('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a>',''),'<article data-testid="tweet"><button data-testid="SideNav_AccountSwitcher_Button"><img></button></article>']){
    const a=adapter(html+'<div data-testid="primaryColumn"></div>');a.flush();
    expect(a.observations.some(value=>value.active&&value.options?.attributes)).toBe(false);
  }
});
it('emits only bounded quote failure structure for an eligible trusted body click without routing it',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><a href="/quoted">Private Name</a><time>date</time></div><p id="quoted-body">Private body</p></div>';
  const a=adapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>','foryou','x.com',true);a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
  const event=clickTarget(a,a.doc.querySelector('#quoted-body')!);expect(event.preventDefault).not.toHaveBeenCalled();
  const envelope=new URL(a.message.mock.calls[0]![0].slice('ATSUMI_X_SURFACE_V1:'.length));expect(envelope.searchParams.get('kind')).toBe('routeProbe');
  const shape=JSON.parse(envelope.searchParams.get('value')!);
  expect(shape).toMatchObject({reason:'missingPath',role:'link',hasTweetArticle:true,nested:true,userNames:1,ownUserNames:1,anchors:1,times:1,timeLinks:0,mediaLinks:0,uniqueMedia:false,authorMatch:false,targetAnchor:false,scopedResolved:false});
  expect(JSON.stringify(shape)).not.toMatch(/Private|quoted|creator|123456|href|https/);
});
it('does not emit routing probes for synthetic, hidden, other modified, native controls or release clicks',()=>{
  const quote='<div role="button"><p id="quote">Quote without observable identity</p></div>';
  for(const diagnostics of [false,true]){
    const a=adapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>','foryou','x.com',diagnostics);a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
    for(const extra of [{isTrusted:false},{ctrlKey:true,shiftKey:true},{button:2}])clickTarget(a,a.doc.querySelector('#quote')!,extra);
    for(const target of a.doc.querySelectorAll('button,img,video'))clickTarget(a,target);
    expect(a.message).not.toHaveBeenCalled();
    if(!diagnostics){clickTarget(a,a.doc.querySelector('#quote')!);expect(a.message).not.toHaveBeenCalled();}
    a.win.__atsumiXSurface!.visibility(false);clickTarget(a,a.doc.querySelector('#quote')!);expect(a.message).not.toHaveBeenCalled();
  }
});
it('caps diagnostic-only quote probes and scope counts without suppressing native navigation',()=>{
  const users='<div data-testid="User-Name"><a href="/quoted">Name</a><time>date</time></div>'.repeat(40);
  const links='<a href="/quoted">Name</a>'.repeat(140);
  const quote='<div role="button" data-testid="quoteTweet">'+users+links+'<p id="quote">Body</p></div>';
  const a=adapter(identity+'<div data-testid="primaryColumn">'+post.replace('</article>',quote+'</article>')+'</div>','foryou','x.com',true);a.flush();a.win.__atsumiXSurface!.visibility(true);a.flush();a.message.mockClear();
  for(let i=0;i<70;i++)expect(clickTarget(a,a.doc.querySelector('#quote')!).preventDefault).not.toHaveBeenCalled();
  expect(a.message).toHaveBeenCalledTimes(64);
  const shape=JSON.parse(bridgeValues(a)[0]!);
  expect(shape).toMatchObject({reason:'missingScope',role:'quoteMarker',hasTweetArticle:true,nested:true,userNames:32,ownUserNames:32,anchors:128,times:32,timeLinks:0,mediaLinks:0,uniqueMedia:false,authorMatch:false});
  expect(a.location.assign).not.toHaveBeenCalled();
});
