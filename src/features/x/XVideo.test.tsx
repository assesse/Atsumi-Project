import {act,useEffect,useRef} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {beforeEach,afterEach,describe,expect,it,vi} from 'vitest';
import {XVideo} from './XVideo';
import {useCenteredVideo} from './videoFocus';
import type {XApi,XPost} from './api';

const center=vi.hoisted(()=>({value:true}));
const hlsState=vi.hoisted(()=>({
  supported:false,
  instances:[] as Array<{config:{autoStartLoad?:boolean};emit:(event:string,data?:{fatal?:boolean})=>void;destroy:ReturnType<typeof vi.fn>;loadSource:ReturnType<typeof vi.fn>;startLoad:ReturnType<typeof vi.fn>;stopLoad:ReturnType<typeof vi.fn>}>,
}));
vi.mock('./videoFocus',()=>({useCenteredVideo:vi.fn((_:unknown,enabled:boolean)=>enabled&&center.value)}));
vi.mock('hls.js',()=>({default:class{
  static Events={ERROR:'error',MANIFEST_PARSED:'manifest'};
  static isSupported(){return hlsState.supported;}
  callbacks=new Map<string,(event:string,data:{fatal?:boolean})=>void>();
  destroy=vi.fn();loadSource=vi.fn();attachMedia=vi.fn();startLoad=vi.fn();stopLoad=vi.fn();
  constructor(readonly config:{autoStartLoad?:boolean}){hlsState.instances.push(this);}
  on(event:string,callback:(event:string,data:{fatal?:boolean})=>void){this.callbacks.set(event,callback);}
  emit(event:string,data:{fatal?:boolean}={}){
    this.callbacks.get(event)?.(event,data);
    // Hls 1.7.2 autostarts after MANIFEST_PARSED listeners, before React commits.
    if(event==='manifest'&&this.config.autoStartLoad!==false&&!this.destroy.mock.calls.length)this.startLoad(-1);
  }
}}));

const post:XPost={id:'123456789',handle:'artist',name:'Artist',text:'',time:'',media:[],video:true,truncated:false};
const sourceUrl='http://x-video.localhost/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
  host=document.createElement('div');document.body.append(host);root=createRoot(host);
  center.value=true;hlsState.supported=false;hlsState.instances=[];
  vi.spyOn(HTMLMediaElement.prototype,'play').mockResolvedValue();
  vi.spyOn(HTMLMediaElement.prototype,'pause').mockImplementation(()=>{});
  vi.spyOn(HTMLMediaElement.prototype,'load').mockImplementation(()=>{});
});
afterEach(async()=>{
  await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();vi.clearAllMocks();
});
const api=()=>({
  video:vi.fn().mockResolvedValue({url:sourceUrl,hls:false}),
  cancel:vi.fn().mockResolvedValue(undefined),image:vi.fn(),
} as unknown as XApi);
const render=(value:XApi,allowed=true,active=true,item=post,generation=1)=>act(async()=>root.render(<XVideo api={value} post={item} generation={generation} active={active} autoPlayAllowed={allowed}/>));
const event=async(name:string)=>act(async()=>{host.querySelector('video')!.dispatchEvent(new Event(name));});
const click=async(label:string)=>act(async()=>{host.querySelector<HTMLButtonElement>('[aria-label="'+label+'"]')!.click();});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;const promise=new Promise<T>(done=>{resolve=done;});return{promise,resolve};};

it('autoplays the centered video muted with app controls, not native controls',async()=>{
  const value=api();await render(value);
  expect(value.video).toHaveBeenCalledOnce();expect(host.querySelector('video')).not.toHaveAttribute('controls');
  expect(host.querySelector('video')!.muted).toBe(true);expect(host.querySelector('[aria-label="동영상 조작"]')).not.toBeNull();
  expect(HTMLMediaElement.prototype.play).toHaveBeenCalled();
  expect(host.querySelector('[aria-label="재생 위치"]')).not.toBeNull();expect(host.querySelector('[aria-label="음량"]')).not.toBeNull();
});

