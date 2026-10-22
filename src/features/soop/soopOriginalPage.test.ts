import { afterEach, describe, expect, it, vi } from 'vitest';
import page from '../../../src-tauri/src/soop/original_page.js?raw';
import capture from '../../../src-tauri/src/soop/original_capture.js?raw';
const vmName='node:vm';
const {runInNewContext}=await import(vmName) as {runInNewContext(source:string,context:Record<string,unknown>):unknown};
type Adapter={configure(v:unknown):boolean;layout(v:unknown):void;status():Record<string,unknown>;start(id:string):Promise<unknown>;reply(id:string,v:unknown):void};
function fixture(url='https://play.sooplive.com/fixture/12345/embed?fromApi=1',nested=false,beforeHtml=false){
  const messages:string[]=[],posts:unknown[]=[],styles=new Map<string,string>(),nodes=new Map<string,unknown>(),events=new Map<string,(event:unknown)=>void>();let tick=()=>{};
  const video={tagName:'VIDEO',readyState:4,videoWidth:960,ended:false};nodes.set('livePlayer',video);nodes.set('chatting_area',{getBoundingClientRect:()=>({width:240,height:500})});
  const html={appendChild:vi.fn((node:{id:string})=>nodes.set(node.id,node)),style:{setProperty:(key:string,value:string)=>styles.set(key,value)}};
  const document={documentElement:beforeHtml?null:html,head:null,getElementById:(id:string)=>nodes.get(id),createElement:()=>({id:'',textContent:''}),addEventListener:(name:string,handler:(event:unknown)=>void)=>events.set(name,handler),get cookie():never{throw Error('No cookie access');}};
  const location=new URL(url);const original={start:vi.fn().mockResolvedValue({}),getStatus:()=>({ready:true,recording:false,recordingId:null})};
  const window={top:null as unknown,livePlayer:{liveAPI:{getResponse:()=>({CHANNEL:{BJID:'fixture',BNO:12345}})}},liveView:{isLoaded:()=>Promise.resolve(),isLogin:()=>true},chrome:{webview:{postMessage:(v:string)=>messages.push(v)}},postMessage:(v:unknown)=>posts.push(v),addEventListener:vi.fn(),__atsumiSoopEncodedCapture:original,__atsumiSoopOriginal:undefined as Adapter|undefined};window.top=nested?{}:window;
  runInNewContext(page,{window,location,document,crypto:{randomUUID:()=>crypto.randomUUID()},setInterval:(fn:()=>void)=>{tick=fn;return 1;},clearInterval:vi.fn(),setTimeout,clearTimeout});
  return {window,adapter:window.__atsumiSoopOriginal,posts,messages,styles,events,tick:()=>tick(),original,html,nodes,parseHtml:()=>{document.documentElement=html;events.get('DOMContentLoaded')?.({});}};
}
afterEach(()=>vi.restoreAllMocks());
describe('SOOP original page adapter',()=>{
  it('installs before HTML exists and retries scoped styles once after DOMContentLoaded',async()=>{
    const f=fixture(undefined,false,true);expect(f.adapter).toBeDefined();
    expect(f.adapter!.configure({epoch:1,channel:'fixture'})).toBe(true);
    f.adapter!.layout({player:{x:0,y:0,width:800,height:450},chat:{x:800,y:0,width:480,height:450}});
    expect(f.html.appendChild).not.toHaveBeenCalled();f.parseHtml();
    for(let i=0;i<8;i++)await Promise.resolve();
    expect(f.nodes.has('atsumi-soop-live-controls')).toBe(true);
    expect(f.nodes.has('atsumi-soop-original-layout')).toBe(true);
    const appended=f.html.appendChild.mock.calls.length;f.parseHtml();f.tick();f.adapter!.layout({player:null,chat:null});
    expect(f.html.appendChild).toHaveBeenCalledTimes(appended);expect(f.posts).toHaveLength(1);
  });
  it('initializes the official player once with visible original chat, after native identity approval',async()=>{const f=fixture();f.tick();expect(f.posts).toHaveLength(0);expect(f.adapter!.configure({epoch:1,channel:'different'})).toBe(false);f.adapter!.configure({epoch:1,channel:'fixture'});for(let i=0;i<8;i++)await Promise.resolve();f.tick();for(let i=0;i<8;i++)await Promise.resolve();expect(f.posts).toEqual([{cmd:'Pload',id:'fixture',no:12345,showChat:true,autoPlay:true,mutePlay:false,isAdShow:true,showQualityBox:true,fromApi:1}]);});
  it.each(['https://chzzk.naver.com/fixture/12345/embed','http://play.sooplive.com/fixture/12345/embed','https://play.sooplive.com.evil.test/fixture/12345/embed'])('rejects foreign realm %s',url=>{expect(fixture(url).adapter).toBeUndefined();});
  it('does not install into subframes',()=>expect(fixture(undefined,true).adapter).toBeUndefined());
  it('keeps original chat and video geometry separate without another media element',()=>{const f=fixture();f.adapter!.layout({player:{x:20,y:30,width:600,height:400},chat:{x:900,y:30,width:220,height:400}});expect(f.styles.get('--atsumi-player-x')).toBe('20px');expect(f.styles.get('--atsumi-chat-x')).toBe('900px');expect(f.styles.get('--atsumi-chat-display')).toBe('block');f.adapter!.layout({player:null,chat:{x:0,y:0,width:220,height:400}});expect(f.styles.get('--atsumi-player-display')).toBe('none');});
  it('returns only official boolean login state and bounded structural capability',()=>{const f=fixture();expect(f.adapter!.status()).toEqual({state:'playing',authenticated:true,chatVisible:true,captureReady:true,recording:false,recordingId:null,actualChannel:'fixture',actualBroadcast:'12345'});for(const forbidden of ['document.cookie','localStorage','fetch(','XMLHttpRequest','captureStream(','new MediaRecorder','iframe','chzzk.naver']){expect(page).not.toContain(forbidden);expect(capture).not.toContain(forbidden);}});
  it('rejects a navigationless broadcast replacement before native recording begins',async()=>{const f=fixture();f.window.livePlayer.liveAPI.getResponse=()=>({CHANNEL:{BJID:'fixture',BNO:67890}});expect(f.adapter!.status().captureReady).toBe(false);await expect(f.adapter!.start('30000000-0000-4000-8000-000000000001')).rejects.toThrow('broadcast_changed');expect(f.original.start).not.toHaveBeenCalled();});
  it('captures only the original non-ad livePlayer and keeps a SOOP-only observer name',()=>{expect(capture).toContain('document.getElementById("livePlayer")');expect(capture).toContain('"https://play.sooplive.com"');expect(capture).toContain('__atsumiSoopEncodedCapture');expect(capture).not.toContain('__atsumiEncodedCapture');expect(capture).not.toContain('__atsumiPageChat');expect(capture).toContain('const result = Reflect.apply(original, this, arguments);');});
  it('never captures while typing chat, composing, repeating or receiving synthetic keys',()=>{
    const f=fixture();f.adapter!.configure({epoch:1,channel:'fixture'});
    const event={isTrusted:true,key:'s',preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),target:{closest:(selector:string)=>selector==='#player'?{}:null}};
    for(const change of [{isTrusted:false},{isComposing:true},{keyCode:229},{repeat:true},{ctrlKey:true},{target:{closest:()=>({})}}])f.events.get('keydown')!({...event,...change});
    expect(f.messages).toHaveLength(0);expect(event.preventDefault).not.toHaveBeenCalled();
    f.events.get('keydown')!(event);expect(JSON.parse(f.messages[0]!.slice('ATSUMI_SOOP_V1:'.length))).toMatchObject({type:'screenshot_shortcut',input:'s',epoch:1});
    expect(event.preventDefault).toHaveBeenCalledOnce();
  });
  it('captures original fullscreen pointerdown once and blocks the later original click',()=>{
    const f=fixture();f.adapter!.configure({epoch:1,channel:'fixture'});
    const event={isTrusted:true,button:0,preventDefault:vi.fn(),stopImmediatePropagation:vi.fn(),target:{closest:(selector:string)=>selector==='#player'||selector==='.btn_fullScreen_mode'?{}:null}};
    f.events.get('pointerdown')!(event);f.events.get('click')!(event);
    const actions=f.messages.map(message=>JSON.parse(message.slice('ATSUMI_SOOP_V1:'.length))).filter(message=>message.type!=='ui_pointer');
    expect(actions).toHaveLength(1);expect(actions[0]).toMatchObject({type:'fullscreen',input:'pointer',enabled:true});
    expect(event.stopImmediatePropagation).toHaveBeenCalledTimes(2);
  });
});
