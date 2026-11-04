import {afterEach,expect,it,vi} from 'vitest';
import script from '../../../src-tauri/src/x_surface_video.js?raw';
type Handler=(e:Record<string,unknown>)=>void;
function setup(full=false) {
 const doc=new DOMParser().parseFromString('<body><div data-testid="videoPlayer" id="one"><div data-testid="videoComponent"><video id="v1"></video><div id="background"></div><a href="/source/status/123456" id="source">Source</a><button aria-label="Pause" id="pause"></button><button aria-label="Play" id="play"></button><button aria-label="Fullscreen" id="full"></button><div role="slider" id="slider"></div></div></div><input id="editing"><button id="other">Other</button></body>','text/html');
 const handlers:Record<string,Handler[]>={};
 const original=doc.addEventListener.bind(doc);
 vi.spyOn(doc,'addEventListener').mockImplementation(((name:string,handler:EventListener,options?:boolean)=>{(handlers[name]??=[]).push(handler as unknown as Handler);original(name,handler,options);}) as typeof doc.addEventListener);
 vi.spyOn(window,'addEventListener').mockImplementation(((name:string,handler:EventListener)=>{(handlers[name]??=[]).push(handler as unknown as Handler);}) as typeof window.addEventListener);
 const location={hostname:'x.com',protocol:'https:',pathname:full?'/author/status/123456/mediaviewer':'/home'};
 Object.defineProperty(location,'href',{get:()=>location.protocol+'//'+location.hostname+location.pathname});
 const allowed={value:true};const claim=vi.fn(()=>allowed.value);
 const api={videoInteractionReady:()=>allowed.value,videoInput:claim,sourceMediaViewerAvailable:()=>true,videoSourceTarget:(n:Element)=>n.closest('a')};
 Object.defineProperty(window,'__atsumiXSurface',{value:api,configurable:true});
 const media={time:10,paused:false,muted:false,volume:.5};
 const video=doc.querySelector('video')!;
 Object.defineProperties(video,{currentTime:{get:()=>media.time,set:v=>media.time=v},duration:{value:60},paused:{get:()=>media.paused},ended:{value:false},muted:{get:()=>media.muted,set:v=>media.muted=v},volume:{get:()=>media.volume,set:v=>media.volume=v}});
 const pause=vi.fn(()=>{media.paused=true;});const play=vi.fn(()=>{media.paused=false;});
 doc.querySelector('#pause')!.addEventListener('click',pause);doc.querySelector('#play')!.addEventListener('click',play);
 for(const el of doc.querySelectorAll('*'))vi.spyOn(el,'getBoundingClientRect').mockReturnValue({x:0,y:0,top:0,left:0,right:500,bottom:400,width:500,height:400,toJSON:()=>({})});
 Object.defineProperty(doc,'elementFromPoint',{value:()=>doc.querySelector('#background')});
 new Function('window','document','location','getComputedStyle','innerWidth','innerHeight',script.replace('__ATSUMI_X_VIDEO__','{}'))(window,doc,location,(el:HTMLElement)=>({display:el.hidden?'none':'block',visibility:'visible',opacity:'1'}),900,700);
 const fire=(type:string,selector='#background',extra:Record<string,unknown>={})=>{
   const event={type,isTrusted:true,defaultPrevented:false,button:0,target:doc.querySelector(selector),preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),...extra};
   handlers[type]?.forEach(handler=>handler(event));return event;
 };
 return {doc,video,media,api,claim,location,fire,pause,play,allowed};
}
afterEach(()=>{vi.restoreAllMocks();vi.useRealTimers();delete (window as unknown as Record<string,unknown>).__atsumiXSurface;delete (window as unknown as Record<string,unknown>).__atsumiXVideo;});
it('toggles inline playback without running the native background route',()=>{
 const a=setup(),navigate=vi.fn();a.doc.querySelector('#background')!.addEventListener('click',navigate);
 expect(a.fire('mousedown').preventDefault).toHaveBeenCalledOnce();expect(a.fire('mouseup').preventDefault).toHaveBeenCalledOnce();
 expect(a.fire('click').preventDefault).toHaveBeenCalledOnce();expect(a.pause).toHaveBeenCalledOnce();expect(a.location.pathname).toBe('/home');expect(navigate).not.toHaveBeenCalled();
 a.fire('click');expect(a.play).toHaveBeenCalledOnce();
});
it('keeps source links, progress, play controls and untrusted/modified clicks untouched',()=>{
 const a=setup();
 for(const selector of ['#source','#slider','#play','#full','#editing'])if(selector!=='#full')expect(a.fire('click',selector).preventDefault).not.toHaveBeenCalled();
 for(const extra of [{isTrusted:false},{ctrlKey:true},{shiftKey:true},{altKey:true},{button:2}])expect(a.fire('click','#background',extra).preventDefault).not.toHaveBeenCalled();
 a.allowed.value=false;expect(a.fire('click').preventDefault).not.toHaveBeenCalled();expect(a.pause).not.toHaveBeenCalled();
});
it('uses the original route entry only when fullscreen is explicitly selected',()=>{
 const a=setup();a.doc.querySelector('#background')!.addEventListener('mouseup',()=>{a.location.pathname='/author/status/123456/mediaviewer';});
 const e=a.fire('click','#full');expect(e.preventDefault).toHaveBeenCalledOnce();expect(a.location.pathname).toContain('/mediaviewer');expect(a.claim).toHaveBeenCalledOnce();
});
it('seeks and clamps only the fullscreen video, and supports pause, mute and volume',()=>{
 vi.useFakeTimers();const a=setup(true);
 a.fire('keydown','body',{key:'a'});expect(a.media.time).toBe(5);
 a.fire('keydown','body',{key:'ArrowRight'});expect(a.media.time).toBe(10);
 a.media.time=59;a.fire('keydown','body',{key:'d'});expect(a.media.time).toBe(60);
 a.media.time=1;a.fire('keydown','body',{key:'ArrowLeft'});expect(a.media.time).toBe(0);
 a.fire('keydown','body',{key:' '});expect(a.media.paused).toBe(true);
 a.fire('keydown','body',{key:' ',repeat:true});expect(a.media.paused).toBe(true);
 a.fire('keydown','body',{key:'m'});expect(a.media.muted).toBe(true);
 a.fire('keydown','body',{key:'m',repeat:true});expect(a.media.muted).toBe(true);
 a.fire('keydown','body',{key:'ArrowUp'});expect(a.media.volume).toBeCloseTo(.55);
});
it('preserves input editing, sliders, button activation, composition and pane revocation',()=>{
 const a=setup(true);
 for(const selector of ['#editing','#slider'])expect(a.fire('keydown',selector,{key:'ArrowRight'}).preventDefault).not.toHaveBeenCalled();
 expect(a.fire('keydown','#other',{key:' '}).preventDefault).not.toHaveBeenCalled();
 for(const extra of [{isComposing:true},{ctrlKey:true},{isTrusted:false}])expect(a.fire('keydown','body',{key:'d',...extra}).preventDefault).not.toHaveBeenCalled();
 a.allowed.value=false;expect(a.fire('keydown','body',{key:'d'}).preventDefault).not.toHaveBeenCalled();expect(a.media.time).toBe(10);
 const b=setup();expect(b.fire('keydown','body',{key:'d'}).preventDefault).not.toHaveBeenCalled();
});
it('chooses the clicked player when a post has two videos',()=>{
 const a=setup();const second=a.doc.querySelector('#one')!.cloneNode(true) as HTMLElement;second.id='two';a.doc.body.append(second);
 const v=second.querySelector('video')!;vi.spyOn(v,'getBoundingClientRect').mockReturnValue(a.video.getBoundingClientRect());
 Object.defineProperty(v,'paused',{value:false});const pause=vi.fn();second.querySelector('#pause')!.addEventListener('click',pause);
 a.fire('click','#two #background');expect(pause).toHaveBeenCalledOnce();expect(a.pause).not.toHaveBeenCalled();expect(a.claim).toHaveBeenCalledWith(expect.anything(),v);
});
it('fades only native controls after motion, retaining the video and attribution',()=>{
 vi.useFakeTimers();const a=setup(true),bar=a.doc.createElement('div');bar.innerHTML='<div data-testid="scrubber"></div><button aria-label="Play"></button><button></button>';a.doc.querySelector('[data-testid="videoComponent"]')!.append(bar);
 a.fire('mousemove');expect(bar.closest('[data-atsumi-video-awake]')).not.toBeNull();
 vi.advanceTimersByTime(2500);expect(bar.closest('[data-atsumi-video-awake]')).toBeNull();
 expect(bar.hasAttribute('data-atsumi-video-controls')).toBe(true);expect(a.doc.querySelector('#source')!.hasAttribute('data-atsumi-video-controls')).toBe(false);
});


