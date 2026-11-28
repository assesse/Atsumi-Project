import { ScreenshotPreview } from '../platforms/ScreenshotPreview';
import { useEffect, useImperativeHandle, useRef, useState, type CSSProperties, type Ref } from 'react';
import { listen } from '@tauri-apps/api/event';
import { youtubeOriginalApi, type YouTubeAccountSnapshot, type YouTubeOriginalApi, type YouTubeOriginalSnapshot, type YouTubeVodClock } from '../../api/youtubeOriginal';
import { measureNativePlayerGeometry, observeNativePlayerGeometry } from '../platforms/nativePlayerGeometry';
import { clampLiveChat, liveSplit, liveSplitKey, LIVE_SPLITTER_SIZE } from '../platforms/liveSplit';
import '../platforms/PlatformLivePlayer.css';
import './YouTubeOriginalPlayer.css';
import { useYouTubeScreenshot } from './useYouTubeScreenshot';
const errorText=(e:unknown)=>typeof e==='string'?e:e instanceof Error?e.message:'YouTube 방송을 연결하지 못했습니다.';
export type YouTubeRecordingObservation = { videoId:string; token:string; canonicalChannelId:string|null; state:string; recordingJob:string|null; recordingStopping:boolean };
type ObservationOwner = { instanceId:string; attempt:number; videoId:string };
type VodLoginAttempt = { owner:number; source:string; started:number; waiting:boolean };
export type YouTubeViewerObservation = ObservationOwner & ({kind:'begin'} | {kind:'update'; value:YouTubeRecordingObservation} | {kind:'clear'; detached:boolean});
export type YouTubeOriginalPlayerHandle = {seek(seconds:number):Promise<void>;pause():Promise<void>;resume():Promise<void>;setRate(rate:number):Promise<void>};
export type { YouTubeVodClock } from '../../api/youtubeOriginal';
/** Native official document is retained; these are geometry slots, never another video. */
export function YouTubeOriginalPlayer({channelId,title,active,privacyMode,videoVisible=true,flattened=false,sideBySide=false,onClose,onObservation,onClock,ref,resumeRequest=0,playbackKind='live',allowAccountRecovery=false,api=youtubeOriginalApi}:{channelId:string;title:string;active:boolean;privacyMode:boolean;videoVisible?:boolean;flattened?:boolean;sideBySide?:boolean;onClose?:()=>void;onObservation?:(event:YouTubeViewerObservation)=>void;onClock?:(clock:YouTubeVodClock|null)=>void;ref?:Ref<YouTubeOriginalPlayerHandle>;resumeRequest?:number;playbackKind?:'live'|'vod';allowAccountRecovery?:boolean;api?:YouTubeOriginalApi}) {
  const live=playbackKind==='live';
  const [snapshot,setSnapshot]=useState<YouTubeOriginalSnapshot|null>(null),[error,setError]=useState(''),[pollError,setPollError]=useState(''),[attempt,setAttempt]=useState(0);
  const [trayMuted,setTrayMuted]=useState(false),[chatWidth,setChatWidth]=useState(480),[fraction,setFraction]=useState(.44),[size,setSize]=useState({width:0,height:0});
  const [nativeFullscreen,setNativeFullscreen]=useState(false);
  const [accountBusy,setAccountBusy]=useState(false),[accountError,setAccountError]=useState('');
  const accountAttempt=useRef<VodLoginAttempt|null>(null),accountAllowed=useRef(false);
  accountAllowed.current=!live&&allowAccountRecovery&&active&&!privacyMode&&!trayMuted&&videoVisible;
  const pair=useRef<HTMLDivElement>(null),player=useRef<HTMLDivElement>(null),chat=useRef<HTMLDivElement>(null),token=useRef<string|null>(null),generation=useRef(0);
  const sequence=useRef(0);
  const accountCurrent=(expected:VodLoginAttempt)=>accountAttempt.current===expected&&expected.owner===generation.current&&expected.source===channelId;
  const accountFailed=(expected:VodLoginAttempt,message:string)=>{if(!accountCurrent(expected))return;accountAttempt.current=null;setAccountBusy(false);setAccountError(message);};
  const accountObserved=(expected:VodLoginAttempt,value:YouTubeAccountSnapshot)=>{
    if(!accountCurrent(expected))return;
    if(value.error){accountFailed(expected,value.error);return;}
    if(value.open){expected.waiting=true;return;}
    if(value.authenticated!==true){accountFailed(expected,'YouTube 로그인이 확인되지 않았습니다. 로그인하거나 다시 시도해 주세요.');return;}
    // This confirms a website session only, not the upload owner or video access.
    // Retrying the exact video leaves those checks to its actual content clock.
    if(!accountAllowed.current){expected.waiting=true;return;}
    accountAttempt.current=null;setAccountBusy(false);setAccountError('');setAttempt(value=>value+1);
  };
  const loginForPlayback=async()=>{
    if(!accountAllowed.current||accountAttempt.current)return;
    const expected:VodLoginAttempt={owner:generation.current,source:channelId,started:Date.now(),waiting:false};
    accountAttempt.current=expected;setAccountBusy(true);setAccountError('');
    try{accountObserved(expected,await api.login());}catch(cause){accountFailed(expected,errorText(cause));}
  };
  const accountObserver=useRef(accountObserved);accountObserver.current=accountObserved;
  const accountFailure=useRef(accountFailed);accountFailure.current=accountFailed;
  useEffect(()=>{
    if(!accountBusy||!accountAllowed.current)return;
    let gone=false,polling=false;
    const timer=setInterval(()=>{const expected=accountAttempt.current;if(gone||polling||!expected?.waiting||!accountAllowed.current)return;
      if(Date.now()-expected.started>600000){accountFailure.current(expected,'로그인 확인이 지연되고 있습니다. 로그인 창을 확인한 뒤 다시 시도해 주세요.');return;}
      polling=true;void api.account().then(value=>{if(!gone)accountObserver.current(expected,value);}).catch(cause=>{if(!gone)accountFailure.current(expected,errorText(cause));}).finally(()=>{polling=false;});
    },1500);
    return()=>{gone=true;clearInterval(timer);};
  },[api,accountBusy,active,privacyMode,trayMuted,videoVisible,allowAccountRecovery,live]);
  const clock=useRef<YouTubeVodClock|null>(null),clockWatermark=useRef<YouTubeVodClock|null>(null),clockReceived=useRef(0),clockCallback=useRef(onClock);clockCallback.current=onClock;
  const acceptClock=useRef((value:YouTubeVodClock)=>{void value;});
  acceptClock.current=value=>{
    if(live||value.token!==token.current||value.videoId!==channelId||!Number.isSafeInteger(value.documentEpoch)||value.documentEpoch<1||!Number.isSafeInteger(value.sequence)||value.sequence<1||!Number.isSafeInteger(value.sourceGeneration)||value.sourceGeneration<1||!Number.isSafeInteger(value.seekGeneration)||value.seekGeneration<1||!Number.isFinite(value.ageMs)||value.ageMs<0||value.ageMs>1500)return;
    const previous=clockWatermark.current;
    if(previous&&(value.documentEpoch<previous.documentEpoch||value.documentEpoch===previous.documentEpoch&&(value.sequence<=previous.sequence||value.sourceGeneration<previous.sourceGeneration||value.seekGeneration<previous.seekGeneration||value.sourceGeneration===previous.sourceGeneration&&value.sourceId!==previous.sourceId)))return;
    if(value.valid&&(value.adState!=='content'||value.currentTime===null||!Number.isFinite(value.currentTime)||value.currentTime<0||value.duration===null||!Number.isFinite(value.duration)||value.duration<=0||value.currentTime>value.duration+2))return;
    if(!value.valid&&(value.currentTime!==null||value.duration!==null||!['ad','unknown'].includes(value.adState)))return;
    clock.current=value;clockWatermark.current=value;clockReceived.current=Date.now()-value.ageMs;clockCallback.current?.(value);
  };
  useImperativeHandle(ref,()=>{
    const control=async(action:'seek'|'pause'|'resume'|'rate',amount?:number)=>{
      const value=clock.current,lease=token.current;
      if(live||!value||!lease||value.token!==lease||Date.now()-clockReceived.current>1500||!api.vodControl||action!=='pause'&&!value.valid||action==='resume'&&(!active||privacyMode||trayMuted||!videoVisible)||action==='seek'&&(!Number.isFinite(amount)||amount!<0||amount!>(value.duration??0))||action==='rate'&&(!Number.isFinite(amount)||amount!<.1||amount!>16))throw new Error('영상이 준비되면 다시 시도해 주세요.');
      await api.vodControl(lease,{requestId:crypto.randomUUID(),documentEpoch:value.documentEpoch,sourceId:value.sourceId,sourceGeneration:value.sourceGeneration,action,...(amount===undefined?{}:action==='rate'?{rate:amount}:{seconds:amount})});
    };
    return {seek:seconds=>control('seek',seconds),pause:()=>control('pause'),resume:()=>control('resume'),setRate:rate=>control('rate',rate)};
  },[api,live,active,privacyMode,trayMuted,videoVisible]);
  const instance=useRef<string|null>(null);if(!instance.current)instance.current=crypto.randomUUID();
  const observation=useRef(onObservation);observation.current=onObservation;
  const screenshot=useYouTubeScreenshot(snapshot?.token??null,live&&active&&!privacyMode&&!trayMuted&&videoVisible&&snapshot?.state==='playing',api);
  const screenshotSave=useRef(screenshot.save);screenshotSave.current=screenshot.save;
  const closeAction=useRef(onClose);closeAction.current=onClose;
  const resumeConsumed=useRef(0);
  const controls=useRef(()=>Promise.resolve());controls.current=async()=>{const lease=token.current;if(lease)await api.controls?.(lease,!!closeAction.current&&active&&!privacyMode&&videoVisible,!active||privacyMode||trayMuted||!videoVisible||snapshot?.state!=='playing'||screenshot.busy||!!screenshot.preview?.undoing);};
  useEffect(()=>{void controls.current().catch(e=>setError(errorText(e)));},[snapshot,active,privacyMode,trayMuted,videoVisible,screenshot.busy,screenshot.preview?.undoing,onClose]);
  useEffect(()=>{let gone=false,stop:(()=>void)|undefined;
    const receive=(value:{token:string;action:string})=>{if(gone||value.token!==token.current)return;if(value.action==='viewer_close')closeAction.current?.();else if(live&&value.action==='screenshot')void screenshotSave.current();};
    void (api.events?api.events.action(receive):listen<{token:string;action:string}>('youtube-original:action',event=>receive(event.payload))).then(value=>{if(gone)value();else stop=value;}).catch(()=>undefined);
    return()=>{gone=true;stop?.();};},[api,live]);
  useEffect(()=>{if(live)return;let gone=false,stop:(()=>void)|undefined;
    const receive=(value:YouTubeVodClock)=>{if(!gone)acceptClock.current(value);};
    void (api.events?api.events.clock(receive):listen<YouTubeVodClock>('youtube-original:vod-clock',event=>receive(event.payload))).then(value=>{if(gone)value();else stop=value;}).catch(()=>undefined);
    const timer=setInterval(()=>{if(clock.current&&Date.now()-clockReceived.current>1500){clock.current=null;clockCallback.current?.(null);}},250);
    return()=>{gone=true;stop?.();clearInterval(timer);};
  },[api,live]);

  const effectiveFlattened=live&&flattened&&!nativeFullscreen;
  const split=liveSplit(size.width,size.height,chatWidth,fraction,sideBySide||nativeFullscreen),compact=!effectiveFlattened&&split.compact;
  useEffect(()=>{const el=pair.current;if(!el)return;const measure=()=>{const r=el.getBoundingClientRect();if(r.width>0&&r.height>0)setSize(previous=>previous.width===r.width&&previous.height===r.height?previous:{width:r.width,height:r.height});};measure();const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(measure);observer?.observe(el);return()=>observer?.disconnect();},[]);
  useEffect(()=>{let gone=false,polling=false;const current=++generation.current;setSnapshot(null);setError('');setPollError('');clock.current=null;clockWatermark.current=null;if(!live)clockCallback.current?.(null);
    accountAttempt.current=null;setAccountBusy(false);setAccountError('');
    const owner={instanceId:instance.current!,attempt:current,videoId:channelId};
    const clear=(detached=false)=>{if(live)observation.current?.({...owner,kind:'clear',detached});};
    const publish=(value:YouTubeOriginalSnapshot,lease:string)=>{
      if(!live)return;
      if(value.token!==lease||value.channelId!==channelId||value.broadcastId!==channelId){clear();return;}
      observation.current?.({...owner,kind:'update',value:{videoId:channelId,token:lease,state:value.state,canonicalChannelId:typeof value.canonicalChannelId==='string'&&/^UC[A-Za-z0-9_-]{22}$/.test(value.canonicalChannelId)?value.canonicalChannelId:null,recordingJob:value.recordingJob,recordingStopping:value.recordingStopping}});
    };
    if(live)observation.current?.({...owner,kind:'begin'});
    const requestId=crypto.randomUUID();let opening=true;
    void (live?api.open(channelId,requestId):api.open(channelId,requestId,'vod')).then(value=>{opening=false;if(gone){void api.close(value.token).catch(()=>undefined);return;}token.current=value.token;setSnapshot(value);publish(value,value.token);if(!live&&api.vodClock)void api.vodClock(value.token).then(value=>{if(value&&!gone&&generation.current===current)acceptClock.current(value);}).catch(()=>undefined);}).catch(e=>{opening=false;if(!gone){setError(errorText(e));clear();}});
    const timer=setInterval(()=>{const lease=token.current;if(!lease||gone||polling)return;polling=true;void api.snapshot(lease).then(value=>{if(!gone&&generation.current===current&&token.current===lease){setSnapshot(value);setPollError('');publish(value,lease);}}).catch(e=>{if(!gone&&generation.current===current&&token.current===lease){setPollError(errorText(e));clear();}}).finally(()=>{polling=false;});},1500);
    return()=>{gone=true;++generation.current;if(opening&&api.cancelOpen)void api.cancelOpen(requestId).catch(()=>undefined);clearInterval(timer);clear(true);clock.current=null;clockWatermark.current=null;if(!live)clockCallback.current?.(null);const lease=token.current;token.current=null;if(lease)void api.close(lease).catch(()=>undefined);};
  },[api,channelId,attempt,live]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;void listen<boolean>('platform-media:tray-muted',event=>setTrayMuted(event.payload)).then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;
    const subscription=api.events?api.events.screenshot(value=>{if(!gone&&value.token===token.current)void screenshotSave.current();}):listen<{channelId:string}>('youtube-original:screenshot',event=>{if(!gone&&event.payload.channelId===channelId)void screenshotSave.current();});
    void subscription.then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[api,channelId]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;
    const subscription=api.events?api.events.fullscreen(value=>{if(!gone&&value.token===token.current)setNativeFullscreen(value.enabled);}):listen<{channelId:string;enabled:boolean}>('youtube-original:fullscreen',event=>{if(!gone&&event.payload.channelId===channelId)setNativeFullscreen(event.payload.enabled);});
    void subscription.then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[api,channelId]);
  useEffect(()=>{if(snapshot)setNativeFullscreen(snapshot.fullscreen);},[snapshot?.fullscreen]);
  useEffect(()=>{if(nativeFullscreen&&(!active||privacyMode||!videoVisible)){setNativeFullscreen(false);const lease=token.current;if(lease)void api.fullscreen(lease,false).catch(e=>setError(errorText(e)));}},[api,nativeFullscreen,active,privacyMode,videoVisible]);
  useEffect(()=>{if(!nativeFullscreen||!active||privacyMode||!videoVisible)return;const key=(event:KeyboardEvent)=>{if(event.defaultPrevented||event.isComposing||event.keyCode===229||event.ctrlKey||event.altKey||event.metaKey||event.target instanceof Element&&event.target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"])'))return;if(event.key==='Escape'&&token.current){event.preventDefault();event.stopPropagation();void api.fullscreen(token.current,false).catch(e=>setError(errorText(e)));}};document.addEventListener('keydown',key);return()=>document.removeEventListener('keydown',key);},[api,nativeFullscreen,active,privacyMode,videoVisible]);
  useEffect(()=>{const lease=snapshot?.token;if(!lease)return;let previous='',disposed=false,sending=false,queued:ReturnType<typeof measureNativePlayerGeometry>|null=null;
    const send=async()=>{if(sending||disposed)return;sending=true;try{while(queued&&!disposed){const next=queued;queued=null;await api.viewport(lease,next);if(disposed||token.current!==lease)continue;await controls.current();if(live&&next.visible&&next.player&&!trayMuted&&resumeRequest>resumeConsumed.current){resumeConsumed.current=resumeRequest;await api.resume?.(lease);}}}catch(e){if(!disposed)setError(errorText(e));}finally{sending=false;}};
    const measure=()=>{const viewport=measureNativePlayerGeometry(videoVisible?player.current:null,live?chat.current:null,{sequence:++sequence.current,visible:active&&!privacyMode,muted:privacyMode||trayMuted});const key=JSON.stringify({...viewport,sequence:0});if(key===previous)return;previous=key;queued=viewport;void send();};
    const stop=observeNativePlayerGeometry([player.current,chat.current],measure);measure();return()=>{disposed=true;stop();};
  },[api,snapshot?.token,active,privacyMode,trayMuted,videoVisible,flattened,sideBySide,nativeFullscreen,live,resumeRequest]);
  const resize=(pixels:number)=>{const next=clampLiveChat(pixels,split.min,split.max);if(compact)setFraction(split.available?next/split.available:.44);else setChatWidth(next);};
  const showAccountRecovery=!live&&allowAccountRecovery&&!!snapshot?.playability&&!privacyMode;
  return <div ref={pair} data-platform-native-fullscreen={nativeFullscreen?'true':undefined} data-playback-kind={playbackKind} className={`platform-live-pair youtube-original-pair${live?' has-chat':' youtube-original-vod'}${live&&compact?' is-compact':''}${effectiveFlattened?' is-flattened':''}${live&&!videoVisible?' chat-only':''}${nativeFullscreen?' is-native-fullscreen':''}`} style={{'--platform-live-chat-size':`${size.width?split.pixels:chatWidth}px`} as CSSProperties} inert={!active||undefined}>
    <div ref={player} className="platform-live-player youtube-original-slot" aria-label={privacyMode?'방송':title} aria-hidden={!videoVisible||undefined}>
      {privacyMode?<div className="platform-live-privacy">프라이버시 모드</div>:null}
      {(!snapshot||snapshot.state==='loading'||snapshot.state==='preparing')&&!error?<div className="platform-live-status" role="status"><span className="platform-live-spinner"/>{live?'라이브 연결 중':'영상 연결 중'}</div>:null}
      {error||snapshot?.error||pollError||accountError?<div className={`platform-live-error youtube-original-notice${showAccountRecovery?' has-account-recovery':''}`} role="alert" data-platform-native-occlusion="true"><span>{accountError||error||snapshot?.error||pollError}</span>
        {showAccountRecovery?<><span>영상을 업로드한 계정으로 YouTube에 로그인해 주세요.</span><button type="button" disabled={!accountAllowed.current||accountBusy} onClick={()=>void loginForPlayback()}>{accountBusy?'로그인 확인 중…':'YouTube 재생 계정 로그인'}</button></>:null}
        <button disabled={accountBusy} onClick={()=>setAttempt(value=>value+1)}>다시 시도</button></div>:null}
      {screenshot.busy?<div className="platform-live-capture-notice" role="status" data-platform-native-occlusion="true">스크린샷 저장 중…</div>:null}
      {screenshot.error?<div className="platform-live-capture-notice is-error" role="alert" data-platform-native-occlusion="true"><span>{screenshot.error}</span><button type="button" aria-label="스크린샷 오류 닫기" onClick={screenshot.clearError}>×</button></div>:null}
      {screenshot.preview?<ScreenshotPreview preview={screenshot.preview} onDismiss={screenshot.dismiss} onUndo={()=>void screenshot.undo()}/>:null}
    </div>
    {live&&videoVisible?<div className="platform-live-resize" role="separator" aria-label={compact?'채팅 높이':'채팅 너비'} aria-orientation={compact?'horizontal':'vertical'} aria-valuemin={Math.round(split.min)} aria-valuemax={Math.round(split.max)} aria-valuenow={Math.round(split.pixels)} tabIndex={active&&!effectiveFlattened?0:-1}
      onPointerDown={event=>{if(!active||effectiveFlattened||event.button!==0)return;event.preventDefault();event.currentTarget.focus();event.currentTarget.setPointerCapture(event.pointerId);}}
      onPointerMove={event=>{if(active&&!effectiveFlattened&&event.currentTarget.hasPointerCapture(event.pointerId)){const r=pair.current?.getBoundingClientRect();if(r)resize((compact?r.bottom-event.clientY:r.right-event.clientX)-LIVE_SPLITTER_SIZE/2);}}}
      onPointerUp={event=>{if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);}}
      onPointerCancel={event=>{if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);}}
      onKeyDown={event=>{if(!active||effectiveFlattened||event.altKey||event.ctrlKey||event.metaKey)return;const next=liveSplitKey(event.key,split.pixels,split.min,split.max,compact);if(next!==null){event.preventDefault();resize(next);}}}/>:null}
    {live?<div ref={chat} className="platform-live-chat youtube-original-chat" aria-label="YouTube 원본 채팅">{privacyMode?<span>프라이버시 모드</span>:null}</div>:null}
  </div>;
}
