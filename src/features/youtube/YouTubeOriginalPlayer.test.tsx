import { act, createRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { YouTubeOriginalPlayer, type YouTubeOriginalPlayerHandle, type YouTubeVodClock } from './YouTubeOriginalPlayer';
import type { YouTubeOriginalApi, YouTubeOriginalSnapshot } from '../../api/youtubeOriginal';
const events=vi.hoisted(()=>new Map<string,(event:{payload:unknown})=>void>());
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async(name:string,handler:(event:{payload:unknown})=>void)=>{events.set(name,handler);return()=>events.delete(name);})}));
vi.mock('../platforms/nativePlayerGeometry',()=>({measureNativePlayerGeometry:(_p:unknown,_c:unknown,options:Record<string,unknown>)=>({...options,player:_p?{x:0,y:0,width:600,height:400}:null,chat:null,occlusions:[]}),observeNativePlayerGeometry:()=>()=>undefined}));
vi.mock('../platforms/platformScreenshots',()=>({platformScreenshotsApi:{image:vi.fn(async()=> 'data:image/png;base64,AAAA')}}));
let root:Root,container:HTMLDivElement,api:YouTubeOriginalApi;
const state:YouTubeOriginalSnapshot={token:'lease-a',channelId:'abcdefghijk',broadcastId:'abcdefghijk',state:'playing',authenticated:false,chatVisible:true,captureReady:true,recordingJob:null,recordingStopping:false,fullscreen:false,error:null};
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);vi.useFakeTimers();events.clear();container=document.createElement('div');document.body.append(container);root=createRoot(container);api={controls:vi.fn(async()=>undefined),resume:vi.fn(async()=>undefined),open:vi.fn(async()=>state),snapshot:vi.fn(async()=>state),close:vi.fn(async()=>undefined),viewport:vi.fn(async()=>undefined),record:vi.fn(async(_token:string,enabled:boolean)=>({...state,recordingJob:enabled?'job':null})),login:vi.fn(async()=>({authenticated:null,open:true,error:null})),logout:vi.fn(async()=>({authenticated:null,open:true,error:null})),account:vi.fn(async()=>({authenticated:false,open:false,error:null})),fullscreen:vi.fn(async()=>undefined),screenshot:vi.fn(async()=>({id:'shot',provider:'youtube' as const,reference:{provider:'youtube' as const,kind:'live' as const,externalId:'abcdefghijk'},title:'live',channelName:'channel',createdAt:1,bytes:12,width:10,height:10,fileName:'shot.png'})),undoScreenshot:vi.fn(async()=>undefined)};});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();vi.useRealTimers();vi.unstubAllGlobals();vi.restoreAllMocks();});
const render=async(props:Partial<Parameters<typeof YouTubeOriginalPlayer>[0]>={})=>act(async()=>root.render(<YouTubeOriginalPlayer channelId="abcdefghijk" title="sample" active privacyMode={false} api={api} {...props}/>));
const click=async(selector:string)=>act(async()=>container.querySelector<HTMLButtonElement>(selector)!.click());
const emit=async(name:string,payload:unknown)=>act(async()=>events.get(name)?.({payload}));
it('creates one native receiver without another HTML video/iframe and preserves it across hidden/layout changes',async()=>{
 await render();expect(api.open).toHaveBeenCalledOnce();expect(container.querySelector('video,iframe')).toBeNull();
 await render({active:false,flattened:true});expect(api.close).not.toHaveBeenCalled();expect(api.open).toHaveBeenCalledOnce();expect(api.viewport).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({visible:false,muted:false}));
 await render({privacyMode:true});expect(api.viewport).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({visible:false,muted:true}));
 await render({sideBySide:true});expect(api.open).toHaveBeenCalledOnce();
});
it('keeps monotonic presentation sequence and separately handles tray mute',async()=>{
 await render();await render({active:false});await render();await emit('platform-media:tray-muted',true);
 const values=vi.mocked(api.viewport).mock.calls.map(call=>call[1].sequence);expect(values.every((value,index)=>index===0||value>values[index-1]!)).toBe(true);
 expect(api.viewport).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({muted:true}));expect(api.close).not.toHaveBeenCalled();
});
it('uses existing native capture and exposes save/undo preview without consuming chat keys',async()=>{
 await render();await emit('youtube-original:presentation',{channelId:'abcdefghijk',broadcastId:'abcdefghijk',opacity:1});
 await emit('youtube-original:action',{token:'lease-a',action:'screenshot'});expect(api.screenshot).toHaveBeenCalledWith('lease-a',expect.any(String));expect(container.querySelector('.platform-live-capture-preview img')).toHaveAttribute('src','data:image/png;base64,AAAA');
 await click('.platform-live-capture-cancel');expect(api.undoScreenshot).toHaveBeenCalledWith('lease-a','shot');expect(container.querySelector('.platform-live-capture-preview')).toBeNull();
 await act(async()=>container.querySelector('.youtube-original-chat')!.dispatchEvent(new KeyboardEvent('keydown',{key:'s',bubbles:true})));expect(api.screenshot).toHaveBeenCalledOnce();
 await emit('youtube-original:screenshot',{channelId:'differentId'});expect(api.screenshot).toHaveBeenCalledOnce();await emit('youtube-original:screenshot',{channelId:'abcdefghijk'});expect(api.screenshot).toHaveBeenCalledTimes(2);
});
it('projects native controls without an app overlay and accepts only the current token close',async()=>{
 const onClose=vi.fn();await render({onClose});expect(container.querySelector('.youtube-original-actions')).toBeNull();expect(api.controls).toHaveBeenCalledWith('lease-a',true,false);expect(api.record).not.toHaveBeenCalled();
 await emit('youtube-original:action',{token:'foreign',action:'viewer_close'});expect(onClose).not.toHaveBeenCalled();
 await emit('youtube-original:action',{token:'lease-a',action:'viewer_close'});expect(onClose).toHaveBeenCalledOnce();expect(api.record).not.toHaveBeenCalled();
});
it('clears recovered polling failure without clearing a failed recording or reopening the receiver',async()=>{
 await render();vi.mocked(api.snapshot).mockRejectedValueOnce(new Error('일시적인 상태 확인 오류'));
 await act(async()=>vi.advanceTimersByTime(1500));expect(container.querySelector('[role="alert"]')).toHaveTextContent('일시적인 상태 확인 오류');
 await act(async()=>vi.advanceTimersByTime(1500));expect(container.querySelector('[role="alert"]')).toBeNull();expect(api.open).toHaveBeenCalledOnce();expect(api.close).not.toHaveBeenCalled();
 vi.mocked(api.snapshot).mockResolvedValue({...state,error:'녹화 저장 실패'});
 await act(async()=>vi.advanceTimersByTime(1500));expect(container.querySelector('[role="alert"]')).toHaveTextContent('녹화 저장 실패');expect(api.open).toHaveBeenCalledOnce();
});
it('uses native fullscreen ownership while preserving player and enabling its own splitter',async()=>{
 await render({flattened:true});const slot=container.querySelector('.youtube-original-slot');await emit('youtube-original:fullscreen',{channelId:'abcdefghijk',enabled:true});
 expect(container.querySelector('.youtube-original-pair')).toHaveAttribute('data-platform-native-fullscreen','true');expect(container.querySelector('.youtube-original-slot')).toBe(slot);expect(container.querySelector('[role="separator"]')).toHaveAttribute('tabindex','0');
 await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));expect(api.fullscreen).toHaveBeenCalledWith('lease-a',false);expect(api.open).toHaveBeenCalledOnce();
});
it('rejects stale late open by closing only the abandoned lease',async()=>{
 let finish!:(value:YouTubeOriginalSnapshot)=>void;vi.mocked(api.open).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));await render();await render({channelId:'bcdefghijkl'});await act(async()=>finish({...state,token:'abandoned'}));expect(api.close).toHaveBeenCalledWith('abandoned');
});
it('releases native fullscreen on provider switch and never consumes background or composing Escape',async()=>{
 await render();await emit('youtube-original:fullscreen',{channelId:'abcdefghijk',enabled:true});
 await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,isComposing:true})));expect(api.fullscreen).not.toHaveBeenCalled();
 await render({active:false});expect(api.fullscreen).toHaveBeenCalledWith('lease-a',false);expect(container.querySelector('[data-platform-native-fullscreen]')).toBeNull();const count=vi.mocked(api.fullscreen).mock.calls.length;
 await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));expect(api.fullscreen).toHaveBeenCalledTimes(count);
});
it('keeps unsupported viewing while projecting native capture state without duplicate controls',async()=>{
 vi.mocked(api.open).mockResolvedValueOnce({...state,captureReady:false,captureUnavailableReason:'지원되지 않는 영상 형식'});await render();expect(container.querySelector('.youtube-original-actions')).toBeNull();expect(container.querySelector('.youtube-original-slot')).not.toBeNull();expect(api.close).not.toHaveBeenCalled();expect(api.controls).toHaveBeenCalled();
});
it('opens watch-only VOD in the provider profile without live chat or recording and retains ended playback',async()=>{
 vi.mocked(api.open).mockResolvedValueOnce({...state,token:'youtube-vod-test',chatVisible:false,captureReady:false,state:'ended'});
 vi.mocked(api.snapshot).mockResolvedValue({...state,token:'youtube-vod-test',chatVisible:false,captureReady:false,state:'playing'});
 await render({playbackKind:'vod'});
 expect(api.open).toHaveBeenCalledWith('abcdefghijk',expect.any(String),'vod');expect(container.querySelector('iframe,video,.youtube-original-chat,[role="separator"],.youtube-original-actions')).toBeNull();
 expect(container.querySelector('[data-playback-kind="vod"]')).not.toBeNull();expect(api.close).not.toHaveBeenCalled();expect(api.record).not.toHaveBeenCalled();
 await act(async()=>vi.advanceTimersByTime(1500));await render({playbackKind:'vod',active:false});
 expect(api.open).toHaveBeenCalledOnce();expect(api.close).not.toHaveBeenCalled();expect(api.viewport).toHaveBeenLastCalledWith('youtube-vod-test',expect.objectContaining({chat:null,visible:false,muted:false}));
 await emit('youtube-original:screenshot',{channelId:'abcdefghijk'});expect(api.screenshot).not.toHaveBeenCalled();
});
it('resumes only a new explicit request after the visible viewport ACK, never passive layout',async()=>{
 await render();await render({sideBySide:true});expect(api.resume).not.toHaveBeenCalled();
 let done!:()=>void;vi.mocked(api.viewport).mockImplementationOnce(()=>new Promise(resolve=>{done=resolve;}));
 await render({resumeRequest:1});expect(api.resume).not.toHaveBeenCalled();await act(async()=>done());expect(api.resume).toHaveBeenCalledWith('lease-a');
 await render({resumeRequest:1,active:false});await render({resumeRequest:1});expect(api.resume).toHaveBeenCalledOnce();
 await render({resumeRequest:2,privacyMode:true});expect(api.resume).toHaveBeenCalledOnce();await render({resumeRequest:2});expect(api.resume).toHaveBeenCalledTimes(2);
 expect(api.open).toHaveBeenCalledOnce();
});
it('starts chat at480 without changing the receiver on resize',async()=>{await render();expect(container.querySelector<HTMLElement>('.youtube-original-pair')!.style.getPropertyValue('--platform-live-chat-size')).toBe('480px');});