it('handles the native immersive sibling-controls card without inline wrappers',()=>{
 vi.useFakeTimers();const a=setup(true),root=a.doc.querySelector('#one')!,component=root.querySelector('[data-testid="videoComponent"]')!;
 root.removeAttribute('data-testid');component.removeAttribute('data-testid');
 const bar=a.doc.createElement('div');bar.append(a.doc.querySelector('#play')!,a.doc.querySelector('#pause')!,a.doc.querySelector('#slider')!);bar.querySelector('#slider')!.setAttribute('aria-label','스크러버');root.append(bar);
 const mute=a.doc.createElement('button');mute.setAttribute('aria-label','Mute');const onMute=vi.fn(()=>{a.media.muted=!a.media.muted;});mute.addEventListener('click',onMute);root.append(mute);
 a.fire('click','#background',{clientX:200,clientY:200});expect(a.pause).toHaveBeenCalledOnce();
 expect(a.fire('click','#background',{clientX:200,clientY:600}).preventDefault).not.toHaveBeenCalled();
 a.fire('keydown','body',{key:'m'});expect(onMute).toHaveBeenCalledOnce();expect(a.media.muted).toBe(true);
 a.fire('keydown','body',{key:' '});expect(a.play).toHaveBeenCalledOnce();
 a.fire('mousemove');expect(bar.hasAttribute('data-atsumi-video-controls')).toBe(true);expect(root.hasAttribute('data-atsumi-video-awake')).toBe(true);
 vi.advanceTimersByTime(2500);expect(root.hasAttribute('data-atsumi-video-awake')).toBe(false);
});

