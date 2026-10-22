import { afterEach, expect, it, vi } from "vitest";
import script from "../../../src-tauri/src/x_surface_actions.js?raw";
import inputScript from "../../../src-tauri/src/x_surface_input.js?raw";
import surfaceScript from "../../../src-tauri/src/x_surface.js?raw";

const image="https://pbs.twimg.com/media/Fixture?format=jpg&name=large";
const otherImage="https://pbs.twimg.com/media/Other?format=jpg&name=large";
const quoted='<div data-testid="quoteTweet" role="link"><div data-testid="User-Name"><a href="/quoted/status/654321"><time>quoted</time></a></div><p id="quote-body">Quote</p><div data-testid="tweetPhoto"><img id="quote-image" src="https://pbs.twimg.com/media/Quote?format=jpg"></div></div>';
const tweet=(extra="")=>'<article data-testid="tweet"><div data-testid="User-Name"><a id="profile" href="/creator">Creator</a><a id="permalink" href="/creator/status/123456"><time>today</time></a></div><p data-testid="tweetText">Saved post</p><div data-testid="tweetPhoto"><img id="image" src="'+image.replaceAll("&","&amp;")+'"></div>'+extra+'<div role="group"><button data-testid="like">Like</button><button data-testid="bookmark">Bookmark</button><button id="share">Share</button></div></article>';
type Envelope={kind:string;value:string};
type SearchOptions={searchEnabled?:unknown;searchCapabilities?:unknown};
const enabledSearch={searchEnabled:true,searchCapabilities:{image:true,video:true}};
const nativeAddEventListener=EventTarget.prototype.addEventListener;
function adapter(html=tweet(),options:{host?:string;subframe?:boolean;photoOwner?:string;path?:string;diagnostics?:boolean;hostTimers?:boolean}&SearchOptions={}) {
  const host=options.host??"x.com",doc=new DOMParser().parseFromString(html,"text/html"),timers:Array<()=>void>=[];
  const cancelledTimers=new Set<number>();let timerSerial=0;
  const base=doc.createElement("base");base.href="https://"+host+"/";doc.head.append(base);
  const listeners=new Map<EventTarget,Map<string,Array<(event:unknown)=>void>>>();
  vi.spyOn(EventTarget.prototype,"addEventListener").mockImplementation(function(this:EventTarget,name:string,handler:EventListenerOrEventListenerObject|null,options?:boolean|AddEventListenerOptions){
    if(typeof handler==="function"){
      let target=listeners.get(this);if(!target){target=new Map();listeners.set(this,target);}
      const group=target.get(name)??[];group.push(handler as (event:unknown)=>void);target.set(name,group);
    }
    return nativeAddEventListener.call(this,name,handler,options);
  });
  const path=options.path??"/home",message=vi.fn(),location={origin:"https://"+host,href:"https://"+host+path,pathname:path};
  const proof=vi.fn(()=>options.photoOwner??"");
  const windowEvents=new EventTarget();
  const win={chrome:{webview:{postMessage:message}},top:null as unknown,innerWidth:800,innerHeight:900,
    addEventListener:windowEvents.addEventListener.bind(windowEvents),
    __atsumiXSurface:{archivePhotoOwner:proof,dismissMenu:vi.fn()},__atsumiXActions:undefined as undefined|{visibility(value:boolean):void;update():void;dismissMenus():boolean;progress(id:string,value:unknown):void;receipt(id:string,value:unknown):void;result(id:string,state:string,message:string):void;ownsPhotoControl(node:unknown):boolean;photoDiagnostic():Record<string,unknown>|null;takeSearch(id:string):Record<string,unknown>|null;searchResult(id:string,value:unknown):void}};
  win.top=options.subframe?{}:win;
  let serial=0;
  const crypto={randomUUID:()=>`00000000-0000-4000-8000-${String(++serial).padStart(12,"0")}`};
  const computedStyle=(node:HTMLElement)=>({display:node.style.display||"block",visibility:node.style.visibility||"visible",opacity:node.style.opacity||"1",backgroundImage:node.style.backgroundImage||"none",position:node.style.position||"static",overflow:node.style.overflow||"visible",overflowX:node.style.overflowX||"",overflowY:node.style.overflowY||"",transform:node.style.transform,translate:node.style.translate,rotate:node.style.rotate,scale:node.style.scale,perspective:node.style.perspective,filter:node.style.filter,backdropFilter:node.style.backdropFilter,contain:node.style.contain,willChange:node.style.willChange,containerType:node.style.containerType,contentVisibility:node.style.contentVisibility,transformStyle:node.style.transformStyle});
  new Function("window","document","location","URL","Element","setTimeout","clearTimeout","innerWidth","innerHeight","crypto","getComputedStyle",script.replace("__ATSUMI_X_ACTIONS__",JSON.stringify({nonce:"fixture",diagnostics:options.diagnostics===true,searchEnabled:options.searchEnabled,searchCapabilities:options.searchCapabilities})))(
    win,doc,location,URL,Element,(f:()=>void,delay:number)=>{if(options.hostTimers)return setTimeout(f,delay);const id=++timerSerial;timers.push(()=>{if(!cancelledTimers.has(id))f();});return id;},
    (id:ReturnType<typeof setTimeout>)=>{if(options.hostTimers)clearTimeout(id);else cancelledTimers.add(Number(id));},800,900,crypto,computedStyle);
  const emit=(target:EventTarget,type:string,event:object)=>{for(const handler of listeners.get(target)?.get(type)??[])handler(event);};
  const event=(target:Element,extra:Record<string,unknown>={})=>({target,isTrusted:true,preventDefault:vi.fn(),stopPropagation:vi.fn(),stopImmediatePropagation:vi.fn(),clientX:120,clientY:200,...extra});
  const activate=(button:HTMLButtonElement,extra:Record<string,unknown>={})=>{const e=event(button,extra);button.onclick?.call(button,e as unknown as PointerEvent);return e;};
  const context=(target:Element,extra:Record<string,unknown>={})=>{const e=event(target,extra);emit(doc,"contextmenu",e);return e;};
  const messages=():Envelope[]=>message.mock.calls.map(([value])=>{const url=new URL(value.slice("ATSUMI_X_SURFACE_V1:".length));return{kind:url.searchParams.get("kind")!,value:url.searchParams.get("value")!};});
  const actions=()=>messages().filter(value=>value.kind==="archive").map(({value})=>JSON.parse(value));
  const visible=()=>win.__atsumiXActions?.visibility(true);
  const menuButton=(text:string)=>[...doc.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent===text)!;
  const railButton=(label:string)=>[...doc.querySelectorAll<HTMLButtonElement>('[data-expanded] button:not([hidden])')].find(button=>button.title===label)!;
  return {doc,win,location,message,messages,actions,emit,event,activate,context,visible,menuButton,railButton,timers,proof,windowEvents};
}
afterEach(()=>{vi.useRealTimers();vi.restoreAllMocks();});
const actualProgress=(sequence:number,overrides:Record<string,unknown>={})=>({sequence,stage:"downloading",processedMedia:0,totalMedia:2,downloadedBytes:4096,totalBytes:null,partial:false,currentTransfer:null,...overrides});
const mediaLines=(a:ReturnType<typeof adapter>)=>a.doc.querySelectorAll<HTMLElement>('.atsumi-x-media-host > .atsumi-x-media-progress,[data-atsumi-photo-progress]');
const firstMediaLine=(a:ReturnType<typeof adapter>)=>{const line=mediaLines(a)[0];if(!line)throw new Error('Expected exact fixture media progress');return line;};
const startAll=(a:ReturnType<typeof adapter>)=>{a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const action=a.actions().at(-1);if(!action)throw new Error('Expected exact fixture save submission');return action.id as string;};
const receipt=(a:ReturnType<typeof adapter>,id:string,state:string,progress:unknown=null,message='')=>a.win.__atsumiXActions!.receipt(id,{state,progress,message});
const timerRunner=(a:ReturnType<typeof adapter>)=>{let cursor=0;return(count=1)=>{for(let n=0;n<count;n++){const timer=a.timers[cursor++];if(!timer)throw new Error('Expected scheduled fixture timer');timer();}};};
const jobRow=(a:ReturnType<typeof adapter>,id:string)=>a.doc.querySelector<HTMLElement>(`.atsumi-x-save-row[data-request="${id}"]`)!;
const anotherPost=(index:number)=>tweet().replaceAll('/creator/status/123456',`/creator/status/${123456+index}`).replaceAll(' id="',' data-fixture-id="');
const showSearchTarget=(node:Element)=>vi.spyOn(node,'getBoundingClientRect').mockReturnValue({x:20,y:30,left:20,top:30,right:420,bottom:330,width:400,height:300,toJSON:()=>({})});
const searchIntent=(a:ReturnType<typeof adapter>)=>{const message=a.messages().find(item=>item.kind==='search');if(!message)throw new Error('Expected explicit search intent');return JSON.parse(message.value) as {id:string;kind:string;provider:string;path:string};};
const jpeg='data:image/jpeg;base64,/9j/2Q==';
const expectUnavailableSearch=(fixture:ReturnType<typeof adapter>)=>{
  expect(fixture.doc.querySelector('.atsumi-x-save-notice')?.textContent).toBe('검색 서비스 연결 검증이 아직 끝나지 않았습니다. 미디어는 전송하지 않았습니다.');
  expect(fixture.messages()).toHaveLength(0);
  expect(fixture.doc.querySelector('canvas')).toBeNull();
  expect(fixture.win.__atsumiXActions!.takeSearch('00000000-0000-4000-8000-000000000001')).toBeNull();
};
function videoSearchFixture(quotedMedia=false,options:SearchOptions=enabledSearch) {
  const poster='https://pbs.twimg.com/ext_tw_video_thumb/123456/pu/img/frame.jpg';
  const media=`<div data-testid="videoPlayer"><video id="search-video" src="blob:https://x.com/search" poster="${poster}"></video></div>`;
  const a=adapter(tweet(quotedMedia?`<div data-testid="quoteTweet" role="link">${media}</div>`:media),options);
  const video=a.doc.querySelector<HTMLVideoElement>('#search-video')!;showSearchTarget(video);
  Object.defineProperties(video,{videoWidth:{value:3840,configurable:true},videoHeight:{value:2160,configurable:true},readyState:{value:2,configurable:true},mediaKeys:{value:null,configurable:true}});
  video.currentTime=12.25;const seek=vi.spyOn(video,'currentTime','set'),play=vi.spyOn(video,'play'),pause=vi.spyOn(video,'pause');
  const drawImage=vi.fn(),toDataURL=vi.spyOn(HTMLCanvasElement.prototype,'toDataURL').mockReturnValue(jpeg);
  vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue({drawImage} as unknown as CanvasRenderingContext2D);
  a.visible();a.context(video);return {...a,video,seek,play,pause,drawImage,toDataURL};
}
it.each([undefined,false,'true',1,null])("keeps search dormant without explicit native capability (%s) while preserving downloads and external choices",searchEnabled=>{
  const options={...enabledSearch,searchEnabled};
  const a=adapter(undefined,options),target=a.doc.querySelector('#image')!;showSearchTarget(target);a.visible();a.context(target);
  expect(a.menuButton('작가·원본 찾기')).toBeUndefined();expect(a.menuButton('전체')).toBeDefined();expect(a.menuButton('외부 브라우저에서 열기')).toBeDefined();
  a.activate(a.menuButton('전체'));expect(a.actions()).toHaveLength(1);expect(a.actions()[0]).toMatchObject({kind:'all',path:'/creator/status/123456'});
  a.context(target);a.activate(a.menuButton('외부 브라우저에서 열기'));expect(a.messages().at(-1)).toEqual({kind:'external',value:'/creator/status/123456'});
  const video=videoSearchFixture(false,options);expect(video.menuButton('현재 장면으로 검색')).toBeUndefined();
  expect(video.drawImage).not.toHaveBeenCalled();expect(video.toDataURL).not.toHaveBeenCalled();expect(video.doc.querySelector('canvas')).toBeNull();
  const photo=photoAdapter(undefined,options);photo.visible();photo.context(photo.doc.querySelector('#full-photo')!);
  expect(photo.menuButton('작가·원본 찾기')).toBeUndefined();expect(photo.menuButton('개별')).toBeDefined();expect(photo.menuButton('외부 브라우저에서 열기')).toBeDefined();
  const quote=adapter(tweet(quoted),options);quote.visible();const quoteEvent=quote.context(quote.doc.querySelector('#quote-image')!);
  expect(quoteEvent.preventDefault).not.toHaveBeenCalled();expect(quote.doc.querySelector('[role="menu"]')).toBeNull();
  for(const fixture of [a,video,photo,quote]){
    expect(fixture.messages().some(message=>message.kind==='search')).toBe(false);
    expect(fixture.win.__atsumiXActions!.takeSearch('00000000-0000-4000-8000-000000000001')).toBeNull();
    fixture.win.__atsumiXActions!.searchResult('00000000-0000-4000-8000-000000000001',{state:'opened',message:'Unsupported search result'});
    expect(fixture.doc.body.textContent).not.toContain('Unsupported search result');
  }
});
it.each([
  {image:false,video:false},{image:true,video:false},{image:false,video:true},{image:true,video:true},
])("always offers exact-media search menus while gating capture independently across feed, quote and expanded photo: %j",searchCapabilities=>{
  const options={searchEnabled:true,searchCapabilities};
  const a=adapter(undefined,options),target=a.doc.querySelector('#image')!;showSearchTarget(target);a.visible();a.context(target);
  expect(a.menuButton('작가·원본 찾기')).toBeDefined();
  expect(a.menuButton('전체')).toBeDefined();expect(a.menuButton('외부 브라우저에서 열기')).toBeDefined();
  a.activate(a.menuButton('작가·원본 찾기'));
  if(searchCapabilities.image){const intent=searchIntent(a);expect(a.win.__atsumiXActions!.takeSearch(intent.id)?.provider).toBe('saucenao');}
  else expectUnavailableSearch(a);
  const video=videoSearchFixture(false,options);expect(video.menuButton('현재 장면으로 검색')).toBeDefined();video.activate(video.menuButton('현재 장면으로 검색'));
  if(searchCapabilities.video){const intent=searchIntent(video);expect(video.win.__atsumiXActions!.takeSearch(intent.id)?.provider).toBe('lens');expect(video.drawImage).toHaveBeenCalledOnce();}
  else{expectUnavailableSearch(video);expect(video.drawImage).not.toHaveBeenCalled();expect(video.toDataURL).not.toHaveBeenCalled();}
  const photo=photoAdapter(undefined,options);photo.visible();photo.context(photo.doc.querySelector('#full-photo')!);
  expect(photo.menuButton('작가·원본 찾기')).toBeDefined();expect(photo.menuButton('개별')).toBeDefined();photo.activate(photo.menuButton('작가·원본 찾기'));
  if(searchCapabilities.image)expect(photo.win.__atsumiXActions!.takeSearch(searchIntent(photo).id)?.provider).toBe('saucenao');
  else expectUnavailableSearch(photo);
  const quote=adapter(tweet(quoted),options),quotedImage=quote.doc.querySelector('#quote-image')!;showSearchTarget(quotedImage);quote.visible();const event=quote.context(quotedImage);
  expect(quote.menuButton('작가·원본 찾기')).toBeDefined();expect(event.preventDefault).toHaveBeenCalledOnce();quote.activate(quote.menuButton('작가·원본 찾기'));
  if(searchCapabilities.image)expect(quote.win.__atsumiXActions!.takeSearch(searchIntent(quote).id)?.provider).toBe('saucenao');
  else expectUnavailableSearch(quote);
  const quotedVideo=videoSearchFixture(true,options);expect(quotedVideo.menuButton('현재 장면으로 검색')).toBeDefined();
  expect(quotedVideo.menuButton('개별')).toBeUndefined();
  quotedVideo.activate(quotedVideo.menuButton('현재 장면으로 검색'));
  if(searchCapabilities.video)expect(quotedVideo.win.__atsumiXActions!.takeSearch(searchIntent(quotedVideo).id)?.provider).toBe('lens');
  else{expectUnavailableSearch(quotedVideo);expect(quotedVideo.drawImage).not.toHaveBeenCalled();expect(quotedVideo.toDataURL).not.toHaveBeenCalled();}
  for(const fixture of [a,video,quote,quotedVideo])expect(fixture.location.href).toBe('https://x.com/home');
  expect(photo.location.href).toBe('https://x.com/creator/status/123456/photo/1');
});
it.each([undefined,null,false,true,'true',[],{image:true},{video:true},{image:true,video:'true'},{image:1,video:true},{image:true,video:true,extra:true}])(
  "keeps search menus visible but reports malformed readiness without capture or dispatch: %j",searchCapabilities=>{
    const options={searchEnabled:true,searchCapabilities};
    const a=adapter(undefined,options),target=a.doc.querySelector('#image')!;showSearchTarget(target);a.visible();a.context(target);
    expect(a.menuButton('작가·원본 찾기')).toBeDefined();expect(a.menuButton('전체')).toBeDefined();a.activate(a.menuButton('작가·원본 찾기'));
    const video=videoSearchFixture(false,options);expect(video.menuButton('현재 장면으로 검색')).toBeDefined();video.activate(video.menuButton('현재 장면으로 검색'));expect(video.drawImage).not.toHaveBeenCalled();expect(video.toDataURL).not.toHaveBeenCalled();
    const photo=photoAdapter(undefined,options);photo.visible();photo.context(photo.doc.querySelector('#full-photo')!);expect(photo.menuButton('작가·원본 찾기')).toBeDefined();photo.activate(photo.menuButton('작가·원본 찾기'));
    const quote=adapter(tweet(quoted),options),quotedImage=quote.doc.querySelector('#quote-image')!;showSearchTarget(quotedImage);quote.visible();quote.context(quotedImage);expect(quote.menuButton('작가·원본 찾기')).toBeDefined();quote.activate(quote.menuButton('작가·원본 찾기'));
    const quotedVideo=videoSearchFixture(true,options);expect(quotedVideo.menuButton('현재 장면으로 검색')).toBeDefined();quotedVideo.activate(quotedVideo.menuButton('현재 장면으로 검색'));expect(quotedVideo.drawImage).not.toHaveBeenCalled();expect(quotedVideo.toDataURL).not.toHaveBeenCalled();
    for(const fixture of [a,video,photo,quote,quotedVideo])expectUnavailableSearch(fixture);
  },
);
it("offers SauceNAO for the exact clicked image without navigating the source or sending bytes in the bridge",()=>{
  const a=adapter(undefined,enabledSearch);const target=a.doc.querySelector('#image')!;showSearchTarget(target);a.visible();a.context(target);
  const choice=a.menuButton('작가·원본 찾기');expect(choice.title).toContain('SauceNAO');expect(choice.title).toContain('이미지 주소');
  a.activate(choice);const intent=searchIntent(a);expect(intent).toEqual({id:expect.any(String),kind:'image',provider:'saucenao',path:'/creator/status/123456'});
  expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toEqual({...intent,sourceUrl:'https://x.com/home',imageUrl:image});
  expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toBeNull();expect(a.actions()).toHaveLength(0);expect(a.location.href).toBe('https://x.com/home');
  expect(a.messages().some(item=>['preview','previewBackground','external'].includes(item.kind))).toBe(false);
});
it("uses a clicked grouped image instead of the first sibling and rejects a recycled target before dispatch",()=>{
  const a=adapter(tweet().replace('</div><div role="group">',`<img id="second" src="${otherImage}"></div><div role="group">`),enabledSearch);
  const target=a.doc.querySelector<HTMLImageElement>('#second')!;showSearchTarget(target);a.visible();a.context(target);a.activate(a.menuButton('작가·원본 찾기'));
  expect(a.win.__atsumiXActions!.takeSearch(searchIntent(a).id)?.imageUrl).toBe(otherImage);
  a.context(target);const stale=a.menuButton('작가·원본 찾기');target.src=image;a.activate(stale);expect(a.messages().filter(item=>item.kind==='search')).toHaveLength(1);
});
it("searches only the proved expanded photo and keeps its route and existing photo controls unchanged",()=>{
  const a=photoAdapter(undefined,enabledSearch);a.visible();a.context(a.doc.querySelector('#full-photo')!);const location=a.location.href,save=a.doc.querySelector('[data-atsumi-photo-save]');
  a.activate(a.menuButton('작가·원본 찾기'));const intent=searchIntent(a);
  expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toEqual({...intent,sourceUrl:location,imageUrl:image});
  expect(a.location.href).toBe(location);expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBe(save);expect(a.actions()).toHaveLength(0);
});
it("captures only current decoded video pixels for Lens without seeking or changing playback",()=>{
  const a=videoSearchFixture(),button=a.menuButton('현재 장면으로 검색');expect(button.title).toContain('Google Lens');expect(button.title).toContain('현재 영상 장면');
  a.activate(button);const intent=searchIntent(a);expect(intent.kind).toBe('video');expect(intent.provider).toBe('lens');
  expect(a.drawImage).toHaveBeenCalledExactlyOnceWith(a.video,0,0,2048,1152);expect(a.toDataURL).toHaveBeenCalledExactlyOnceWith('image/jpeg',.9);
  expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toEqual({...intent,sourceUrl:'https://x.com/home',dataUrl:jpeg,width:2048,height:1152,time:12.25});
  expect(a.seek).not.toHaveBeenCalled();expect(a.play).not.toHaveBeenCalled();expect(a.pause).not.toHaveBeenCalled();
  expect(JSON.stringify(a.messages())).not.toContain('base64');expect(JSON.stringify(a.messages())).not.toContain('frame.jpg');
});
it("searches exact quoted media without borrowing parent media or granting quoted download ownership",()=>{
  const a=adapter(tweet(quoted),enabledSearch),image=a.doc.querySelector('#quote-image')!;showSearchTarget(image);a.visible();a.context(image);
  expect(a.menuButton('개별')).toBeUndefined();expect(a.menuButton('전체')).toBeUndefined();expect(a.menuButton('게시물 보관')).toBeUndefined();
  a.activate(a.menuButton('작가·원본 찾기'));const request=searchIntent(a),payload=a.win.__atsumiXActions!.takeSearch(request.id);
  expect(payload?.imageUrl).toBe('https://pbs.twimg.com/media/Quote?format=jpg');expect(payload?.path).toBe('/creator/status/123456');expect(payload).not.toHaveProperty('artist');
  const b=videoSearchFixture(true);expect(b.menuButton('개별')).toBeUndefined();b.activate(b.menuButton('현재 장면으로 검색'));
  expect(b.win.__atsumiXActions!.takeSearch(searchIntent(b).id)?.dataUrl).toBe(jpeg);expect(b.drawImage.mock.calls[0]?.[0]).toBe(b.video);
});
it("rejects quoted media moved to another quote or ambiguous grouped media before capture",()=>{
  const a=adapter(tweet(quoted),enabledSearch),target=a.doc.querySelector('#quote-image')!;showSearchTarget(target);a.visible();a.context(target);const choice=a.menuButton('작가·원본 찾기');
  const replacement=a.doc.createElement('div');replacement.setAttribute('data-testid','quoteTweet');a.doc.querySelector('article')!.append(replacement);replacement.append(target.closest('[data-testid="tweetPhoto"]')!);
  a.activate(choice);expect(a.messages()).toHaveLength(0);
  const b=adapter(tweet().replace('</div><div role="group">',`<img src="${otherImage}"></div><div role="group">`),enabledSearch);b.visible();b.context(b.doc.querySelector('[data-testid="tweetPhoto"]')!);
  expect(b.menuButton('작가·원본 찾기')).toBeUndefined();
});
it.each(['taint','no-context','too-large','wrong-format','not-ready','zero-size','protected','invalid-time','huge-input'])("reports %s frame failure without sending a poster or another frame",failure=>{
  const a=videoSearchFixture();
  if(failure==='taint')a.toDataURL.mockImplementation(()=>{throw new DOMException('Blocked','SecurityError');});
  if(failure==='no-context')vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);
  if(failure==='too-large')a.toDataURL.mockReturnValue('data:image/jpeg;base64,'+'AAAA'.repeat(1398102));
  if(failure==='wrong-format')a.toDataURL.mockReturnValue('data:image/png;base64,AAAA');
  if(failure==='not-ready')Object.defineProperty(a.video,'readyState',{value:1});
  if(failure==='zero-size')Object.defineProperty(a.video,'videoWidth',{value:0});
  if(failure==='protected')Object.defineProperty(a.video,'mediaKeys',{value:{}});
  if(failure==='invalid-time')Object.defineProperty(a.video,'currentTime',{value:NaN});
  if(failure==='huge-input')Object.defineProperty(a.video,'videoWidth',{value:20000});
  a.activate(a.menuButton('현재 장면으로 검색'));expect(a.messages().some(item=>item.kind==='search')).toBe(false);expect(a.actions()).toHaveLength(0);
  expect(a.doc.querySelector('.atsumi-x-save-notice')!.textContent).toMatch(/접근할 수 없습니다|읽을 수 없습니다|보호된 영상/);
});
it.each(['route','hidden','detached','source','expired'])("rejects a %s one-shot search payload after native dispatch",change=>{
  vi.spyOn(Date,'now').mockReturnValue(0);const a=videoSearchFixture();a.activate(a.menuButton('현재 장면으로 검색'));const intent=searchIntent(a);
  if(change==='route')a.location.href='https://x.com/creator/status/999999';
  if(change==='hidden')a.win.__atsumiXActions!.visibility(false);
  if(change==='detached')a.video.remove();
  if(change==='source')a.video.src='blob:https://x.com/other';
  if(change==='expired')vi.mocked(Date.now).mockReturnValue(10001);
  expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toBeNull();expect(a.win.__atsumiXActions!.takeSearch(intent.id)).toBeNull();
});
it("bounds search cache and ignores untrusted menu activation, foreign receipts and hidden result notifications",()=>{
  const a=videoSearchFixture();a.activate(a.menuButton('현재 장면으로 검색'),{isTrusted:false});expect(a.messages()).toHaveLength(0);
  for(let n=0;n<5;n++){a.context(a.video);a.activate(a.menuButton('현재 장면으로 검색'));}
  const ids=a.messages().filter(item=>item.kind==='search').map(item=>JSON.parse(item.value).id as string);expect(ids).toHaveLength(4);
  a.win.__atsumiXActions!.searchResult('unknown',{state:'opened',message:'foreign'});expect(a.doc.body.textContent).not.toContain('foreign');
  a.win.__atsumiXActions!.searchResult(ids[0]!,{state:'opened',message:'opened',extra:true});expect(a.doc.body.textContent).not.toContain('opened');
  a.win.__atsumiXActions!.takeSearch(ids[0]!);a.win.__atsumiXActions!.searchResult(ids[0]!,{state:'opened',message:'검색 탭 열림'});expect(a.doc.body.textContent).toContain('검색 탭 열림');
  a.win.__atsumiXActions!.visibility(false);a.win.__atsumiXActions!.searchResult(ids[1]!,{state:'failed',message:'hidden'});expect(a.doc.body.textContent).not.toContain('hidden');expect(a.win.__atsumiXActions!.takeSearch(ids[2]!)).toBeNull();
});
it("expires unclaimed payloads and permits another explicit search without replaying old requests",()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const a=adapter(undefined,{hostTimers:true,...enabledSearch}),target=a.doc.querySelector('#image')!;showSearchTarget(target);a.visible();
  for(let n=0;n<4;n++){a.context(target);a.activate(a.menuButton('작가·원본 찾기'));}
  const old=searchIntent(a).id;vi.advanceTimersByTime(10001);expect(a.win.__atsumiXActions!.takeSearch(old)).toBeNull();
  a.context(target);a.activate(a.menuButton('작가·원본 찾기'));expect(a.messages().filter(item=>item.kind==='search')).toHaveLength(5);
  a.win.__atsumiXActions!.searchResult(old,{state:'opened',message:'stale'});expect(a.doc.body.textContent).not.toContain('stale');
});
it("leaves explicit external links around media to the external-link handler",()=>{
  const a=adapter(tweet().replace('<img id="image"', '<a href="https://example.org/art"><img id="image"').replace('></div><div role="group">','></a></div><div role="group">'));
  a.visible();const event=a.context(a.doc.querySelector('#image')!);expect(event.preventDefault).not.toHaveBeenCalled();expect(a.doc.querySelector('[role="menu"]')).toBeNull();expect(a.messages()).toHaveLength(0);
});
it("coalesces file-boundary bursts to the latest cumulative text every 750ms without delaying real bars",()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const a=adapter(undefined,{hostTimers:true}),id=startAll(a);
  const update=(sequence:number,bytes:number,transfer:unknown=null)=>a.win.__atsumiXActions!.progress(id,actualProgress(sequence,{downloadedBytes:bytes*1048576,totalBytes:10*1048576,currentTransfer:transfer}));
  update(1,1,{receivedBytes:0,totalBytes:3*1048576});expect(jobRow(a,id).textContent).toContain('받은 1.0 MB');
  for(let n=2;n<=7;n++){
    vi.advanceTimersByTime(100);update(n,n,n%2?null:{receivedBytes:1048576,totalBytes:2*1048576});a.win.__atsumiXActions!.update();
    expect(jobRow(a,id).textContent).toContain('받은 1.0 MB');expect(firstMediaLine(a).getAttribute('aria-valuenow')).toBe(String(n*10));
    expect(vi.getTimerCount()).toBe(2); // One receipt poll and one numeric-text timer.
  }
  vi.advanceTimersByTime(149);expect(jobRow(a,id).textContent).toContain('받은 1.0 MB');
  vi.advanceTimersByTime(1);expect(jobRow(a,id).textContent).toContain('받은 7.0 MB');expect(vi.getTimerCount()).toBe(1);
  update(8,8);vi.advanceTimersByTime(749);expect(jobRow(a,id).textContent).toContain('받은 7.0 MB');
  vi.advanceTimersByTime(1);expect(jobRow(a,id).textContent).toContain('받은 8.0 MB');expect(jobRow(a,id).textContent).not.toContain('현재');
});
it("keeps per-request trailing text independent and immediately changes phase, cancel and unknown states",()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const a=adapter(tweet()+anotherPost(1),{hostTimers:true});a.visible();
  for(const button of a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]'))a.activate(button);
  const first=a.actions()[0].id as string,second=a.actions()[1].id as string;
  for(const id of [first,second])a.win.__atsumiXActions!.progress(id,actualProgress(1,{downloadedBytes:1048576}));
  vi.advanceTimersByTime(100);
  for(const id of [first,second])a.win.__atsumiXActions!.progress(id,actualProgress(2,{downloadedBytes:2097152}));
  a.win.__atsumiXActions!.progress(first,actualProgress(3,{stage:'writing',downloadedBytes:2097152}));expect(jobRow(a,first).textContent).toContain('저장 마무리 중');
  vi.advanceTimersByTime(650);expect(jobRow(a,second).textContent).toContain('받은 2.0 MB');expect(jobRow(a,first).textContent).not.toContain('받은');
  a.activate(jobRow(a,second).querySelector('button')!);expect(jobRow(a,second).textContent).toContain('취소 요청 확인 중');
  vi.advanceTimersByTime(5250);expect(jobRow(a,first).textContent).toContain('저장 상태 확인 필요');expect(jobRow(a,second).textContent).toContain('저장 상태 확인 필요');
  expect(jobRow(a,second).querySelector('button')).not.toBeNull();
});
it("invalidates numeric timers on concealment and final results without painting an old request onto a new one",()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const a=adapter(undefined,{hostTimers:true}),id=startAll(a);
  a.win.__atsumiXActions!.progress(id,actualProgress(1,{downloadedBytes:1048576}));vi.advanceTimersByTime(100);
  a.win.__atsumiXActions!.progress(id,actualProgress(2,{downloadedBytes:2097152}));a.win.__atsumiXActions!.visibility(false);
  vi.advanceTimersByTime(750);expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();
  a.visible();expect(jobRow(a,id).textContent).toContain('받은 2.0 MB');
  a.win.__atsumiXActions!.progress(id,actualProgress(3,{downloadedBytes:3145728}));
  receipt(a,id,'complete',actualProgress(4,{stage:'complete',downloadedBytes:3145728}),'저장됨 · 로컬 저장소에서 확인');
  const check=jobRow(a,id).querySelector('.atsumi-x-save-check');expect(check).not.toBeNull();expect(jobRow(a,id).textContent).not.toContain('로컬 저장소');
  receipt(a,id,'complete',actualProgress(4,{stage:'complete',downloadedBytes:3145728}),'duplicate');expect(jobRow(a,id).querySelector('.atsumi-x-save-check')).toBe(check);
  vi.advanceTimersByTime(750);expect(jobRow(a,id).querySelector('.atsumi-x-save-check')).toBe(check);expect(jobRow(a,id).textContent).not.toContain('받은');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const next=a.actions().at(-1).id as string;
  vi.advanceTimersByTime(750);expect(jobRow(a,next).textContent).toContain('요청 확인 중');expect(jobRow(a,next).querySelector('.atsumi-x-save-check')).toBeNull();
});
it("draws an accessible reduced-motion-aware success check only after final fully successful confirmation",()=>{
  const a=adapter(),id=startAll(a),complete=actualProgress(1,{stage:'complete'});
  a.win.__atsumiXActions!.progress(id,complete);expect(jobRow(a,id).querySelector('.atsumi-x-save-check')).toBeNull();
  receipt(a,id,'complete',complete,'저장 완료');const check=jobRow(a,id).querySelector('.atsumi-x-save-check')!;
  expect(check.getAttribute('role')).toBe('img');expect(check.getAttribute('aria-label')).toBe('저장 완료');expect(check.querySelector('path')!.getAttribute('pathLength')).toBe('1');
  const css=[...a.doc.querySelectorAll('style')].map(node=>node.textContent).join('');expect(css).toContain('stroke-dashoffset:1');expect(css).toContain('@media(prefers-reduced-motion:reduce){.atsumi-x-save-check path{animation:none;stroke-dashoffset:0}}');
});
it.each(['partial','failed','cancelled'] as const)("retains truthful %s text instead of a success check",state=>{
  vi.useFakeTimers();vi.setSystemTime(0);const a=adapter(undefined,{hostTimers:true}),id=startAll(a);
  a.win.__atsumiXActions!.progress(id,actualProgress(1,{downloadedBytes:1048576}));vi.advanceTimersByTime(100);a.win.__atsumiXActions!.progress(id,actualProgress(2,{downloadedBytes:2097152}));
  receipt(a,id,state,actualProgress(3,{stage:state==='partial'?'complete':state,partial:state==='partial',downloadedBytes:2097152}),`확정된 ${state}`);
  expect(jobRow(a,id).textContent).toContain(`확정된 ${state}`);expect(jobRow(a,id).querySelector('.atsumi-x-save-check')).toBeNull();
  vi.advanceTimersByTime(750);expect(jobRow(a,id).textContent).toContain(`확정된 ${state}`);
});
it("never converts a sticky partial result into a full-success check",()=>{
  const a=adapter(),id=startAll(a);a.win.__atsumiXActions!.progress(id,actualProgress(1,{partial:true}));
  receipt(a,id,'complete',actualProgress(2,{stage:'complete',partial:true}),'일부만 저장됨');expect(jobRow(a,id).querySelector('.atsumi-x-save-check')).toBeNull();expect(jobRow(a,id).textContent).toContain('일부만 저장됨');
});
it("keeps multiple job rows, controls and order stable while progress arrives in any order",()=>{
  const a=adapter(tweet()+anotherPost(1));a.visible();const buttons=a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]');a.activate(buttons[0]!);a.activate(buttons[1]!);
  const [first,second]=a.actions(),firstId=first.id as string,secondId=second.id as string;
  const firstRow=jobRow(a,firstId),secondRow=jobRow(a,secondId),firstCancel=firstRow.querySelector<HTMLButtonElement>('button')!,secondCancel=secondRow.querySelector<HTMLButtonElement>('button')!;
  receipt(a,secondId,'active',actualProgress(0,{stage:'queued',downloadedBytes:0,totalMedia:1}));
  a.win.__atsumiXActions!.progress(firstId,actualProgress(1,{totalMedia:1,currentTransfer:{receivedBytes:4096,totalBytes:8192}}));
  a.win.__atsumiXActions!.progress(secondId,actualProgress(1,{totalMedia:1,downloadedBytes:8192}));a.win.__atsumiXActions!.update();
  expect(jobRow(a,firstId)).toBe(firstRow);expect(jobRow(a,secondId)).toBe(secondRow);
  expect(firstRow.querySelector('button')).toBe(firstCancel);expect(secondRow.querySelector('button')).toBe(secondCancel);
  expect([...a.doc.querySelectorAll('.atsumi-x-save-row')]).toEqual([firstRow,secondRow]);expect(a.doc.querySelectorAll('.atsumi-x-save-notice')).toHaveLength(1);
  expect(firstRow.textContent).toContain('받은 0.0 MB');expect(secondRow.textContent).toContain('받은');expect(mediaLines(a)).toHaveLength(2);
  a.activate(secondCancel);expect(a.messages().at(-1)).toEqual({kind:'archiveCancel',value:secondId});expect(firstCancel.disabled).toBe(false);
  receipt(a,secondId,'cancelled',actualProgress(2,{stage:'cancelled',totalMedia:1}),'두 번째 취소');
  expect(firstRow.querySelector('button')).toBe(firstCancel);expect(jobRow(a,secondId).textContent).toContain('두 번째 취소');
  a.activate(firstCancel);expect(a.messages().at(-1)).toEqual({kind:'archiveCancel',value:firstId});
});
it("reuses only the same pending action and permits different post, kind and exact media requests",()=>{
  const a=adapter(tweet('<div data-testid="tweetPhoto"><img src="'+otherImage+'"></div>')+anotherPost(1));a.visible();
  const all=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;a.activate(all);a.activate(all);expect(a.actions()).toHaveLength(1);
  a.emit(all,'pointerenter',a.event(all));a.activate(a.railButton('개별'));expect(a.actions()).toHaveLength(2);
  const secondMedia=a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]')[1]!;
  a.emit(secondMedia,'pointerenter',a.event(secondMedia));a.activate(a.railButton('개별'));expect(a.actions()).toHaveLength(3);
  a.activate(a.doc.querySelector('[data-atsumi-post-save]')!);expect(a.actions()).toHaveLength(4);
  a.activate(a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]')[2]!);expect(a.actions()).toHaveLength(5);
  expect(new Set(a.actions().map(value=>value.id)).size).toBe(5);expect(a.doc.querySelectorAll('.atsumi-x-save-row')).toHaveLength(5);
  const firstLines=a.doc.querySelectorAll<HTMLElement>('article:first-child .atsumi-x-media-host:first-of-type > .atsumi-x-media-progress');
  expect([...mediaLines(a)].filter(line=>line.parentElement===a.doc.querySelector('.atsumi-x-media-host')).map(line=>line.style.bottom)).toEqual(['0px','3px']);
  expect(firstLines.length).toBeLessThanOrEqual(2);
});
it("bounds local admission at22 without replacing existing cancellation controls or resending",()=>{
  const a=adapter(Array.from({length:23},(_,i)=>anotherPost(i)).join(''));a.visible();
  const buttons=a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]');for(const b of buttons)a.activate(b);
  expect(a.actions()).toHaveLength(22);expect(a.doc.querySelectorAll('.atsumi-x-save-row[data-request]:has(button)')).toHaveLength(22);
  expect(a.doc.body.textContent).toContain('요청이 가득');expect(a.doc.querySelectorAll('.atsumi-x-save-notice')).toHaveLength(1);
  const first=a.actions()[0].id as string;receipt(a,first,'absent',null,'미접수');a.activate(buttons[22]!);expect(a.actions()).toHaveLength(23);
  expect(a.doc.querySelectorAll('.atsumi-x-save-row[data-request]:has(button)')).toHaveLength(22);
});
it("restores all pending statuses after concealment but never reveals hidden terminal results",()=>{
  const a=adapter(tweet()+anotherPost(1)+anotherPost(2));a.visible();for(const b of a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]'))a.activate(b);
  const ids=a.actions().map(value=>value.id as string);a.win.__atsumiXActions!.visibility(false);expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();
  receipt(a,ids[0]!,'failed',null,'숨김 오류');receipt(a,ids[1]!,'active',actualProgress(0,{stage:'queued'}));a.visible();
  expect(jobRow(a,ids[0]!)).toBeNull();expect(jobRow(a,ids[1]!).textContent).toContain('대기 중');expect(jobRow(a,ids[2]!)).not.toBeNull();
  expect(a.doc.body.textContent).not.toContain('숨김 오류');expect(a.actions()).toHaveLength(3);
  expect(a.timers.length).toBeGreaterThanOrEqual(5);expect(a.doc.querySelectorAll('.atsumi-x-save-row')).toHaveLength(2);
});
it("renders cumulative bytes regardless of file boundaries without inflating media count or whole-job percentage",()=>{
  const a=adapter(),id=startAll(a);receipt(a,id,'active',actualProgress(0,{stage:'queued',downloadedBytes:0,totalMedia:1}));
  expect(jobRow(a,id).textContent).toContain('대기 중');
  const transfer={receivedBytes:1048576,totalBytes:3145728};receipt(a,id,'active',actualProgress(1,{totalMedia:1,downloadedBytes:2097152,currentTransfer:transfer}));
  expect(jobRow(a,id).textContent).toContain('0/1 · 받은 2.0 MB');expect(firstMediaLine(a).hasAttribute('aria-valuenow')).toBe(false);
  expect(jobRow(a,id).textContent).not.toContain('미디어 저장 중');
  const cancel=jobRow(a,id).querySelector<HTMLButtonElement>('button')!;a.activate(cancel);expect(cancel.disabled).toBe(true);
  receipt(a,id,'active',actualProgress(1,{totalMedia:1,downloadedBytes:2097152,currentTransfer:{...transfer}}));expect(cancel.disabled).toBe(false);expect(cancel.textContent).toBe('취소 다시 요청');
  // A separate ordinary job proves unknown-length received-only wording.
  receipt(a,id,'cancelled',actualProgress(2,{stage:'cancelled',totalMedia:1}),'취소됨');a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const nextId=a.actions().at(-1).id as string;
  a.win.__atsumiXActions!.progress(nextId,actualProgress(2,{totalMedia:1,downloadedBytes:4194304,currentTransfer:{receivedBytes:2097152,totalBytes:null}}));
  expect(jobRow(a,nextId).textContent).toContain('0/1 · 받은 4.0 MB');expect(firstMediaLine(a).hasAttribute('aria-valuenow')).toBe(false);
});
it("preserves every running cancellation row when an unsent request or another completion reports",()=>{
  const a=adapter(tweet()+anotherPost(1)+anotherPost(2));a.visible();const buttons=a.doc.querySelectorAll<HTMLButtonElement>('[data-atsumi-media-save]');a.activate(buttons[0]!);a.activate(buttons[1]!);
  const [first,second]=a.actions(),row=jobRow(a,second.id),cancel=row.querySelector('button');
  Object.defineProperty(a.win.chrome.webview,'postMessage',{value:undefined,writable:true});a.activate(buttons[2]!);
  expect(jobRow(a,second.id)).toBe(row);expect(row.querySelector('button')).toBe(cancel);expect(a.doc.body.textContent).toContain('저장 요청을 보내지 못했습니다');
  receipt(a,first.id,'complete',actualProgress(1,{stage:'complete',processedMedia:2}),'첫 요청 완료');
  expect(jobRow(a,second.id)).toBe(row);expect(row.querySelector('button')).toBe(cancel);expect(row.textContent).toContain('요청 확인 중');
  expect([...a.doc.querySelectorAll('style')].map(node=>node.textContent).join('')).toContain('max-height:min(240px,35vh);overflow:auto');
});
it("rejects malformed or nonclosed current-transfer objects and backward same-sequence values",()=>{
  const a=adapter(),id=startAll(a);a.win.__atsumiXActions!.progress(id,actualProgress(1));const before=firstMediaLine(a).outerHTML;
  for(const currentTransfer of [undefined,[],{},1,{receivedBytes:-1,totalBytes:null},{receivedBytes:4097,totalBytes:null},{receivedBytes:1,totalBytes:0},{receivedBytes:1.5,totalBytes:null},{receivedBytes:1,totalBytes:Infinity},{receivedBytes:1,totalBytes:null,url:'private'}]){
    a.win.__atsumiXActions!.progress(id,actualProgress(2,{currentTransfer}));expect(firstMediaLine(a).outerHTML).toBe(before);
  }
  receipt(a,id,'active',actualProgress(1,{currentTransfer:{receivedBytes:1,totalBytes:3}}));expect(firstMediaLine(a).outerHTML).toBe(before);
});
it("does not claim downloading or a percentage before native acceptance",()=>{
  const a=adapter(),id=startAll(a),line=firstMediaLine(a);
  expect(a.doc.body.textContent).toContain('요청 확인 중');expect(a.doc.body.textContent).not.toContain('받은');
  expect(line.dataset.state).toBe('awaiting');expect(line.hasAttribute('aria-valuenow')).toBe(false);
  receipt(a,id,'active');expect(a.doc.body.textContent).toContain('저장 요청이 접수되었습니다');expect(line.hasAttribute('aria-valuenow')).toBe(false);
  a.win.__atsumiXActions!.progress(id,actualProgress(0,{stage:'capture',totalMedia:0,downloadedBytes:0}));
  expect(a.doc.body.textContent).toContain('저장할 내용을 확인하는 중');
  a.win.__atsumiXActions!.progress(id,actualProgress(1));expect(a.doc.body.textContent).toContain('받은');
  expect(a.actions()).toHaveLength(1);expect(a.timers).toHaveLength(1);
});
it("fails a definitively missing transport before creating a pending lock",()=>{
  const a=adapter();a.visible();const all=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  Object.defineProperty(a.win.chrome.webview,'postMessage',{value:undefined,writable:true});a.activate(all);
  expect(a.actions()).toHaveLength(0);expect(mediaLines(a)).toHaveLength(0);expect(a.doc.body.textContent).toContain('저장 요청을 보내지 못했습니다');
  Object.defineProperty(a.win.chrome.webview,'postMessage',{value:a.message});a.activate(all);
  expect(a.actions()).toHaveLength(1);expect(a.doc.body.textContent).toContain('요청 확인 중');
});
it("retains an uncertain throwing transport until authoritative absence without resubmitting",()=>{
  const a=adapter();a.message.mockImplementation(()=>{throw new Error('Delivery may already have happened');});
  const id=startAll(a),run=timerRunner(a);expect(a.doc.body.textContent).toContain('저장 상태 확인 필요');
  a.message.mockImplementation(()=>undefined);a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);
  expect(a.actions()).toHaveLength(1);run();expect(a.messages().at(-1)).toEqual({kind:'archiveStatus',value:id});
  receipt(a,id,'absent',null,'접수되지 않은 요청입니다. 다시 눌러 주세요.');
  expect(mediaLines(a)).toHaveLength(0);expect(a.doc.body.textContent).toContain('접수되지 않은 요청');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
  expect(a.actions()[1].id).not.toBe(id);
});
it("does not unlock if the bridge throws after a native acceptance callback",()=>{
  const a=adapter();a.visible();
  a.message.mockImplementation(value=>{
    const url=new URL(value.slice('ATSUMI_X_SURFACE_V1:'.length));
    if(url.searchParams.get('kind')==='archive'){
      const {id}=JSON.parse(url.searchParams.get('value')!);receipt(a,id,'active');throw new Error('After delivery');
    }
  });
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id;
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);
  expect(a.doc.body.textContent).toContain('저장 상태 확인 필요');receipt(a,id,'active');
  expect(a.doc.body.textContent).toContain('저장 요청이 접수되었습니다');expect(a.actions()).toHaveLength(1);
});
it("submits and reconciles even when source DOM rendering throws before dispatch",()=>{
  const a=adapter();a.visible();vi.spyOn(a.doc.body,'append').mockImplementationOnce(()=>{throw new Error('Source changed DOM');});
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);const id=a.actions()[0].id;
  receipt(a,id,'active',actualProgress(1));expect(a.doc.body.textContent).toContain('받은');
  const line=firstMediaLine(a);line.firstElementChild!.remove();
  expect(()=>a.win.__atsumiXActions!.progress(id,actualProgress(2,{processedMedia:1}))).not.toThrow();
  expect(firstMediaLine(a).firstElementChild).not.toBeNull();
  receipt(a,id,'failed',actualProgress(3,{stage:'failed'}),'실제 저장 거절 이유');expect(a.doc.body.textContent).toContain('실제 저장 거절 이유');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it("rejects oversized encoded envelopes before sending instead of silently pending",()=>{
  const a=adapter();a.doc.querySelector<HTMLImageElement>('#image')!.src='https://pbs.twimg.com/media/A?x='+'='.repeat(1800);a.visible();
  const all=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;a.emit(all,'pointerenter',a.event(all));a.activate(a.railButton('개별'));
  expect(a.actions()).toHaveLength(0);expect(a.doc.body.textContent).toContain('허용 길이를 초과');expect(mediaLines(a)).toHaveLength(0);
  a.doc.querySelector<HTMLImageElement>('#image')!.src=image;a.emit(all,'pointerenter',a.event(all));a.activate(a.railButton('개별'));
  expect(a.actions()).toHaveLength(1);expect(a.message.mock.calls[0]![0].slice('ATSUMI_X_SURFACE_V1:'.length).length).toBeLessThanOrEqual(4096);
});
it("bounds missing-receipt polling and offers an explicit status check without releasing or retrying",()=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);run(15);
  expect(a.messages().filter(v=>v.kind==='archiveStatus')).toHaveLength(15);expect(a.actions()).toHaveLength(1);expect(a.timers).toHaveLength(15);
  const check=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-save-status]')!;expect(check.hidden).toBe(false);
  expect(a.doc.body.textContent).toContain('저장 상태 확인 필요');expect(firstMediaLine(a).hasAttribute('aria-valuenow')).toBe(false);
  expect(firstMediaLine(a).hasAttribute('data-indeterminate')).toBe(false);
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);expect(a.timers).toHaveLength(15);
  a.activate(check);expect(a.messages().at(-1)).toEqual({kind:'archiveStatus',value:id});expect(a.timers).toHaveLength(16);expect(check.hidden).toBe(true);
  receipt(a,id,'absent',null,'접수되지 않았습니다');a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it("maintains one polling timer and resets unanswered checks only on authoritative updates",()=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);receipt(a,id,'active');receipt(a,id,'active');expect(a.timers).toHaveLength(1);
  run(2);expect(a.timers).toHaveLength(3);a.win.__atsumiXActions!.progress(id,actualProgress(1));expect(a.timers).toHaveLength(3);
  run(2);expect(a.doc.body.textContent).not.toContain('저장 상태 확인 필요');run();expect(a.doc.body.textContent).toContain('저장 상태 확인 필요');
  receipt(a,id,'active',actualProgress(2));expect(a.doc.body.textContent).not.toContain('저장 상태 확인 필요');
  a.win.__atsumiXActions!.visibility(false);const count=a.messages().length;run();expect(a.messages()).toHaveLength(count);expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();
  receipt(a,id,'active',actualProgress(3));expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();a.visible();
  expect(a.doc.body.textContent).toContain('받은');run();expect(a.messages().at(-1)).toEqual({kind:'archiveStatus',value:id});
});
it("recovers a lost terminal callback by receipt and ignores all late replies to that old request",()=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);a.win.__atsumiXActions!.progress(id,actualProgress(1,{stage:'complete',processedMedia:2}));
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);run();expect(a.messages().at(-1)).toEqual({kind:'archiveStatus',value:id});
  receipt(a,id,'complete',actualProgress(1,{stage:'complete',processedMedia:2}),'검증된 원래 저장 완료');
  expect(jobRow(a,id).querySelector('[aria-label="저장 완료"]')).not.toBeNull();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const next=a.actions()[1].id;
  for(const state of ['absent','failed','complete','active'])receipt(a,id,state,null,'오래된 결과');
  a.win.__atsumiXActions!.progress(id,actualProgress(9));a.win.__atsumiXActions!.result(id,'failed','오래된 결과');
  expect(a.doc.body.textContent).not.toContain('오래된 결과');expect(a.doc.querySelector('.atsumi-x-save-notice')!.getAttribute('data-request')).toBe(next);
  for(const timer of [...a.timers])timer();expect(a.doc.querySelector('.atsumi-x-save-notice')!.getAttribute('data-request')).toBe(next);expect(a.actions()).toHaveLength(2);
});
it("rejects malformed, foreign and non-closed receipts without changing the pending truth",()=>{
  const a=adapter(),id=startAll(a),before=firstMediaLine(a).outerHTML;
  for(const value of [null,[],{}, {state:'madeup',progress:null,message:''},{state:'absent',progress:null,message:'',url:'https://evil.test'}, {state:'absent',progress:null,message:'x'.repeat(513)}, {state:'active',progress:actualProgress(1,{url:'private'}),message:''}, {state:'active',progress:null}]){
    a.win.__atsumiXActions!.receipt(id,value);expect(firstMediaLine(a).outerHTML).toBe(before);
  }
  receipt(a,'00000000-0000-4000-8000-000000000999','absent');expect(firstMediaLine(a).outerHTML).toBe(before);
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);
});
it("keeps the exact original target and cancel receipt after route and virtualization changes",()=>{
  const a=adapter(),id=startAll(a);receipt(a,id,'active',actualProgress(1));a.doc.querySelector('article')!.remove();a.location.href='https://x.com/other/status/654321';a.location.pathname='/other/status/654321';a.win.__atsumiXActions!.update();
  expect(mediaLines(a)).toHaveLength(0);expect(a.doc.body.textContent).toContain('@creator · 전체');
  expect(a.doc.querySelector<HTMLElement>('[data-atsumi-notice-target]')!.title).toBe('https://x.com/creator/status/123456');
  a.activate(a.doc.querySelector('.atsumi-x-save-notice button')!);expect(a.messages().at(-1)).toEqual({kind:'archiveCancel',value:id});
  receipt(a,id,'cancelled',actualProgress(2,{stage:'cancelled'}),'원래 요청 취소 완료');expect(a.doc.body.textContent).toContain('원래 요청 취소 완료');expect(mediaLines(a)).toHaveLength(0);
});
it("conceals terminal receipts in a hidden pane without resurrecting private status or retrying",()=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);a.win.__atsumiXActions!.visibility(false);
  receipt(a,id,'failed',null,'숨긴 원래 요청의 오류');run();a.visible();
  expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();expect(a.doc.body.textContent).not.toContain('숨긴 원래 요청의 오류');expect(a.actions()).toHaveLength(1);
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it("allows exact pending cancellation before acceptance without assuming cancellation succeeded",()=>{
  const a=adapter(),id=startAll(a);a.activate(a.doc.querySelector('.atsumi-x-save-notice button')!);
  expect(a.messages().at(-1)).toEqual({kind:'archiveCancel',value:id});expect(a.doc.body.textContent).toContain('취소 요청 확인 중');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);
  receipt(a,id,'cancelled',null,'요청 취소 확인');expect(a.doc.body.textContent).toContain('요청 취소 확인');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it.each(['lost','throw'])("allows an explicit cancel retry only after authoritative still-active status (%s)",mode=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);receipt(a,id,'active',actualProgress(1));
  a.message.mockImplementation(value=>{if(mode==='throw'&&value.includes('kind=archiveCancel'))throw new Error('Uncertain cancel delivery');});
  const cancel=a.doc.querySelector<HTMLButtonElement>('.atsumi-x-save-notice button')!;a.activate(cancel);
  expect(cancel.disabled).toBe(true);run();expect(a.messages().filter(v=>v.kind==='archiveCancel')).toHaveLength(1);
  receipt(a,id,'active',actualProgress(1));expect(cancel.disabled).toBe(false);expect(cancel.textContent).toBe('취소 다시 요청');
  expect(a.doc.body.textContent).toContain('취소 요청 확인 중');expect(a.messages().filter(v=>v.kind==='archiveCancel')).toHaveLength(1);
  a.activate(cancel);expect(cancel.disabled).toBe(true);expect(a.messages().filter(v=>v.kind==='archiveCancel')).toEqual([{kind:'archiveCancel',value:id},{kind:'archiveCancel',value:id}]);
  receipt(a,id,'cancelled',actualProgress(2,{stage:'cancelled'}),'취소 확인 완료');expect(a.doc.body.textContent).toContain('취소 확인 완료');
  expect(a.actions()).toHaveLength(1);expect(a.doc.querySelector('.atsumi-x-save-notice button')).toBeNull();
});
it("does not regress newer progress or release on stale and contradictory receipts",()=>{
  const a=adapter(),id=startAll(a),run=timerRunner(a);receipt(a,id,'active',actualProgress(5,{processedMedia:1}));run(3);
  const before=firstMediaLine(a).outerHTML;
  for(const [state,progress] of [
    ['active',null],['active',actualProgress(4)],['active',actualProgress(5)],
    ['active',actualProgress(6,{stage:'complete'})],['complete',actualProgress(6)],
    ['complete',null],['failed',null],['cancelled',actualProgress(4,{stage:'cancelled'})],
    ['partial',actualProgress(6,{stage:'complete',partial:false})],['absent',null],
  ] as const){
    receipt(a,id,state,progress,'무효 결과');expect(firstMediaLine(a).outerHTML).toBe(before);expect(a.doc.body.textContent).not.toContain('무효 결과');
  }
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);
  receipt(a,id,'active',actualProgress(5,{processedMedia:1}));expect(a.doc.body.textContent).not.toContain('저장 상태 확인 필요');
  receipt(a,id,'complete',actualProgress(6,{stage:'complete',processedMedia:2,downloadedBytes:8192}),'검증된 완료');
  expect(firstMediaLine(a).getAttribute('aria-valuenow')).toBe('100');expect(jobRow(a,id).querySelector('[aria-label="저장 완료"]')).not.toBeNull();
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it("keeps a legacy terminal progress locked until its matching terminal outcome without accepting an older active ack",()=>{
  const a=adapter(),id=startAll(a);a.win.__atsumiXActions!.progress(id,actualProgress(5,{stage:'failed'}));const before=firstMediaLine(a).outerHTML;
  for(const progress of [null,actualProgress(4),actualProgress(6)])receipt(a,id,'active',progress);
  expect(firstMediaLine(a).outerHTML).toBe(before);receipt(a,id,'complete',actualProgress(4,{stage:'complete'}),'늦은 완료');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(1);
  receipt(a,id,'failed',actualProgress(5,{stage:'failed'}),'정확한 실패 원인');
  expect(a.doc.body.textContent).toContain('정확한 실패 원인');a.win.__atsumiXActions!.update();expect(a.doc.body.textContent).toContain('정확한 실패 원인');
  a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);expect(a.actions()).toHaveLength(2);
});
it("uses two diagonally overlapped single-download glyphs for the all action",()=>{
  const a=adapter();a.visible();const all=a.doc.querySelector('[data-atsumi-media-save] svg')!;
  const paths=all.querySelectorAll('path');expect(paths).toHaveLength(2);
  expect(paths[0]!.getAttribute('d')).toBe(paths[1]!.getAttribute('d'));
  expect(paths[0]!.getAttribute('transform')).toBe('translate(1 1) scale(.68)');
  expect(paths[1]!.getAttribute('transform')).toBe('translate(7 7) scale(.68)');
  expect(paths[0]!.getAttribute('d')).toBe(a.doc.querySelector('[data-atsumi-single-save] path')!.getAttribute('d'));
});
it("shows real byte and item progress on the exact affected media without fake HLS percent",()=>{
  const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);
  const id=a.actions()[0].id,line=firstMediaLine(a);expect(line.dataset.indeterminate).toBe('true');expect(line.hasAttribute('aria-valuenow')).toBe(false);
  expect(a.actions()).toEqual([{id,path:'/creator/status/123456',kind:'all',media:null}]);
  a.win.__atsumiXActions!.progress(id,actualProgress(1,{processedMedia:1,downloadedBytes:2097152}));
  expect(line.getAttribute('aria-valuetext')).toBe('1/2 · 받은 2.0 MB');
  expect(a.doc.querySelector('.atsumi-x-save-notice')!.textContent).toContain('1/2 · 받은 2.0 MB');
  expect(line.hasAttribute('aria-valuenow')).toBe(false);
  a.win.__atsumiXActions!.progress(id,actualProgress(2,{downloadedBytes:50,totalBytes:100}));
  expect(line.getAttribute('aria-valuenow')).toBe('50');expect(line.hasAttribute('data-indeterminate')).toBe(false);
  a.win.__atsumiXActions!.progress(id,actualProgress(3,{stage:'writing',processedMedia:2,downloadedBytes:100,totalBytes:100}));
  expect(line.getAttribute('aria-valuenow')).toBe('99');expect(a.doc.body.textContent).toContain('저장 마무리 중');
  a.win.__atsumiXActions!.progress(id,actualProgress(4,{stage:'complete',processedMedia:2}));
  expect(line.getAttribute('aria-valuenow')).toBe('100');expect(line.dataset.state).toBe('complete');
  a.win.__atsumiXActions!.result(id,'complete','검증된 저장 완료');expect(jobRow(a,id).querySelector('[aria-label="저장 완료"]')).not.toBeNull();
  a.win.__atsumiXActions!.progress(id,actualProgress(5));expect(line.dataset.state).toBe('complete');
});
it("rejects foreign, out-of-order, malformed and post-terminal progress updates",()=>{
  const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id,line=firstMediaLine(a);
  a.win.__atsumiXActions!.progress(id,actualProgress(2));const before=line.outerHTML;
  for(const value of [actualProgress(1),actualProgress(2),actualProgress(3,{stage:'invented'}),actualProgress(3,{processedMedia:3}),actualProgress(3,{totalMedia:4097}),actualProgress(3,{downloadedBytes:-1}),actualProgress(3,{downloadedBytes:NaN}),actualProgress(3,{sequence:Number.MAX_SAFE_INTEGER+1}),actualProgress(3,{totalBytes:2}),actualProgress(3,{partial:'yes'}),{...actualProgress(3),url:'https://evil.test'},null,[]]){
    a.win.__atsumiXActions!.progress(id,value);expect(line.outerHTML).toBe(before);
  }
  a.win.__atsumiXActions!.progress('other',actualProgress(3));expect(line.outerHTML).toBe(before);
  a.win.__atsumiXActions!.progress(id,actualProgress(4,{stage:'failed'}));expect(line.dataset.state).toBe('failed');expect(line.hasAttribute('data-indeterminate')).toBe(false);
  a.win.__atsumiXActions!.progress(id,actualProgress(5,{stage:'complete'}));expect(line.dataset.state).toBe('failed');
});
it("keeps partial, failed and cancelled outcomes distinct from successful completion",()=>{
  for(const state of ['partial','failed','cancelled']){
    const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id;
    a.win.__atsumiXActions!.result(id,state,'실제 백엔드 결과');const line=firstMediaLine(a);
    expect(line.dataset.state).toBe(state);expect(line.hasAttribute('data-indeterminate')).toBe(false);
    expect(line.getAttribute('aria-valuenow')).toBe(state==='partial'?'100':null);
    expect(a.doc.querySelector('.atsumi-x-save-notice button')).toBeNull();
  }
});
it("keeps post archive progress in its bottom notification without changing native media",()=>{
  const a=adapter();a.visible();const imageNode=a.doc.querySelector('#image')!,before=imageNode.outerHTML;
  a.activate(a.doc.querySelector('[data-atsumi-post-save]')!);const id=a.actions()[0].id;
  a.win.__atsumiXActions!.progress(id,actualProgress(1,{processedMedia:1}));
  expect(a.doc.querySelector('[data-atsumi-notice-progress]')!.getAttribute('aria-valuetext')).toContain('1/2');
  expect(mediaLines(a)).toHaveLength(0);expect(imageNode.outerHTML).toBe(before);
});
it("isolates all-media progress to the current post and single-media progress to its exact resource",()=>{
  const extra='<div data-testid="tweetPhoto"><img src="'+otherImage+'"></div>'+quoted;
  for(const kind of ['all','one']){
    const a=adapter(tweet(extra));a.visible();const all=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
    if(kind==='all')a.activate(all);else{a.emit(all,'pointerenter',a.event(all));a.activate(a.railButton('개별'));}
    expect(a.actions()[0]).toMatchObject({kind,media:kind==='one'?image:null});expect(mediaLines(a)).toHaveLength(kind==='all'?2:1);
    expect(a.doc.querySelector('[data-testid="quoteTweet"] .atsumi-x-media-progress')).toBeNull();
    if(kind==='one'){
      a.doc.querySelector<HTMLImageElement>('#image')!.src=otherImage;a.win.__atsumiXActions!.progress(a.actions()[0].id,actualProgress(1));
      expect(mediaLines(a)).toHaveLength(0);a.win.__atsumiXActions!.update();expect(a.doc.querySelector('.atsumi-x-save-notice')!.textContent).toContain('받은');
    }
  }
});
it("does not attach progress to a recycled post or a changed document route",()=>{
  for(const change of ['post','route','detach']){
    const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id;
    if(change==='post')a.doc.querySelector<HTMLAnchorElement>('#permalink')!.href='/other/status/654321';
    else if(change==='route')a.location.href='https://x.com/creator/status/123456';else a.doc.querySelector('article')!.remove();
    a.win.__atsumiXActions!.progress(id,actualProgress(1));a.win.__atsumiXActions!.update();
    expect(mediaLines(a)).toHaveLength(0);expect(a.doc.querySelector('.atsumi-x-save-notice')!.textContent).toContain('받은');
    a.win.__atsumiXActions!.result(id,'complete','원래 요청 저장 완료');expect(jobRow(a,id).querySelector('[aria-label="저장 완료"]')).not.toBeNull();expect(mediaLines(a)).toHaveLength(0);
  }
});
it("conceals progress while hidden and restores only a still-current job with a working cancellation",()=>{
  const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id;
  a.win.__atsumiXActions!.visibility(false);expect(mediaLines(a)).toHaveLength(0);
  a.win.__atsumiXActions!.progress(id,actualProgress(1,{processedMedia:1}));expect(a.doc.querySelector('.atsumi-x-save-notice')).toBeNull();
  a.visible();expect(mediaLines(a)).toHaveLength(1);expect(a.doc.body.textContent).toContain('1/2');
  a.activate(a.doc.querySelector('.atsumi-x-save-notice button')!);expect(a.messages().at(-1)).toEqual({kind:'archiveCancel',value:id});
  expect(a.doc.body.textContent).toContain('취소 요청 확인 중');
});
it("gives busy feedback without duplicate requests, auto-retry or replacing the working cancel button",()=>{
  const a=adapter();a.visible();const all=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;a.activate(all);
  const cancel=a.doc.querySelector('.atsumi-x-save-notice button');a.activate(all);a.activate(all);
  expect(a.actions()).toHaveLength(1);expect(a.doc.querySelector('.atsumi-x-save-notice button')).toBe(cancel);
  expect(a.doc.querySelectorAll('[data-atsumi-save-busy]')).toHaveLength(1);expect(a.doc.body.textContent).toContain('이미 요청됨');
  for(const timer of [...a.timers])timer();expect(a.actions()).toHaveLength(1);
});
it("replaces cancelling text with the truthful terminal outcome even when commit won the race",()=>{
  for(const [state,label] of [['cancelled','저장 취소됨'],['failed','저장 실패'],['complete','저장 완료']] as const){
    const a=adapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-media-save]')!);const id=a.actions()[0].id;
    a.activate(a.doc.querySelector('.atsumi-x-save-notice button')!);expect(firstMediaLine(a).getAttribute('aria-valuetext')).toBe('취소 요청 확인 중…');
    a.win.__atsumiXActions!.progress(id,actualProgress(1,{stage:state}));expect(firstMediaLine(a).getAttribute('aria-valuetext')).toBe(label);
    a.win.__atsumiXActions!.result(id,state,label);expect(firstMediaLine(a).getAttribute('aria-valuetext')).toBe(label);
  }
});
it("anchors the current-photo progress line to the proven photo and removes it after navigation",()=>{
  const a=photoAdapter();a.visible();a.activate(a.doc.querySelector('[data-atsumi-photo-save]')!);const id=a.actions()[0].id;
  const line=firstMediaLine(a);expect(line.dataset.atsumiPhotoProgress).toBe('true');expect(line.style.left).toBe('60px');expect(line.style.top).toBe('687px');
  expect(line.style.width).toBe('640px');expect(a.doc.querySelector('#full-photo')!.getAttribute('style')).toBeNull();
  a.location.pathname='/creator/status/123456/photo/2';a.location.href='https://x.com'+a.location.pathname;
  a.win.__atsumiXActions!.progress(id,actualProgress(1));a.win.__atsumiXActions!.update();expect(mediaLines(a)).toHaveLength(0);
});
it("rechecks ad ownership for already-created save buttons and open context actions without social writes",()=>{
  for(const kind of ["button","menu","single"]){
    const a=adapter();a.visible();let ad=false;
    Object.defineProperty(a.win,"__atsumiXAds",{value:{blocked:()=>ad}});
    const article=a.doc.querySelector("article")!,before=article.querySelector("img")!.getAttribute("src");
    let button:HTMLButtonElement;
    if(kind==="button")button=article.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
    else if(kind==="menu"){a.context(article.querySelector('[data-testid="tweetText"]')!);button=a.menuButton("게시물 보관");}
    else{
      const all=article.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
      a.emit(all,"pointerenter",a.event(all));button=a.railButton("개별");
    }
    ad=true;a.activate(button);
    expect(a.actions()).toHaveLength(0);expect(a.messages()).toHaveLength(0);
    expect(article.querySelector("img")!.getAttribute("src")).toBe(before);
  }
});
it("dismisses native action menus idempotently without source clicks, focus changes or account actions",()=>{
  const a=adapter();a.visible();
  a.context(a.doc.querySelector('[data-testid="tweetText"]')!);
  expect(a.doc.querySelector('[role="menu"]')?.getAttribute("aria-label")).toBe("게시물 메뉴");
  expect(a.win.__atsumiXSurface.dismissMenu).toHaveBeenCalledOnce();
  const active=a.doc.activeElement,messages=a.messages().length;
  expect(a.win.__atsumiXActions!.dismissMenus()).toBe(true);expect(a.win.__atsumiXActions!.dismissMenus()).toBe(false);
  expect(a.doc.querySelector('[role="menu"]')).toBeNull();
  expect(a.doc.activeElement).toBe(active);
  expect(a.messages()).toHaveLength(messages);expect(a.actions()).toHaveLength(0);
});
it("closes the expanded media choices through the same cross-pane dismissal bridge",()=>{
  const a=adapter();a.visible();
  const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.emit(button,"pointerenter",a.event(button));
  expect(a.doc.querySelector('[data-expanded]')).not.toBeNull();
  a.win.__atsumiXActions!.dismissMenus();
  expect(a.doc.querySelector('[data-expanded]')).toBeNull();
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(a.actions()).toHaveLength(0);
});
it("uses the shared black context treatment including keyboard focus for action menus",()=>{
  const a=adapter();const css=[...a.doc.querySelectorAll("style")].map(n=>n.textContent).join("");
  expect(css).toContain("background:#000;color:#e7e9ea;border:1px solid #2f3336;border-radius:12px");
  expect(css).toContain(".atsumi-x-download-menu button:focus-visible{background:#16181c;");
});
it("collects no current-photo diagnostics in release configuration",()=>{
  const a=photoAdapter();a.visible();a.proof.mockClear();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toBeNull();
  expect(a.proof).not.toHaveBeenCalled();
});
it("reports the same photo admission result without changing source styles, controls or identity",()=>{
  const a=photoAdapter(undefined,{diagnostics:true}),imageNode=a.doc.querySelector<HTMLImageElement>("#full-photo")!;
  Object.defineProperties(imageNode,{naturalWidth:{value:640},naturalHeight:{value:640},complete:{value:true}});
  a.visible();const before=a.doc.documentElement.outerHTML;
  const shape=a.win.__atsumiXActions!.photoDiagnostic()!;
  expect(shape).toMatchObject({reason:"ready",allowedImages:1,imageLoaded:1,imageComplete:1,
    nonzeroRect:1,inViewport:1,directlyShown:1,opacityZero:0,exactPaint:0,candidates:1,controlPresent:true});
  expect(a.doc.documentElement.outerHTML).toBe(before);expect(a.message).not.toHaveBeenCalled();
  expect(JSON.stringify(shape)).not.toMatch(/alice|creator|123456|Fixture|https|pbs\.twimg|src|style|url/i);
  const scalar=(v:unknown)=>typeof v==="boolean"||typeof v==="number"&&v>=0&&v<=128||typeof v==="string"&&v==="ready";
  expect(Object.entries(shape).every(([key,v])=>key==="carouselProof"?v==="absent":key==="slideShape"?Object.values(v as Record<string,unknown>).every(scalar):scalar(v))).toBe(true);
});
it("diagnoses failed current-photo gates without weakening their control admission",()=>{
  const cases:Array<[string,string]>=[
    [photoHtml+'<img src="'+otherImage+'">',"ambiguous"],
    [photoHtml+'<img>'.repeat(128),"imageLimit"],
    [photoHtml+'<article data-testid="tweet"></article>',"articleScope"],
    ['<div role="dialog" hidden>'+photoHtml+'</div>',"dialogAmbiguous"],
    ['<div role="dialog">'+photoHtml+'</div><div role="dialog">'+photoHtml+'</div>',"dialogAmbiguous"],
    [('<div role="dialog">'+photoHtml+'</div>').repeat(9),"dialogLimit"],
    ['<div style="opacity:0">'+photoHtml+'</div>',"none"],
  ];
  for(const [html,reason] of cases){
    const a=photoAdapter(html,{diagnostics:true});a.visible();
    expect(a.win.__atsumiXActions!.photoDiagnostic()!.reason).toBe(reason);
    expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();expect(a.actions()).toHaveLength(0);
  }
  const denied=photoAdapter(undefined,{diagnostics:true,photoOwner:""});denied.visible();
  expect(denied.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"owner",scannedImages:0});
  const hidden=photoAdapter(undefined,{diagnostics:true});
  expect(hidden.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"hidden",scannedImages:0});
});
it("distinguishes exact transparent photo paint, mismatched paint and offscreen geometry using counts only",()=>{
  const a=photoAdapter(undefined,{diagnostics:true}),host=a.doc.querySelector<HTMLElement>("#photo-host")!,img=a.doc.querySelector<HTMLElement>("#full-photo")!;
  host.style.backgroundImage='url("'+image+'")';img.style.opacity="0";a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ready",opacityZero:1,backgrounds:1,exactPaint:1,directlyShown:0,candidates:1});
  host.style.backgroundImage='url("'+otherImage+'")';a.win.__atsumiXActions!.update();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"none",opacityZero:1,backgrounds:1,exactPaint:0,candidates:0});
  img.style.opacity="1";
  vi.mocked(img.getBoundingClientRect).mockReturnValue({x:900,y:0,left:900,top:0,right:1000,bottom:200,width:100,height:200,toJSON:()=>({})});
  a.win.__atsumiXActions!.update();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"none",inViewport:0,nonzeroRect:1,directlyShown:1,candidates:0});
});
const photoHtml='<div id="photo-host"><img id="full-photo" src="'+image.replaceAll("&","&amp;")+'"></div>';
function photoAdapter(html=photoHtml,options:{host?:string;photoOwner?:string;path?:string;diagnostics?:boolean}&SearchOptions={}) {
  const a=adapter(html,{photoOwner:"alice",path:"/creator/status/123456/photo/1",...options});
  for(const node of a.doc.querySelectorAll<HTMLElement>("body *")){
    vi.spyOn(node,"getBoundingClientRect").mockReturnValue({x:60,y:50,left:60,top:50,right:700,bottom:690,width:640,height:640,toJSON:()=>({})});
  }
  return a;
}
const slideHtml=(first="",second="")=>'<span>슬라이드 1/2 - 회전 슬라이드</span><div aria-roledescription="carousel"><ul><li '+first+'>'+photoHtml+'</li><li '+second+'><div><img id="other-photo" src="'+otherImage+'"></div></li></ul></div>';
it("observes explicit carousel position and counter agreement without resolving ambiguous photos",()=>{
  const a=photoAdapter(slideHtml('aria-posinset="1" aria-setsize="2" aria-current="true"','aria-posinset="2" aria-setsize="2"'),{diagnostics:true});
  a.visible();const before=a.doc.documentElement.outerHTML;
  const result=a.win.__atsumiXActions!.photoDiagnostic()!;
  expect(result).toMatchObject({reason:"ambiguous",candidates:2,controlPresent:false,slideShape:{
    routeOrdinal:1,carousels:1,lists:1,slides:2,candidateSlides:2,indexedSlides:2,indexedRouteMatches:1,
    currentSlides:1,currentRouteMatches:1,declaredSizes:2,matchingSetSizes:2,counterLabels:1,counterRouteMatches:1,
    counterSizeMatches:1,ordinalItemCandidates:1,unclipped:2,overflow:false,
  }});
  expect(a.doc.documentElement.outerHTML).toBe(before);expect(a.actions()).toHaveLength(0);
  expect(JSON.stringify(result)).not.toMatch(/슬라이드|creator|alice|https|Fixture|Other|aria-/);
});
it("distinguishes adjacent slivers and ancestor clipping without treating size as selected-slide proof",()=>{
  const a=photoAdapter(slideHtml(),{diagnostics:true});
  const second=a.doc.querySelector<HTMLElement>("#other-photo")!;
  vi.mocked(second.getBoundingClientRect).mockReturnValue({x:790,y:50,left:790,top:50,right:1430,bottom:690,width:640,height:640,toJSON:()=>({})});
  a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ambiguous",slideShape:{
    partlyClipped:1,unclipped:1,currentSlides:0,indexedSlides:0,ordinalItemCandidates:1,
  }});
  const list=a.doc.querySelector<HTMLElement>("ul")!;list.style.overflowX="hidden";
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ready",slideShape:{fullyClipped:1,unclipped:1}});
  expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
});
it("reports conflicting current flags and mismatched route counters rather than borrowing identity",()=>{
  const a=photoAdapter(slideHtml('aria-posinset="1" aria-setsize="4" aria-current="true"','aria-posinset="2" aria-setsize="2" aria-current="true" aria-selected="true"'),{diagnostics:true,path:"/creator/status/123456/photo/2"});
  a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ambiguous",slideShape:{
    routeOrdinal:2,currentSlides:2,currentRouteMatches:1,selectedSlides:1,selectedRouteMatches:1,
    matchingSetSizes:1,counterRouteMatches:0,counterSizeMatches:1,
  }});
  expect(a.actions()).toHaveLength(0);expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
});
it("keeps nested list item diagnostics within their own list and bounds structural expansion",()=>{
  const nested='<div aria-roledescription="carousel"><ul><li><ul><li>'+photoHtml+'</li></ul></li><li><img src="'+otherImage+'"></li></ul></div>';
  const a=photoAdapter(nested,{diagnostics:true});a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ambiguous",slideShape:{lists:2,slides:3,candidateSlides:2}});
  const many=photoAdapter('<div aria-roledescription="carousel"><ul>'+Array.from({length:9},(_,i)=>'<li><img src="https://pbs.twimg.com/media/Proof'+i+'?format=jpg"></li>').join('')+'</ul></div>',{diagnostics:true});
  many.visible();
  expect(many.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ambiguous",slideShape:{candidateSlides:8,overflow:true}});
});
function provedCarousel(options:{path?:string;photoOwner?:string;diagnostics?:boolean}={}) {
  const a=photoAdapter(slideHtml(),options);
  const bounds=(left:number)=>({x:left,y:50,left,top:50,right:left+640,bottom:690,width:640,height:640,toJSON:()=>({})});
  const place=(selector:string,left:number)=>{
    const img=a.doc.querySelector<HTMLElement>(selector)!;
    for(const node of [img,img.parentElement!,img.closest("li")!])vi.mocked(node.getBoundingClientRect).mockReturnValue(bounds(left));
  };
  a.doc.querySelector<HTMLElement>("ul")!.style.overflowX="hidden";
  place("#other-photo",710);
  const slide=(ordinal:number)=>{
    a.location.pathname="/creator/status/123456/photo/"+ordinal;
    a.location.href="https://x.com"+a.location.pathname;
    a.doc.querySelector("span")!.textContent=`슬라이드 ${ordinal}/2 - 회전 슬라이드`;
    place("#full-photo",ordinal===1?60:-590);
    place("#other-photo",ordinal===2?60:710);
  };
  return {...a,slide,place};
}
it.each<[string,string,(a:ReturnType<typeof provedCarousel>)=>void]>([
  ["missing counter","counterMissing",a=>a.doc.querySelector("span")!.remove()],
  ["mismatched counter","counterMismatch",a=>{a.doc.querySelector("span")!.textContent="슬라이드 2/2 - 회전 슬라이드";}],
  ["extra list","lists",a=>a.doc.querySelector('[aria-roledescription]')!.append(a.doc.createElement("ul"))],
  ["nested carousel","nestedRoot",a=>a.doc.body.setAttribute("role","carousel")],
  ["explicit wrong ordinal","positionConflict",a=>a.doc.querySelector("li")!.setAttribute("aria-posinset","2")],
  ["explicit wrong size","sizeConflict",a=>a.doc.querySelector("li")!.setAttribute("aria-setsize","4")],
  ["explicit no current item","currentConflict",a=>a.doc.querySelector("li")!.setAttribute("aria-current","false")],
  ["explicit no selected item","selectedConflict",a=>a.doc.querySelector("li")!.setAttribute("aria-selected","false")],
  ["unbound absolute image","unboundAbsolute",a=>{a.doc.querySelector<HTMLElement>("#other-photo")!.style.position="absolute";}],
  ["fixed image","fixedImage",a=>{a.doc.querySelector<HTMLElement>("#other-photo")!.style.position="fixed";}],
  ["uncertain fixed ancestor","fixedAncestor",a=>{a.doc.querySelector<HTMLElement>("ul")!.style.position="fixed";a.doc.body.style.transform="translateX(1px)";}],
  ["visible neighbour","neighbourExposed",a=>a.place("#other-photo",690)],
  ["clipped current item","currentClipped",a=>a.place("#full-photo",710)],
  ["deduplicated current item missing","currentMissing",a=>{a.doc.querySelector<HTMLImageElement>("#other-photo")!.src=image;}],
])("reports only the existing first carousel rejection without changing admission: %s",(_label,proof,change)=>{
  const enabled=provedCarousel({diagnostics:true});change(enabled);enabled.visible();
  const before=enabled.doc.documentElement.outerHTML;
  const result=enabled.win.__atsumiXActions!.photoDiagnostic();
  expect(result).toMatchObject({reason:"ambiguous",carouselProof:proof,controlPresent:false});
  expect(enabled.doc.documentElement.outerHTML).toBe(before);
  expect(JSON.stringify(result)).not.toMatch(/https|Fixture|Other|creator|alice|aria-|슬라이드/);
  const disabled=provedCarousel();change(disabled);disabled.visible();
  expect(disabled.win.__atsumiXActions!.photoDiagnostic()).toBeNull();
  expect(disabled.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  expect(enabled.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  expect(enabled.actions()).toEqual(disabled.actions());
});
it("admits the exact four-slide current photo inside a viewport-fixed viewer without selecting its clipped neighbour",()=>{
  const a=provedCarousel({diagnostics:true});
  const list=a.doc.querySelector<HTMLElement>("ul")!;list.append(a.doc.createElement("li"),a.doc.createElement("li"));
  a.doc.querySelector("span")!.textContent="슬라이드 1/4 - 회전 슬라이드";
  list.style.position="fixed";
  for(const [selector,url] of [["#full-photo",image],["#other-photo",otherImage]] as const){
    const img=a.doc.querySelector<HTMLElement>(selector)!;img.style.opacity="0";img.parentElement!.style.backgroundImage=`url("${url}")`;
  }
  a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ready",carouselProof:"ready",candidates:2,exactPaint:2,controlPresent:true,
    slideShape:{slides:4,ordinalItemCandidates:1,counterRouteMatches:1,counterSizeMatches:1,fullyClipped:1,unclipped:1}});
  a.activate(a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!);
  expect(a.actions()[0]).toMatchObject({kind:"one",media:image});
});
it.each(["transform","translate","rotate","scale","perspective","filter","backdropFilter","contain","willChange","containerType","contentVisibility","transformStyle"])("rejects uncertain %s containing blocks above a fixed viewer",property=>{
  const values:Record<string,string>={transform:"translateX(1px)",translate:"1px",rotate:"1deg",scale:"1",perspective:"100px",filter:"blur(0px)",backdropFilter:"blur(0px)",contain:"paint",willChange:"transform",containerType:"inline-size",contentVisibility:"auto",transformStyle:"preserve-3d"};
  const a=provedCarousel({diagnostics:true});a.doc.querySelector<HTMLElement>("ul")!.style.position="fixed";
  Object.assign(a.doc.body.style,{[property]:values[property]});a.visible();
  expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({carouselProof:"fixedAncestor",controlPresent:false});
  expect(a.actions()).toHaveLength(0);
});
it("never uses overflow above a fixed viewer to prove a visible neighbouring slide is clipped",()=>{
  const a=provedCarousel({diagnostics:true});const list=a.doc.querySelector<HTMLElement>("ul")!;
  list.style.position="fixed";list.style.overflowX="visible";a.doc.body.style.overflowX="hidden";
  vi.spyOn(a.doc.body,"getBoundingClientRect").mockReturnValue({x:60,y:50,left:60,top:50,right:700,bottom:690,width:640,height:640,toJSON:()=>({})});
  a.visible();expect(a.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({carouselProof:"neighbourExposed",controlPresent:false});
  expect(a.actions()).toHaveLength(0);
});
it("keeps ready-carousel and absent-carousel fallback actions identical with diagnostics disabled",()=>{
  for(const carousel of [true,false]){
    const setup=(diagnostics:boolean)=>carousel?provedCarousel({diagnostics}):photoAdapter(photoHtml,{diagnostics});
    const enabled=setup(true);enabled.visible();
    const before=enabled.doc.documentElement.outerHTML;
    expect(enabled.win.__atsumiXActions!.photoDiagnostic()).toMatchObject({reason:"ready",carouselProof:carousel?"ready":"absent",controlPresent:true});
    expect(enabled.doc.documentElement.outerHTML).toBe(before);
    enabled.activate(enabled.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!);
    const disabled=setup(false);disabled.visible();
    expect(disabled.win.__atsumiXActions!.photoDiagnostic()).toBeNull();
    disabled.activate(disabled.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!);
    expect(enabled.actions()).toEqual(disabled.actions());
    expect(enabled.actions()).toHaveLength(1);
  }
});
it("saves the proved current carousel resource, not the clipped neighbour, for both route ordinals",()=>{
  const a=provedCarousel();a.visible();
  const first=a.doc.querySelector<HTMLButtonElement>("[data-atsumi-photo-save]")!;
  expect(first).not.toBeNull();a.activate(first);
  expect(a.actions().at(-1)).toMatchObject({path:"/creator/status/123456",kind:"one",media:image});
  a.win.__atsumiXActions!.result(a.actions().at(-1).id,"complete","저장 완료");
  a.slide(2);a.win.__atsumiXActions!.update();
  const second=a.doc.querySelector<HTMLButtonElement>("[data-atsumi-photo-save]")!;
  expect(second).not.toBeNull();a.activate(second);
  expect(a.actions().at(-1)).toMatchObject({path:"/creator/status/123456",kind:"one",media:otherImage});
  expect(a.messages().every(m=>["archive"].includes(m.kind))).toBe(true);
});
it("keeps exact transparent-paint identity and grants photo-control keys only for the current settled slide",()=>{
  const a=provedCarousel();
  for(const [selector,url] of [["#full-photo",image],["#other-photo",otherImage]] as const){
    const img=a.doc.querySelector<HTMLElement>(selector)!;img.style.opacity="0";img.parentElement!.style.backgroundImage=`url("${url}")`;
  }
  a.visible();const control=a.doc.querySelector<HTMLButtonElement>("[data-atsumi-photo-save]")!;
  expect(control).not.toBeNull();expect(a.win.__atsumiXActions!.ownsPhotoControl(control)).toBe(true);
  a.location.pathname="/creator/status/123456/photo/2";
  expect(a.win.__atsumiXActions!.ownsPhotoControl(control)).toBe(false);
  a.slide(2);a.win.__atsumiXActions!.update();
  expect(a.win.__atsumiXActions!.ownsPhotoControl(control)).toBe(true);
  a.activate(control);expect(a.actions().at(-1)).toMatchObject({media:otherImage});
  a.win.__atsumiXActions!.visibility(false);
  expect(a.win.__atsumiXActions!.ownsPhotoControl(control)).toBe(false);
});
it.each<[string,(a:ReturnType<typeof provedCarousel>)=>void]>([
  ["missing source counter",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelector("span")!.remove()],
  ["wrong source counter",(a:ReturnType<typeof provedCarousel>)=>{a.doc.querySelector("span")!.textContent="슬라이드 2/2 - 회전 슬라이드";}],
  ["wrong total",(a:ReturnType<typeof provedCarousel>)=>{a.doc.querySelector("span")!.textContent="슬라이드 1/3 - 회전 슬라이드";}],
  ["conflicting accessible counter",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelector('[aria-roledescription]')!.setAttribute("aria-label","Slide 2 of 2")],
  ["wrong position",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelector("li")!.setAttribute("aria-posinset","2")],
  ["wrong current flag",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelectorAll("li")[1]!.setAttribute("aria-current","true")],
  ["wrong selected flag",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelectorAll("li")[1]!.setAttribute("aria-selected","true")],
  ["visible adjacent image",(a:ReturnType<typeof provedCarousel>)=>a.place("#other-photo",690)],
  ["current hidden",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelector("li")!.setAttribute("aria-hidden","true")],
  ["fixed image escaping clip",(a:ReturnType<typeof provedCarousel>)=>{a.doc.querySelector<HTMLElement>("#other-photo")!.style.position="fixed";}],
  ["absolute image escaping clip",(a:ReturnType<typeof provedCarousel>)=>{a.doc.querySelector<HTMLElement>("#other-photo")!.style.position="absolute";}],
  ["nested carousel",(a:ReturnType<typeof provedCarousel>)=>a.doc.body.setAttribute("role","carousel")],
  ["second list",(a:ReturnType<typeof provedCarousel>)=>a.doc.querySelector('[aria-roledescription]')!.append(a.doc.createElement("ul"))],
])("refuses incomplete or conflicting carousel proof: %s",(_label,change)=>{
  const a=provedCarousel();change(a);a.visible();
  expect(a.doc.querySelector("[data-atsumi-photo-save]")).toBeNull();
  a.context(a.doc.querySelector("#full-photo")!);
  expect(a.doc.querySelector('[role="menu"]')).toBeNull();expect(a.actions()).toHaveLength(0);
});
it("does not fall back to one stale candidate while route and source counter disagree",()=>{
  const a=provedCarousel();a.doc.querySelector("#other-photo")!.remove();
  a.location.pathname="/creator/status/123456/photo/2";a.visible();
  expect(a.doc.querySelector("[data-atsumi-photo-save]")).toBeNull();
  expect(a.actions()).toHaveLength(0);
});
it("accepts absolute sizing images only when a positioned parent proves the overflow relationship",()=>{
  const a=provedCarousel();
  for(const selector of ["#full-photo","#other-photo"]){
    const img=a.doc.querySelector<HTMLElement>(selector)!;
    img.style.position="absolute";img.parentElement!.style.position="relative";
  }
  a.visible();const button=a.doc.querySelector<HTMLButtonElement>("[data-atsumi-photo-save]")!;
  expect(button).not.toBeNull();a.activate(button);
  expect(a.actions().at(-1)).toMatchObject({media:image});
});
it("does not borrow a deduplicated identical resource from another slide to prove the current item",()=>{
  const a=provedCarousel();a.doc.querySelector<HTMLImageElement>("#other-photo")!.src=image;
  a.visible();expect(a.doc.querySelector("[data-atsumi-photo-save]")).toBeNull();
  expect(a.actions()).toHaveLength(0);
  a.slide(2);a.win.__atsumiXActions!.update();
  const button=a.doc.querySelector<HTMLButtonElement>("[data-atsumi-photo-save]")!;
  expect(button).not.toBeNull();a.activate(button);
  expect(a.actions().at(-1)).toMatchObject({media:image});
});
it("rechecks the photo binding when a stale context menu is activated after a slide change",()=>{
  const a=provedCarousel();a.visible();a.context(a.doc.querySelector("#full-photo")!);
  const old=a.menuButton("개별");
  expect(old).not.toBeUndefined();
  a.slide(2);a.activate(old);
  expect(a.actions()).toHaveLength(0);
  a.win.__atsumiXActions!.update();a.context(a.doc.querySelector("#other-photo")!);
  a.activate(a.menuButton("개별"));
  expect(a.actions().at(-1)).toMatchObject({media:otherImage});
});
it("settles a photo only on bounded source load, transition or resize and cancels stale owners/routes/visibility",()=>{
  const a=provedCarousel();a.place("#other-photo",690);a.visible();
  expect(a.doc.querySelector("[data-atsumi-photo-save]")).toBeNull();
  a.place("#other-photo",710);
  const root=a.doc.querySelector('[aria-roledescription]')!;
  for(let n=0;n<5;n++)a.emit(a.doc,"transitionend",a.event(root,{type:"transitionend"}));
  expect(a.timers).toHaveLength(1);a.timers.shift()?.();
  expect(a.doc.querySelector("[data-atsumi-photo-save]")).not.toBeNull();
  for(const invalidate of ["route","owner","hidden"]){
    const b=provedCarousel();b.place("#other-photo",690);b.visible();b.place("#other-photo",710);
    b.emit(b.doc,"load",b.event(b.doc.querySelector("#full-photo")!,{type:"load"}));
    if(invalidate==="route")b.location.pathname="/home";
    if(invalidate==="owner")b.proof.mockReturnValue("someone_else");
    if(invalidate==="hidden")b.win.__atsumiXActions!.visibility(false);
    b.timers.shift()?.();expect(b.doc.querySelector("[data-atsumi-photo-save]")).toBeNull();expect(b.actions()).toHaveLength(0);
  }
  const resized=provedCarousel();resized.place("#other-photo",690);resized.visible();resized.place("#other-photo",710);
  resized.emit(resized.windowEvents,"resize",{isTrusted:true,type:"resize"});
  resized.timers.shift()?.();expect(resized.doc.querySelector("[data-atsumi-photo-save]")).not.toBeNull();
});

it("initializes all concatenated native helpers before any document root exists and attaches styles after DOM readiness",()=>{
  const doc=new DOMParser().parseFromString("","text/html");doc.documentElement.remove();
  const timers:Array<()=>void>=[],message=vi.fn(),config=JSON.stringify({nonce:"fixture",owner:"alice",feed:"foryou"});
  const win={
    top:null as unknown,chrome:{webview:{postMessage:message}},addEventListener:vi.fn(),getSelection:()=>({toString:()=>""}),
    __atsumiXActions:undefined as undefined|{visibility(value:boolean):void;update():void},
    __atsumiXInput:undefined as undefined|{visibility(value:boolean):void},
    __atsumiXSurface:undefined as undefined|{visibility(value:boolean):void},
  };win.top=win;
  class Observer {observe(){}disconnect(){}}
  const location={origin:"https://x.com",protocol:"https:",hostname:"x.com",href:"https://x.com/home",pathname:"/home"};
  const combined=[
    script.replace("__ATSUMI_X_ACTIONS__",config),
    inputScript.replace("__ATSUMI_X_INPUT__",config),
    surfaceScript.replace("__ATSUMI_X_SURFACE__",config),
  ].join("\n");
  expect(doc.head).toBeNull();expect(doc.documentElement).toBeNull();
  expect(()=>new Function("window","document","location","URL","Element","HTMLMediaElement","MutationObserver","setTimeout","innerWidth","innerHeight",combined)(
    win,doc,location,URL,Element,HTMLMediaElement,Observer,(f:()=>void)=>{timers.push(f);return timers.length;},800,900,
  )).not.toThrow();
  expect(win.__atsumiXActions).toBeDefined();expect(win.__atsumiXInput).toBeDefined();expect(win.__atsumiXSurface).toBeDefined();
  expect(message).not.toHaveBeenCalled();
  const root=doc.createElement("html");
  root.innerHTML='<head><base href="https://x.com/"></head><body><a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main><div data-testid="primaryColumn">'+tweet()+'</div></main></body>';
  doc.append(root);doc.dispatchEvent(new Event("DOMContentLoaded"));
  expect([...doc.querySelectorAll("style")].filter(style=>style.textContent?.includes(".atsumi-x-inline-action"))).toHaveLength(1);
  timers.shift()?.();
  const lastMessage=message.mock.calls.at(-1)![0] as string;
  expect(JSON.parse(new URL(lastMessage.slice("ATSUMI_X_SURFACE_V1:".length)).searchParams.get("value")!).state).toBe("ready");
  expect(doc.querySelector('[data-atsumi-post-save]')).toBeNull();
  win.__atsumiXSurface!.visibility(true);
  expect(doc.querySelector('[data-atsumi-post-save]')).not.toBeNull();
  doc.dispatchEvent(new Event("DOMContentLoaded"));
  expect([...doc.querySelectorAll("style")].filter(style=>style.textContent?.includes(".atsumi-x-inline-action"))).toHaveLength(1);
});
it("installs native controls only in an admitted top-level visible X document",()=>{
  for(const options of [{host:"x.com.evil.test"},{host:"evil.test"},{subframe:true}]){
    const a=adapter(undefined,options);a.visible();expect(a.win.__atsumiXActions).toBeUndefined();expect(a.doc.querySelector('[data-atsumi-post-save],[data-atsumi-media-save]')).toBeNull();
  }
  const a=adapter();expect(a.doc.querySelector('[data-atsumi-post-save]')).toBeNull();a.visible();expect(a.doc.querySelector('[data-atsumi-post-save]')).not.toBeNull();expect(Object.isFrozen(a.win.__atsumiXActions)).toBe(true);
});
it("places save after existing social controls and download inside the actual media host without duplicates",()=>{
  const a=adapter();a.visible();a.win.__atsumiXActions!.update();a.win.__atsumiXActions!.update();
  const group=a.doc.querySelector('[data-testid="like"]')!.closest('[role="group"]')!,media=a.doc.querySelector('[data-testid="tweetPhoto"]')!;
  expect(group.lastElementChild?.getAttribute("data-atsumi-post-save")).toBe("true");expect(group.querySelectorAll('[data-atsumi-post-save]')).toHaveLength(1);
  expect(media.lastElementChild?.getAttribute("data-atsumi-media-rail")).toBe("true");expect(media.querySelectorAll(':scope > [data-atsumi-media-rail] > [data-atsumi-media-save]')).toHaveLength(1);
  expect(a.doc.querySelectorAll('[data-testid="like"],[data-testid="bookmark"],#share,img')).toHaveLength(4);
  const style=a.doc.querySelector("style")!.textContent!;expect(style).toContain("place-items:center!important");expect(style).toContain("top:8px!important;right:8px!important");expect(style).toContain("pointer-events:none");
});
it.each(["absolute","relative","fixed","sticky"])("preserves existing native %s media positioning and its image paint",position=>{
  const a=adapter();const root=a.doc.querySelector<HTMLElement>('[data-testid="tweetPhoto"]')!;
  root.style.cssText=`position:${position};inset:0;opacity:1;background-image:url("${image}")`;
  const before=root.getAttribute("style"),imageNode=root.querySelector("img");
  a.visible();a.win.__atsumiXActions!.update();
  expect(root.getAttribute("style")).toBe(before);expect(root.hasAttribute("data-atsumi-relative-host")).toBe(false);expect(root.querySelector("img")).toBe(imageNode);
  expect(a.doc.querySelector("style")!.textContent).not.toContain(".atsumi-x-media-host{position:relative!important}");
});
it("relativizes only static media hosts with a low-specificity rule and yields to later native positioning",()=>{
  const a=adapter();const root=a.doc.querySelector<HTMLElement>('[data-testid="tweetPhoto"]')!;a.visible();
  expect(root.getAttribute("data-atsumi-relative-host")).toBe("true");expect(root.style.position).toBe("");
  expect(a.doc.querySelector("style")!.textContent).toContain(":where(.atsumi-x-media-host[data-atsumi-relative-host]){position:relative}");
  root.style.position="absolute";a.win.__atsumiXActions!.update();
  expect(root.style.position).toBe("absolute");expect(root.hasAttribute("data-atsumi-relative-host")).toBe(false);
});
it("sends a single closed archive intent only from a trusted visible activation",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  button.click();expect(a.message).not.toHaveBeenCalled();a.activate(button,{isTrusted:false});expect(a.message).not.toHaveBeenCalled();
  const e=a.activate(button);expect(e.preventDefault).toHaveBeenCalledOnce();expect(e.stopImmediatePropagation).toHaveBeenCalledOnce();
  expect(a.actions()).toEqual([{id:expect.stringMatching(/^[0-9a-f-]{36}$/),path:"/creator/status/123456",kind:"archive",media:null}]);
  expect(a.message.mock.calls[0]![0]).toMatch(/^ATSUMI_X_SURFACE_V1:https:\/\/x\.com\/__atsumi_surface__\/fixture\?kind=archive&value=/);
  a.win.__atsumiXActions!.visibility(false);a.activate(button);expect(a.actions()).toHaveLength(1);
});
it("handles trusted app-owned clicks before an ancestor capture handler can trigger a native reaction",()=>{
  const a=adapter();a.visible();
  const ancestorLike=vi.fn(),button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  // Follow browser capture order: document first, then the native React root.
  const capture=(target:Element,extra:Record<string,unknown>={})=>{
    const event=a.event(target,extra);a.emit(a.doc,"click",event);
    if(event.stopImmediatePropagation.mock.calls.length===0)ancestorLike();
    return event;
  };
  const trusted=capture(button.querySelector("svg")!);
  expect(trusted.preventDefault).toHaveBeenCalled();expect(ancestorLike).not.toHaveBeenCalled();expect(a.actions()).toHaveLength(1);
  a.win.__atsumiXActions!.result(a.actions()[0].id,"done","Saved");
  capture(a.doc.querySelector('[data-testid="like"]')!);expect(ancestorLike).toHaveBeenCalledOnce();expect(a.actions()).toHaveLength(1);
  capture(button,{isTrusted:false});expect(a.actions()).toHaveLength(1);
  a.win.__atsumiXActions!.visibility(false);capture(button);expect(a.actions()).toHaveLength(1);
});
it("isolates pointer and keyboard activation without suppressing browser focus or button key defaults",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  for(const type of ["pointerdown","pointerup","mousedown","mouseup"]){
    const event=a.event(button,{button:0});a.emit(a.doc,type,event);
    expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();expect(event.preventDefault).not.toHaveBeenCalled();
  }
  for(const type of ["keydown","keyup"]){
    for(const key of ["Enter"," "]){
      const event=a.event(button,{key});a.emit(a.doc,type,event);
      expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();expect(event.preventDefault).not.toHaveBeenCalled();
    }
    const arrow=a.event(button,{key:"ArrowLeft"});a.emit(a.doc,type,arrow);expect(arrow.stopImmediatePropagation).not.toHaveBeenCalled();
  }
  expect(a.messages()).toEqual([{kind:"focus",value:""},{kind:"focus",value:""},{kind:"focus",value:""}]);expect(a.actions()).toHaveLength(0);
});
it("captures menu choices and cancellation but never hijacks spoofed markers or detached controls",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.emit(button,"pointerenter",a.event(button));const choice=a.railButton("개별");
  const choose=a.event(choice);a.emit(a.doc,"click",choose);expect(choose.stopImmediatePropagation).toHaveBeenCalled();expect(a.actions()[0].kind).toBe("one");
  const cancel=a.doc.querySelector<HTMLButtonElement>(".atsumi-x-save-notice button")!,cancelEvent=a.event(cancel);
  a.emit(a.doc,"click",cancelEvent);expect(cancelEvent.stopImmediatePropagation).toHaveBeenCalled();expect(a.messages().at(-1)?.kind).toBe("archiveCancel");
  const spoof=a.doc.createElement("button");spoof.dataset.atsumiMediaSave="true";a.doc.body.append(spoof);
  const spoofEvent=a.event(spoof);a.emit(a.doc,"click",spoofEvent);expect(spoofEvent.stopImmediatePropagation).not.toHaveBeenCalled();
  button.remove();const detached=a.event(button);a.emit(a.doc,"click",detached);expect(detached.stopImmediatePropagation).not.toHaveBeenCalled();
  expect(a.actions()).toHaveLength(1);
});
it("defaults a media-button click to all media and exposes one/all choices on a trusted hover",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.emit(button,"pointerenter",a.event(button,{isTrusted:false}));expect(a.doc.querySelector('[data-expanded]')).toBeNull();
  a.emit(button,"pointerenter",a.event(button));expect(a.railButton("개별")).not.toBeUndefined();expect(a.railButton("전체")).toBe(button);
  a.activate(a.railButton("개별"));expect(a.actions()[0]).toMatchObject({path:"/creator/status/123456",kind:"one",media:image});
  a.win.__atsumiXActions!.result(a.actions()[0].id,"done","Saved");
  a.activate(button);expect(a.actions()[1]).toMatchObject({kind:"all",media:null});expect(a.doc.querySelector('[role="menu"]')).toBeNull();
});
it("uses exactly two centered icon-only controls with brief tooltips and the all action pinned at the right edge",()=>{
  const a=adapter();a.visible();
  const rail=a.doc.querySelector<HTMLElement>('[data-atsumi-media-rail]')!,button=rail.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!,single=rail.querySelector<HTMLButtonElement>('[data-atsumi-single-save]')!;
  expect(rail.getAttribute("role")).toBe("toolbar");expect(single.hidden).toBe(true);
  expect(button.title).toBe("전체");expect(single.title).toBe("개별");
  expect(rail.textContent).toBe("");expect(rail.querySelectorAll("button")).toHaveLength(2);
  expect(rail.lastElementChild).toBe(button);expect(single.querySelector("svg")).not.toBeNull();
  a.emit(button,"pointerenter",a.event(button));
  expect(rail.getAttribute("data-expanded")).toBe("true");expect(single.hidden).toBe(false);
  expect(button.getAttribute("aria-expanded")).toBe("true");expect(a.doc.querySelector('[role="menu"]')).toBeNull();
  const css=a.doc.querySelector("style")!.textContent!;
  expect(css).toContain("right:8px");expect(css).toContain("justify-content:flex-end");
  expect(css).toContain(".atsumi-x-download-rail[data-expanded]{width:68px}");
  expect(css).toContain(".atsumi-x-download-rail>[hidden]{display:none!important}");
  expect(css).toContain(".atsumi-x-media-host:hover>.atsumi-x-download-rail,.atsumi-x-media-host:focus-within>.atsumi-x-download-rail{opacity:1;pointer-events:auto}");
  // Presentation controls must not replace the first native reaction-count group.
  expect(a.doc.querySelector('[role="group"] [data-testid="like"]')).not.toBeNull();
});
it("collapses the extra icon on trusted pointer/focus leave, outside input and Escape, but preserves travel within the rail",()=>{
  const a=adapter();a.visible();
  const rail=a.doc.querySelector<HTMLElement>('[data-atsumi-media-rail]')!,button=rail.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!,single=rail.querySelector<HTMLButtonElement>('[data-atsumi-single-save]')!;
  const open=()=>a.emit(button,"pointerenter",a.event(button));
  open();a.emit(rail,"pointerleave",a.event(rail,{relatedTarget:a.doc.body,isTrusted:false}));expect(single.hidden).toBe(false);
  a.emit(rail,"pointerleave",a.event(rail,{relatedTarget:single}));expect(single.hidden).toBe(false);
  a.emit(rail,"focusout",a.event(button,{relatedTarget:single}));expect(single.hidden).toBe(false);
  for(const type of ["pointerleave","focusout"]){
    a.emit(rail,type,a.event(rail,{relatedTarget:a.doc.body}));expect(single.hidden).toBe(true);
    expect(button.getAttribute("aria-expanded")).toBe("false");a.activate(single);expect(a.actions()).toHaveLength(0);open();
  }
  a.emit(a.doc,"pointerdown",a.event(a.doc.body));expect(single.hidden).toBe(true);open();
  const escape=a.event(button,{key:"Escape"});a.emit(a.doc,"keydown",escape);
  expect(escape.preventDefault).toHaveBeenCalledOnce();expect(escape.stopImmediatePropagation).toHaveBeenCalledOnce();expect(single.hidden).toBe(true);
  expect(a.actions()).toHaveLength(0);
});
it("keeps only the current rail expanded and revokes it on a stale article or media binding",()=>{
  const a=adapter(tweet('<div id="second" data-testid="tweetPhoto"><img src="'+otherImage+'"></div>'));a.visible();
  const first=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!,second=a.doc.querySelector<HTMLButtonElement>('#second [data-atsumi-media-save]')!;
  a.emit(first,"focus",a.event(first));a.emit(second,"focus",a.event(second));
  expect(a.doc.querySelectorAll('[data-expanded]')).toHaveLength(1);expect(second.parentElement!.hasAttribute("data-expanded")).toBe(true);
  a.doc.querySelector('#second img')!.setAttribute("src",image);a.win.__atsumiXActions!.update();
  expect(a.doc.querySelector('[data-expanded]')).toBeNull();
  expect(a.actions()).toHaveLength(0);
});
it("uses a muted outlined archive icon and aligns save to the unchanged native action row",()=>{
  const a=adapter();const group=a.doc.querySelector('[role="group"]')!,before=group.getAttribute("style");a.visible();
  const save=group.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  expect(group.getAttribute("style")).toBe(before);expect(group.className).toBe("");
  expect(save.querySelector("path")!.getAttribute("d")).toBe("M3 4h18v4H3zM5 8v12h14V8M10 12h4");
  expect(save.querySelector("svg")!.getAttribute("fill")).toBe("none");
  expect(a.doc.querySelector("style")!.textContent).toContain("[data-atsumi-post-save]{color:#71767b!important;align-self:center!important;margin-block:0!important");
});
it("uses the observed video poster as a selector, not an invented stream URL",()=>{
  const poster="https://pbs.twimg.com/ext_tw_video_thumb/123456/pu/img/Fixture.jpg";
  const a=adapter(tweet('<div data-testid="videoPlayer"><video src="blob:https://x.com/fixture" poster="'+poster+'"></video><div data-testid="tweetPhoto"><img src="'+image+'"></div></div>'));a.visible();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);
  const button=a.doc.querySelector<HTMLButtonElement>('[data-testid="videoPlayer"] > [data-atsumi-media-rail] > [data-atsumi-media-save]')!;
  a.emit(button,"focus",a.event(button));a.activate(a.railButton("개별"));
  expect(a.actions()[0]).toMatchObject({kind:"one",media:poster});expect(a.message.mock.calls[0]![0]).not.toContain("blob");
});
it.each([
  ["tweetPhoto","videoPlayer"],
  ["videoPlayer","tweetPhoto"],
  ["tweetPhoto","tweetPhoto"],
  ["videoPlayer","videoPlayer"],
])("installs one media control across nested %s/%s boundaries",(outer,inner)=>{
  const poster="https://pbs.twimg.com/ext_tw_video_thumb/123456/pu/img/Fixture.jpg";
  const a=adapter(tweet('<div id="outer" data-testid="'+outer+'"><div id="inner" data-testid="'+inner+'"><video src="blob:https://x.com/video" poster="'+poster+'"></video></div></div>'));a.visible();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);
  expect(a.doc.querySelectorAll('#outer > [data-atsumi-media-rail] > [data-atsumi-media-save]')).toHaveLength(1);
  expect(a.doc.querySelector('#inner [data-atsumi-media-save]')).toBeNull();
  a.context(a.doc.querySelector("video")!);a.activate(a.menuButton("개별"));
  expect(a.actions()[0]).toMatchObject({kind:"one",media:poster});
});
it("reconciles existing controls when native hydration wraps, replaces or unwraps a media boundary",()=>{
  const poster="https://pbs.twimg.com/ext_tw_video_thumb/123456/pu/img/Fixture.jpg";
  const a=adapter(tweet('<div id="hydrating" data-testid="videoPlayer"><video poster="'+poster+'"></video></div>'));a.visible();
  const inner=a.doc.querySelector<HTMLElement>("#hydrating")!,video=inner.querySelector("video")!,oldButton=inner.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.emit(oldButton,"pointerenter",a.event(oldButton));const oldChoice=a.railButton("개별");
  const outer=a.doc.createElement("div");outer.dataset.testid="tweetPhoto";outer.id="new-boundary";
  inner.before(outer);outer.append(inner);
  a.activate(oldChoice);a.activate(oldButton);expect(a.message).not.toHaveBeenCalled();
  a.win.__atsumiXActions!.update();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);expect(oldButton.isConnected).toBe(false);
  expect(inner.classList.contains("atsumi-x-media-host")).toBe(false);expect(outer.classList.contains("atsumi-x-media-host")).toBe(true);
  const outerRail=outer.querySelector<HTMLElement>(":scope > [data-atsumi-media-rail]")!;
  inner.append(outerRail.cloneNode(true));a.win.__atsumiXActions!.update();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);expect(inner.querySelector('[data-atsumi-media-save]')).toBeNull();
  outer.before(inner);outer.remove();a.win.__atsumiXActions!.update();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);
  expect(inner.querySelectorAll(':scope > [data-atsumi-media-rail] > [data-atsumi-media-save]')).toHaveLength(1);expect(video.isConnected).toBe(true);
});
it("keeps separate sibling media controls and their observed one-media selectors independent",()=>{
  const a=adapter(tweet('<div id="sibling" data-testid="tweetPhoto"><img id="other-image" src="'+otherImage+'"></div>'));a.visible();
  expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(2);
  a.context(a.doc.querySelector("#other-image")!);a.activate(a.menuButton("개별"));
  expect(a.actions()[0]).toMatchObject({kind:"one",media:otherImage});
  a.win.__atsumiXActions!.result(a.actions()[0].id,"done","Saved");
  a.context(a.doc.querySelector("#image")!);a.activate(a.menuButton("개별"));
  expect(a.actions()[1]).toMatchObject({kind:"one",media:image});
});
it.each(["https://evil.test/media.jpg","https://pbs.twimg.com.evil.test/media.jpg","http://pbs.twimg.com/media.jpg","https://pbs.twimg.com:444/media.jpg","https://user:pass@pbs.twimg.com/media.jpg","https://pbs.twimg.com/media.jpg#fragment","file:///C:/private.jpg"])("never submits an unadmitted displayed media selector %s",url=>{
  const a=adapter(tweet().replace(image.replaceAll("&","&amp;"),url));a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.emit(button,"pointerenter",a.event(button));expect(a.railButton("개별")).toBeUndefined();expect(button.title).toBe("전체");
  a.activate(button);expect(a.actions()[0]).toMatchObject({kind:"all",media:null});
});
it("revalidates the article permalink for direct controls after virtualized content changes",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  a.doc.querySelector("#permalink")!.setAttribute("href","/different/status/999999");a.activate(button);expect(a.actions()[0]).toMatchObject({path:"/different/status/999999"});
  a.doc.querySelector("#permalink")!.setAttribute("href","/home");a.activate(button);expect(a.actions()).toHaveLength(1);
});
it("admits the main detail post from its own footer timestamp only on its exact current route",()=>{
  const detail=tweet().replace('<a id="permalink" href="/creator/status/123456"><time>today</time></a>','').replace('<p data-testid="tweetText">Saved post</p>','<p data-testid="tweetText">Saved post</p><a id="footer-time" href="/creator/status/123456"><time>today</time></a>');
  const a=adapter(detail,{path:"/creator/status/123456"});a.visible();
  expect(a.doc.querySelectorAll('[data-atsumi-post-save]')).toHaveLength(1);expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(1);
  a.activate(a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!);expect(a.actions()[0].path).toBe("/creator/status/123456");
  for(const path of ["/home","/creator/status/999999"]){
    const other=adapter(detail,{path});other.visible();expect(other.doc.querySelector('[data-atsumi-post-save],[data-atsumi-media-save]')).toBeNull();
  }
});
it("never borrows a quoted footer timestamp or resolves ambiguous own detail timestamps",()=>{
  const base=tweet().replace('<a id="permalink" href="/creator/status/123456"><time>today</time></a>','');
  const own='<a href="/creator/status/123456"><time>today</time></a>';
  for(const extra of [
    '<div data-testid="quoteTweet">'+own+'</div>',
    '<div role="link">'+own+'</div>',
    own+'<a href="/other/status/999999"><time>other</time></a>',
  ]){
    const a=adapter(base.replace('<p data-testid="tweetText">',extra+'<p data-testid="tweetText">'),{path:"/creator/status/123456"});a.visible();
    expect(a.doc.querySelector('[data-atsumi-post-save],[data-atsumi-media-save]')).toBeNull();
  }
});
it("does not borrow a quoted permalink or duplicate an action into a quote",()=>{
  const a=adapter(tweet(quoted));a.visible();
  expect(a.doc.querySelectorAll('[data-atsumi-post-save]')).toHaveLength(1);expect(a.doc.querySelectorAll('[data-atsumi-media-save]')).toHaveLength(1);
  expect(a.doc.querySelector('[data-testid="quoteTweet"] [data-atsumi-media-save]')).toBeNull();
  a.doc.querySelector("#permalink")!.remove();a.doc.querySelector('[data-atsumi-post-save]')?.remove();a.win.__atsumiXActions!.update();expect(a.doc.querySelector('[data-atsumi-post-save]')).toBeNull();
});
it("revokes direct controls when native DOM recycling moves their root into a quote",()=>{
  const a=adapter(tweet(quoted));a.visible();const download=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!,save=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  const quote=a.doc.querySelector('[data-testid="quoteTweet"]')!;
  quote.append(download.parentElement!,save.parentElement!);
  a.activate(download);a.activate(save);a.emit(download,"pointerenter",a.event(download));
  expect(a.message).not.toHaveBeenCalled();expect(a.doc.querySelector('[role="menu"]')).toBeNull();
});
it("does not intercept exact author/quoted links or linkless quoted text as the outer post",()=>{
  const a=adapter(tweet(quoted));a.visible();
  for(const selector of ["#profile",'[data-testid="quoteTweet"] a',"#quote-body"]){
    const event=a.context(a.doc.querySelector(selector)!);expect(event.preventDefault).not.toHaveBeenCalled();expect(event.stopImmediatePropagation).not.toHaveBeenCalled();
  }
  expect(a.doc.querySelector('[role="menu"]')).toBeNull();expect(a.message).not.toHaveBeenCalled();
});
it("offers one/all media and external context actions for the exact owned media",()=>{
  const a=adapter();a.visible();const e=a.context(a.doc.querySelector("#image")!);
  expect(e.preventDefault).toHaveBeenCalledOnce();expect(e.stopImmediatePropagation).toHaveBeenCalledOnce();
  expect(a.menuButton("개별")).not.toBeUndefined();expect(a.menuButton("게시물 보관")).not.toBeUndefined();
  a.activate(a.menuButton("외부 브라우저에서 열기"));expect(a.messages()).toEqual([{kind:"external",value:"/creator/status/123456"}]);
});
it("opens body/comment selection only through the trusted current-post context choice",()=>{
  const a=adapter();a.visible();a.context(a.doc.querySelector("#image")!);
  const choose=a.menuButton("본문·댓글 선택 보관");a.activate(choose,{isTrusted:false});expect(a.message).not.toHaveBeenCalled();
  a.activate(choose);expect(a.messages()).toEqual([{kind:"archiveChoose",value:"/creator/status/123456"}]);expect(a.actions()).toHaveLength(0);
});
it("clamps the complete context menu after its final choices are present",()=>{
  vi.spyOn(HTMLElement.prototype,"offsetWidth","get").mockImplementation(function(this:HTMLElement){return this.getAttribute("role")==="menu"?232:0;});
  vi.spyOn(HTMLElement.prototype,"offsetHeight","get").mockImplementation(function(this:HTMLElement){return this.getAttribute("role")==="menu"?this.childElementCount*40+8:0;});
  const a=adapter();a.visible();a.context(a.doc.querySelector("#image")!,{clientX:799,clientY:899});
  const menu=a.doc.querySelector<HTMLElement>('[role="menu"]')!;
  expect(menu.childElementCount).toBe(5);expect(menu.style.top).toBe("684px");expect(menu.style.left).toBe("560px");
  expect(a.doc.querySelector("style")!.textContent).toContain("max-height:calc(100vh - 16px);overflow-y:auto");
});
it("discards a media menu if its article or displayed resource changes before selection",()=>{
  for(const change of ["path","media","detached"]){
    const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
    a.emit(button,"pointerenter",a.event(button));const choice=a.railButton("개별");
    if(change==="path")a.doc.querySelector("#permalink")!.setAttribute("href","/different/status/999999");
    else if(change==="media")a.doc.querySelector("#image")!.setAttribute("src",otherImage);
    else a.doc.querySelector("article")!.remove();
    a.activate(choice);expect(a.message).not.toHaveBeenCalled();
  }
});
it("keeps input context menus native and requires trusted context events",()=>{
  const a=adapter(tweet('<input id="typing"><textarea id="notes"></textarea><div contenteditable="true" id="edit">Editable</div>'));a.visible();
  for(const selector of ["#typing","#notes","#edit"])expect(a.context(a.doc.querySelector(selector)!).preventDefault).not.toHaveBeenCalled();
  expect(a.context(a.doc.querySelector("#image")!,{isTrusted:false}).preventDefault).not.toHaveBeenCalled();expect(a.doc.querySelector('[role="menu"]')).toBeNull();
});
it("revokes open menu/toast on concealment and ignores unknown or stale result IDs",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-media-save]')!;
  a.activate(button);const id=a.actions()[0].id;a.win.__atsumiXActions!.result("unknown","done","Foreign");expect(a.doc.body.textContent).not.toContain("Foreign");
  a.emit(button,"pointerenter",a.event(button));expect(a.doc.querySelector('[data-expanded]')).not.toBeNull();
  a.win.__atsumiXActions!.visibility(false);expect(a.doc.querySelector('[data-expanded],[role="menu"],.atsumi-x-save-notice')).toBeNull();
  a.win.__atsumiXActions!.result(id,"done","Finished while hidden");a.visible();expect(a.doc.body.textContent).not.toContain("Finished while hidden");
});
it.each([
  ["[data-atsumi-post-save]","요청 확인 중…"],
  ["[data-atsumi-media-save]","요청 확인 중…"],
])("restores the existing cancellable job after hide/show without submitting or cancelling it: %s",(selector,message)=>{
  const a=adapter();a.visible();a.activate(a.doc.querySelector<HTMLButtonElement>(selector)!);const id=a.actions()[0].id;
  a.win.__atsumiXActions!.visibility(false);expect(a.doc.querySelector(".atsumi-x-save-notice")).toBeNull();expect(a.messages()).toHaveLength(1);
  a.visible();const cancel=a.doc.querySelector<HTMLButtonElement>(".atsumi-x-save-notice button")!;
  expect(a.doc.querySelector(".atsumi-x-save-notice")!.textContent).toContain(message);expect(cancel.disabled).toBe(false);
  expect(a.actions()).toHaveLength(1);expect(a.messages()).toHaveLength(1);
  a.visible();expect(a.doc.querySelector(".atsumi-x-save-notice button")).toBe(cancel);
  a.activate(cancel);expect(a.messages().at(-1)).toEqual({kind:"archiveCancel",value:id});
  a.win.__atsumiXActions!.visibility(false);a.visible();
  const restoredCancel=a.doc.querySelector<HTMLButtonElement>(".atsumi-x-save-notice button")!;
  expect(restoredCancel.disabled).toBe(true);a.activate(restoredCancel);
  expect(a.messages().filter(value=>value.kind==="archiveCancel")).toHaveLength(1);
  a.win.__atsumiXActions!.result(id,"done","Cancelled");a.win.__atsumiXActions!.visibility(false);a.visible();
  expect(a.doc.querySelector(".atsumi-x-save-notice")).toBeNull();
});
it("keeps one cancellable job per document so another job cannot replace its progress toast",()=>{
  const a=adapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-post-save]')!;
  a.activate(button);a.activate(button);a.activate(button);expect(a.actions()).toHaveLength(1);
  const cancel=a.doc.querySelector<HTMLButtonElement>(".atsumi-x-save-notice button")!;
  cancel.click();expect(a.messages().filter(value=>value.kind==="archiveCancel")).toHaveLength(0);a.activate(cancel);
  expect(a.messages().at(-1)).toEqual({kind:"archiveCancel",value:a.actions()[0].id});expect(cancel.disabled).toBe(true);
  a.win.__atsumiXActions!.result("unknown","done","Unrelated result");expect(a.doc.querySelector(".atsumi-x-save-notice button")).toBe(cancel);
  a.win.__atsumiXActions!.result(a.actions()[0].id,"done","Saved");a.activate(button);expect(a.actions()).toHaveLength(2);
});
it("offers only the current observed photo on article-less full-page routes with proven ownership",()=>{
  const a=photoAdapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
  expect(button?.getAttribute("aria-label")).toBe("개별");expect(button.style.left).toBe("660px");expect(button.style.top).toBe("58px");
  expect(a.doc.querySelector('[data-atsumi-post-save],[data-atsumi-media-save]')).toBeNull();
  a.emit(button,"pointerenter",a.event(button));expect(a.doc.querySelector('[role="menu"]')).toBeNull();
  a.context(a.doc.querySelector("#full-photo")!);expect(a.doc.querySelectorAll('[role="menuitem"]')).toHaveLength(2);
  expect(a.menuButton("전체")).toBeUndefined();expect(a.menuButton("본문·댓글 선택 보관")).toBeUndefined();
  a.activate(a.menuButton("개별"));
  expect(a.actions()[0]).toMatchObject({path:"/creator/status/123456",kind:"one",media:image});
  a.win.__atsumiXActions!.result(a.actions()[0].id,"done","Saved");a.activate(button);
  expect(a.actions()[1]).toMatchObject({path:"/creator/status/123456",kind:"one",media:image});
});
it("reveals the standalone photo button only over the proved photo/control or keyboard focus and hides it on exit",()=>{
  const a=photoAdapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!,photo=a.doc.querySelector("#full-photo")!;
  expect(button.hasAttribute("data-media-hover")).toBe(false);
  a.emit(a.doc,"pointerover",a.event(photo,{isTrusted:false}));expect(button.hasAttribute("data-media-hover")).toBe(false);
  a.emit(a.doc,"pointerover",a.event(photo));expect(button.hasAttribute("data-media-hover")).toBe(true);
  a.emit(a.doc,"pointerout",a.event(photo,{relatedTarget:button}));expect(button.hasAttribute("data-media-hover")).toBe(true);
  a.emit(a.doc,"pointerout",a.event(button,{relatedTarget:a.doc.body}));expect(button.hasAttribute("data-media-hover")).toBe(false);
  a.emit(a.doc,"pointerover",a.event(a.doc.body));expect(button.hasAttribute("data-media-hover")).toBe(false);
  expect(a.doc.querySelector("style")!.textContent).toContain(".atsumi-x-photo-action[data-media-hover],.atsumi-x-photo-action:focus{opacity:1;pointer-events:auto}");
  expect(a.actions()).toHaveLength(0);
});
it("gives the current-photo context menu an exact photo route without claiming full-post scope",()=>{
  const a=photoAdapter();a.visible();expect(a.context(a.doc.body).preventDefault).not.toHaveBeenCalled();
  expect(a.context(a.doc.querySelector("#full-photo")!).preventDefault).toHaveBeenCalledOnce();
  expect([...a.doc.querySelectorAll('[role="menuitem"]')].map(item=>item.textContent)).toEqual(["개별","외부 브라우저에서 열기"]);
  a.activate(a.menuButton("외부 브라우저에서 열기"));expect(a.messages()).toEqual([{kind:"external",value:"/creator/status/123456/photo/1"}]);
});
it("offers only the independent current modal photo while ignoring background feed and article media",()=>{
  const modal=photoHtml.replaceAll(image.replaceAll("&","&amp;"),otherImage.replaceAll("&","&amp;"));
  const a=photoAdapter(tweet()+'<div role="dialog">'+modal+tweet()+'</div>');a.visible();
  const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
  expect(button?.getAttribute("aria-label")).toBe("개별");
  a.emit(button,"pointerenter",a.event(button));expect(a.doc.querySelector('[role="menu"]')).toBeNull();
  a.activate(button);
  expect(a.actions()[0]).toMatchObject({path:"/creator/status/123456",kind:"one",media:otherImage});
});
it("uses one visible leaf photo dialog and never guesses among unrelated overlays or a hidden modal",()=>{
  const allowed=[
    '<div role="dialog"><img src="'+otherImage+'"><div aria-modal="true">'+photoHtml+'</div></div>',
    '<div role="dialog" hidden><img src="'+otherImage+'"></div><div role="dialog">'+photoHtml+'</div>',
    '<img src="'+otherImage+'">'.repeat(130)+'<div role="dialog">'+photoHtml+'</div>',
  ];
  for(const html of allowed){
    const a=photoAdapter(html);a.visible();expect(a.doc.querySelector('[data-atsumi-photo-save]')).not.toBeNull();
  }
  const denied=[
    '<div role="dialog">'+photoHtml+'</div><div role="dialog"><img src="'+otherImage+'"></div>',
    '<div role="dialog" hidden>'+photoHtml+'</div>',
    '<div role="dialog">'+photoHtml+'<div aria-modal="true">Confirm</div></div>',
    '<div role="dialog">'+photoHtml+'<img src="'+otherImage+'"></div>',
    photoHtml+'<div role="dialog">No photo</div>',
    '<div role="dialog">'+tweet()+'</div>',
    '<div role="dialog">'.repeat(9)+photoHtml+'</div>'.repeat(9),
  ];
  for(const html of denied){
    const a=photoAdapter(html);a.visible();expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  }
});
it("uses modal context only for its actual photo and rejects stale overlay identity before dispatch",()=>{
  const a=photoAdapter(tweet()+'<div id="modal" role="dialog">'+photoHtml+'</div>');a.visible();
  const imageNode=a.doc.querySelector("#full-photo")!;
  expect(a.context(imageNode).preventDefault).toHaveBeenCalledOnce();
  expect(a.menuButton("전체")).toBeUndefined();
  const choice=a.menuButton("개별");
  const next=a.doc.createElement("div");next.setAttribute("role","dialog");next.append(a.doc.querySelector("#photo-host")!);
  a.doc.querySelector("#modal")!.replaceWith(next);
  a.activate(choice);expect(a.message).not.toHaveBeenCalled();
  a.win.__atsumiXActions!.update();a.context(imageNode);a.activate(a.menuButton("개별"));
  expect(a.actions()[0]).toMatchObject({kind:"one",media:image});
});
it("supports the existing exact painted sizing-photo case inside the scoped modal",()=>{
  const a=photoAdapter(tweet()+'<div role="dialog">'+photoHtml+'</div>');
  a.doc.querySelector<HTMLElement>("#photo-host")!.style.backgroundImage='url("'+image+'")';
  a.doc.querySelector<HTMLElement>("#full-photo")!.style.opacity="0";a.visible();
  const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
  expect(button).not.toBeNull();a.activate(button);expect(a.actions()[0]).toMatchObject({kind:"one",media:image});
});
it("does not create photo controls from a guessed account, non-photo route, dialogs or an existing article",()=>{
  for(const options of [{photoOwner:""},{photoOwner:"invalid/name"},{path:"/creator/status/123456"},{path:"/creator/status/123456/video/1"},{host:"www.x.com"}]){
    const a=photoAdapter(photoHtml,options);a.visible();expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  }
  for(const extra of [tweet(),'<div role="dialog"></div>','<div aria-modal="true"></div>']){
    const a=photoAdapter(photoHtml+extra);a.visible();expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  }
});
it("requires one unique visible in-viewport permitted photo within the bounded full-page scan",()=>{
  for(const html of [
    photoHtml+'<img src="'+otherImage+'">',
    photoHtml.replace("pbs.twimg.com","evil.test"),
    photoHtml.replace("/media/","/profile_images/"),
    '<div hidden>'+photoHtml+'</div>',
    '<div aria-hidden="true">'+photoHtml+'</div>',
    '<div style="opacity:0">'+photoHtml+'</div>',
    photoHtml+'<img>'.repeat(128),
  ]){
    const a=photoAdapter(html);a.visible();expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  }
  const outside=photoAdapter();vi.mocked(outside.doc.querySelector<HTMLElement>("#full-photo")!.getBoundingClientRect).mockReturnValue({x:900,y:0,left:900,top:0,right:1000,bottom:200,width:100,height:200,toJSON:()=>({})});
  outside.visible();expect(outside.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
  const hiddenOther=photoAdapter(photoHtml+'<img hidden src="'+otherImage+'">');hiddenOther.visible();expect(hiddenOther.doc.querySelector('[data-atsumi-photo-save]')).not.toBeNull();
});
it("matches only the exact visible painted background for a transparent full-page sizing image",()=>{
  const a=photoAdapter();const host=a.doc.querySelector<HTMLElement>("#photo-host")!,sizing=a.doc.querySelector<HTMLElement>("#full-photo")!;
  host.style.backgroundImage='url("'+image+'")';sizing.style.opacity="0";a.visible();
  expect(a.doc.querySelector('[data-atsumi-photo-save]')).not.toBeNull();
  expect(a.context(host).preventDefault).toHaveBeenCalledOnce();a.activate(a.menuButton("개별"));expect(a.actions()[0].media).toBe(image);
  const b=photoAdapter();b.doc.querySelector<HTMLElement>("#photo-host")!.style.backgroundImage='url("'+otherImage+'")';b.doc.querySelector<HTMLElement>("#full-photo")!.style.opacity="0";b.visible();
  expect(b.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();
});
it("revokes photo controls and pending menu choices when proof, route, image or visibility changes",()=>{
  for(const change of ["proof","route","image","hidden"]){
    const a=photoAdapter();a.visible();const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
    a.context(a.doc.querySelector("#full-photo")!);const choice=a.menuButton("개별");
    if(change==="proof")a.proof.mockReturnValue("");
    else if(change==="route")a.location.pathname="/creator/status/123456/photo/2";
    else if(change==="image")a.doc.querySelector("#full-photo")!.setAttribute("src",otherImage);
    else a.win.__atsumiXActions!.visibility(false);
    a.activate(choice);expect(a.message).not.toHaveBeenCalled();
    if(change==="proof"||change==="hidden"){a.win.__atsumiXActions!.update();expect(a.doc.querySelector('[data-atsumi-photo-save]')).toBeNull();}
  }
});
it("grants photo keyboard focus only to the exact current connected photo control with its action menu closed",()=>{
  const a=photoAdapter('<div role="dialog">'+photoHtml+'</div>');a.visible();
  const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!,owns=a.win.__atsumiXActions!.ownsPhotoControl;
  expect(owns(button)).toBe(true);expect(owns(button.querySelector("svg"))).toBe(true);
  const ordinary=a.doc.createElement("button");ordinary.dataset.atsumiPhotoSave="true";a.doc.body.append(ordinary);
  expect(owns(ordinary)).toBe(false);expect(owns(null)).toBe(false);expect(owns(a.doc.querySelector("#full-photo"))).toBe(false);
  a.context(a.doc.querySelector("#full-photo")!);
  expect(owns(button)).toBe(false);expect(owns(a.menuButton("개별"))).toBe(false);
  a.activate(a.menuButton("개별"));expect(owns(button)).toBe(true);
  expect(owns(a.doc.querySelector(".atsumi-x-save-notice button"))).toBe(false);
  a.win.__atsumiXActions!.visibility(false);expect(owns(button)).toBe(false);
  a.visible();expect(owns(button)).toBe(false);
  expect(owns(a.doc.querySelector('.atsumi-x-photo-action'))).toBe(true);
});
it("revokes photo keyboard focus before a changed image, route, owner or modal is reconciled",()=>{
  for(const change of ["image","route","owner","scope","detached"]){
    const a=photoAdapter('<div id="dialog" role="dialog">'+photoHtml+'</div>');a.visible();
    const button=a.doc.querySelector<HTMLButtonElement>('[data-atsumi-photo-save]')!;
    expect(a.win.__atsumiXActions!.ownsPhotoControl(button)).toBe(true);
    if(change==="image")a.doc.querySelector("#full-photo")!.setAttribute("src",otherImage);
    else if(change==="route")a.location.pathname="/creator/status/123456/photo/2";
    else if(change==="owner")a.proof.mockReturnValue("bob");
    else if(change==="scope"){
      const next=a.doc.createElement("div");next.setAttribute("role","dialog");next.append(a.doc.querySelector("#photo-host")!);a.doc.querySelector("#dialog")!.replaceWith(next);
    }else button.remove();
    expect(a.win.__atsumiXActions!.ownsPhotoControl(button)).toBe(false);expect(a.message).not.toHaveBeenCalled();
  }
  const feed=adapter();feed.visible();expect(feed.win.__atsumiXActions!.ownsPhotoControl(feed.doc.querySelector('[data-atsumi-media-save]'))).toBe(false);
});
it("contains no direct remote request, private-site API or arbitrary application RPC",()=>{
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\s*\(|XMLHttpRequest|__INITIAL_STATE__|graphql|__TAURI|invoke\s*\(|location\.assign|window\.open/);
  expect(script).toContain("pending.size>=MAX_PENDING");expect(script).toContain("slice(0,100)");expect(script).toContain("roots.slice(0,16)");
});