it.each([false,true])('names the owned player and excludes only the lazy raw media node (GIF=%s)',async(gif)=>{
  center.value=false;
  const value=api();await render(value,true,true,{...post,gif});
  const player=host.querySelector('.x-inline-video')!,element=host.querySelector('video')!;
  expect(player).toHaveAttribute('role','group');
  expect(player).toHaveAttribute('aria-label',gif?'게시물 GIF':'게시물 동영상');
  expect(player.closest('[aria-hidden="true"]')).toBeNull();
  expect(element).toHaveAttribute('aria-hidden','true');
  expect(element).not.toHaveAttribute('aria-label');expect(element).not.toHaveAttribute('controls');
  expect(element).not.toHaveAttribute('tabindex');expect(element).not.toHaveAttribute('inert');
  expect(value.video).not.toHaveBeenCalled();
  for(const control of player.querySelectorAll('button,input,[role="group"]'))expect(control.closest('[aria-hidden="true"]')).toBeNull();
});

it('keeps owned loading and actual error controls accessible outside the hidden media node',async()=>{
  const value=api(),source=deferred<{url:string;hls:boolean}>();
  vi.mocked(value.video!).mockReturnValue(source.promise);await render(value);
  const status=host.querySelector('[role="status"]')!;
  expect(status).toHaveTextContent('동영상 불러오는 중');expect(status.closest('[aria-hidden="true"]')).toBeNull();
  expect(host.querySelector('video')).toHaveAttribute('aria-hidden','true');
  await act(async()=>source.resolve({url:'https://invalid.example/media.mp4',hls:false}));
  const alert=host.querySelector('[role="alert"]')!,retry=alert.querySelector('button')!;
  expect(alert).toHaveTextContent('동영상 응답 주소를 검증하지 못했습니다.');expect(alert.closest('[aria-hidden="true"]')).toBeNull();
  expect(retry).toHaveTextContent('다시 시도');expect(retry).not.toBeDisabled();expect(retry.closest('[aria-hidden="true"]')).toBeNull();
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('video')).not.toHaveAttribute('controls');
});

it('pauses a timeline video when preview priority revokes it without reloading its source',async()=>{
  const value=api();await render(value);vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
  await render(value,false);expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();expect(value.video).toHaveBeenCalledOnce();
  await render(value);expect(value.video).toHaveBeenCalledOnce();
});

it('clears a ready timeline player buffering indicator when a preview revokes playback',async()=>{
  vi.useFakeTimers();const value=api();await render(value);await event('playing');await event('waiting');
  expect(host.querySelector('[role="status"]')).toHaveTextContent('불러오는 중');
  await render(value,false);
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('[aria-label="재생"]')).not.toBeNull();
  expect(host.querySelector('video')!.getAttribute('src')).toBe(sourceUrl);
  await act(async()=>vi.advanceTimersByTime(35001));expect(host.querySelector('[role="alert"]')).toBeNull();
  await render(value);expect(value.video).toHaveBeenCalledOnce();
});

it('does not leave a loading label when a pending play is paused after source acquisition',async()=>{
  const value=api(),pending=deferred<void>();vi.mocked(HTMLMediaElement.prototype.play).mockReturnValue(pending.promise);
  await render(value);expect(host.querySelector('[role="status"]')).not.toBeNull();
  await render(value,false);expect(host.querySelector('[role="status"]')).toBeNull();
  await act(async()=>pending.resolve());expect(host.querySelector('[role="status"]')).toBeNull();
});

it('does not request playback for an off-center or hidden player',async()=>{
  const value=api();center.value=false;await render(value);expect(value.video).not.toHaveBeenCalled();
  center.value=true;await render(value,false);expect(value.video).not.toHaveBeenCalled();
  await render(value,true,false);expect(value.video).not.toHaveBeenCalled();
});

