// @ts-expect-error Node-only isolated script test.
import { runInNewContext } from 'node:vm';
import { expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/youtube/original_vod_page.js?raw';
function setup(origin='https://www.youtube.com',pathname='/watch'){
 const messages:Record<string,unknown>[]=[],listeners=new Map<string,Array<(e:Record<string,unknown>)=>void>>(),styles=new Map<string,unknown>();
 const response={videoDetails:{videoId:'abcdefghijk'},playabilityStatus:{status:'OK'}},video={readyState:4,videoWidth:1280,ended:false},resize=vi.fn();
 let cancel:ReturnType<typeof cancelButton>|null=null,mutation=()=>{};
 const player={querySelector:(selector:string)=>selector==='video'?video:cancel,getPlayerResponse:()=>response,classList:{contains:vi.fn(()=>false)}};
 const root={style:{setProperty:vi.fn()},appendChild:(style:{id:string})=>styles.set(style.id,style)};
 const document={documentElement:root,head:root,getElementById:(id:string)=>id==='movie_player'?player:styles.get(id),createElement:()=>({id:'',textContent:''}),querySelector:()=>null,addEventListener:(name:string,fn:(e:Record<string,unknown>)=>void)=>listeners.set(name,[...(listeners.get(name)??[]),fn])};
 const window:Record<string,unknown>={document,chrome:{webview:{postMessage:(value:string)=>messages.push(JSON.parse(value.replace('ATSUMI_YOUTUBE_VOD_V1:','')))}},ytcfg:{get:()=>true},getComputedStyle:(element:{style:unknown})=>element.style,dispatchEvent:resize,addEventListener:(name:string,fn:(e:Record<string,unknown>)=>void)=>listeners.set(name,[...(listeners.get(name)??[]),fn])};window.top=window;
 const location={origin,pathname,search:'?v=abcdefghijk'},disconnect=vi.fn();
 let interval=()=>{};
 runInNewContext(script,{window,document,location,URLSearchParams,MutationObserver:class{constructor(fn:()=>void){mutation=fn;}observe=vi.fn();disconnect=disconnect;},crypto:{randomUUID:()=> '11111111-1111-4111-8111-111111111111'},setInterval:(fn:()=>void)=>{interval=fn;return 1;},clearInterval:vi.fn(),Event:class{constructor(public type:string){}}});
 type Bridge={configure(v:{epoch:number;videoId:string;fullscreen?:boolean}):boolean;layout(v:unknown):void;status():Record<string,unknown>};
 return{bridge:window.__atsumiYouTubeVod as Bridge,video,response,messages,resize,location,player,disconnect,mutation:()=>mutation(),setCancel:(button:ReturnType<typeof cancelButton>)=>{cancel=button;},styleText:()=>[...styles.values()].map(style=>(style as {textContent:string}).textContent).join('\n'),tick:()=>interval(),dispatch:(name:string,patch:Record<string,unknown>)=>{const event={isTrusted:true,button:0,key:'',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...patch};listeners.get(name)?.forEach(fn=>fn(event));return event;}};
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
 const s=setup();s.bridge.configure({epoch:1,videoId:'abcdefghijk'});expect(s.bridge.status()).toMatchObject({state:'playing',authenticated:true,unavailable:false});s.video.ended=true;expect(s.bridge.status().state).toBe('ended');s.video.ended=false;expect(s.bridge.status().state).toBe('playing');s.video.readyState=0;s.response.playabilityStatus.status='LOGIN_REQUIRED';expect(s.bridge.status()).toMatchObject({state:'error',unavailable:true});
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