it('projects manual jobs through only the existing poll and recovers unknown without remounting',async()=>{
 const first=vi.fn(),next=vi.fn(),canonicalChannelId='UCabcdefghijklmnopqrstuv';
 vi.mocked(api.open).mockResolvedValue({...state,canonicalChannelId,recordingJob:'manual-a'});
 await render({onObservation:first});expect(first.mock.calls.map(call=>call[0].kind)).toEqual(['begin','update']);
 expect(first).toHaveBeenLastCalledWith(expect.objectContaining({kind:'update',value:expect.objectContaining({canonicalChannelId,recordingJob:'manual-a'})}));
 await render({onObservation:next,privacyMode:true});expect(api.open).toHaveBeenCalledOnce();expect(api.close).not.toHaveBeenCalled();
 vi.mocked(api.snapshot).mockResolvedValueOnce({...state,canonicalChannelId,recordingJob:'manual-a',recordingStopping:true});
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(next).toHaveBeenLastCalledWith(expect.objectContaining({kind:'update',value:expect.objectContaining({recordingStopping:true})}));
 vi.mocked(api.snapshot).mockRejectedValueOnce(new Error('temporary'));
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(next).toHaveBeenLastCalledWith(expect.objectContaining({kind:'clear',detached:false}));
 vi.mocked(api.snapshot).mockResolvedValueOnce({...state,canonicalChannelId,recordingJob:'manual-b'});
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(next).toHaveBeenLastCalledWith(expect.objectContaining({kind:'update',value:expect.objectContaining({recordingJob:'manual-b'})}));
 expect(api.snapshot).toHaveBeenCalledTimes(3);expect(api.open).toHaveBeenCalledOnce();expect(api.record).not.toHaveBeenCalled();
});
it('never projects VOD, wrong video, wrong token or malformed canonical identity as live proof',async()=>{
 const onObservation=vi.fn();await render({onObservation,playbackKind:'vod'});await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(onObservation).not.toHaveBeenCalled();
 vi.mocked(api.open).mockResolvedValueOnce({...state,channelId:'bcdefghijkl'});await render({onObservation});expect(onObservation).toHaveBeenLastCalledWith(expect.objectContaining({kind:'clear'}));
 vi.mocked(api.snapshot).mockResolvedValueOnce({...state,token:'foreign'});await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(onObservation).toHaveBeenLastCalledWith(expect.objectContaining({kind:'clear'}));
 vi.mocked(api.snapshot).mockResolvedValueOnce({...state,canonicalChannelId:'@same-name',recordingJob:'manual'});await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(onObservation).toHaveBeenLastCalledWith(expect.objectContaining({kind:'update',value:expect.objectContaining({canonicalChannelId:null,recordingJob:'manual'})}));
});
it('does not publish a delayed prior open or prior poll after its source is replaced',async()=>{
 const onObservation=vi.fn();let oldOpen!:(value:YouTubeOriginalSnapshot)=>void;
 vi.mocked(api.open).mockImplementationOnce(()=>new Promise(done=>{oldOpen=done;}));await render({onObservation});
 const replacement={...state,token:'lease-b',channelId:'bcdefghijkl',broadcastId:'bcdefghijkl'};
 vi.mocked(api.open).mockResolvedValueOnce(replacement);await render({onObservation,channelId:'bcdefghijkl'});
 await act(async()=>oldOpen({...state,recordingJob:'old'}));expect(onObservation.mock.calls.some(call=>call[0].value?.recordingJob==='old')).toBe(false);
 let oldPoll!:(value:YouTubeOriginalSnapshot)=>void;vi.mocked(api.snapshot).mockImplementationOnce(()=>new Promise(done=>{oldPoll=done;}));await act(async()=>vi.advanceTimersByTime(1500));
 await render({onObservation});await act(async()=>oldPoll({...replacement,recordingJob:'old-poll'}));
 expect(onObservation.mock.calls.some(call=>call[0].value?.recordingJob==='old-poll')).toBe(false);
 expect(onObservation.mock.calls.filter(call=>call[0].kind==='clear'&&call[0].detached)).toHaveLength(2);
 expect(api.close).toHaveBeenCalledWith('lease-b');
});
const contentClock=(patch:Partial<YouTubeVodClock>={}):YouTubeVodClock=>({token:'lease-a',videoId:'abcdefghijk',documentEpoch:2,sourceId:'source-a',sourceGeneration:1,sequence:1,seekGeneration:1,currentTime:10,duration:100,paused:false,seeking:false,playbackRate:1,buffering:false,ended:false,adState:'content',valid:true,ageMs:0,...patch});
it('publishes only current VOD clocks and rejects stale epochs sequences sources and foreign tokens',async()=>{
 const onClock=vi.fn();await render({playbackKind:'vod',onClock});onClock.mockClear();
 await emit('youtube-original:vod-clock',contentClock());expect(onClock).toHaveBeenCalledTimes(1);
 for(const patch of [{token:'old'},{videoId:'bcdefghijkl'},{documentEpoch:1,sequence:10},{sequence:1},{sourceId:'other',sequence:2},{currentTime:NaN,sequence:2}])await emit('youtube-original:vod-clock',contentClock(patch));
 expect(onClock).toHaveBeenCalledTimes(1);
 await emit('youtube-original:vod-clock',contentClock({sequence:2,sourceGeneration:2,sourceId:'ad',valid:false,currentTime:null,duration:null,adState:'ad'}));expect(onClock).toHaveBeenLastCalledWith(expect.objectContaining({adState:'ad',currentTime:null}));
 await act(async()=>vi.advanceTimersByTimeAsync(1750));expect(onClock).toHaveBeenLastCalledWith(null);
 await render({playbackKind:'vod',onClock:vi.fn()});expect(api.open).toHaveBeenCalledOnce();
});
it('acknowledges explicit VOD ref controls without optimistic clock updates and handles native close',async()=>{
 const ref=createRef<YouTubeOriginalPlayerHandle>(),onClock=vi.fn(),onClose=vi.fn();api.vodControl=vi.fn(async()=>undefined);api.vodClock=vi.fn(async()=>contentClock());
 await render({playbackKind:'vod',ref,onClock,onClose});expect(api.vodClock).toHaveBeenCalledWith('lease-a');
 await act(async()=>ref.current!.seek(50));expect(api.vodControl).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({action:'seek',seconds:50,documentEpoch:2,sourceId:'source-a'}));expect(onClock).toHaveBeenLastCalledWith(expect.objectContaining({currentTime:10}));
 await act(async()=>ref.current!.pause());expect(api.vodControl).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({action:'pause'}));
 await act(async()=>ref.current!.resume());expect(api.vodControl).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({action:'resume'}));
 await act(async()=>ref.current!.setRate(1.5));expect(api.vodControl).toHaveBeenLastCalledWith('lease-a',expect.objectContaining({action:'rate',rate:1.5}));
 for(const value of [0,-1,17,NaN])await expect(ref.current!.setRate(value)).rejects.toThrow();
 await expect(ref.current!.seek(101)).rejects.toThrow();await render({playbackKind:'vod',ref,onClock,onClose,privacyMode:true});await expect(ref.current!.resume()).rejects.toThrow();
 await emit('youtube-original:action',{token:'other',action:'viewer_close'});expect(onClose).not.toHaveBeenCalled();await emit('youtube-original:action',{token:'lease-a',action:'viewer_close'});expect(onClose).toHaveBeenCalledOnce();
 expect(api.record).not.toHaveBeenCalled();expect(api.screenshot).not.toHaveBeenCalled();expect(api.open).toHaveBeenCalledOnce();
});
it('never requests or publishes VOD clocks in live mode',async()=>{
 const onClock=vi.fn();api.vodClock=vi.fn(async()=>contentClock());await render({onClock});await emit('youtube-original:vod-clock',contentClock());expect(api.vodClock).not.toHaveBeenCalled();expect(onClock).not.toHaveBeenCalled();
});
it('ignores delayed initial clock and events from a released VOD alias',async()=>{
 const onClock=vi.fn();let done!:(value:YouTubeVodClock|null)=>void;api.vodClock=vi.fn().mockImplementationOnce(()=>new Promise(resolve=>{done=resolve;})).mockResolvedValue(null);
 await render({playbackKind:'vod',onClock});
 const next={...state,token:'lease-b',channelId:'bcdefghijkl',broadcastId:'bcdefghijkl'};vi.mocked(api.open).mockResolvedValueOnce(next);await render({playbackKind:'vod',onClock,channelId:'bcdefghijkl'});onClock.mockClear();
 await act(async()=>done(contentClock({sequence:100})));await emit('youtube-original:vod-clock',contentClock({sequence:200}));expect(onClock).not.toHaveBeenCalled();
 await emit('youtube-original:vod-clock',contentClock({token:'lease-b',videoId:'bcdefghijkl'}));expect(onClock).toHaveBeenCalledOnce();
});
it('rejects seek during ads or expired clocks and accepts a fresh document generation',async()=>{
 const ref=createRef<YouTubeOriginalPlayerHandle>(),onClock=vi.fn();api.vodControl=vi.fn(async()=>undefined);await render({playbackKind:'vod',ref,onClock});
 await emit('youtube-original:vod-clock',contentClock({sequence:100}));
 await emit('youtube-original:vod-clock',contentClock({sequence:101,sourceGeneration:2,sourceId:'ad',valid:false,adState:'ad',currentTime:null,duration:null}));await expect(ref.current!.seek(10)).rejects.toThrow();expect(api.vodControl).not.toHaveBeenCalled();
 await emit('youtube-original:vod-clock',contentClock({documentEpoch:3,sequence:1}));expect(onClock).toHaveBeenLastCalledWith(expect.objectContaining({documentEpoch:3,sequence:1}));await act(async()=>ref.current!.seek(10));expect(api.vodControl).toHaveBeenCalledOnce();
 await act(async()=>vi.advanceTimersByTimeAsync(1750));await expect(ref.current!.seek(10)).rejects.toThrow();expect(api.vodControl).toHaveBeenCalledOnce();
});

