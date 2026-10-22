// @ts-expect-error Node-only test utility; production renderer has no Node globals.
import { runInNewContext } from 'node:vm';
import page from '../../../src-tauri/src/youtube/original_page.js?raw';
import capture from '../../../src-tauri/src/youtube/original_capture.js?raw';
import { expect, it, vi } from 'vitest';
function setup(chat=false,origin='https://www.youtube.com',beforeHtml=false){
 const listeners=new Map<string,Array<(event:Record<string,unknown>)=>void>>(),messages:Record<string,unknown>[]=[],styles=new Map<string,unknown>(),resize=vi.fn();
 let iframe:{src:string}|null=null;
 const video={readyState:4,videoWidth:854,ended:false},root={style:{setProperty:vi.fn()},appendChild:vi.fn((style:{id:string})=>styles.set(style.id,style))};
 const player={querySelector:()=>video,getPlayerResponse:()=>({videoDetails:{videoId:'abcdefghijk',channelId:'UC1234567890123456789012',isLive:true},playabilityStatus:{status:'OK'}})};
 const chatRoot={getBoundingClientRect:()=>({width:360,height:600})};
 const document={head:beforeHtml?null:root,documentElement:beforeHtml?null:root,getElementById:(id:string)=>id==='movie_player'?player:styles.get(id),querySelector:(selector:string)=>selector==='iframe#chatframe'?iframe:selector==='yt-live-chat-app'?chatRoot:selector==='#movie_player video'?video:selector==='#movie_player .ytp-chrome-bottom'?{}:null,createElement:()=>({id:'',textContent:''}),addEventListener:(name:string,callback:(event:Record<string,unknown>)=>void)=>listeners.set(name,[...(listeners.get(name)??[]),callback])};
 const window:Record<string,unknown>={document,chrome:{webview:{postMessage:(data:string)=>messages.push(JSON.parse(data.replace('ATSUMI_YOUTUBE_V1:','')))}},__atsumiYouTubeEncodedCapture:{getStatus:()=>({ready:true}),start:vi.fn(async()=>undefined),stop:vi.fn()},ytcfg:{get:()=>false},ytInitialData:{contents:{twoColumnWatchNextResults:{conversationBar:{liveChatRenderer:{}}}}},dispatchEvent:resize,addEventListener:vi.fn()};window.top=window;
 let opacity='1';let interval:()=>void=()=>undefined;
 runInNewContext(page,{window,document,location:{origin,pathname:chat?'/live_chat':'/watch',search:'?v=abcdefghijk'},URL,URLSearchParams,crypto:{randomUUID:()=> '11111111-1111-4111-8111-111111111111'},performance:{now:()=>0},setTimeout,clearTimeout,setInterval:(callback:()=>void)=>{interval=callback;return 1;},clearInterval:vi.fn(),requestAnimationFrame:()=>1,cancelAnimationFrame:vi.fn(),getComputedStyle:()=>({display:'block',visibility:'visible',opacity}),Event:class{constructor(public type:string){}}});
 type Bridge={configure(value:{epoch:number;channel:string;fullscreen?:boolean}):boolean;layout(value:unknown):void;status():Record<string,unknown>;start(id:string):Promise<unknown>};
 return{window,bridge:window.__atsumiYouTubeOriginal as Bridge,messages,resize,root,styles,parseHtml:()=>{document.head=root;document.documentElement=root;listeners.get('DOMContentLoaded')?.forEach(callback=>callback({}));},setIframe:(src:string|null)=>{iframe=src===null?null:{src};},styleText:()=>(styles.get('atsumi-youtube-original-layout') as {textContent:string}|undefined)?.textContent??'',tick:()=>interval(),setOpacity:(value:string)=>{opacity=value;},dispatch:(name:string,patch:Record<string,unknown>)=>{const event={isTrusted:true,button:0,key:'',code:'',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...patch};listeners.get(name)?.forEach(callback=>callback(event));return event;}};
}
it.each([false,true])('installs safely before HTML and applies one style set after parsing (chat=%s)',chat=>{
 const state=setup(chat,'https://www.youtube.com',true);expect(state.bridge).toBeDefined();
 expect(state.bridge.configure({epoch:1,channel:'abcdefghijk'})).toBe(true);
 state.bridge.layout({visible:true,player:{x:0,y:0,width:800,height:450},chat:{x:800,y:0,width:480,height:450}});
 expect(state.root.appendChild).not.toHaveBeenCalled();state.parseHtml();
 expect(state.styles.has('atsumi-youtube-original-layout')).toBe(true);
 expect(state.styles.has('atsumi-live-chat-header')).toBe(chat);
 const count=state.root.appendChild.mock.calls.length;state.parseHtml();state.tick();
 expect(state.root.appendChild).toHaveBeenCalledTimes(count);
});
it('installs only in the canonical provider and requires matching current video for configure',()=>{
 expect(setup(false,'https://chzzk.naver.com').bridge).toBeUndefined();expect(setup(false,'https://www.youtube.com.evil.test').bridge).toBeUndefined();
 const state=setup();expect(state.bridge.configure({epoch:1,channel:'other123456'})).toBe(false);expect(state.messages).toHaveLength(0);expect(state.bridge.configure({epoch:1,channel:'abcdefghijk'})).toBe(true);
});
it('publishes only bounded public channel descriptor and observed auth state',()=>{
 const state=setup();const status=state.bridge.status();expect(status.authenticated).toBe(false);expect(status.chatDescriptor).toEqual({videoId:'abcdefghijk',channelId:'UC1234567890123456789012',isLive:true,chatAvailable:true});expect(status.captureReady).toBe(true);
 (state.window.ytInitialData as {contents:unknown}).contents={};expect(state.bridge.status().chatDescriptor).toBeNull();
});
it('keeps absent chat data unknown until the same exact live document advertises chat',()=>{
 const state=setup();const initial=state.window.ytInitialData as {contents:unknown};
 for(const watch of [{},{conversationBar:{}},{conversationBar:{liveChatRenderer:false}},{conversationBar:{liveChatRenderer:[]}}]){
  initial.contents={twoColumnWatchNextResults:watch};
  expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:null});
 }
 initial.contents={twoColumnWatchNextResults:{conversationBar:{liveChatRenderer:{}}}};
 expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:true});
});
it('recognizes only the observed explicit official disabled-chat notices',()=>{
 const state=setup();const initial=state.window.ytInitialData as {contents:unknown};
 const notice=(text:unknown)=>({twoColumnWatchNextResults:{conversationBar:{conversationBarRenderer:{availabilityMessage:{messageRenderer:{text}}}}}});
 for(const text of ['Chat is disabled for this live stream.','채팅을 사용할 수 없는 실시간 스트림입니다.']){
  initial.contents=notice({simpleText:text});expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:false});
  initial.contents=notice({runs:[{text}]});expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:false});
 }
 for(const text of [{simpleText:'Sign in to chat.'},{simpleText:''},{runs:[]},{simpleText:'unrecognized locale'},{runs:[{text:'Chat is disabled for this live stream.'},{text:7}]}]){
  initial.contents=notice(text);expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:null});
 }
});
it('never treats an iframe alone as evidence of available chat even at the exact official address',()=>{
 const state=setup();(state.window.ytInitialData as {contents:unknown}).contents={twoColumnWatchNextResults:{}};
 for(const src of ['about:blank','https://evil.invalid/live_chat?v=abcdefghijk','https://www.youtube.com/live_chat?v=other123456','https://www.youtube.com/live_chat?v=abcdefghijk&v=other123456','https://user@www.youtube.com/live_chat?v=abcdefghijk','https://www.youtube.com:444/live_chat?v=abcdefghijk','https://www.youtube.com/watch?v=abcdefghijk']){
  state.setIframe(src);expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:null});
 }
 state.setIframe('https://www.youtube.com/live_chat?v=abcdefghijk&embed_domain=www.youtube.com');
 expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:null});
});
it('keeps an explicit disabled notice authoritative over an iframe or stale renderer shell',()=>{
 const state=setup();state.setIframe('https://www.youtube.com/live_chat?v=abcdefghijk');
 (state.window.ytInitialData as {contents:unknown}).contents={twoColumnWatchNextResults:{conversationBar:{liveChatRenderer:{},conversationBarRenderer:{availabilityMessage:{messageRenderer:{text:{simpleText:'Chat is disabled for this live stream.'}}}}}}};
 expect(state.bridge.status().chatDescriptor).toMatchObject({chatAvailable:false});
 expect(state.bridge.status().chatVisible).toBe(false);
});
it('retains safe start failure across periodic ticks and restores native fullscreen ownership',async()=>{
 const state=setup();state.bridge.configure({epoch:1,channel:'abcdefghijk',fullscreen:true});const cap=state.window.__atsumiYouTubeEncodedCapture as {start:ReturnType<typeof vi.fn>};cap.start.mockRejectedValueOnce(Object.assign(new Error('secret-private-text'),{code:'BROWSER_ACK_TIMEOUT'}));await state.bridge.start('request');state.tick();expect(state.bridge.status()).toMatchObject({captureStartError:'BROWSER_ACK_TIMEOUT',error:'capture_start_failed'});expect(JSON.stringify(state.messages)).not.toContain('secret-private');state.dispatch('pointerdown',{target:{closest:()=>({})}});expect(state.messages.at(-1)).toMatchObject({type:'fullscreen',enabled:false});await state.bridge.start('retry');expect(state.bridge.status().captureStartError).toBeNull();
});
it('measures original chat document independently and cannot start recording from chat',()=>{
 const state=setup(true);state.bridge.configure({epoch:1,channel:'abcdefghijk'});state.tick();expect(state.messages.every(message=>message.type==='chat_status')).toBe(true);expect(state.messages.at(-1)).toMatchObject({visible:true});
 state.dispatch('keydown',{key:'s',code:'KeyS'});state.dispatch('pointerdown',{target:{closest:()=>({})}});expect(state.messages.every(message=>message.type==='chat_status')).toBe(true);
});
it('applies layout only when geometry changes, without reload or repeat resize loops',()=>{
 const state=setup();const viewport={player:{x:10,y:20,width:600,height:400}};state.bridge.layout(viewport);state.bridge.layout({...viewport});expect(state.resize).toHaveBeenCalledOnce();expect(state.root.style.setProperty).toHaveBeenCalledWith('--atsumi-youtube-w','600px');state.tick();state.tick();expect(state.resize).toHaveBeenCalledOnce();
});
it('keeps a positive initial receiver and retains its real dimensions while hidden',()=>{
 const state=setup();state.bridge.layout({visible:false,player:null});
 expect(state.root.style.setProperty).toHaveBeenCalledWith('--atsumi-youtube-w','1280px');
 expect(state.root.style.setProperty).toHaveBeenCalledWith('--atsumi-youtube-h','720px');
 const viewport={visible:true,player:{x:10,y:20,width:960,height:720}};state.bridge.layout(viewport);
 expect(state.resize).toHaveBeenCalledTimes(2);
 state.bridge.layout({visible:false,player:null});state.bridge.layout({visible:false,player:{x:0,y:0,width:0,height:0}});state.bridge.layout(viewport);
 expect(state.resize).toHaveBeenCalledTimes(2);
 expect(state.root.style.setProperty).not.toHaveBeenCalledWith('--atsumi-youtube-w','0px');
 state.bridge.layout({player:{x:10,y:20,width:800,height:450}});expect(state.resize).toHaveBeenCalledTimes(3);
 expect(state.root.style.setProperty).toHaveBeenCalledWith('--atsumi-youtube-w','800px');
});
it('does not apply the hidden video fallback to the separate original chat',()=>{
 const state=setup(true);state.bridge.layout({visible:false,chat:null});
 expect(state.root.style.setProperty).toHaveBeenCalledWith('--atsumi-youtube-w','0px');
 expect(state.root.style.setProperty).not.toHaveBeenCalledWith('--atsumi-youtube-w','1280px');
});
it('reports automatic stability separately without changing manual capture capability',()=>{
 const state=setup();expect(state.bridge.status()).toMatchObject({captureReady:true,captureStable:false});
 (state.window.__atsumiYouTubeEncodedCapture as {readyForAutomatic?:()=>boolean}).readyForAutomatic=()=>true;
 expect(state.bridge.status()).toMatchObject({captureReady:true,captureStable:true});
});
it('admits only trusted player gestures and leaves editable/IME shortcuts alone',()=>{
 const state=setup();state.bridge.configure({epoch:1,channel:'abcdefghijk'});state.messages.length=0;
 state.dispatch('pointerdown',{isTrusted:false,target:{closest:()=>({})}});state.dispatch('keydown',{key:'s',code:'KeyS',isComposing:true});state.dispatch('keydown',{key:'s',target:{closest:()=>({})}});expect(state.messages).toHaveLength(0);
 state.dispatch('pointerdown',{target:{closest:()=>({})}});expect(state.messages.at(-1)).toMatchObject({type:'fullscreen',enabled:true,inputKey:1});state.dispatch('keydown',{key:'s',code:'KeyS'});expect(state.messages.at(-1)).toMatchObject({type:'screenshot',inputKey:0x53});
});
it('forwards a plain body Escape only for owned fullscreen and excludes chat, editable and composing input',()=>{
 const state=setup();state.bridge.configure({epoch:2,channel:'abcdefghijk',fullscreen:true});state.messages.length=0;
 const event=state.dispatch('keydown',{key:'Escape',target:{closest:()=>null}});
 expect(event.preventDefault).toHaveBeenCalledOnce();expect(state.messages.at(-1)).toMatchObject({type:'fullscreen',enabled:false,inputKey:0x1B,epoch:2});
 state.messages.length=0;
 for(const patch of [{isTrusted:false},{repeat:true},{ctrlKey:true},{altKey:true},{metaKey:true},{shiftKey:true},{isComposing:true},{keyCode:229},{target:{closest:()=>({})}}]) {
  expect(state.dispatch('keydown',{key:'Escape',...patch}).preventDefault).not.toHaveBeenCalled();
 }
 expect(state.messages).toHaveLength(0);
 state.bridge.configure({epoch:2,channel:'abcdefghijk',fullscreen:false});state.messages.length=0;state.dispatch('keydown',{key:'Escape'});expect(state.messages).toHaveLength(0);
 const chat=setup(true);chat.bridge.configure({epoch:2,channel:'abcdefghijk',fullscreen:true});chat.messages.length=0;
 expect(chat.dispatch('keydown',{key:'Escape'}).preventDefault).not.toHaveBeenCalled();expect(chat.messages).toHaveLength(0);
});
it('follows original control opacity without recreating any video receiver',()=>{
 const state=setup();state.bridge.configure({epoch:1,channel:'abcdefghijk'});state.tick();expect(state.messages.filter(message=>message.type==='presentation')).toHaveLength(1);state.tick();expect(state.messages.filter(message=>message.type==='presentation')).toHaveLength(1);state.setOpacity('0');state.tick();expect(state.messages.at(-1)).toMatchObject({type:'presentation',opacity:0});
 for(const forbidden of ['fetch(', 'XMLHttpRequest', 'document.cookie', 'localStorage', 'captureStream(']){expect(page).not.toContain(forbidden);expect(capture).not.toContain(forbidden);}
 expect(capture).toContain('ad-showing');expect(capture).toContain('mediaKeys');
});
it('hides only watch siblings while preserving original controls, menus and their own hidden states',()=>{
 const state=setup();state.bridge.layout({player:{x:34,y:176,width:951,height:539}});
 const style=document.createElement('style');style.textContent=state.styleText();document.head.append(style);
 document.body.innerHTML='<ytd-app><ytd-masthead><button id="header">header</button></ytd-masthead><ytd-watch-flexy><div id="movie_player"><button id="play">play</button><div class="ytp-chrome-bottom" style="opacity:0"><button id="faded">faded</button></div><div id="closed-menu" style="display:none">menu</div><div class="ytp-popup" id="open-menu">menu</div></div><div id="secondary"><div id="chat"><button id="duplicate-chat" style="visibility:visible">chat</button></div></div><div id="below"><ytd-watch-metadata><button id="metadata" style="visibility:visible">title</button></ytd-watch-metadata></div></ytd-watch-flexy><ytd-popup-container><button id="popup">popup</button></ytd-popup-container></ytd-app>';
 const computed=(id:string)=>getComputedStyle(document.getElementById(id)!);
 try{
  for(const id of ['header','secondary','chat','duplicate-chat','below','metadata']){expect(computed(id).visibility,id).toBe('hidden');expect(computed(id).pointerEvents,id).toBe('none');}
  for(const id of ['play','open-menu','popup'])expect(computed(id).visibility,id).toBe('visible');
  expect(computed('movie_player').backgroundColor).toBe('rgb(0, 0, 0)');expect(computed('closed-menu').display).toBe('none');expect(computed('faded').visibility).toBe('visible');expect(getComputedStyle(document.querySelector('.ytp-chrome-bottom')!).opacity).toBe('0');
  expect(computed('secondary').display).not.toBe('none');expect(state.resize).toHaveBeenCalledOnce();
 }finally{style.remove();document.body.replaceChildren();}
});
it('does not hide a candidate region containing the original player or modify the separate chat stylesheet',()=>{
 const state=setup();state.bridge.layout({player:{x:34,y:176,width:951,height:539}});
 const style=document.createElement('style');style.textContent=state.styleText();document.head.append(style);
 document.body.innerHTML='<ytd-watch-flexy><div id="below"><div id="movie_player"><button id="play">play</button></div></div></ytd-watch-flexy>';
 try{expect(getComputedStyle(document.getElementById('below')!).visibility).toBe('visible');expect(getComputedStyle(document.getElementById('play')!).visibility).toBe('visible');}
 finally{style.remove();document.body.replaceChildren();}
 const chat=setup(true);chat.bridge.layout({chat:{x:993,y:176,width:239,height:539}});expect(chat.styleText()).not.toContain('watch-flexy');expect(chat.styleText()).not.toContain('visibility:hidden');expect(chat.styleText()).not.toContain('#movie_player');
});
