import { ScreenshotPreview } from '../platforms/ScreenshotPreview';
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { youtubeOriginalApi, type YouTubeOriginalApi, type YouTubeOriginalSnapshot } from '../../api/youtubeOriginal';
import { measureNativePlayerGeometry, observeNativePlayerGeometry } from '../platforms/nativePlayerGeometry';
import { clampLiveChat, liveSplit, liveSplitKey, LIVE_SPLITTER_SIZE } from '../platforms/liveSplit';
import '../platforms/PlatformLivePlayer.css';
import './YouTubeOriginalPlayer.css';
import { useYouTubeScreenshot } from './useYouTubeScreenshot';
const errorText=(e:unknown)=>typeof e==='string'?e:e instanceof Error?e.message:'YouTube 방송을 연결하지 못했습니다.';
export type YouTubeRecordingObservation = { videoId:string; token:string; canonicalChannelId:string|null; state:string; recordingJob:string|null; recordingStopping:boolean };
type ObservationOwner = { instanceId:string; attempt:number; videoId:string };
export type YouTubeViewerObservation = ObservationOwner & ({kind:'begin'} | {kind:'update'; value:YouTubeRecordingObservation} | {kind:'clear'; detached:boolean});
/** Native official document is retained; these are geometry slots, never another video. */
export function YouTubeOriginalPlayer({channelId,title,active,privacyMode,videoVisible=true,flattened=false,sideBySide=false,onClose,onObservation,resumeRequest=0,playbackKind='live',api=youtubeOriginalApi}:{channelId:string;title:string;active:boolean;privacyMode:boolean;videoVisible?:boolean;flattened?:boolean;sideBySide?:boolean;onClose?:()=>void;onObservation?:(event:YouTubeViewerObservation)=>void;resumeRequest?:number;playbackKind?:'live'|'vod';api?:YouTubeOriginalApi}) {
  const live=playbackKind==='live';
  const [snapshot,setSnapshot]=useState<YouTubeOriginalSnapshot|null>(null),[error,setError]=useState(''),[pollError,setPollError]=useState(''),[attempt,setAttempt]=useState(0);
  const [trayMuted,setTrayMuted]=useState(false),[chatWidth,setChatWidth]=useState(480),[fraction,setFraction]=useState(.44),[size,setSize]=useState({width:0,height:0});
  const [nativeFullscreen,setNativeFullscreen]=useState(false);
  const pair=useRef<HTMLDivElement>(null),player=useRef<HTMLDivElement>(null),chat=useRef<HTMLDivElement>(null),token=useRef<string|null>(null),generation=useRef(0);
  const sequence=useRef(0);
  const instance=useRef<string|null>(null);if(!instance.current)instance.current=crypto.randomUUID();
  const observation=useRef(onObservation);observation.current=onObservation;
  const screenshot=useYouTubeScreenshot(snapshot?.token??null,live&&active&&!privacyMode&&!trayMuted&&videoVisible&&snapshot?.state==='playing',api);
  const screenshotSave=useRef(screenshot.save);screenshotSave.current=screenshot.save;
  const closeAction=useRef(onClose);closeAction.current=onClose;
  const resumeConsumed=useRef(0);
  const controls=useRef(()=>Promise.resolve());controls.current=async()=>{const lease=token.current;if(lease&&live)await api.controls?.(lease,!!closeAction.current&&active&&!privacyMode&&videoVisible,!active||privacyMode||trayMuted||!videoVisible||snapshot?.state!=='playing'||screenshot.busy||!!screenshot.preview?.undoing);};
  useEffect(()=>{void controls.current().catch(e=>setError(errorText(e)));},[snapshot,active,privacyMode,trayMuted,videoVisible,screenshot.busy,screenshot.preview?.undoing,onClose]);
  useEffect(()=>{let gone=false,stop:(()=>void)|undefined;void listen<{token:string;action:string}>('youtube-original:action',event=>{if(event.payload.token!==token.current||!live)return;if(event.payload.action==='viewer_close')closeAction.current?.();else if(event.payload.action==='screenshot')void screenshotSave.current();}).then(value=>{if(gone)value();else stop=value;}).catch(()=>undefined);return()=>{gone=true;stop?.();};},[live]);

  const effectiveFlattened=live&&flattened&&!nativeFullscreen;
  const split=liveSplit(size.width,size.height,chatWidth,fraction,sideBySide||nativeFullscreen),compact=!effectiveFlattened&&split.compact;
  useEffect(()=>{const el=pair.current;if(!el)return;const measure=()=>{const r=el.getBoundingClientRect();if(r.width>0&&r.height>0)setSize(previous=>previous.width===r.width&&previous.height===r.height?previous:{width:r.width,height:r.height});};measure();const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(measure);observer?.observe(el);return()=>observer?.disconnect();},[]);
  useEffect(()=>{let gone=false,polling=false;const current=++generation.current;setSnapshot(null);setError('');setPollError('');
    const owner={instanceId:instance.current!,attempt:current,videoId:channelId};
    const clear=(detached=false)=>{if(live)observation.current?.({...owner,kind:'clear',detached});};
    const publish=(value:YouTubeOriginalSnapshot,lease:string)=>{
      if(!live)return;
      if(value.token!==lease||value.channelId!==channelId||value.broadcastId!==channelId){clear();return;}
      observation.current?.({...owner,kind:'update',value:{videoId:channelId,token:lease,state:value.state,canonicalChannelId:typeof value.canonicalChannelId==='string'&&/^UC[A-Za-z0-9_-]{22}$/.test(value.canonicalChannelId)?value.canonicalChannelId:null,recordingJob:value.recordingJob,recordingStopping:value.recordingStopping}});
    };
    if(live)observation.current?.({...owner,kind:'begin'});
    const requestId=crypto.randomUUID();
    void (live?api.open(channelId,requestId):api.open(channelId,requestId,'vod')).then(value=>{if(gone){void api.close(value.token).catch(()=>undefined);return;}token.current=value.token;setSnapshot(value);publish(value,value.token);}).catch(e=>{if(!gone){setError(errorText(e));clear();}});
    const timer=setInterval(()=>{const lease=token.current;if(!lease||gone||polling)return;polling=true;void api.snapshot(lease).then(value=>{if(!gone&&generation.current===current&&token.current===lease){setSnapshot(value);setPollError('');publish(value,lease);}}).catch(e=>{if(!gone&&generation.current===current&&token.current===lease){setPollError(errorText(e));clear();}}).finally(()=>{polling=false;});},1500);
    return()=>{gone=true;++generation.current;clearInterval(timer);clear(true);const lease=token.current;token.current=null;if(lease)void api.close(lease).catch(()=>undefined);};
  },[api,channelId,attempt,live]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;void listen<boolean>('platform-media:tray-muted',event=>setTrayMuted(event.payload)).then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;void listen<{channelId:string}>('youtube-original:screenshot',event=>{if(event.payload.channelId===channelId)void screenshotSave.current();}).then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[channelId]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;void listen<{channelId:string;enabled:boolean}>('youtube-original:fullscreen',event=>{if(event.payload.channelId===channelId)setNativeFullscreen(event.payload.enabled);}).then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[channelId]);
  useEffect(()=>{if(snapshot)setNativeFullscreen(snapshot.fullscreen);},[snapshot?.fullscreen]);
  useEffect(()=>{if(nativeFullscreen&&(!active||privacyMode||!videoVisible)){setNativeFullscreen(false);const lease=token.current;if(lease)void api.fullscreen(lease,false).catch(e=>setError(errorText(e)));}},[api,nativeFullscreen,active,privacyMode,videoVisible]);
  useEffect(()=>{if(!nativeFullscreen||!active||privacyMode||!videoVisible)return;const key=(event:KeyboardEvent)=>{if(event.defaultPrevented||event.isComposing||event.keyCode===229||event.ctrlKey||event.altKey||event.metaKey||event.target instanceof Element&&event.target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"])'))return;if(event.key==='Escape'&&token.current){event.preventDefault();event.stopPropagation();void api.fullscreen(token.current,false).catch(e=>setError(errorText(e)));}};document.addEventListener('keydown',key);return()=>document.removeEventListener('keydown',key);},[api,nativeFullscreen,active,privacyMode,videoVisible]);
  useEffect(()=>{const lease=snapshot?.token;if(!lease)return;let previous='',disposed=false,sending=false,queued:ReturnType<typeof measureNativePlayerGeometry>|null=null;
    const send=async()=>{if(sending||disposed)return;sending=true;try{while(queued&&!disposed){const next=queued;queued=null;await api.viewport(lease,next);if(disposed||token.current!==lease)continue;await controls.current();if(live&&next.visible&&next.player&&!trayMuted&&resumeRequest>resumeConsumed.current){resumeConsumed.current=resumeRequest;await api.resume?.(lease);}}}catch(e){if(!disposed)setError(errorText(e));}finally{sending=false;}};
    const measure=()=>{const viewport=measureNativePlayerGeometry(videoVisible?player.current:null,live?chat.current:null,{sequence:++sequence.current,visible:active&&!privacyMode,muted:privacyMode||trayMuted});const key=JSON.stringify({...viewport,sequence:0});if(key===previous)return;previous=key;queued=viewport;void send();};
    const stop=observeNativePlayerGeometry([player.current,chat.current],measure);measure();return()=>{disposed=true;stop();};
  },[api,snapshot?.token,active,privacyMode,trayMuted,videoVisible,flattened,sideBySide,nativeFullscreen,live,resumeRequest]);
  const resize=(pixels:number)=>{const next=clampLiveChat(pixels,split.min,split.max);if(compact)setFraction(split.available?next/split.available:.44);else setChatWidth(next);};
  return <div ref={pair} data-platform-native-fullscreen={nativeFullscreen?'true':undefined} data-playback-kind={playbackKind} className={`platform-live-pair youtube-original-pair${live?' has-chat':' youtube-original-vod'}${live&&compact?' is-compact':''}${effectiveFlattened?' is-flattened':''}${live&&!videoVisible?' chat-only':''}${nativeFullscreen?' is-native-fullscreen':''}`} style={{'--platform-live-chat-size':`${size.width?split.pixels:chatWidth}px`} as CSSProperties} inert={!active||undefined}>
    <div ref={player} className="platform-live-player youtube-original-slot" aria-label={privacyMode?'방송':title} aria-hidden={!videoVisible||undefined}>
      {privacyMode?<div className="platform-live-privacy">프라이버시 모드</div>:null}
      {(!snapshot||snapshot.state==='loading'||snapshot.state==='preparing')&&!error?<div className="platform-live-status" role="status"><span className="platform-live-spinner"/>{live?'라이브 연결 중':'영상 연결 중'}</div>:null}
      {error||snapshot?.error||pollError?<div className="platform-live-error youtube-original-notice" role="alert" data-platform-native-occlusion="true"><span>{error||snapshot?.error||pollError}</span><button onClick={()=>setAttempt(value=>value+1)}>다시 시도</button></div>:null}
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