const blockedVod=()=>({...state,state:'error',chatVisible:false,captureReady:false,playability:'LOGIN_REQUIRED' as const,error:'이 영상을 볼 수 있는 YouTube 계정으로 로그인해 주세요.'});
const blockVod=()=>{vi.mocked(api.open).mockResolvedValue(blockedVod());vi.mocked(api.snapshot).mockResolvedValue(blockedVod());};
it('offers website login only for an explicitly opted-in unavailable VOD and never starts it automatically',async()=>{
 blockVod();await render({playbackKind:'vod'});expect(container.querySelector('.has-account-recovery')).toBeNull();
 await render({playbackKind:'vod',allowAccountRecovery:true});expect(container.querySelector('.has-account-recovery')).toHaveTextContent('YouTube 재생 계정 로그인');expect(api.login).not.toHaveBeenCalled();
 await render({playbackKind:'vod',allowAccountRecovery:true,privacyMode:true});expect(container.querySelector('.has-account-recovery')).toBeNull();
 await render({playbackKind:'vod',allowAccountRecovery:true,active:false});expect(container.querySelector<HTMLButtonElement>('.has-account-recovery button')).toBeDisabled();
 await render({allowAccountRecovery:true});expect(container.querySelector('.has-account-recovery')).toBeNull();expect(api.login).not.toHaveBeenCalled();
});
it('retries the exact VOD once after official login closes with a verified website observation',async()=>{
 blockVod();vi.mocked(api.account).mockResolvedValueOnce({authenticated:true,open:true,error:null}).mockResolvedValue({authenticated:true,open:false,error:null});
 await render({playbackKind:'vod',allowAccountRecovery:true});await click('.has-account-recovery button');expect(api.login).toHaveBeenCalledOnce();expect(container.querySelector('.has-account-recovery')).toHaveTextContent('로그인 확인 중');
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(api.open).toHaveBeenCalledOnce();
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(api.open).toHaveBeenCalledTimes(2);expect(api.open).toHaveBeenLastCalledWith('abcdefghijk',expect.any(String),'vod');expect(api.close).toHaveBeenCalledWith('lease-a');
 await act(async()=>vi.advanceTimersByTimeAsync(4500));expect(api.open).toHaveBeenCalledTimes(2);expect(api.logout).not.toHaveBeenCalled();expect(container.textContent).not.toContain('로그인됨');
});
it('does not call a canceled or failed website login successful and allows an explicit retry',async()=>{
 blockVod();await render({playbackKind:'vod',allowAccountRecovery:true});await click('.has-account-recovery button');
 await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(api.open).toHaveBeenCalledOnce();expect(container.textContent).toContain('로그인이 확인되지');
 vi.mocked(api.login).mockRejectedValueOnce(new Error('공식 로그인 창을 열지 못했습니다.'));await click('.has-account-recovery button');expect(container.textContent).toContain('공식 로그인 창을 열지 못했습니다');expect(api.open).toHaveBeenCalledOnce();
 await click('.has-account-recovery button:last-child');expect(api.open).toHaveBeenCalledTimes(2);
});
it('ignores website login completion from an old source and never closes the account window itself',async()=>{
 blockVod();let finish!:(value:{authenticated:boolean;open:boolean;error:null})=>void;
 vi.mocked(api.login).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
 await render({playbackKind:'vod',allowAccountRecovery:true});await click('.has-account-recovery button');
 vi.mocked(api.open).mockResolvedValue({...blockedVod(),token:'lease-b',channelId:'bcdefghijkl',broadcastId:'bcdefghijkl'});
 await render({playbackKind:'vod',allowAccountRecovery:true,channelId:'bcdefghijkl'});await act(async()=>finish({authenticated:true,open:false,error:null}));
 expect(api.open).toHaveBeenCalledTimes(2);expect(api.open).toHaveBeenLastCalledWith('bcdefghijkl',expect.any(String),'vod');expect(api.logout).not.toHaveBeenCalled();
});
it('defers a confirmed website login retry while private or inactive and never resumes a hidden video',async()=>{
 blockVod();vi.mocked(api.account).mockResolvedValue({authenticated:true,open:false,error:null});
 await render({playbackKind:'vod',allowAccountRecovery:true});await click('.has-account-recovery button');
 await render({playbackKind:'vod',allowAccountRecovery:true,privacyMode:true});await act(async()=>vi.advanceTimersByTimeAsync(3000));expect(api.account).not.toHaveBeenCalled();expect(api.open).toHaveBeenCalledOnce();
 await render({playbackKind:'vod',allowAccountRecovery:true,active:false});await act(async()=>vi.advanceTimersByTimeAsync(3000));expect(api.account).not.toHaveBeenCalled();expect(api.open).toHaveBeenCalledOnce();
 await render({playbackKind:'vod',allowAccountRecovery:true});await act(async()=>vi.advanceTimersByTimeAsync(1500));expect(api.open).toHaveBeenCalledTimes(2);expect(api.resume).not.toHaveBeenCalled();
});
it('retains generic private-access failure after a different logged-in account cannot play the video',async()=>{
 const unavailable={...blockedVod(),authenticated:true,playability:'UNAVAILABLE' as const,error:'YouTube에서 이 영상을 재생할 수 없습니다.'};
 vi.mocked(api.open).mockResolvedValue(unavailable);vi.mocked(api.snapshot).mockResolvedValue(unavailable);vi.mocked(api.login).mockResolvedValue({authenticated:true,open:false,error:null});
 await render({playbackKind:'vod',allowAccountRecovery:true});await click('.has-account-recovery button');
 expect(api.open).toHaveBeenCalledTimes(2);expect(container.querySelector('[role="alert"]')).toHaveTextContent('재생할 수 없습니다');expect(container.querySelector('.has-account-recovery')).not.toBeNull();
 await act(async()=>vi.advanceTimersByTimeAsync(3000));expect(api.open).toHaveBeenCalledTimes(2);
});