it('bounds an unresolved source, cancels it once, and ignores its late success',async()=>{
  vi.useFakeTimers();const value=api(),late=deferred<{url:string;hls:boolean}>();
  vi.mocked(value.video!).mockReturnValue(late.promise);await render(value);
  await act(async()=>vi.advanceTimersByTime(35001));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('지연');expect(host.querySelector('[role="status"]')).toBeNull();
  expect(value.cancel).toHaveBeenCalledOnce();
  await act(async()=>late.resolve({url:sourceUrl,hls:false}));
  expect(host.querySelector('video')!.getAttribute('src')).toBeNull();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  expect(value.video).toHaveBeenCalledOnce();
});

it('discards source success after the player unmounts',async()=>{
  const value=api(),late=deferred<{url:string;hls:boolean}>();
  vi.mocked(value.video!).mockReturnValue(late.promise);await render(value);
  const element=host.querySelector('video')!;
  await act(async()=>root.render(null));expect(value.cancel).toHaveBeenCalledOnce();
  await act(async()=>late.resolve({url:sourceUrl,hls:false}));
  expect(element.getAttribute('src')).toBeNull();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
});

it.each(['https://evil.test/video.mp4','http://x-video.localhost/------------------------------------',sourceUrl+'?other=1'])('never loads an unvalidated response URL: %s',async(url)=>{
  const value=api();vi.mocked(value.video!).mockResolvedValue({url,hls:false});await render(value);
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('검증');expect(host.querySelector('video')!.getAttribute('src')).toBeNull();
});

it('cancels a source request when scrolling away and never lets it overwrite a later request',async()=>{
  const value=api(),old=deferred<{url:string;hls:boolean}>(),fresh=deferred<{url:string;hls:boolean}>();
  vi.mocked(value.video!).mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
  await render(value);const firstId=vi.mocked(value.video!).mock.calls[0]![2];
  center.value=false;await render(value);expect(value.cancel).toHaveBeenCalledWith(firstId);
  expect(host.querySelector('[role="status"]')).toBeNull();
  center.value=true;await render(value);expect(value.video).toHaveBeenCalledTimes(2);
  await act(async()=>fresh.resolve({url:sourceUrl,hls:false}));
  const freshUrl=host.querySelector('video')!.getAttribute('src');
  await act(async()=>old.resolve({url:'http://x-video.localhost/11111111-2222-3333-4444-555555555555',hls:false}));
  expect(host.querySelector('video')!.getAttribute('src')).toBe(freshUrl);
});

it('canplay does not hide a stalled play promise or bypass its playback deadline',async()=>{
  vi.useFakeTimers();const value=api();vi.mocked(HTMLMediaElement.prototype.play).mockReturnValue(new Promise(()=>{}));
  await render(value);await event('canplay');
  expect(host.querySelector('[role="status"]')).toHaveTextContent('불러오는 중');
  await act(async()=>vi.advanceTimersByTime(15001));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생이 지연');
  expect(host.querySelector('video')!.getAttribute('src')).toBeNull();
});

it('keeps the playback deadline after play alone and ignores late events after it fails',async()=>{
  vi.useFakeTimers();const value=api(),pending=deferred<void>();
  vi.mocked(HTMLMediaElement.prototype.play).mockReturnValue(pending.promise);
  await render(value);await event('play');
  await act(async()=>vi.advanceTimersByTime(15001));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생이 지연');
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(value.cancel).toHaveBeenCalledOnce();
  await event('play');await event('playing');await event('waiting');
  await act(async()=>pending.resolve());
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생이 지연');
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(value.video).toHaveBeenCalledOnce();expect(value.cancel).toHaveBeenCalledOnce();
});

it('settles the startup deadline only when playing follows the play event',async()=>{
  vi.useFakeTimers();const value=api(),pending=deferred<void>();
  vi.mocked(HTMLMediaElement.prototype.play).mockReturnValue(pending.promise);
  await render(value);await event('play');
  await act(async()=>vi.advanceTimersByTime(14999));
  expect(host.querySelector('[role="status"]')).toHaveTextContent('불러오는 중');
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await event('playing');
  expect(host.querySelector('[role="status"]')).toBeNull();
  await act(async()=>vi.advanceTimersByTime(15001));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(host.querySelector('[aria-label="일시 정지"]')).not.toBeDisabled();
  expect(value.cancel).not.toHaveBeenCalled();
  await act(async()=>pending.resolve());
});