it('toggles the current native video even when immersive controls are unmounted',()=>{
 const a=setup(true),root=a.doc.querySelector('#one')!,component=root.firstElementChild!;
 root.removeAttribute('data-testid');component.removeAttribute('data-testid');
 for(const control of root.querySelectorAll('button,[role="slider"]'))control.remove();
 const pause=vi.spyOn(a.video,'pause').mockImplementation(()=>{a.media.paused=true;});
 const play=vi.spyOn(a.video,'play').mockImplementation(async()=>{a.media.paused=false;});
 const sibling=root.cloneNode(true) as HTMLElement;sibling.id='next';a.doc.body.append(sibling);
 for(const node of sibling.querySelectorAll('*'))vi.spyOn(node,'getBoundingClientRect').mockReturnValue({x:0,y:900,top:900,left:0,right:500,bottom:1300,width:500,height:400,toJSON:()=>({})});
 a.fire('click','#background',{clientX:200,clientY:200});expect(pause).toHaveBeenCalledOnce();
 a.fire('click','#background',{clientX:200,clientY:200});expect(play).toHaveBeenCalledOnce();
 a.fire('keydown','body',{key:' '});expect(pause).toHaveBeenCalledTimes(2);
 expect(a.claim).toHaveBeenLastCalledWith(expect.anything(),a.video);
});

it('wakes the original immersive HUD on motion and hides it after idle without playing or reacting',()=>{
 vi.useFakeTimers();const a=setup(true),root=a.doc.querySelector('#one')!,component=root.firstElementChild!;
 for(const c of root.querySelectorAll('button,[role="slider"]'))c.remove();
 const overlay=a.doc.createElement('div');overlay.id='hudOverlay';overlay.setAttribute('data-testid','immersive-video-controls-123456');component.append(overlay);
 let hidden=true;const toggle=vi.fn(()=>{hidden=!hidden;root.querySelector('[data-testid="immersive-tweet-ui-content-container-123456"]')?.remove();if(!hidden){const hud=a.doc.createElement('div');hud.setAttribute('data-testid','immersive-tweet-ui-content-container-123456');root.append(hud);}});overlay.addEventListener('click',toggle);
 a.fire('mousemove','#hudOverlay');expect(toggle).toHaveBeenCalledOnce();expect(hidden).toBe(false);
 a.fire('mousemove','#hudOverlay');expect(toggle).toHaveBeenCalledOnce();
 vi.advanceTimersByTime(2500);expect(hidden).toBe(true);expect(toggle).toHaveBeenCalledTimes(2);
 a.fire('mousemove','#hudOverlay');expect(hidden).toBe(true); // avoid toggling twice during source mount/fade
 vi.advanceTimersByTime(400);a.fire('mousemove','#hudOverlay');expect(hidden).toBe(false);
 expect(a.pause).not.toHaveBeenCalled();expect(a.play).not.toHaveBeenCalled();
});
it('never synthesizes an immersive control click on unknown or interactive overlays',()=>{
 const a=setup(true),root=a.doc.querySelector('#one')!,overlay=a.doc.createElement('div');overlay.setAttribute('data-testid','immersive-video-controls-123456');overlay.innerHTML='<button>Like</button>';root.firstElementChild!.append(overlay);const click=vi.fn();overlay.addEventListener('click',click);
 a.fire('mousemove');expect(click).not.toHaveBeenCalled();
 overlay.innerHTML='';overlay.setAttribute('data-testid','immersive-video-controls-unknown');a.fire('mousemove');expect(click).not.toHaveBeenCalled();
});