it('keeps owned replay events separate from ordinary same-video windows',async()=>{
 const owned=new Map<string,(value:any)=>void>(),stops:ReturnType<typeof vi.fn>[]=[];
 const subscribe=(kind:string)=>(handler:(value:any)=>void)=>{owned.set(kind,handler);const stop=vi.fn(()=>owned.delete(kind));stops.push(stop);return Promise.resolve(stop);};
 api.events={action:subscribe('action'),clock:subscribe('clock'),fullscreen:subscribe('fullscreen'),screenshot:subscribe('screenshot')};
 const onClock=vi.fn(),onClose=vi.fn();await render({playbackKind:'vod',onClock,onClose});onClock.mockClear();
 expect([...events.keys()]).toEqual(['platform-media:tray-muted']);
 await emit('youtube-original:action',{token:'lease-a',action:'viewer_close'});
 await emit('youtube-original:vod-clock',contentClock());
 await emit('youtube-original:fullscreen',{channelId:'abcdefghijk',enabled:true});
 expect(onClose).not.toHaveBeenCalled();expect(onClock).not.toHaveBeenCalled();expect(container.querySelector('[data-platform-native-fullscreen]')).toBeNull();
 await act(async()=>{owned.get('action')!({token:'foreign',action:'viewer_close'});owned.get('fullscreen')!({token:'foreign',enabled:true});owned.get('clock')!(contentClock({token:'foreign'}));});
 expect(onClose).not.toHaveBeenCalled();expect(onClock).not.toHaveBeenCalled();expect(container.querySelector('[data-platform-native-fullscreen]')).toBeNull();
 await act(async()=>{owned.get('clock')!(contentClock());owned.get('fullscreen')!({token:'lease-a',enabled:true});owned.get('action')!({token:'lease-a',action:'viewer_close'});});
 expect(onClock).toHaveBeenLastCalledWith(contentClock());expect(onClose).toHaveBeenCalledOnce();expect(container.querySelector('[data-platform-native-fullscreen]')).not.toBeNull();
 const late=owned.get('action')!;await act(async()=>root.render(null));expect(stops.every(stop=>stop.mock.calls.length===1)).toBe(true);
 await act(async()=>late({token:'lease-a',action:'viewer_close'}));expect(onClose).toHaveBeenCalledOnce();
});