it('allows pause while the source loads and keeps the centered paused card eligible',async()=>{
  const value=api();vi.mocked(value.video!).mockReturnValue(new Promise(()=>{}));
  await render(value);await click('일시 정지');
  expect(value.cancel).toHaveBeenCalledOnce();expect(host.querySelector('[role="status"]')).toBeNull();
  expect(vi.mocked(useCenteredVideo).mock.lastCall?.[1]).toBe(true);
  await render(value);expect(value.video).toHaveBeenCalledOnce();
});

it('keeps explicit pause across updates and resumes the retained source only on explicit play',async()=>{
  const value=api();await render(value);await event('playing');await click('일시 정지');
  const calls=vi.mocked(HTMLMediaElement.prototype.play).mock.calls.length;
  await render(value);expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(calls);
  expect(vi.mocked(useCenteredVideo).mock.lastCall?.[1]).toBe(true);
  await click('재생');expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(calls+1);expect(value.video).toHaveBeenCalledOnce();
});

it('releases media when inactive and does not inherit old intent into a new generation',async()=>{
  const value=api();await render(value);await event('playing');await click('일시 정지');
  const oldElement=host.querySelector('video')!;
  await render(value,true,false);expect(oldElement.getAttribute('src')).toBeNull();
  expect(value.video).toHaveBeenCalledOnce();
  await render(value,true,true,post,2);
  expect(host.querySelector('video')).not.toBe(oldElement);expect(value.video).toHaveBeenCalledTimes(2);
  expect(host.querySelector('video')!.muted).toBe(true);
});

it('loops GIFs but leaves an ended ordinary video paused until explicitly replayed',async()=>{
  const value=api();await render(value,true,true,{...post,gif:true});
  expect(host.querySelector('video')).toHaveAttribute('loop');expect(host.querySelector('.x-inline-video')).toHaveAttribute('aria-label','게시물 GIF');
  await render(value,true,true,{...post,id:'987654321',gif:false});
  expect(host.querySelector('video')).not.toHaveAttribute('loop');
  await event('playing');await event('ended');
  const calls=vi.mocked(HTMLMediaElement.prototype.play).mock.calls.length;
  await click('재생');expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(calls+1);
});

it('does not let a late play promise restart a preview-revoked timeline',async()=>{
  const value=api(),pending=deferred<void>();
  vi.mocked(HTMLMediaElement.prototype.play).mockReturnValue(pending.promise);
  await render(value);await render(value,false);
  vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
  await act(async()=>pending.resolve());
  expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();expect(HTMLMediaElement.prototype.play).toHaveBeenCalledOnce();
});

it('does not pause a newer playback attempt when an older play promise resolves late',async()=>{
  const value=api(),old=deferred<void>();
  vi.mocked(HTMLMediaElement.prototype.play).mockReturnValueOnce(old.promise).mockResolvedValue();
  await render(value);await render(value,false);await render(value);
  expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);
  vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
  await act(async()=>old.resolve());
  expect(HTMLMediaElement.prototype.pause).not.toHaveBeenCalled();expect(value.video).toHaveBeenCalledOnce();
});

it('lets explicit play resume after a browser autoplay-policy rejection without re-fetching the source',async()=>{
  const value=api(),blocked=new Error('User gesture required');blocked.name='NotAllowedError';
  vi.mocked(HTMLMediaElement.prototype.play).mockRejectedValueOnce(blocked).mockResolvedValue();
  await render(value);expect(host.querySelector('[role="status"]')).toBeNull();expect(host.querySelector('[role="alert"]')).toBeNull();
  await click('재생');expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);expect(value.video).toHaveBeenCalledOnce();
});

it('destroys a fatal HLS attempt and ignores a late manifest event',async()=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;expect(player.loadSource).toHaveBeenCalledWith(sourceUrl);
  await act(async()=>player.emit('error',{fatal:true}));
  expect(player.destroy).toHaveBeenCalledOnce();expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생하지 못했습니다');
  await act(async()=>player.emit('manifest'));
  expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();expect(value.video).toHaveBeenCalledOnce();
});

