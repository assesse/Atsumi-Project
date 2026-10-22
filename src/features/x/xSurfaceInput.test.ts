import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_surface_input.js?raw';
import actionsScript from '../../../src-tauri/src/x_surface_actions.js?raw';

const image='<img src="https://pbs.twimg.com/media/Fixture?format=jpg">';
const buttons='<button aria-label="Previous" id="previous"></button><button aria-label="Next" id="next"></button><button aria-label="Close" id="close"></button>';
function setup(html='<div role="dialog">'+image+buttons+'</div>',feed='preview',path='/creator/status/123456/photo/1',withActions=false){
  const doc=new DOMParser().parseFromString(html,'text/html');
  const handlers:Record<string,((event:unknown)=>void)[]>={};
  const original=doc.addEventListener.bind(doc);
  vi.spyOn(doc,'addEventListener').mockImplementation(((type:string,listener:EventListener)=>{(handlers[type]??=[]).push(listener as (event:unknown)=>void);original(type,listener);}) as typeof doc.addEventListener);
  vi.spyOn(Element.prototype,'getClientRects').mockImplementation(function(this:Element){return (this.hasAttribute('data-zero-size')?[]:[{width:30,height:30}]) as unknown as DOMRectList;});
  const message=vi.fn();
  const win={top:null as unknown,chrome:{webview:{postMessage:message}},getComputedStyle:window.getComputedStyle.bind(window),__atsumiXInput:undefined as undefined|{visibility(value:boolean):void;key(value:string):boolean},__atsumiXSurface:undefined as undefined|{dismissMenu?():boolean;archivePhotoOwner?():string},__atsumiXActions:undefined as undefined|{visibility(value:boolean):void;update():void;ownsPhotoControl(node:Element):boolean}};
  win.top=win;
  const location={origin:'https://x.com',href:'https://x.com'+path,protocol:'https:',hostname:'x.com',pathname:path};
  if(withActions){
    vi.spyOn(Element.prototype,'getBoundingClientRect').mockReturnValue({x:20,y:20,left:20,top:20,right:620,bottom:620,width:600,height:600,toJSON:()=>({})});
    win.__atsumiXSurface={archivePhotoOwner:()=> 'alice'};
    new Function('window','document','location','URL','Element','getComputedStyle','innerWidth','innerHeight',actionsScript.replace('__ATSUMI_X_ACTIONS__',JSON.stringify({nonce:'fixture',feed,owner:'alice'})))(
      win,doc,location,URL,Element,win.getComputedStyle,1000,1000,
    );
    win.__atsumiXActions!.visibility(true);
  }
  new Function('window','document','location','URL','Element',script.replace('__ATSUMI_X_INPUT__',JSON.stringify({nonce:'fixture',feed,owner:'alice'})))(win,doc,location,URL,Element);
  win.__atsumiXInput!.visibility(true);
  const press=(key:string,target:Element=doc.body,extra={})=>{
    let stopped=false;
    const event={key,target,isTrusted:true,defaultPrevented:false,button:0,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(()=>{stopped=true;}),...extra};
    for(const handler of handlers.keydown??[]){handler(event);if(stopped)break;}return event;
  };
  const messages=()=>message.mock.calls.map(([raw])=>{const url=new URL(raw.slice('ATSUMI_X_SURFACE_V1:'.length));return [url.searchParams.get('kind'),url.searchParams.get('value')];});
  return {doc,win,location,message,messages,press,handlers};
}
afterEach(()=>vi.restoreAllMocks());