it('disposes late scoped subscriptions without attaching to a replacement player',async()=>{
 let resolve!:(stop:()=>void)=>void;const lateStop=vi.fn(),noop=async()=>()=>{};
 api.events={action:()=>new Promise(done=>{resolve=done;}),clock:noop,fullscreen:noop,screenshot:noop};
 await render({playbackKind:'vod'});await act(async()=>root.render(null));await act(async()=>resolve(lateStop));expect(lateStop).toHaveBeenCalledOnce();
 expect(events.has('youtube-original:action')).toBe(false);
});

it('cancels a scoped unresolved open on unmount before receiving a token and retains late cleanup',async()=>{
 let finish!:(value:YouTubeOriginalSnapshot)=>void;
 api.cancelOpen=vi.fn(async()=>undefined);vi.mocked(api.open).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
 await render({playbackKind:'vod'});const request=vi.mocked(api.open).mock.calls[0]![1];
 await act(async()=>root.render(null));expect(api.cancelOpen).toHaveBeenCalledExactlyOnceWith(request);expect(api.close).not.toHaveBeenCalled();
 await act(async()=>finish({...state,token:'late-owned'}));expect(api.close).toHaveBeenCalledExactlyOnceWith('late-owned');
});
it('cancels only the prior pending source and never the replacement that already opened',async()=>{
 let finish!:(value:YouTubeOriginalSnapshot)=>void;
 api.cancelOpen=vi.fn(async()=>undefined);vi.mocked(api.open).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
 await render({playbackKind:'vod'});const request=vi.mocked(api.open).mock.calls[0]![1];
 vi.mocked(api.open).mockResolvedValueOnce({...state,token:'replacement',channelId:'bcdefghijkl',broadcastId:'bcdefghijkl'});
 await render({playbackKind:'vod',channelId:'bcdefghijkl'});
 expect(api.cancelOpen).toHaveBeenCalledExactlyOnceWith(request);
 await act(async()=>finish({...state,token:'abandoned'}));expect(api.close).toHaveBeenCalledExactlyOnceWith('abandoned');
 await act(async()=>root.render(null));expect(api.cancelOpen).toHaveBeenCalledOnce();expect(api.close).toHaveBeenLastCalledWith('replacement');
});
it('does not cancel a rejected open and tolerates a rejected pending cancellation',async()=>{
 api.cancelOpen=vi.fn(async()=>{throw new Error('native busy');});vi.mocked(api.open).mockRejectedValueOnce(new Error('open failed'));
 await render({playbackKind:'vod'});await act(async()=>root.render(null));expect(api.cancelOpen).not.toHaveBeenCalled();
 let finish!:(value:YouTubeOriginalSnapshot)=>void;vi.mocked(api.open).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
 await render({playbackKind:'vod'});await act(async()=>root.render(null));expect(api.cancelOpen).toHaveBeenCalledOnce();
 await act(async()=>finish({...state,token:'late-after-failed-cancel'}));expect(api.close).toHaveBeenCalledExactlyOnceWith('late-after-failed-cancel');
});