it('settles a playing HLS failure without a pause event and ignores late playback events',async()=>{
  const value=api(),pending=deferred<void>();hlsState.supported=true;
  vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  vi.mocked(HTMLMediaElement.prototype.play).mockReturnValueOnce(pending.promise);
  await render(value);const player=hlsState.instances[0]!;
  await act(async()=>player.emit('manifest'));await event('playing');
  expect(host.querySelector('[aria-label="일시 정지"]')).not.toBeNull();
  await act(async()=>player.emit('error',{fatal:true}));
  expect(host.querySelector('[aria-label="일시 정지"]')).toBeNull();
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('button.x-video-overlay')).toBeNull();
  expect(player.destroy).toHaveBeenCalledOnce();expect(value.cancel).toHaveBeenCalledOnce();
  await act(async()=>player.emit('manifest'));await event('playing');await event('waiting');
  await act(async()=>pending.resolve());
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생하지 못했습니다');
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(HTMLMediaElement.prototype.play).toHaveBeenCalledOnce();expect(value.video).toHaveBeenCalledOnce();
});

it('does not retry a failed source through play clicks, scroll return or preview reactivation',async()=>{
  vi.useFakeTimers();const value=api();vi.mocked(value.video!).mockRejectedValue(new Error('source unavailable'));
  await render(value);expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  await click('재생');await act(async()=>host.querySelector('video')!.click());
  center.value=false;await render(value);center.value=true;await render(value);
  await render(value,false);await render(value);
  await render(value,true,false);await render(value);
  await render({...value});
  await act(async()=>vi.advanceTimersByTime(70001));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('source unavailable');
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(value.video).toHaveBeenCalledOnce();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
});

it('recovers with one explicit retry and never loops automatically after another HLS failure',async()=>{
  vi.useFakeTimers();const value=api();hlsState.supported=true;
  vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const first=hlsState.instances[0]!;
  await act(async()=>first.emit('manifest'));await event('playing');
  await act(async()=>first.emit('error',{fatal:true}));
  const firstRequest=vi.mocked(value.video!).mock.calls[0]![2];
  await act(async()=>host.querySelector<HTMLButtonElement>('[role="alert"] button')!.click());
  expect(value.video).toHaveBeenCalledTimes(2);
  expect(vi.mocked(value.video!).mock.calls[1]![2]).not.toBe(firstRequest);
  const second=hlsState.instances[1]!;
  await act(async()=>{first.emit('manifest');first.emit('error',{fatal:true});});
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async()=>second.emit('manifest'));await event('playing');
  expect(host.querySelector('[aria-label="일시 정지"]')).not.toBeDisabled();
  await click('일시 정지');const calls=vi.mocked(HTMLMediaElement.prototype.play).mock.calls.length;
  await click('재생');expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(calls+1);
  expect(value.video).toHaveBeenCalledTimes(2);
  await act(async()=>second.emit('error',{fatal:true}));
  center.value=false;await render(value);center.value=true;await render(value);
  await act(async()=>vi.advanceTimersByTime(70001));
  expect(value.video).toHaveBeenCalledTimes(2);expect(value.cancel).toHaveBeenCalledTimes(2);
  expect(first.destroy).toHaveBeenCalledOnce();expect(second.destroy).toHaveBeenCalledOnce();
  expect(host.querySelector('[aria-label="재생"]')).toBeDisabled();
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생하지 못했습니다');
});

it('starts HLS only after the manifest is ready and cancels it on privacy/inactive',async()=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  await act(async()=>player.emit('manifest'));expect(HTMLMediaElement.prototype.play).toHaveBeenCalled();
  await render(value,true,false);expect(player.destroy).toHaveBeenCalledOnce();
});

it('gives the intent effect exclusive ownership of the initial HLS load',async()=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;
  expect(player.config.autoStartLoad).toBe(false);
  expect(player.startLoad).not.toHaveBeenCalled();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  await act(async()=>player.emit('manifest'));
  expect(player.startLoad).toHaveBeenCalledExactlyOnceWith(-1);
  expect(HTMLMediaElement.prototype.play).toHaveBeenCalledOnce();
  await render(value);expect(player.startLoad).toHaveBeenCalledOnce();
});

