import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { YouTubeOriginalPlayer } from './YouTubeOriginalPlayer';
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

