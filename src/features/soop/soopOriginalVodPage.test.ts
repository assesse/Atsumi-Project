// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/soop/original_vod_page.js?raw';
const vmName='node:vm';
const {runInNewContext}=await import(vmName) as {runInNewContext(source:string,context:Record<string,unknown>):unknown};
const endpoint='https://api.m.sooplive.com/station/video/a/view';
type Adapter={configure(v:unknown):boolean;layout(v:unknown):void;status():Record<string,unknown>};
function fixture(url='https://vod.sooplive.com/player/209244227/embed?showChat=true&replayType=1',nested=false){
 const nodes=new Map<string,unknown>(),styles=new Map<string,string>(),events=new Map<string,(v:unknown)=>void>(),messages:Record<string,unknown>[]=[];
 const video={readyState:4,videoWidth:1280,paused:false,ended:false,error:null as unknown};let missing=false,tick=()=>{};
 const document={documentElement:{style:{setProperty:(key:string,value:string)=>styles.set(key,value)},appendChild:(node:{id:string})=>nodes.set(node.id,node)},head:null,getElementById:(id:string)=>nodes.get(id),createElement:()=>({id:'',textContent:''}),querySelector:(selector:string)=>selector==='#videoLayer video#video.af_video'?video:selector==='#webplayer.vod_404'&&missing?{}:null,addEventListener:(name:string,handler:(v:unknown)=>void)=>events.set(name,handler),get cookie():never{throw Error('No cookie access');}};
 class Xhr{responseType='';responseURL=endpoint;responseText='';status=200;listeners=new Map<string,()=>void>();send=()=>undefined;addEventListener(name:string,fn:()=>void){this.listeners.set(name,fn);}removeEventListener(name:string){this.listeners.delete(name);}finish(value:unknown){this.responseText=typeof value==='string'?value:JSON.stringify(value);this.listeners.get('loadend')?.();}}
 // Native send is on the prototype, just as in the real WebView.
 Object.defineProperty(Xhr.prototype,'send',{value:vi.fn(),writable:true});
 const fetch=vi.fn(),resize=vi.fn();
 const window={top:null as unknown,fetch,XMLHttpRequest:Xhr,chrome:{webview:{postMessage:(s:string)=>messages.push(JSON.parse(s.slice('ATSUMI_SOOP_VOD_V1:'.length)))}},dispatchEvent:resize,addEventListener:vi.fn(),__atsumiSoopVod:undefined as Adapter|undefined};window.top=nested?{}:window;
 runInNewContext(script,{window,document,location:new URL(url),URL,TextEncoder,TextDecoder,crypto:{randomUUID:()=>crypto.randomUUID()},setInterval:(fn:()=>void)=>{tick=fn;return 1;},clearInterval:vi.fn(),setTimeout,clearTimeout,Event:class{constructor(public type:string){}}});
 const xhr=()=>{const value=new Xhr();delete (value as unknown as {send?:unknown}).send;value.send();return value;};
 return{window,adapter:window.__atsumiSoopVod!,video,styles,nodes,events,messages,fetch,resize,xhr,tick:()=>tick(),missing:()=>{missing=true;}};
}
const metadata=(show_chat:unknown='Y',title_no:unknown=209244227,result:unknown=1)=>({result,data:{title_no,show_chat,unwantedSecret:'must-not-leave-page'}});
async function settle(){for(let n=0;n<30;n++)await new Promise(resolve=>setTimeout(resolve,0));}
describe('SOOP original VOD watch-only adapter',()=>{
 it('installs only on the exact top-level official embed and cannot record or access account secrets',()=>{
  for(const url of ['https://vod.sooplive.com.evil.test/player/209244227/embed?showChat=true','http://vod.sooplive.com/player/209244227/embed?showChat=true','https://vod.sooplive.com/player/209244227/chat?showChat=true','https://vod.sooplive.com/player/209244227/embed?showChat=true&fromApi=1'])expect(fixture(url).adapter).toBeUndefined();expect(fixture(undefined,true).adapter).toBeUndefined();
  const f=fixture();expect(f.adapter.configure({epoch:1,videoId:'209244228'})).toBe(false);expect(f.adapter.configure({epoch:1,videoId:'209244227'})).toBe(true);expect(f.adapter).not.toHaveProperty('start');
  for(const forbidden of ['document.cookie','localStorage','captureStream','encoded_begin','MediaRecorder','__atsumiSoopEncodedCapture','livePlayer','location.href ='])expect(script).not.toContain(forbidden);
  expect(f.fetch).not.toHaveBeenCalled();
 });
 it('leaves official playback and replay intact, reporting content rather than adVideo',()=>{
  const f=fixture();expect(f.adapter.status()).toMatchObject({state:'playing',chatState:'pending'});f.video.paused=true;expect(f.adapter.status().state).toBe('paused');f.video.ended=true;expect(f.adapter.status().state).toBe('ended');f.video.ended=false;f.video.paused=false;expect(f.adapter.status().state).toBe('playing');f.video.readyState=0;f.missing();expect(f.adapter.status()).toMatchObject({state:'error',unavailable:true});
 });
 it('requires the official single-title stop option without changing global autoplay preferences',()=>{
  const base='https://vod.sooplive.com/player/209244227/embed?';
  for(const query of ['showChat=true','showChat=true&replayType=0','showChat=true&replayType=1&replayType=0','replayType=1&showChat=true','showChat=true&replayType=1&fromApi=1'])expect(fixture(base+query).adapter).toBeUndefined();
  expect(fixture(base+'showChat=true&replayType=1').adapter).toBeDefined();
  expect(script).not.toContain('VodAutoPlay'); expect(script).not.toContain('bEmbedAutoPlay');
 });
 it('projects only exact successful video metadata and known chat values from XHR',()=>{
  const f=fixture();f.adapter.configure({epoch:1,videoId:'209244227'});
  for(const value of [metadata('Y',209244228),metadata('Y',209244227,'1'),metadata('maybe'),'{broken',metadata('Y',{})]){f.xhr().finish(value);expect(f.adapter.status().chatState).toBe('pending');}
  const foreign=f.xhr();foreign.responseURL='https://api.m.sooplive.com.evil.test/station/video/a/view';foreign.finish(metadata());expect(f.adapter.status().chatState).toBe('pending');
  const failed=f.xhr();failed.status=403;failed.finish(metadata());expect(f.adapter.status().chatState).toBe('pending');
  f.xhr().finish(' '.repeat(1024*1024)+JSON.stringify(metadata()));expect(f.adapter.status().chatState).toBe('pending');
  f.xhr().finish(metadata());expect(f.adapter.status().chatState).toBe('available');f.xhr().finish(metadata('N'));expect(f.adapter.status().chatState).toBe('unavailable');expect(JSON.stringify(f.messages)).not.toContain('unwantedSecret');expect(JSON.stringify(f.messages)).not.toContain('show_chat');
 });
 it('preserves the exact fetch promise and caller body while observing only a bounded clone',async()=>{
  const f=fixture();const response=new Response(JSON.stringify(metadata()));Object.defineProperty(response,'url',{value:endpoint});const promise=Promise.resolve(response);f.fetch.mockReturnValueOnce(promise);
  const actual=f.window.fetch(endpoint);expect(actual).toBe(promise);expect(await (await actual).json()).toEqual(metadata());await settle();expect(f.adapter.status().chatState).toBe('available');
  const large=new Response(' '.repeat(1024*1024)+JSON.stringify(metadata('N')));Object.defineProperty(large,'url',{value:endpoint});f.fetch.mockResolvedValueOnce(large);await f.window.fetch(endpoint);await settle();expect(f.adapter.status().chatState).toBe('available');
 });
 it('does not accept a valid-looking prefix from a metadata stream cancelled by its deadline',async()=>{
  vi.useFakeTimers();
  try {
   const f=fixture();const stream=new ReadableStream<Uint8Array>({start(controller){controller.enqueue(new TextEncoder().encode(JSON.stringify(metadata())));}});
   const response=new Response(stream);Object.defineProperty(response,'url',{value:endpoint});f.fetch.mockResolvedValueOnce(response);await f.window.fetch(endpoint);
   await vi.advanceTimersByTimeAsync(10_001);expect(f.adapter.status().chatState).toBe('pending');
   void response.body?.cancel();
  } finally { vi.useRealTimers(); }
 });
 it('applies exact independent slots once without media reload or changing user keyboard/IME',()=>{
  const f=fixture();f.adapter.configure({epoch:1,videoId:'209244227'});const viewport={player:{x:10,y:20,width:640,height:360},chat:{x:654,y:20,width:200,height:360}};f.adapter.layout(viewport);f.adapter.layout(viewport);f.tick();expect(f.resize).toHaveBeenCalledOnce();expect(f.styles.get('--atsumi-vod-player-w')).toBe('640px');expect(f.styles.get('--atsumi-vod-chat-x')).toBe('654px');f.adapter.layout({player:viewport.player,chat:null});expect(f.styles.get('--atsumi-vod-chat-display')).toBe('none');
  f.messages.length=0;const event={isTrusted:true,key:'f',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),target:{closest:(selector:string)=>selector==='#player'?{}:null}};
  for(const patch of [{isTrusted:false},{isComposing:true},{keyCode:229},{repeat:true},{ctrlKey:true},{target:{closest:()=>({})}}])f.events.get('keydown')!({...event,...patch});expect(f.messages).toHaveLength(0);expect(event.preventDefault).not.toHaveBeenCalled();f.events.get('keydown')!(event);expect(f.messages[0]).toMatchObject({type:'fullscreen',inputKey:0x46,enabled:true});
 });
 it('exits existing fullscreen from body or html without widening F or nonfullscreen Escape',()=>{
  const f=fixture();f.adapter.configure({epoch:1,videoId:'209244227',fullscreen:true});f.messages.length=0;
  const event=(key:string,nodeName:string)=>({isTrusted:true,key,target:{nodeName,closest:()=>null},preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()});
  for(const name of ['BODY','HTML']){const value=event('Escape',name);f.events.get('keydown')!(value);expect(value.preventDefault).toHaveBeenCalledOnce();expect(value.stopImmediatePropagation).toHaveBeenCalledOnce();}
  expect(f.messages).toHaveLength(2);for(const value of f.messages)expect(value).toMatchObject({type:'fullscreen',inputKey:0x1B,enabled:false});
  f.messages.length=0;const outsideF=event('f','BODY');f.events.get('keydown')!(outsideF);expect(f.messages).toHaveLength(0);expect(outsideF.preventDefault).not.toHaveBeenCalled();
  f.adapter.configure({epoch:1,videoId:'209244227',fullscreen:false});f.messages.length=0;const outsideEscape=event('Escape','BODY');f.events.get('keydown')!(outsideEscape);expect(f.messages).toHaveLength(0);expect(outsideEscape.preventDefault).not.toHaveBeenCalled();
 });
 it('keeps document-wide fullscreen exit out of chat, editable controls, menus and IME',()=>{
  const f=fixture();f.adapter.configure({epoch:1,videoId:'209244227',fullscreen:true});f.messages.length=0;
  const excluded=['#chatting_area','input','textarea','select','[contenteditable]:not([contenteditable="false"])','[role="textbox"]','[role="combobox"]','[role="menu"]'];
  const base={isTrusted:true,key:'Escape',target:{closest:()=>null}};
  const patches=[{isTrusted:false},{defaultPrevented:true},{repeat:true},{isComposing:true},{keyCode:229},{ctrlKey:true},{altKey:true},{metaKey:true},{shiftKey:true},...excluded.map(part=>({target:{closest:(selector:string)=>selector.split(',').includes(part)?{}:null}}))];
  for(const patch of patches){const value={...base,...patch,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn()};f.events.get('keydown')!(value);expect(value.preventDefault).not.toHaveBeenCalled();expect(value.stopImmediatePropagation).not.toHaveBeenCalled();}
  expect(f.messages).toHaveLength(0);
 });
});