it.each(['manual pause','preview takeover'])('does not autostart HLS when %s wins the manifest commit',async(mode)=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;
  await act(async()=>{
    player.emit('manifest');
    if(mode==='manual pause')host.querySelector<HTMLButtonElement>('[aria-label="일시 정지"]')!.click();
    else root.render(<XVideo api={value} post={post} generation={1} active autoPlayAllowed={false}/>);
  });
  expect(player.startLoad).not.toHaveBeenCalled();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('[aria-label="재생"]')).not.toBeNull();
  expect(value.video).toHaveBeenCalledOnce();
});

it.each(['manual pause','preview takeover'])('ignores the initial manifest after %s canceled its source',async(mode)=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;
  if(mode==='manual pause')await click('일시 정지');else await render(value,false);
  expect(player.destroy).toHaveBeenCalledOnce();expect(value.cancel).toHaveBeenCalledOnce();
  await act(async()=>player.emit('manifest'));
  expect(player.startLoad).not.toHaveBeenCalled();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  expect(value.video).toHaveBeenCalledOnce();
});

it('stops HLS background loading while preview-paused and resumes the same source at its retained position',async()=>{
  const value=api();hlsState.supported=true;vi.mocked(value.video!).mockResolvedValue({url:sourceUrl,hls:true});
  await render(value);const player=hlsState.instances[0]!;
  expect(player.startLoad).not.toHaveBeenCalled();await act(async()=>player.emit('manifest'));
  expect(player.startLoad).toHaveBeenLastCalledWith(-1);
  const element=host.querySelector('video')!;element.currentTime=42;
  player.stopLoad.mockClear();await render(value,false);expect(player.stopLoad).toHaveBeenCalled();
  expect(player.destroy).not.toHaveBeenCalled();expect(element.currentTime).toBe(42);
  player.startLoad.mockClear();await render(value);expect(player.startLoad).toHaveBeenCalledWith(-1);
  expect(element.currentTime).toBe(42);expect(value.video).toHaveBeenCalledOnce();
  await render(value,true,false);player.startLoad.mockClear();
  await act(async()=>player.emit('manifest'));
  expect(player.startLoad).not.toHaveBeenCalled();expect(player.destroy).toHaveBeenCalledOnce();
});

it('bounds a later buffering stall even after an earlier play promise succeeded',async()=>{
  vi.useFakeTimers();const value=api();await render(value);await event('playing');await event('waiting');
  expect(host.querySelector('[role="status"]')).toHaveTextContent('불러오는 중');
  await act(async()=>vi.advanceTimersByTime(15001));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('재생이 지연');
  expect(host.querySelector('video')!.getAttribute('src')).toBeNull();
});

it('has accessible seek, mute, volume and owned fullscreen controls',async()=>{
  const value=api();await render(value);
  const element=host.querySelector('video')!;
  Object.defineProperty(element,'duration',{configurable:true,value:120});await event('loadedmetadata');
  expect(host.querySelector('[aria-label="재생 위치"]')).not.toBeDisabled();
  await click('소리 켜기');expect(element.muted).toBe(false);
  await click('음소거');expect(element.muted).toBe(true);
  const container=host.querySelector<HTMLDivElement>('.x-inline-video')!,fullscreen=vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(container,'requestFullscreen',{value:fullscreen});
  await click('동영상 전체 화면');expect(fullscreen).toHaveBeenCalledOnce();
  expect(host.querySelector('video')).not.toHaveAttribute('controls');
});