it('sends a closed focus claim only for trusted visible pointer input',()=>{
  const a=setup();
  a.handlers.pointerdown![0]!({isTrusted:true,button:0});expect(a.messages()).toEqual([['focus','']]);
  for(const event of [{isTrusted:false,button:0},{isTrusted:true,button:1},{isTrusted:true,button:0,ctrlKey:true}])a.handlers.pointerdown![0]!(event);
  expect(a.message).toHaveBeenCalledOnce();a.win.__atsumiXInput!.visibility(false);
  a.handlers.pointerdown![0]!({isTrusted:true,button:0});expect(a.message).toHaveBeenCalledOnce();
});
it('right-click claims the source pane and its local context menu Escape never reaches a previously focused preview',()=>{
  const a=setup('<article>Feed</article>','foryou','/home');
  a.handlers.pointerdown![0]!({isTrusted:true,button:2});
  a.handlers.contextmenu![0]!({isTrusted:true,button:2});
  const dismissMenu=vi.fn(()=>true);a.win.__atsumiXSurface={dismissMenu};
  const event=a.press('Escape');
  expect(event.preventDefault).toHaveBeenCalledOnce();expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
  expect(dismissMenu).toHaveBeenCalledOnce();expect(a.messages()).toEqual([['focus',''],['focus',''],['focus','']]);
});
it('forwards Escape for last-pane arbitration instead of closing its own preview eagerly',()=>{
  const a=setup('<article>Ordinary preview</article>','preview','/creator/status/123456');
  const event=a.press('Escape');
  expect(event.preventDefault).toHaveBeenCalledOnce();expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
  expect(a.messages()).toEqual([['key','Escape']]);
});
it('consumes A/D and arrows once and maps them to two closed key values',()=>{
  const a=setup();
  for(const key of ['a','A','ArrowLeft','d','D','ArrowRight']){
    const event=a.press(key);expect(event.preventDefault).toHaveBeenCalledOnce();expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
  }
  expect(a.messages()).toEqual([['key','ArrowLeft'],['key','ArrowLeft'],['key','ArrowLeft'],['key','ArrowRight'],['key','ArrowRight'],['key','ArrowRight']]);
  const repeat=a.press('ArrowRight',a.doc.body,{repeat:true});expect(repeat.preventDefault).toHaveBeenCalledOnce();expect(a.message).toHaveBeenCalledTimes(6);
});
it('clicks exactly one original semantic photo control and does not synthesize another key',()=>{
  const a=setup();
  const previous=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#previous')!,'click');
  const next=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#next')!,'click');
  expect(a.win.__atsumiXInput!.key('ArrowLeft')).toBe(true);expect(previous).toHaveBeenCalledOnce();
  expect(a.win.__atsumiXInput!.key('ArrowRight')).toBe(true);expect(next).toHaveBeenCalledOnce();
  expect(a.message).not.toHaveBeenCalled();
});
it('keeps photo shortcuts working after an original previous/next or autofocus close button holds keyboard focus',()=>{
  const a=setup();
  for(const id of ['previous','next','close']){
    expect(a.press('d',a.doc.querySelector('#'+id)!).preventDefault).toHaveBeenCalledOnce();
  }
  expect(a.messages()).toEqual([['key','ArrowRight'],['key','ArrowRight'],['key','ArrowRight']]);
});
it('keeps A/D on the exact current photo-download control after its menu closes without taking menu keys',()=>{
  const a=setup(undefined,'preview','/creator/status/123456/photo/1',true);
  const download=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
  expect(download).not.toBeNull();expect(a.win.__atsumiXActions!.ownsPhotoControl(download)).toBe(true);
  const next=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#next')!,'click');
  const previous=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#previous')!,'click');
  expect(a.press('d',download).preventDefault).toHaveBeenCalledOnce();
  a.win.__atsumiXInput!.key('ArrowRight');expect(next).toHaveBeenCalledOnce();
  // The actions helper is registered first and owns Escape while its menu is open.
  const target=a.doc.querySelector('img')!;
  a.handlers.contextmenu![0]!({target,isTrusted:true,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),clientX:30,clientY:30});
  expect(a.doc.querySelector('[role="menu"]')).not.toBeNull();
  expect(a.win.__atsumiXActions!.ownsPhotoControl(download)).toBe(false);
  expect(a.press('d',download).preventDefault).not.toHaveBeenCalled();
  expect(a.press('a',a.doc.querySelector('[role="menuitem"]')!).preventDefault).not.toHaveBeenCalled();
  a.press('Escape',download);expect(a.doc.querySelector('[role="menu"]')).toBeNull();
  expect(a.press('a',download).preventDefault).toHaveBeenCalledOnce();
  a.win.__atsumiXInput!.key('ArrowLeft');expect(previous).toHaveBeenCalledOnce();
  expect(a.messages().filter(([kind])=>kind==='key')).toEqual([['key','ArrowRight'],['key','ArrowLeft']]);
});
it('never admits spoofed, detached, changed or hidden photo-download controls for navigation',()=>{
  for(const change of ['spoofed','detached','media','route','hidden']){
    const a=setup(undefined,'preview','/creator/status/123456/photo/1',true);
    let target=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
    if(change==='spoofed'){const clone=target.cloneNode(true) as HTMLButtonElement;a.doc.body.append(clone);target=clone;}
    else if(change==='detached')target.remove();
    else if(change==='media')a.doc.querySelector('img')!.setAttribute('src','https://pbs.twimg.com/media/Changed?format=jpg');
    else if(change==='route')a.location.pathname='/creator/status/123456/photo/2';
    else a.win.__atsumiXActions!.visibility(false);
    expect(a.win.__atsumiXActions!.ownsPhotoControl(target)).toBe(false);
    expect(a.press('d',target).preventDefault).not.toHaveBeenCalled();
    expect(a.messages()).not.toContainEqual(['key','ArrowRight']);
  }
});
it('closes a feed photo before any outer preview and supports full-page photo close',()=>{
  for(const html of ['<div role="dialog">'+image+buttons+'</div>',image+buttons]){
    const a=setup(html,'foryou'),close=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#close')!,'click');
    expect(a.win.__atsumiXInput!.key('Escape')).toBe(true);expect(close).toHaveBeenCalledOnce();expect(a.message).not.toHaveBeenCalled();
  }
});
it('gives original fullscreen exit precedence over photo and outer preview',()=>{
  const a=setup(),exit=vi.fn().mockResolvedValue(undefined),close=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#close')!,'click');
  Object.defineProperty(a.doc,'fullscreenElement',{value:a.doc.querySelector('img')});Object.defineProperty(a.doc,'exitFullscreen',{value:exit});
  expect(a.win.__atsumiXInput!.key('Escape')).toBe(true);expect(exit).toHaveBeenCalledOnce();expect(close).not.toHaveBeenCalled();expect(a.message).not.toHaveBeenCalled();
});
it('preserves a native dialog boundary even when no unique close control is known',()=>{
  for(const html of ['<div role="dialog"><p>Native dialog</p></div>','<div role="dialog"><button aria-label="Close"></button><button aria-label="Close"></button></div>']){
    const a=setup(html,'preview','/creator/status/123456');
    expect(a.win.__atsumiXInput!.key('Escape')).toBe(true);expect(a.message).not.toHaveBeenCalled();
  }
});
it('dismisses the app context menu first and leaves native menus to their own Escape handler',()=>{
  const local=setup('<article>Preview</article>','preview','/creator/status/123456'),dismissMenu=vi.fn(()=>true);
  local.win.__atsumiXSurface={dismissMenu};expect(local.win.__atsumiXInput!.key('Escape')).toBe(true);
  expect(dismissMenu).toHaveBeenCalledOnce();expect(local.message).not.toHaveBeenCalled();
  const native=setup('<div role="menu"><button>Source action</button></div>','preview','/creator/status/123456'),nativeKey=vi.fn();
  native.doc.addEventListener('keydown',nativeKey);
  expect(native.win.__atsumiXInput!.key('Escape')).toBe(true);expect(nativeKey).toHaveBeenCalledOnce();
  expect(nativeKey.mock.calls[0]![0].key).toBe('Escape');expect(native.message).not.toHaveBeenCalled();
});
it('signals outer close only after targeted ordinary preview Escape',()=>{
  const a=setup('<article>Preview</article>','preview','/creator/status/123456');
  expect(a.win.__atsumiXInput!.key('Escape')).toBe(true);expect(a.messages()).toEqual([['close','']]);
  const feed=setup('<article>Feed</article>','foryou','/home');
  expect(feed.win.__atsumiXInput!.key('Escape')).toBe(false);expect(feed.message).not.toHaveBeenCalled();
});
it('does not take composition, modified keys or consumed menu Escape',()=>{
  const a=setup();
  for(const extra of [{isTrusted:false},{isComposing:true},{keyCode:229},{ctrlKey:true},{altKey:true},{metaKey:true},{shiftKey:true},{defaultPrevented:true}]){
    expect(a.press('Escape',a.doc.body,extra).preventDefault).not.toHaveBeenCalled();
    expect(a.press('d',a.doc.body,extra).preventDefault).not.toHaveBeenCalled();
  }
  expect(a.message).not.toHaveBeenCalled();
});
it('leaves text editing, sliders, video and unrelated focused controls untouched',()=>{
  for(const html of ['<input>','<textarea></textarea>','<select></select>','<div contenteditable="true"></div>','<div role="textbox"></div>','<div role="slider"></div>','<button>Action</button>','<video></video>']){
    const a=setup('<div role="dialog">'+image+buttons+'</div>'+html);
    const target=a.doc.body.lastElementChild!;
    expect(a.press('ArrowRight',target).preventDefault).not.toHaveBeenCalled();expect(a.messages()).not.toContainEqual(['key','ArrowRight']);
  }
});
it('does not steal ordinary feed arrows or treat arbitrary image routes as native photos',()=>{
  for(const path of ['/home','/creator/status/123456','/creator/status/123456/photo/0','/creator/status/123456/photo/9','/creator/photo/1']){
    const a=setup(image+buttons,'foryou',path);
    expect(a.press('ArrowLeft').preventDefault).not.toHaveBeenCalled();expect(a.win.__atsumiXInput!.key('ArrowRight')).toBe(false);
  }
});
it('rejects hidden, disabled, zero-size, foreign-media or ambiguous photo controls',()=>{
  for(const extra of ['hidden','aria-hidden="true"','disabled','aria-disabled="true"','style="display:none"','data-zero-size']){
    const a=setup('<div role="dialog">'+image+'<button aria-label="Next" '+extra+'></button></div>');
    const click=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('button')!,'click');
    a.win.__atsumiXInput!.key('ArrowRight');expect(click).not.toHaveBeenCalled();
  }
  const ambiguous=setup('<div role="dialog">'+image+'<button aria-label="Next"></button><button aria-label="Next"></button></div>');
  const clicks=[...ambiguous.doc.querySelectorAll('button')].map(button=>vi.spyOn(button,'click'));
  ambiguous.win.__atsumiXInput!.key('ArrowRight');expect(clicks.every(click=>click.mock.calls.length===0)).toBe(true);
  const foreign=setup(image.replace('pbs.twimg.com','evil.test')+buttons);
  expect(foreign.press('d').preventDefault).not.toHaveBeenCalled();
});
it('recognizes bounded Korean/Japanese labels and exact native control IDs',()=>{
  for(const label of ['다음','다음 이미지','次へ','次の画像']){
    const a=setup('<div role="dialog">'+image+'<button aria-label="'+label+'"></button></div>'),click=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('button')!,'click');
    a.win.__atsumiXInput!.key('ArrowRight');expect(click).toHaveBeenCalledOnce();
  }
  const a=setup('<div role="dialog">'+image+'<button data-testid="carouselNext"></button></div>'),click=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('button')!,'click');
  a.win.__atsumiXInput!.key('ArrowRight');expect(click).toHaveBeenCalledOnce();
});
it('recognizes observed Korean slide buttons and exact English slide equivalents only in a native photo viewer',()=>{
  for(const [previous,next] of [['이전 슬라이드','다음 슬라이드'],['Previous slide','Next slide']]){
    const html='<div role="dialog">'+image+`<button id="previous" aria-label="${previous}"></button><button id="next" aria-label="${next}"></button></div>`;
    const a=setup(html),prevClick=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#previous')!,'click'),nextClick=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#next')!,'click');
    expect(a.press('d',a.doc.querySelector('#next')!).preventDefault).toHaveBeenCalledOnce();
    expect(a.press('a',a.doc.querySelector('#previous')!).preventDefault).toHaveBeenCalledOnce();
    expect(a.messages()).toEqual([['key','ArrowRight'],['key','ArrowLeft']]);
    a.win.__atsumiXInput!.key('ArrowRight');a.win.__atsumiXInput!.key('ArrowLeft');
    expect(nextClick).toHaveBeenCalledOnce();expect(prevClick).toHaveBeenCalledOnce();
    a.location.pathname='/home';expect(a.win.__atsumiXInput!.key('ArrowRight')).toBe(false);
    expect(nextClick).toHaveBeenCalledOnce();
  }
});
it('does not guess slide labels or choose between ambiguous native photo buttons',()=>{
  for(const html of [
    '<div role="dialog">'+image+'<button aria-label="다음 슬라이드"></button><button aria-label="Next slide"></button></div>',
    '<div role="dialog">'+image+'<button aria-label="다음 슬라이드 설명"></button></div>',
  ]){
    const a=setup(html),clicks=[...a.doc.querySelectorAll('button')].map(button=>vi.spyOn(button,'click'));
    a.win.__atsumiXInput!.key('ArrowRight');expect(clicks.every(click=>click.mock.calls.length===0)).toBe(true);
  }
});
it('cannot operate a hidden pane, security gate or arbitrary key',()=>{
  const a=setup(),click=vi.spyOn(a.doc.querySelector<HTMLButtonElement>('#next')!,'click');
  expect(a.win.__atsumiXInput!.key('Delete')).toBe(false);
  a.win.__atsumiXInput!.visibility(false);expect(a.win.__atsumiXInput!.key('ArrowRight')).toBe(false);
  a.win.__atsumiXInput!.visibility(true);a.doc.body.insertAdjacentHTML('beforeend','<input type="password">');
  expect(a.win.__atsumiXInput!.key('ArrowRight')).toBe(false);expect(a.win.__atsumiXInput!.key('Escape')).toBe(false);expect(click).not.toHaveBeenCalled();
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__TAURI|set_focus|\.focus\(/);
});
