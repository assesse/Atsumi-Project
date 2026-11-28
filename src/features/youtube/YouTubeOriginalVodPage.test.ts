// @ts-expect-error Node-only isolated script test.
import { runInNewContext } from 'node:vm';
import { expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/youtube/original_vod_page.js?raw';
function setup(origin='https://www.youtube.com',pathname='/watch'){
 const messages:Record<string,unknown>[]=[],listeners=new Map<string,Array<(e:Record<string,unknown>)=>void>>(),styles=new Map<string,unknown>();
 const response={videoDetails:{videoId:'abcdefghijk',lengthSeconds:'100'},playabilityStatus:{status:'OK'}},video={readyState:4,videoWidth:1280,ended:false,currentTime:15,duration:100,playbackRate:1,paused:false,seeking:false,currentSrc:'blob:selected',addEventListener:vi.fn(),removeEventListener:vi.fn()},resize=vi.fn();
 let cancel:ReturnType<typeof cancelButton>|null=null,mutation=()=>{};
 const buttons:HTMLElement[]=[],chrome={style:{display:'block',visibility:'visible',opacity:'1'}};
 const regions={menus:[] as unknown[],ends:[] as unknown[]};
 const player={querySelector:(selector:string)=>selector==='video'?video:selector==='.ytp-autonav-endscreen-upnext-cancel-button'?cancel:selector==='.ytp-chrome-bottom'?chrome:buttons.find(button=>button.matches(selector))??null,querySelectorAll:(selector:string)=>selector==='.ytp-popup,.ytp-contextmenu'?regions.menus:regions.ends,getPlayerResponse:()=>response,classList:{contains:vi.fn(()=>false)},style:{display:'block',visibility:'visible',opacity:'1'},parentElement:null as HTMLElement|null,getBoundingClientRect:()=>({width:640,height:360}),appendChild:(button:HTMLElement)=>buttons.push(button),seekTo:vi.fn(),playVideo:vi.fn(),pauseVideo:vi.fn(),getAvailablePlaybackRates:vi.fn(()=>[.25,.5,1,1.5,2]),setPlaybackRate:vi.fn()};
 const root={style:{setProperty:vi.fn()},appendChild:(style:{id:string})=>styles.set(style.id,style)};
 const realDocument=globalThis.document;
 const document={hidden:false,documentElement:root,head:root,getElementById:(id:string)=>id==='movie_player'?player:styles.get(id),createElement:(tag:string)=>tag==='style'?{id:'',textContent:''}:realDocument.createElement(tag),createElementNS:(ns:string,tag:string)=>realDocument.createElementNS(ns,tag),querySelector:()=>null,addEventListener:(name:string,fn:(e:Record<string,unknown>)=>void)=>listeners.set(name,[...(listeners.get(name)??[]),fn])};
 const window:Record<string,unknown>={document,chrome:{webview:{postMessage:(value:string)=>messages.push(JSON.parse(value.replace('ATSUMI_YOUTUBE_VOD_V1:','')))}},ytcfg:{get:()=>true},getComputedStyle:(element:{style:unknown})=>element.style,dispatchEvent:resize,addEventListener:(name:string,fn:(e:Record<string,unknown>)=>void)=>listeners.set(name,[...(listeners.get(name)??[]),fn])};window.top=window;
 const location={origin,pathname,search:'?v=abcdefghijk'},disconnect=vi.fn();
 const intervals:Array<()=>void>=[];let now=1000;
 runInNewContext(script,{window,document,location,URLSearchParams,Date:{now:()=>now},MutationObserver:class{constructor(fn:()=>void){mutation=fn;}observe=vi.fn();disconnect=disconnect;},crypto:{randomUUID:()=> '11111111-1111-4111-8111-111111111111'},setInterval:(fn:()=>void)=>{intervals.push(fn);return intervals.length;},clearInterval:vi.fn(),setTimeout:vi.fn(()=>1),clearTimeout:vi.fn(),Event:class{constructor(public type:string){}}});
 type Bridge={configure(v:{epoch:number;videoId:string;fullscreen?:boolean}):boolean;layout(v:unknown):void;status():Record<string,unknown>;control(v:Record<string,unknown>):{accepted:boolean};controls(v:unknown):boolean;beginFrameSample(v:unknown):boolean;takeFrameSample(id:string,cancel?:boolean):{pending?:boolean;error?:string;frames?:Array<{pts90k:number;gray:number[]}>}|null};
 return{bridge:window.__atsumiYouTubeVod as Bridge,video,response,messages,resize,location,document,player,chrome,buttons,regions,disconnect,mutation:()=>mutation(),setCancel:(button:ReturnType<typeof cancelButton>)=>{cancel=button;},styleText:()=>[...styles.values()].map(style=>(style as {textContent:string}).textContent).join('\n'),tick:()=>{now+=250;intervals.forEach(fn=>fn());},dispatch:(name:string,patch:Record<string,unknown>)=>{const event={isTrusted:true,button:0,key:'',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...patch};listeners.get(name)?.forEach(fn=>fn(event));return event;}};
}
function cancelButton(){return{disabled:false,getAttribute:vi.fn(()=>null as string|null),closest:vi.fn(()=>null as object|null),getClientRects:vi.fn(()=>[{}]),style:{display:'block',visibility:'visible',opacity:'1'},click:vi.fn()};}
it('installs only on the exact official watch page and never adds media or account extraction',()=>{
 expect(setup('https://www.youtube.com.evil.test').bridge).toBeUndefined();expect(setup('https://www.youtube.com','/live_chat').bridge).toBeUndefined();
 const s=setup();expect(s.bridge.configure({epoch:1,videoId:'other123456'})).toBe(false);expect(s.messages).toHaveLength(0);expect(s.bridge.configure({epoch:1,videoId:'abcdefghijk'})).toBe(true);
 for(const forbidden of ['fetch(', 'XMLHttpRequest','document.cookie','localStorage','captureStream','encoded_begin','.play(','.pause(','location.href =','setAutonavState(','.setAutonav('])expect(script).not.toContain(forbidden);
 expect(s.bridge).not.toHaveProperty('start');expect(s.bridge).not.toHaveProperty('stop');
});
it('cancels only the visible original one-shot countdown once per selected natural EOF',()=>{
 const s=setup(),cancel=cancelButton();s.setCancel(cancel);s.video.ended=true;s.bridge.status();expect(cancel.click).not.toHaveBeenCalled();
 s.video.ended=false;s.bridge.configure({epoch:1,videoId:'abcdefghijk'});s.tick();s.mutation();expect(cancel.click).not.toHaveBeenCalled();
 s.video.ended=true;s.dispatch('ended',{});s.tick();s.mutation();expect(cancel.click).toHaveBeenCalledOnce();expect(s.bridge.status().state).toBe('ended');
 s.video.ended=false;s.dispatch('seeked',{});expect(s.bridge.status().state).toBe('playing');s.video.ended=true;s.dispatch('ended',{});expect(cancel.click).toHaveBeenCalledTimes(2);
 s.video.ended=false;s.dispatch('playing',{});s.video.ended=true;s.dispatch('ended',{});expect(cancel.click).toHaveBeenCalledTimes(3);
});
it('handles late original cancel insertion but ignores hidden disabled ad and foreign controls',()=>{
 const s=setup();s.bridge.configure({epoch:1,videoId:'abcdefghijk'});s.video.ended=true;s.dispatch('ended',{});const cancel=cancelButton();s.setCancel(cancel);
 cancel.disabled=true;s.mutation();cancel.disabled=false;cancel.getAttribute.mockReturnValue('true');s.tick();cancel.getAttribute.mockReturnValue(null);
 cancel.closest.mockReturnValue({});s.tick();cancel.closest.mockReturnValue(null);cancel.getClientRects.mockReturnValue([]);s.tick();cancel.getClientRects.mockReturnValue([{}]);
 for(const field of ['display','visibility','opacity'] as const){const previous=cancel.style[field];cancel.style[field]=field==='display'?'none':field==='visibility'?'hidden':'0';s.tick();cancel.style[field]=previous;}
 s.player.classList.contains.mockReturnValue(true);s.tick();s.player.classList.contains.mockReturnValue(false);
 s.response.videoDetails.videoId='other123456';s.tick();s.response.videoDetails.videoId='abcdefghijk';
 for(const search of ['?v=other123456','?v=abcdefghijk&v=abcdefghijk']){s.location.search=search;s.tick();}s.location.search='?v=abcdefghijk';
 expect(cancel.click).not.toHaveBeenCalled();s.mutation();expect(cancel.click).toHaveBeenCalledOnce();
 s.video.ended=false;s.tick();s.dispatch('pagehide',{});s.video.ended=true;s.mutation();s.tick();expect(cancel.click).toHaveBeenCalledOnce();expect(s.disconnect).toHaveBeenCalled();
});
it('reports native session state while leaving end replay and seeking under official control',()=>{
 const s=setup();s.bridge.configure({epoch:1,videoId:'abcdefghijk'});expect(s.bridge.status()).toMatchObject({state:'playing',authenticated:true,unavailable:false,playability:null});s.video.ended=true;expect(s.bridge.status().state).toBe('ended');s.video.ended=false;expect(s.bridge.status().state).toBe('playing');s.video.readyState=0;s.response.playabilityStatus.status='LOGIN_REQUIRED';expect(s.bridge.status()).toMatchObject({state:'error',unavailable:true,playability:'LOGIN_REQUIRED'});
 s.response.playabilityStatus.status='UNPLAYABLE';expect(s.bridge.status()).toMatchObject({unavailable:true,playability:'UNAVAILABLE'});
 s.response.playabilityStatus.status='OK';s.video.readyState=4;expect(s.bridge.status()).toMatchObject({state:'playing',unavailable:false,playability:null});
});
it('does not reload or resize repeatedly and leaves IME/editable keyboard input untouched',()=>{
 const s=setup();s.bridge.configure({epoch:1,videoId:'abcdefghijk'});s.messages.length=0;const viewport={player:{x:1,y:2,width:640,height:360}};s.bridge.layout(viewport);s.bridge.layout({...viewport});s.tick();expect(s.resize).toHaveBeenCalledOnce();
 s.messages.length=0;s.dispatch('keydown',{key:'f',isComposing:true});s.dispatch('keydown',{key:'f',target:{closest:()=>({})}});s.dispatch('pointerdown',{isTrusted:false,target:{closest:()=>({})}});expect(s.messages).toHaveLength(0);
 s.dispatch('keydown',{key:'f'});expect(s.messages.at(-1)).toMatchObject({type:'fullscreen',enabled:true,inputKey:0x46});
});
it('keeps Escape exit restricted to trusted noneditable noncomposing fullscreen input',()=>{
 const state=setup();state.bridge.configure({epoch:3,videoId:'abcdefghijk',fullscreen:true});state.messages.length=0;
 expect(state.dispatch('keydown',{key:'Escape',target:{closest:()=>null}}).preventDefault).toHaveBeenCalledOnce();
 expect(state.messages.at(-1)).toMatchObject({type:'fullscreen',enabled:false,inputKey:0x1B,epoch:3});state.messages.length=0;
 for(const patch of [{isTrusted:false},{repeat:true},{ctrlKey:true},{altKey:true},{metaKey:true},{shiftKey:true},{isComposing:true},{keyCode:229},{target:{closest:()=>({})}}]) {
  expect(state.dispatch('keydown',{key:'Escape',...patch}).preventDefault).not.toHaveBeenCalled();
 }
 expect(state.messages).toHaveLength(0);
 state.bridge.configure({epoch:3,videoId:'abcdefghijk',fullscreen:false});state.messages.length=0;state.dispatch('keydown',{key:'Escape'});expect(state.messages).toHaveLength(0);
});
it('isolates VOD watch siblings without changing original controls, popups or fade state',()=>{
 const state=setup(),viewport={player:{x:34,y:176,width:951,height:539}};state.bridge.layout(viewport);state.bridge.layout({...viewport});
 const style=document.createElement('style');style.textContent=state.styleText();document.head.append(style);
 document.body.innerHTML='<ytd-app><ytd-masthead><button id="header">header</button></ytd-masthead><ytd-watch-flexy><div id="movie_player"><button id="play">play</button><div class="ytp-chrome-bottom" style="opacity:0"><button id="faded">faded</button></div><div id="closed-menu" style="display:none">menu</div><div class="ytp-popup" id="open-menu">menu</div></div><div id="secondary"><div id="chat"><button id="duplicate-chat" style="visibility:visible">chat</button></div><button id="recommendation" style="visibility:visible">recommendation</button></div><div id="below"><ytd-watch-metadata><button id="metadata" style="visibility:visible">title</button></ytd-watch-metadata></div></ytd-watch-flexy><ytd-popup-container><button id="popup">popup</button></ytd-popup-container></ytd-app>';
 const computed=(id:string)=>getComputedStyle(document.getElementById(id)!);
 try{
  for(const id of ['header','secondary','recommendation','chat','duplicate-chat','below','metadata']){expect(computed(id).visibility,id).toBe('hidden');expect(computed(id).pointerEvents,id).toBe('none');}
  for(const id of ['play','open-menu','popup'])expect(computed(id).visibility,id).toBe('visible');
  expect(computed('movie_player').backgroundColor).toBe('rgb(0, 0, 0)');expect(computed('closed-menu').display).toBe('none');expect(computed('faded').visibility).toBe('visible');expect(getComputedStyle(document.querySelector('.ytp-chrome-bottom')!).opacity).toBe('0');
  expect(computed('secondary').display).not.toBe('none');expect(state.resize).toHaveBeenCalledOnce();
 }finally{style.remove();document.body.replaceChildren();}
});
it('does not hide a matching VOD region that actually contains the original player',()=>{
 const state=setup();state.bridge.layout({player:{x:34,y:176,width:951,height:539}});
 const style=document.createElement('style');style.textContent=state.styleText();document.head.append(style);
 document.body.innerHTML='<ytd-watch-flexy><div id="below"><div id="movie_player"><button id="play">play</button><div id="hidden-control" style="visibility:hidden">hidden</div></div></div></ytd-watch-flexy>';
 try{expect(getComputedStyle(document.getElementById('below')!).visibility).toBe('visible');expect(getComputedStyle(document.getElementById('play')!).visibility).toBe('visible');expect(getComputedStyle(document.getElementById('hidden-control')!).visibility).toBe('hidden');}
 finally{style.remove();document.body.replaceChildren();}
});
it('reports actual content clock and invalidates it through ads, mismatches and source changes',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});
 const clock=()=>s.messages.filter(m=>m.type==='clock').at(-1)!;
 expect(clock()).toMatchObject({valid:false,currentTime:null,adState:'unknown'});s.tick();
 expect(clock()).toMatchObject({valid:true,currentTime:15,duration:100,playbackRate:1,adState:'content'});
 const sequence=clock().sequence as number;s.bridge.configure({epoch:2,videoId:'abcdefghijk'});expect(clock().sequence as number).toBeGreaterThan(sequence);
 s.video.paused=true;s.video.playbackRate=1.5;s.tick();expect(clock()).toMatchObject({currentTime:15,paused:true,playbackRate:1.5});
 s.video.paused=false;s.video.readyState=2;s.tick();expect(clock()).toMatchObject({currentTime:15,buffering:true});
 s.player.classList.contains.mockReturnValue(true);s.video.currentTime=3;s.tick();expect(clock()).toMatchObject({valid:false,currentTime:null,duration:null,adState:'ad',ended:false});
 const adGeneration=clock().sourceGeneration as number;s.player.classList.contains.mockReturnValue(false);s.video.currentTime=16;s.tick();expect(clock().valid).toBe(false);s.tick();expect(clock()).toMatchObject({valid:true,currentTime:16});expect(clock().sourceGeneration as number).toBeGreaterThan(adGeneration);
 s.video.duration=30;s.tick();expect(clock()).toMatchObject({valid:false,currentTime:null,adState:'unknown'});s.video.duration=100;s.tick();s.tick();
 s.video.currentSrc='blob:replacement';s.tick();expect(clock().valid).toBe(false);s.tick();expect(clock().valid).toBe(true);
 s.response.videoDetails.videoId='other123456';s.tick();expect(clock().valid).toBe(false);
});
it('executes only explicit identity-bound official controls and never invents the sought position',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.tick();
 const clock=s.messages.filter(m=>m.type==='clock').at(-1)!;
 const request={requestId:'control-one',documentEpoch:2,sourceId:clock.sourceId,sourceGeneration:clock.sourceGeneration,action:'seek',seconds:40,expiresAtMs:3500};
 expect(s.bridge.control(request).accepted).toBe(true);expect(s.player.seekTo).toHaveBeenCalledWith(40,true);expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({currentTime:15});
 expect(s.bridge.control({...request,seconds:-1}).accepted).toBe(false);expect(s.bridge.control({...request,documentEpoch:1}).accepted).toBe(false);
 expect(s.bridge.control({...request,sourceGeneration:0}).accepted).toBe(false);
 expect(s.bridge.control({...request,expiresAtMs:1000}).accepted).toBe(false);expect(s.bridge.control({...request,expiresAtMs:100000}).accepted).toBe(false);
 expect(s.bridge.control({...request,action:'pause',seconds:null}).accepted).toBe(true);expect(s.player.pauseVideo).toHaveBeenCalledOnce();
 expect(s.bridge.control({...request,action:'resume',seconds:null}).accepted).toBe(true);expect(s.player.playVideo).toHaveBeenCalledOnce();
 s.player.classList.contains.mockReturnValue(true);expect(s.bridge.control(request).accepted).toBe(false);expect(s.player.seekTo).toHaveBeenCalledOnce();
});
it('keeps ended playback seekable and ignores obsolete media callbacks after document teardown',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.tick();s.video.currentTime=100;s.video.ended=true;s.tick();
 expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({ended:true,currentTime:100});
 const oldGeneration=s.messages.filter(m=>m.type==='clock').at(-1)!.seekGeneration as number;
 s.video.seeking=true;s.video.ended=false;s.video.currentTime=25;s.tick();expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({ended:false,seeking:true,currentTime:25});expect(s.messages.filter(m=>m.type==='clock').at(-1)!.seekGeneration as number).toBeGreaterThan(oldGeneration);
 s.video.seeking=false;s.tick();const count=s.messages.filter(m=>m.type==='clock').length;s.dispatch('pagehide',{});s.tick();expect(s.messages.filter(m=>m.type==='clock')).toHaveLength(count);expect(s.video.removeEventListener).toHaveBeenCalled();
});
it('uses only supported official playback rates and observes the applied speed separately',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.tick();
 const clock=s.messages.filter(m=>m.type==='clock').at(-1)!;
 const request={requestId:'rate-one',documentEpoch:2,sourceId:clock.sourceId,sourceGeneration:clock.sourceGeneration,action:'rate',rate:1.5,expiresAtMs:3500};
 expect(s.bridge.control(request).accepted).toBe(true);expect(s.player.setPlaybackRate).toHaveBeenCalledWith(1.5);
 expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({playbackRate:1});
 s.video.playbackRate=1.5;s.tick();expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({playbackRate:1.5});
 for(const patch of [{rate:1.3},{rate:NaN},{rate:0},{rate:17},{seconds:5},{sourceGeneration:0},{expiresAtMs:1}])expect(s.bridge.control({...request,...patch}).accepted).toBe(false);
 s.player.classList.contains.mockReturnValue(true);expect(s.bridge.control(request).accepted).toBe(false);
 expect(s.player.setPlaybackRate).toHaveBeenCalledOnce();
});
it('renders the large close button inside the original player and follows original chrome opacity',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.bridge.layout({visible:true,player:{x:0,y:0,width:640,height:360}});
 expect(s.bridge.controls({epoch:1,canClose:true})).toBe(false);expect(s.buttons).toHaveLength(0);
 s.bridge.controls({epoch:2,canClose:true});const button=s.buttons[0]!;
 expect(button.style.width).toBe('72px');expect(button.style.left).toBe('50%');expect(button.querySelector('svg')).not.toBeNull();expect(button.hidden).toBe(false);
 s.chrome.style.opacity='.4';s.tick();expect(button.style.opacity).toBe('0.4');s.chrome.style.opacity='0';s.tick();expect(button.hidden).toBe(true);
 s.chrome.style.opacity='1';s.bridge.controls({epoch:2,canClose:false});expect(button.hidden).toBe(true);
});
function endedReplay(){
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.bridge.layout({visible:true,player:{x:0,y:0,width:640,height:360}});s.tick();
 s.video.currentTime=100;s.video.ended=true;s.video.paused=true;s.dispatch('ended',{});
 const button=s.buttons.find(button=>button.hasAttribute('data-atsumi-vod-restart')) as HTMLButtonElement;
 expect(button).toBeDefined();expect(button.hidden).toBe(false);return{...s,button};
}
it('adds the local-style central refresh only at selected content EOF and reuses official seek/play',()=>{
 const s=setup();s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.bridge.layout({visible:true,player:{x:0,y:0,width:640,height:360}});s.tick();
 expect(s.buttons).toHaveLength(0);s.video.currentTime=100;s.video.ended=true;s.tick();
 const button=s.buttons.find(button=>button.hasAttribute('data-atsumi-vod-restart')) as HTMLButtonElement;
 expect(button.type).toBe('button');expect(button.getAttribute('aria-label')).toBe('처음부터 다시보기');expect(button.textContent).toBe('다시보기');
 expect(button.style.width).toBe('112px');expect(button.style.height).toBe('112px');expect(button.style.left).toBe('50%');expect(button.style.top).toBe('50%');expect(button.style.transform).toBe('translate(-50%,-50%)');
 expect(button.querySelector('svg')?.getAttribute('width')).toBe('36');expect(button.querySelector('path')?.getAttribute('d')).toBe('M20 7v5h-5M19.4 12A7.5 7.5 0 1 0 18 17');
 s.chrome.style.opacity='0';s.tick();expect(button.hidden).toBe(false);expect(s.chrome.style.opacity).toBe('0');
 const cancel=cancelButton();s.setCancel(cancel);s.mutation();expect(cancel.click).toHaveBeenCalledOnce();expect(button.hidden).toBe(false);
 s.player.seekTo.mockImplementation(()=>{s.video.currentTime=0;s.video.ended=false;});s.player.playVideo.mockImplementation(()=>{s.video.paused=false;});
 button.click();expect(s.player.seekTo).toHaveBeenCalledWith(0,true);expect(s.player.playVideo).toHaveBeenCalledOnce();expect(s.player.seekTo.mock.invocationCallOrder[0]).toBeLessThan(s.player.playVideo.mock.invocationCallOrder[0] ?? -1);
 expect(button.hidden).toBe(true);expect(s.messages.filter(m=>m.type==='clock').at(-1)).toMatchObject({currentTime:0,ended:false});expect(s.buttons).toHaveLength(1);
});
it('keeps standard Enter and Space button activation without forwarding their player shortcuts',()=>{
 const s=endedReplay();
 for(const key of ['Enter',' '])for(const type of ['keydown','keyup']){
  const after=vi.fn();s.button.addEventListener(type,after);const event=new KeyboardEvent(type,{key,bubbles:true,cancelable:true});
  s.button.dispatchEvent(event);expect(event.defaultPrevented).toBe(false);expect(after).not.toHaveBeenCalled();s.button.removeEventListener(type,after);
 }
 // jsdom does not synthesize native keyboard-click activation; the same
 // standard button click handler is used rather than a second key action.
 expect(s.player.seekTo).not.toHaveBeenCalled();s.button.click();expect(s.player.seekTo).toHaveBeenCalledWith(0,true);
});
it('revalidates EOF identity and refuses ad foreign hidden seeking or torn-down click targets',()=>{
 const invalid:Array<[string,(s:ReturnType<typeof endedReplay>)=>void]>=[
  ['playing',s=>{s.video.ended=false;}],['seeking',s=>{s.video.seeking=true;}],['ad',s=>{s.player.classList.contains.mockReturnValue(true);}],
  ['foreign response',s=>{s.response.videoDetails.videoId='other123456';}],['foreign document',s=>{s.location.search='?v=other123456';}],
  ['duplicate document id',s=>{s.location.search='?v=abcdefghijk&v=abcdefghijk';}],['unplayable',s=>{s.response.playabilityStatus.status='UNPLAYABLE';}],
  ['unready',s=>{s.video.readyState=0;}],
  ['hidden layout',s=>s.bridge.layout({visible:false,player:{x:0,y:0,width:640,height:360}})],['empty layout',s=>s.bridge.layout({visible:true,player:{width:0,height:0}})],
  ['hidden document',s=>{s.document.hidden=true;}],['hidden player',s=>{s.player.style.visibility='hidden';}],['transparent player',s=>{s.player.style.opacity='0';}],
  ['hidden ancestor',s=>{const ancestor=document.createElement('div');ancestor.hidden=true;s.player.parentElement=ancestor;}],
  ['inert ancestor',s=>{const ancestor=document.createElement('div');ancestor.setAttribute('inert','');s.player.parentElement=ancestor;}],
  ['transparent ancestor',s=>{const ancestor=document.createElement('div');ancestor.style.opacity='0';s.player.parentElement=ancestor;}],
  ['teardown',s=>{s.dispatch('pagehide',{});}]
 ];
 for(const [name,invalidate] of invalid){const s=endedReplay();invalidate(s);s.button.click();expect(s.player.seekTo,name).not.toHaveBeenCalled();expect(s.player.playVideo,name).not.toHaveBeenCalled();s.tick();expect(s.button.hidden,name).toBe(true);}
});
it('keeps close controls separate and clears EOF presentation on actual resume or visibility changes',()=>{
 const s=endedReplay();s.bridge.controls({epoch:2,canClose:true});
 const close=s.buttons.find(button=>button.hasAttribute('data-atsumi-vod-close'))!;expect(close).not.toBe(s.button);expect(close.style.width).toBe('72px');
 s.video.ended=false;s.dispatch('playing',{});expect(s.button.hidden).toBe(true);s.video.ended=true;s.tick();expect(s.button.hidden).toBe(false);
 s.document.hidden=true;s.dispatch('visibilitychange',{});expect(s.button.hidden).toBe(true);s.document.hidden=false;s.dispatch('visibilitychange',{});expect(s.button.hidden).toBe(false);
 s.dispatch('pagehide',{});expect(s.button.hidden).toBe(true);s.tick();expect(s.button.hidden).toBe(true);
});
it('allows a player with restored computed visibility under a visibility-hidden ancestor',()=>{
 const s=endedReplay(),ancestor=document.createElement('div');ancestor.style.cssText='display:block;visibility:hidden;opacity:1';
 s.player.parentElement=ancestor;s.player.style.visibility='visible';s.tick();expect(s.button.hidden).toBe(false);
 s.button.click();expect(s.player.seekTo).toHaveBeenCalledWith(0,true);
});
it('rejects a stale source click until the replacement selected media clock stabilizes',()=>{
 const s=endedReplay();s.video.currentSrc='blob:replacement';s.button.click();
 expect(s.player.seekTo).not.toHaveBeenCalled();expect(s.player.playVideo).not.toHaveBeenCalled();expect(s.button.hidden).toBe(true);
 s.tick();expect(s.button.hidden).toBe(false);s.button.click();expect(s.player.seekTo).toHaveBeenCalledWith(0,true);
});
it('places the center above the observed modern grid ancestor layer without changing official layers',()=>{
 const s=endedReplay(),context={style:{display:'block',visibility:'visible',opacity:'1',zIndex:'88'},parentElement:s.player};
 const grid={style:{display:'block',visibility:'visible',opacity:'1',zIndex:'auto'},parentElement:context,getBoundingClientRect:()=>({width:640,height:260})};
 const popup={style:{display:'none',visibility:'visible',opacity:'1',zIndex:'1001'},parentElement:s.player,getBoundingClientRect:()=>({width:220,height:180})};
 s.regions.ends.push(grid);s.regions.menus.push(popup);s.tick();
 expect(s.button.hidden).toBe(false);expect(s.button.style.zIndex).toBe('89');expect(context.style.zIndex).toBe('88');expect(popup.style.zIndex).toBe('1001');
 context.style.zIndex='120';s.tick();expect(s.button.style.zIndex).toBe('121');
});
it('hides and refuses center activation while an official menu is open or no safe layer remains',()=>{
 const s=endedReplay(),grid={style:{display:'block',visibility:'visible',opacity:'1',zIndex:'90'},parentElement:s.player,getBoundingClientRect:()=>({width:640,height:260})};
 const popup={style:{display:'none',visibility:'visible',opacity:'1',zIndex:'1001'},parentElement:s.player,getBoundingClientRect:()=>({width:220,height:180})};
 s.regions.ends.push(grid);s.regions.menus.push(popup);s.tick();expect(s.button.hidden).toBe(false);
 popup.style.display='block';s.button.click();expect(s.player.seekTo).not.toHaveBeenCalled();s.tick();expect(s.button.hidden).toBe(true);
 popup.style.display='none';s.tick();expect(s.button.hidden).toBe(false);grid.style.zIndex='1000';s.tick();expect(s.button.hidden).toBe(true);
 expect(popup.style.display).toBe('none');expect(grid.style.zIndex).toBe('1000');
});
function frameSampler(){
 const s=setup();let callback:((now:number,metadata:{mediaTime:number;presentedFrames:number;expectedDisplayTime:number;presentationTime:number})=>void)|null=null,shown=0;
 const cancel=vi.fn();Object.assign(s.video,{videoHeight:720,requestVideoFrameCallback:(fn:typeof callback)=>{callback=fn;return 1;},cancelVideoFrameCallback:cancel});
 const create=s.document.createElement;
 const context={imageSmoothingQuality:'low',drawImage:vi.fn(),getImageData:()=>({data:new Uint8ClampedArray(32*18*4).fill(120)})};
 s.document.createElement=(tag:string)=>tag==='canvas'?{width:0,height:0,getContext:()=>context} as unknown as HTMLCanvasElement:create(tag);
 s.bridge.configure({epoch:2,videoId:'abcdefghijk'});s.tick();
 const clock=s.messages.filter(m=>m.type==='clock').at(-1)!;
 const request={requestId:'33333333-3333-4333-8333-333333333333',documentEpoch:2,sourceId:clock.sourceId,sourceGeneration:clock.sourceGeneration,seekGeneration:clock.seekGeneration,expiresAtMs:3500};
 return{...s,request,context,cancel,frame:(time:number,advance=1)=>{s.video.currentTime=time;const fn=callback;callback=null;shown+=advance;fn?.(0,{mediaTime:time,presentedFrames:shown,expectedDisplayTime:16,presentationTime:0});}};
}
it('binds bounded RAM pixels to the same decoded media frame without seeking or playing',()=>{
 const s=frameSampler();expect(s.bridge.beginFrameSample(s.request)).toBe(true);
 expect(s.context.imageSmoothingQuality).toBe('high');
 expect(s.bridge.beginFrameSample({...s.request,requestId:'44444444-4444-4444-8444-444444444444'})).toBe(false);
 for(let i=0;i<=20;i++)s.frame(15+i/10);
 const value=s.bridge.takeFrameSample(s.request.requestId)!;expect(value.error).toBeNull();expect(value.frames).toHaveLength(21);
 expect(value.frames![0]).toMatchObject({pts90k:1350000});expect(value.frames![20]).toMatchObject({pts90k:1530000});
 expect(value.frames!.every(f=>f.gray.length===576)).toBe(true);expect(s.bridge.takeFrameSample(s.request.requestId)).toBeNull();
 expect(s.player.seekTo).not.toHaveBeenCalled();expect(s.player.playVideo).not.toHaveBeenCalled();
});
it('throws away sampled pixels if an ad, seek, replacement, or canvas failure appears',()=>{
 for(const alter of [(s:ReturnType<typeof frameSampler>)=>s.player.classList.contains.mockReturnValue(true),(s:ReturnType<typeof frameSampler>)=>{s.video.seeking=true;},(s:ReturnType<typeof frameSampler>)=>{s.video.currentSrc='blob:new';},(s:ReturnType<typeof frameSampler>)=>{s.context.drawImage.mockImplementation(()=>{throw new Error('tainted');});}]){
  const s=frameSampler();expect(s.bridge.beginFrameSample(s.request)).toBe(true);s.frame(15);alter(s);s.frame(15.1);
  const value=s.bridge.takeFrameSample(s.request.requestId)!;expect(value.error).toBeTruthy();expect(value.frames).toEqual([]);
 }
 const s=frameSampler();s.bridge.beginFrameSample(s.request);s.bridge.takeFrameSample(s.request.requestId,true);expect(s.cancel).toHaveBeenCalledOnce();expect(s.bridge.takeFrameSample(s.request.requestId)).toBeNull();
});
it('uses only observed timestamps when presented frames advance between callbacks',()=>{
 const s=frameSampler();s.bridge.beginFrameSample(s.request);
 s.frame(15);s.frame(15.1,2);
 for(let i=2;i<=20;i++)s.frame(15+i/10);
 const result=s.bridge.takeFrameSample(s.request.requestId)!;
 expect(result.error).toBeNull();
 expect(result.frames?.map(f=>f.pts90k)).toEqual(Array.from({length:21},(_,i)=>1350000+9000*i));
});
it('rejects a missing-callback time gap or a non-increasing frame counter or timestamp',()=>{
 for(const [nextTime,advance] of [[15.3,2],[15.1,0],[15.1,-1],[15.05,1],[15.04,1]] as const){
  const s=frameSampler();s.bridge.beginFrameSample(s.request);
  s.frame(15);s.frame(15.05);s.frame(nextTime,advance);
  expect(s.bridge.takeFrameSample(s.request.requestId)).toMatchObject({error:'frame_clock',frames:[]});
 }
});
