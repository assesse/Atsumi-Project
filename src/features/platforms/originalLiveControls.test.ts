// @ts-expect-error Node-only fixture.
import { runInNewContext } from 'node:vm';
import { afterEach, expect, it, vi } from 'vitest';
import youtube from '../../../src-tauri/src/youtube/original_page.js?raw';
import soop from '../../../src-tauri/src/soop/original_page.js?raw';
type Bridge={configure(value:unknown):boolean;layout(value:unknown):void;controls(value:unknown):boolean;resume(epoch:number):boolean;reply(id:string,value:unknown):void};
async function setup(provider:'youtube'|'soop',options:{rejectOwnedHtml?:boolean}={}){
  const yt=provider==='youtube';
  document.body.innerHTML=yt?'<div id="movie_player"><video></video><div class="ytp-chrome-bottom" style="opacity:1"><div><button class="ytp-fullscreen-button">full</button></div></div></div>':'<div id="player"><video id="livePlayer"></video><div class="player_ctrlBox" style="opacity:1"><div><button class="btn_fullScreen_mode">full</button></div></div></div><div id="chatting_area"><div id="chatbox"><div class="area_header"><div class="chat_title"><ul><li id="setbox_close">close</li></ul></div></div><div class="chat_layer"><button class="close">dialog close</button></div></div></div>';
  let blockedHtmlWrites=0;
  if(options.rejectOwnedHtml){
    const setHtml=Object.getOwnPropertyDescriptor(Element.prototype,'innerHTML')!.set!;
    vi.spyOn(Element.prototype,'innerHTML','set').mockImplementation(function(this:Element,value:string){
      if(this.hasAttribute('data-atsumi-live-action')){blockedHtmlWrites++;throw new TypeError("Failed to set the 'innerHTML' property on 'Element': This document requires 'TrustedHTML' assignment.");}
      setHtml.call(this,value);
    });
  }
  const callbacks=new WeakMap<EventTarget,Map<string,EventListener>>();
  vi.spyOn(EventTarget.prototype,'addEventListener').mockImplementation(function(this:EventTarget,name:string,listener:EventListenerOrEventListenerObject|null){if(typeof listener==='function'){let map=callbacks.get(this);if(!map){map=new Map();callbacks.set(this,map);}map.set(name,listener);}});
  const video=document.querySelector('video')!;const play=vi.fn(async()=>undefined);Object.defineProperty(video,'play',{value:play,configurable:true});
  const messages:Array<Record<string,unknown>>=[];let tick=()=>{}, frame:(()=>void)|null=null;const mutations:Array<()=>void>=[];
  const player=document.getElementById(yt?'movie_player':'player')!;
  const rect=(x:number,y:number,width:number,height:number)=>({x,y,width,height,left:x,top:y,right:x+width,bottom:y+height,toJSON:()=>({})});
  let controlRect=rect(720,410,40,40);player.getBoundingClientRect=()=>rect(0,0,800,450);
  player.querySelector(yt?'.ytp-fullscreen-button':'.btn_fullScreen_mode')!.getBoundingClientRect=()=>controlRect;
  if(yt)Object.assign(player,{getPlayerResponse:()=>({videoDetails:{videoId:'abcdefghijk',isLive:true}})});
  const win:Record<string,unknown>={top:null,document,chrome:{webview:{postMessage:(value:string)=>messages.push(JSON.parse(value.slice(value.indexOf(':')+1)))}},addEventListener:vi.fn(),dispatchEvent:vi.fn(),postMessage:vi.fn(),livePlayer:{liveAPI:{getResponse:()=>({CHANNEL:{BJID:'fixture',BNO:12345}})}},liveView:{isLoaded:async()=>undefined,isLogin:()=>false}};win.top=win;
  runInNewContext(yt?youtube:soop,{window:win,document,location:new URL(yt?'https://www.youtube.com/watch?v=abcdefghijk':'https://play.sooplive.com/fixture/12345/embed'),URLSearchParams,crypto,performance,getComputedStyle:(node:Element)=>{const style=getComputedStyle(node);return {opacity:style.opacity||'1',visibility:style.visibility,display:style.display};},MutationObserver:class{constructor(callback:()=>void){mutations.push(callback);}observe(){}disconnect(){}},Event,setTimeout,clearTimeout,setInterval:(fn:()=>void)=>{tick=fn;return 1;},clearInterval:vi.fn(),requestAnimationFrame:(fn:()=>void)=>{frame=fn;return 1;},cancelAnimationFrame:vi.fn()});
  const bridge=win[yt?'__atsumiYouTubeOriginal':'__atsumiSoopOriginal'] as Bridge;
  bridge.configure({epoch:1,channel:yt?'abcdefghijk':'fixture'});for(let n=0;n<8;n++)await Promise.resolve();bridge.layout({visible:true,player:{x:0,y:0,width:800,height:450},chat:{x:800,y:0,width:480,height:450}});
  const controls=(patch:Record<string,unknown>={})=>bridge.controls({epoch:1,canClose:true,canRecord:true,captureBusy:false,recordBusy:false,recording:false,...patch});controls();tick();
  return {bridge,player,video,play,messages,controls,tick,blockedHtmlWrites:()=>blockedHtmlWrites,sample:()=>{for(const callback of mutations)callback();const callback=frame;frame=null;callback?.();},controlRect:(y:number,height=40)=>{controlRect=rect(720,y,40,height);},observePointer:(target:Element,patch:Record<string,unknown>={})=>callbacks.get(document)?.get('pointerdown')?.call(document,{isTrusted:true,button:0,target,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...patch} as unknown as Event),press:(kind:string,patch:Record<string,unknown>={})=>{const button=player.querySelector<HTMLButtonElement>(`[data-atsumi-live-action="${kind}"]`)!;callbacks.get(button)?.get('pointerdown')?.call(button,{isTrusted:true,button:0,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...patch} as unknown as Event);}};
}
afterEach(()=>{vi.restoreAllMocks();document.body.replaceChildren();document.head.querySelectorAll('style').forEach(el=>el.remove());});
for(const provider of ['youtube','soop'] as const){
  it(`${provider}: inserts record/camera before native fullscreen and close inside the same player`,async()=>{const f=await setup(provider);expect(f.player.querySelectorAll('[data-atsumi-live-action]')).toHaveLength(3);const full=f.player.querySelector(provider==='youtube'?'.ytp-fullscreen-button':'.btn_fullScreen_mode')!;expect(full.previousElementSibling).toHaveAttribute('data-atsumi-live-action','camera');expect(full.previousElementSibling?.previousElementSibling).toHaveAttribute('data-atsumi-live-action','record');expect(document.querySelector('#chatting_area [data-atsumi-live-action]')).toBeNull();});
  it(`${provider}: only trusted primary pointer action sends the closed native request`,async()=>{vi.useFakeTimers();try{const f=await setup(provider);f.messages.length=0;f.press('record',{isTrusted:false});f.press('record',{button:2});expect(f.messages).toHaveLength(0);f.press('record');const message=f.messages.at(-1)!;expect(message).toMatchObject({type:'ui_intent',action:'record_start',input:'pointer',epoch:1});f.press('camera');expect(f.messages).toHaveLength(1);f.bridge.reply(String(message.id),{ok:true,value:{}});await Promise.resolve();await Promise.resolve();}finally{vi.useRealTimers();}});
  it(`${provider}: resumes only exact visible bound live and never reloads`,async()=>{const f=await setup(provider);expect(f.bridge.resume(2)).toBe(false);expect(f.play).not.toHaveBeenCalled();expect(f.bridge.resume(1)).toBe(true);expect(f.play).toHaveBeenCalledOnce();f.bridge.layout({visible:false,player:null});expect(f.bridge.resume(1)).toBe(false);expect(f.play).toHaveBeenCalledOnce();});
  it(`${provider}: native projection disables close/capture without enabling chat authority`,async()=>{const f=await setup(provider);expect(f.controls({epoch:2})).toBe(false);f.controls({canClose:false,captureBusy:true,recordBusy:true});expect(f.player.querySelector('[data-atsumi-live-action="record"]')).toBeDisabled();expect(f.player.querySelector('[data-atsumi-live-action="camera"]')).toBeDisabled();expect(f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="close"]')!.hidden).toBe(true);});
  it(`${provider}: centers the 72px close on video and clamps it to a small video without changing permission`,async()=>{
    const f=await setup(provider),close=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="close"]')!;
    expect(close.style.width).toBe('72px');expect(close.style.height).toBe('72px');expect(close.style.left).toBe('50%');expect(close.style.right).toBe('');expect(close.style.top).toBe('12px');expect(close.style.transform).toBe('translateX(-50%)');
    expect(close.querySelector('svg')?.getAttribute('width')).toBe('40');
    f.player.getBoundingClientRect=()=>({x:0,y:0,left:0,top:0,right:240,bottom:64,width:240,height:64,toJSON:()=>({})});f.controls();
    expect(close.style.width).toBe('40px');expect(close.style.height).toBe('40px');
    f.controls({canClose:false});expect(close.hidden).toBe(true);
    f.player.getBoundingClientRect=()=>({x:0,y:0,left:0,top:0,right:20,bottom:20,width:20,height:20,toJSON:()=>({})});f.controls();expect(close.hidden).toBe(true);
  });
}
it('YouTube creates all four glyphs without HTML assignments under Trusted Types enforcement',async()=>{
  const f=await setup('youtube',{rejectOwnedHtml:true});
  const record=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="record"]')!;
  const camera=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="camera"]')!;
  const close=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="close"]')!;
  const full=f.player.querySelector('.ytp-fullscreen-button')!;
  for(const [button,size,stroke] of [[record,'24','1.7'],[camera,'24','1.7'],[close,'40','2']] as const){
    const svg=button.querySelector('svg')!;
    expect(svg).not.toBeNull();expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg');
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24');expect(svg.getAttribute('width')).toBe(size);expect(svg.getAttribute('height')).toBe(size);
    expect(svg.getAttribute('fill')).toBe('none');expect(svg.getAttribute('stroke')).toBe('currentColor');expect(svg.getAttribute('stroke-width')).toBe(stroke);expect(svg.getAttribute('stroke-linecap')).toBe('round');expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.style.pointerEvents).toBe('none');
    expect(svg.style.padding).toBe(button===close?'':'0px');
    for(const shape of svg.children)expect(shape.namespaceURI).toBe('http://www.w3.org/2000/svg');
  }
  expect(record.querySelector('circle')?.getAttribute('r')).toBe('6');expect(record.querySelector('circle')?.getAttribute('fill')).toBe('currentColor');
  expect(camera.querySelector('path')?.getAttribute('d')).toBe('M3 7h5l2-3h4l2 3h5v13H3z');expect(camera.querySelector('circle')?.getAttribute('r')).toBe('4');
  expect(close.querySelector('path')?.getAttribute('d')).toBe('m6 6 12 12M18 6 6 18');expect(close.querySelector('svg')?.style.maxWidth).toBe('60%');expect(close.querySelector('svg')?.style.maxHeight).toBe('60%');
  const initialGlyphs=[record,camera,close].map(button=>button.firstElementChild);
  f.tick();f.controls();
  expect([record,camera,close].map(button=>button.firstElementChild)).toEqual(initialGlyphs);
  f.controls({recording:true,canRecord:false});
  expect(record.querySelector('circle')).toBeNull();expect(record.querySelector('rect')?.getAttribute('width')).toBe('12');expect(record.querySelector('rect')?.getAttribute('rx')).toBe('1');expect(record.querySelector('rect')?.getAttribute('fill')).toBe('currentColor');
  expect(record.dataset.icon).toBe('stop');expect(record).toHaveAttribute('aria-label','녹화 중지');expect(record).toHaveAttribute('aria-pressed','true');expect(record.disabled).toBe(false);expect(record.style.color).toBe('rgb(255, 78, 69)');
  f.controls();expect(record.querySelector('circle')).not.toBeNull();expect(record.querySelector('rect')).toBeNull();expect(record.dataset.icon).toBe('record');
  expect(full.previousElementSibling).toBe(camera);expect(camera.previousElementSibling).toBe(record);expect(close.parentElement).toBe(f.player);
  for(const button of [record,camera,close])expect(button.querySelectorAll('svg')).toHaveLength(1);
  expect(f.blockedHtmlWrites()).toBe(0);
});
it('YouTube retries missing or uncommitted SVGs instead of caching an empty control',async()=>{
  const f=await setup('youtube',{rejectOwnedHtml:true}),record=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="record"]')!;
  record.replaceChildren();expect(record.dataset.icon).toBe('record');f.controls();expect(record.querySelector('circle')).not.toBeNull();
  vi.spyOn(record,'replaceChildren').mockImplementationOnce(()=>{throw new Error('fixture SVG commit failure');});
  expect(()=>f.controls({recording:true})).toThrow('fixture SVG commit failure');expect(record.dataset.icon).toBe('record');expect(record.querySelector('circle')).not.toBeNull();
  f.controls({recording:true});expect(record.dataset.icon).toBe('stop');expect(record.querySelector('rect')).not.toBeNull();expect(record.querySelectorAll('svg')).toHaveLength(1);expect(f.blockedHtmlWrites()).toBe(0);
});
it('YouTube SVG controls preserve native fade and permission checks while HTML assignments are blocked',async()=>{
  const f=await setup('youtube',{rejectOwnedHtml:true}),close=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="close"]')!;
  const chrome=f.player.querySelector<HTMLElement>('.ytp-chrome-bottom')!;
  expect(close.hidden).toBe(false);chrome.style.opacity='0.4';f.tick();expect(close.style.opacity).toBe('0.4');expect(close.hidden).toBe(false);
  chrome.style.opacity='0';f.tick();expect(close.hidden).toBe(true);expect(close.style.pointerEvents).toBe('none');
  chrome.style.opacity='1';f.tick();expect(close.hidden).toBe(false);expect(close.style.opacity).toBe('1');
  f.controls({canClose:false,captureBusy:true,recordBusy:true});expect(close.hidden).toBe(true);expect(close).toBeDisabled();expect(f.player.querySelector('[data-atsumi-live-action="record"]')).toBeDisabled();expect(f.player.querySelector('[data-atsumi-live-action="camera"]')).toBeDisabled();
  f.messages.length=0;f.press('record');f.press('camera');f.press('close');expect(f.messages).toHaveLength(0);
  expect(f.controls({epoch:2})).toBe(false);expect(close.hidden).toBe(true);expect(f.blockedHtmlWrites()).toBe(0);
});
it('SOOP record uses the CHZZK ring/idle-circle and ring/stop-square without moving its slot',async()=>{
  const f=await setup('soop'),record=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="record"]')!;
  const style=record.style.cssText;
  expect(record.querySelector('circle[r="8"]')?.getAttribute('stroke-width')).toBe('1.4');
  expect(record.querySelector('circle[r="4.5"]')?.getAttribute('fill')).toBeNull();
  expect(record.querySelector('svg')?.getAttribute('stroke-width')).toBe('1.8');
  f.controls({recording:true,canRecord:false});expect(record.disabled).toBe(false);
  expect(record.querySelector('circle[r="8"]')).not.toBeNull();expect(record.querySelector('circle[r="4.5"]')).toBeNull();
  expect(record.querySelector('rect')?.getAttribute('width')).toBe('8');expect(record.style.color).toBe('rgb(255, 91, 112)');
  expect(record.style.cssText.replace('rgb(255, 91, 112)','rgb(255, 255, 255)')).toBe(style);
});
it('SOOP only hides the official chat header close, not chat dialogs',async()=>{await setup('soop');expect(getComputedStyle(document.getElementById('setbox_close')!).display).toBe('none');expect(getComputedStyle(document.querySelector('.chat_layer .close')!).display).not.toBe('none');});
it('SOOP video X follows the official controls sliding outside the player even when opacity remains one',async()=>{
  const f=await setup('soop'),close=f.player.querySelector<HTMLButtonElement>('[data-atsumi-live-action="close"]')!;
  f.sample();expect(close.hidden).toBe(false);expect(close.style.opacity).toBe('1');
  f.controlRect(450);f.sample();expect(close.hidden).toBe(true);expect(close.style.pointerEvents).toBe('none');
  f.controlRect(430);f.sample();expect(close.hidden).toBe(false);expect(close.style.opacity).toBe('0.5');
  f.controlRect(410);f.sample();expect(close.style.opacity).toBe('1');
  (f.player.querySelector('.player_ctrlBox') as HTMLElement).style.visibility='hidden';f.sample();expect(close.hidden).toBe(true);
  (f.player.querySelector('.player_ctrlBox') as HTMLElement).style.visibility='visible';f.controlRect(410,0);f.sample();expect(close.hidden).toBe(true);
});
it('SOOP pointer diagnostics are bounded closed categories and never request an action',async()=>{
  const f=await setup('soop');f.messages.length=0;
  const camera=f.player.querySelector('[data-atsumi-live-action="camera"]')!;
  f.observePointer(camera,{isTrusted:false});f.observePointer(camera,{button:2});f.observePointer(document.getElementById('chatting_area')!);expect(f.messages).toHaveLength(0);
  for(const kind of ['record','camera','close'])f.observePointer(f.player.querySelector(`[data-atsumi-live-action="${kind}"]`)!);
  for(let n=0;n<20;n++)f.observePointer(f.video);
  expect(f.messages).toHaveLength(16);expect(f.messages.slice(0,3).map(message=>message.target)).toEqual(['record','camera','close']);
  for(const message of f.messages){expect(Object.keys(message).sort()).toEqual(['epoch','id','target','type']);expect(message.type).toBe('ui_pointer');expect(['record','camera','close','player_other']).toContain(message.target);}
});