describe('real center arbitration',()=>{
  let useRealCenteredVideo:typeof useCenteredVideo;
  let frames:Map<number,FrameRequestCallback>,nextFrame:number,hidden:boolean;
  const rect=(top:number,left=0):DOMRect=>({top,bottom:top+300,left,right:left+500,width:500,height:300,x:left,y:top,toJSON:()=>({})});
  let bounds:Record<string,DOMRect>,trace:string[];
  function Probe({name,enabled=true,priority=0}:{name:string;enabled?:boolean;priority?:number}){
    const ref=useRef<HTMLDivElement>(null);
    const chosen=useRealCenteredVideo(ref,enabled,priority,()=>trace.push('pause '+name));
    useEffect(()=>{if(chosen)trace.push('play '+name);},[chosen,name]);
    return <div ref={ref} data-probe={name} data-chosen={chosen}/>;
  }
  const flush=()=>act(async()=>{const pending=[...frames.values()];frames.clear();for(const callback of pending)callback(0);});
  beforeEach(async()=>{
    useRealCenteredVideo=(await vi.importActual<typeof import('./videoFocus')>('./videoFocus')).useCenteredVideo;
    frames=new Map();nextFrame=0;hidden=false;trace=[];bounds={a:rect(350),b:rect(600)};
    vi.stubGlobal('innerHeight',1000);vi.stubGlobal('innerWidth',1200);
    vi.stubGlobal('requestAnimationFrame',(callback:FrameRequestCallback)=>{frames.set(++nextFrame,callback);return nextFrame;});
    vi.stubGlobal('cancelAnimationFrame',(id:number)=>frames.delete(id));
    vi.spyOn(document,'visibilityState','get').mockImplementation(()=>hidden?'hidden':'visible');
    vi.spyOn(HTMLElement.prototype,'getBoundingClientRect').mockImplementation(function(this:HTMLElement){return bounds[this.dataset.probe??'']??rect(0);});
  });
  it('chooses only the nearest eligible player, not an offscreen horizontal pane',async()=>{
    await act(async()=>root.render(<><Probe name="a"/><Probe name="b"/></>));await flush();
    expect(host.querySelector('[data-probe="a"]')).toHaveAttribute('data-chosen','true');
    expect(host.querySelector('[data-probe="b"]')).toHaveAttribute('data-chosen','false');
    trace=[];bounds.a=rect(350,1400);
    await act(async()=>window.dispatchEvent(new Event('resize')));await flush();
    expect(host.querySelector('[data-probe="b"]')).toHaveAttribute('data-chosen','true');
    expect(trace.indexOf('pause a')).toBeLessThan(trace.indexOf('play b'));
  });
  it('revokes immediately on document hiding without waiting for animation frames',async()=>{
    await act(async()=>root.render(<Probe name="a"/>));await flush();trace=[];hidden=true;
    await act(async()=>document.dispatchEvent(new Event('visibilitychange')));
    expect(trace).toContain('pause a');expect(host.querySelector('[data-probe="a"]')).toHaveAttribute('data-chosen','false');
    expect(frames.size).toBe(0);
  });
  it('prefers the preview, then pauses it before returning to the timeline',async()=>{
    await act(async()=>root.render(<><Probe name="a"/><Probe name="b" priority={1}/></>));await flush();
    expect(host.querySelector('[data-probe="b"]')).toHaveAttribute('data-chosen','true');
    trace=[];await act(async()=>root.render(<><Probe name="a"/><Probe name="b" enabled={false} priority={1}/></>));await flush();
    expect(host.querySelector('[data-probe="a"]')).toHaveAttribute('data-chosen','true');
    expect(trace.indexOf('pause b')).toBeLessThan(trace.indexOf('play a'));
  });
  it('does not let another card steal playback from the fullscreen player',async()=>{
    await act(async()=>root.render(<><Probe name="a"/><Probe name="b" priority={1}/></>));await flush();
    const fullscreen=host.querySelector('[data-probe="a"]');
    Object.defineProperty(document,'fullscreenElement',{configurable:true,get:()=>fullscreen});
    try{
      await act(async()=>document.dispatchEvent(new Event('fullscreenchange')));await flush();
      expect(host.querySelector('[data-probe="a"]')).toHaveAttribute('data-chosen','true');
      expect(host.querySelector('[data-probe="b"]')).toHaveAttribute('data-chosen','false');
    }finally{Reflect.deleteProperty(document,'fullscreenElement');}
  });
});