it('redirects Space from the exact original fullscreen entry after the native route opens',()=>{
 const a=setup();a.doc.querySelector('#background')!.addEventListener('mouseup',()=>{a.location.pathname='/author/status/123456/mediaviewer';});
 a.fire('click','#full');
 const e=a.fire('keydown','#full',{key:' '});
 expect(e.preventDefault).toHaveBeenCalledOnce();expect(a.pause).toHaveBeenCalledOnce();expect(a.location.pathname).toContain('/mediaviewer');
 expect(a.fire('keydown','#other',{key:' '}).preventDefault).not.toHaveBeenCalled();
});
function delayedMute(){
 const a=setup(true),root=a.doc.querySelector('#one')!;
 for(const control of root.querySelectorAll('button,[role="slider"]'))control.remove();
 const overlay=a.doc.createElement('div');overlay.id='nativeOverlay';overlay.setAttribute('data-testid','immersive-video-controls-123456');root.firstElementChild!.append(overlay);
 const mute=vi.fn(()=>{a.media.muted=!a.media.muted;});
 overlay.addEventListener('click',()=>{setTimeout(()=>{const hud=a.doc.createElement('div');hud.setAttribute('data-testid','immersive-tweet-ui-content-container-123456');const button=a.doc.createElement('button');button.setAttribute('aria-label','Mute');button.addEventListener('click',mute);hud.append(button);root.append(hud);},80);});
 return {...a,mute};
}
it('waits for the native mute control after hidden HUD mounting rather than mutating media early',()=>{
 vi.useFakeTimers();const a=delayedMute();a.fire('keydown','body',{key:'m'});
 expect(a.media.muted).toBe(false);vi.advanceTimersByTime(100);
 expect(a.mute).toHaveBeenCalledOnce();expect(a.media.muted).toBe(true);
 a.fire('keydown','body',{key:'m'});expect(a.mute).toHaveBeenCalledTimes(2);expect(a.media.muted).toBe(false);
});
it('preserves rapid mute-toggle parity while the native controls mount',()=>{
 vi.useFakeTimers();const a=delayedMute();a.fire('keydown','body',{key:'m'});a.fire('keydown','body',{key:'m'});vi.advanceTimersByTime(100);
 expect(a.mute).not.toHaveBeenCalled();expect(a.media.muted).toBe(false);
});
it.each(['route','owner','video'] as const)('cancels deferred mute on %s change',kind=>{
 vi.useFakeTimers();const a=delayedMute();a.fire('keydown','body',{key:'m'});
 if(kind==='route')a.location.pathname='/author/status/654321/mediaviewer';
 if(kind==='owner')a.allowed.value=false;
 if(kind==='video')a.video.remove();
 vi.advanceTimersByTime(500);expect(a.mute).not.toHaveBeenCalled();expect(a.media.muted).toBe(false);
});

it('ignores stale inline fullscreen focus when a native viewer card is mounted over the feed',()=>{
 const a=setup(true),native=a.doc.createElement('div');native.id='native';native.innerHTML='<video></video><div data-testid="immersive-video-controls-654321"></div><button aria-label="Pause"></button>';
 a.doc.body.append(native);const video=native.querySelector('video')!;
 for(const node of native.querySelectorAll('*'))vi.spyOn(node,'getBoundingClientRect').mockReturnValue({x:0,y:0,top:0,left:0,right:900,bottom:700,width:900,height:700,toJSON:()=>({})});
 Object.defineProperties(video,{paused:{value:false},ended:{value:false}});
 const pause=vi.fn();native.querySelector('button')!.addEventListener('click',pause);
 const e=a.fire('keydown','#full',{key:' '});
 expect(e.preventDefault).toHaveBeenCalledOnce();expect(pause).toHaveBeenCalledOnce();expect(a.pause).not.toHaveBeenCalled();expect(a.claim).toHaveBeenLastCalledWith(expect.anything(),video);
});

it('uses Space for playback when native immersive back button receives initial focus',()=>{
 const a=setup(true),back=a.doc.createElement('button');back.id='nativeBack';back.setAttribute('data-testid','immersive-tweet-back-button-123456');back.setAttribute('aria-label','뒤로');a.doc.querySelector('#one')!.append(back);
 const close=vi.fn();back.addEventListener('click',close);const e=a.fire('keydown','#nativeBack',{key:' '});
 expect(e.preventDefault).toHaveBeenCalledOnce();expect(a.pause).toHaveBeenCalledOnce();expect(close).not.toHaveBeenCalled();
 expect(a.fire('keydown','#nativeBack',{key:'Enter'}).preventDefault).not.toHaveBeenCalled();
});
