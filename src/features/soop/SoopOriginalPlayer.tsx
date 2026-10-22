import { ScreenshotPreview } from '../platforms/ScreenshotPreview';
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { soopOriginalApi, type SoopOriginalApi, type SoopOriginalSnapshot } from '../../api/soopOriginal';
import { measureNativePlayerGeometry, observeNativePlayerGeometry } from '../platforms/nativePlayerGeometry';
import { clampLiveChat, liveSplit, liveSplitKey, LIVE_SPLITTER_SIZE } from '../platforms/liveSplit';
import { screenshotShortcut } from '../platforms/useLiveScreenshot';
import { useSoopOriginalScreenshot } from './useSoopOriginalScreenshot';
import '../platforms/PlatformLivePlayer.css';
import './SoopOriginalPlayer.css';
const errorText=(e:unknown)=>typeof e==='string'?e:e instanceof Error?e.message:'SOOP 방송을 연결하지 못했습니다.';
/** Native official document is retained; these are geometry slots, never another video. */
export function SoopOriginalPlayer({channelId,title,active,privacyMode,videoVisible=true,flattened=false,sideBySide=false,onClose,resumeRequest=0,api=soopOriginalApi}:{channelId:string;title:string;active:boolean;privacyMode:boolean;videoVisible?:boolean;flattened?:boolean;sideBySide?:boolean;onClose?:()=>void;resumeRequest?:number;api?:SoopOriginalApi}) {
  const [snapshot,setSnapshot]=useState<SoopOriginalSnapshot|null>(null),[error,setError]=useState(''),[attempt,setAttempt]=useState(0);
  const [trayMuted,setTrayMuted]=useState(false),[chatWidth,setChatWidth]=useState(480),[fraction,setFraction]=useState(.44),[size,setSize]=useState({width:0,height:0}),[ownFullscreen,setOwnFullscreen]=useState(false);
  const pair=useRef<HTMLDivElement>(null),player=useRef<HTMLDivElement>(null),chat=useRef<HTMLDivElement>(null),token=useRef<string|null>(null),generation=useRef(0),viewportSequence=useRef(0);
  const screenshot=useSoopOriginalScreenshot({api,token:snapshot?.token??null,enabled:active&&!privacyMode&&!trayMuted&&videoVisible&&snapshot?.state==='playing'});
  const captureAction=useRef(screenshot.save);captureAction.current=screenshot.save;
  const closeAction=useRef(onClose);closeAction.current=onClose;
  const resumeConsumed=useRef(0);
  const controls=useRef(()=>Promise.resolve());controls.current=async()=>{const lease=token.current;if(lease)await api.controls?.(lease,!!closeAction.current&&active&&!privacyMode&&videoVisible,!active||privacyMode||trayMuted||!videoVisible||snapshot?.state!=='playing'||screenshot.busy||!!screenshot.preview?.undoing);};
  useEffect(()=>{void controls.current().catch(e=>setError(errorText(e)));},[snapshot,active,privacyMode,trayMuted,videoVisible,screenshot.busy,screenshot.preview?.undoing,onClose]);
  useEffect(()=>{let gone=false,stop:(()=>void)|undefined;void listen<{token:string;action:string}>('soop-original:action',event=>{if(event.payload.token!==token.current)return;if(event.payload.action==='viewer_close')closeAction.current?.();else if(event.payload.action==='screenshot')void captureAction.current();}).then(value=>{if(gone)value();else stop=value;}).catch(()=>undefined);return()=>{gone=true;stop?.();};},[]);

  const effectiveFlattened=flattened&&!ownFullscreen;
  const split=liveSplit(size.width,size.height,chatWidth,fraction,sideBySide&&!ownFullscreen),compact=!effectiveFlattened&&split.compact;
  useEffect(()=>{const el=pair.current;if(!el)return;const measure=()=>{const r=el.getBoundingClientRect();if(r.width>0&&r.height>0)setSize(previous=>previous.width===r.width&&previous.height===r.height?previous:{width:r.width,height:r.height});};measure();const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(measure);observer?.observe(el);return()=>observer?.disconnect();},[]);
  useEffect(()=>{let gone=false,polling=false;const current=++generation.current;setSnapshot(null);setError('');
    void api.open(channelId,crypto.randomUUID()).then(value=>{if(gone){void api.close(value.token).catch(()=>undefined);return;}token.current=value.token;setSnapshot(value);}).catch(e=>{if(!gone)setError(errorText(e));});
    const timer=setInterval(()=>{const lease=token.current;if(!lease||gone||polling)return;polling=true;void api.snapshot(lease).then(value=>{if(!gone&&generation.current===current&&token.current===lease)setSnapshot(value);}).catch(e=>{if(!gone)setError(errorText(e));}).finally(()=>{polling=false;});},1500);
    return()=>{gone=true;++generation.current;clearInterval(timer);const lease=token.current;token.current=null;if(lease)void api.close(lease).catch(()=>undefined);};
  },[api,channelId,attempt]);
  useEffect(()=>{let gone=false,unlisten:(()=>void)|undefined;void listen<boolean>('platform-media:tray-muted',event=>setTrayMuted(event.payload)).then(fn=>{if(gone)fn();else unlisten=fn;}).catch(()=>undefined);return()=>{gone=true;unlisten?.();};},[]);
  useEffect(()=>{let gone=false;const stops:(()=>void)[]=[];for(const [event,handler]of [
    ['soop-original:fullscreen',(payload:{channelId:string;broadcastId:string;enabled?:boolean})=>setOwnFullscreen(payload.enabled===true)],
    ['soop-original:screenshot',()=>{void captureAction.current();}],
  ] as const){void listen<{channelId:string;broadcastId:string;enabled?:boolean}>(event,message=>{if(message.payload.channelId===channelId&&message.payload.broadcastId===snapshot?.broadcastId)handler(message.payload);}).then(stop=>{if(gone)stop();else stops.push(stop);}).catch(()=>undefined);}return()=>{gone=true;stops.forEach(stop=>stop());};},[channelId,snapshot?.broadcastId]);
  useEffect(()=>{if(!snapshot?.token)return;setOwnFullscreen(snapshot.fullscreen);},[snapshot?.token,snapshot?.fullscreen]);
  useEffect(()=>{if(ownFullscreen&&(!active||privacyMode||!videoVisible)){const lease=token.current;if(lease)void api.fullscreen(lease,false).catch(()=>undefined);setOwnFullscreen(false);}},[api,active,privacyMode,videoVisible,ownFullscreen]);
  useEffect(()=>{const lease=snapshot?.token;if(!lease)return;let previous='',disposed=false,sending=false,queued:ReturnType<typeof measureNativePlayerGeometry>|null=null;
    const send=async()=>{if(sending||disposed)return;sending=true;try{while(queued&&!disposed){const next=queued;queued=null;await api.viewport(lease,next);if(disposed||token.current!==lease)continue;await controls.current();if(next.visible&&next.player&&!trayMuted&&resumeRequest>resumeConsumed.current){resumeConsumed.current=resumeRequest;await api.resume?.(lease);}}}catch(e){if(!disposed)setError(errorText(e));}finally{sending=false;}};
    const measure=()=>{const viewport=measureNativePlayerGeometry(videoVisible?player.current:null,chat.current,{sequence:++viewportSequence.current,visible:active&&!privacyMode,muted:privacyMode||trayMuted});const key=JSON.stringify({...viewport,sequence:0});if(key===previous)return;previous=key;queued=viewport;void send();};
    const stop=observeNativePlayerGeometry([player.current,chat.current],measure);measure();return()=>{disposed=true;stop();};
  },[api,snapshot?.token,active,privacyMode,trayMuted,videoVisible,flattened,sideBySide,ownFullscreen,resumeRequest]);
  const resize=(pixels:number)=>{const next=clampLiveChat(pixels,split.min,split.max);if(compact)setFraction(split.available?next/split.available:.44);else setChatWidth(next);};
  return <div ref={pair} className={`platform-live-pair soop-original-pair has-chat${compact?' is-compact':''}${effectiveFlattened?' is-flattened':''}${!videoVisible?' chat-only':''}${ownFullscreen?' is-native-fullscreen':''}`} data-platform-native-fullscreen={ownFullscreen?'true':undefined} style={{'--platform-live-chat-size':`${size.width?split.pixels:chatWidth}px`} as CSSProperties} inert={!active||undefined}>
    <div ref={player} className="platform-live-player soop-original-slot" tabIndex={active&&!privacyMode&&videoVisible?0:-1} aria-label={privacyMode?'방송':title} aria-hidden={!videoVisible||undefined} onKeyDown={event=>{if(active&&!privacyMode&&videoVisible&&screenshotShortcut(event.nativeEvent,event.currentTarget)){event.preventDefault();event.stopPropagation();void screenshot.save();}}}>
      {privacyMode?<div className="platform-live-privacy">프라이버시 모드</div>:null}
      {(!snapshot||snapshot.state==='loading'||snapshot.state==='preparing')&&!error?<div className="platform-live-status" role="status"><span className="platform-live-spinner"/>라이브 연결 중</div>:null}
      {error||snapshot?.error?<div className="platform-live-error soop-original-notice" role="alert" data-platform-native-occlusion="true"><span>{error||snapshot?.error}</span><button onClick={()=>setAttempt(value=>value+1)}>다시 시도</button></div>:null}
      {screenshot.error?<div className="platform-live-capture-notice is-error soop-native-capture" role="alert" data-platform-native-occlusion="true"><span>{screenshot.error}</span><button type="button" aria-label="스크린샷 오류 닫기" onClick={screenshot.clearError}>×</button></div>:null}
      {screenshot.preview?<ScreenshotPreview preview={screenshot.preview} onDismiss={screenshot.dismiss} onUndo={()=>void screenshot.undo()} className="soop-native-capture"/>:null}
    </div>
    {videoVisible?<div className="platform-live-resize" role="separator" aria-label={compact?'채팅 높이':'채팅 너비'} aria-orientation={compact?'horizontal':'vertical'} aria-valuemin={Math.round(split.min)} aria-valuemax={Math.round(split.max)} aria-valuenow={Math.round(split.pixels)} tabIndex={active&&!effectiveFlattened?0:-1}
      onPointerDown={event=>{if(!active||effectiveFlattened||event.button!==0)return;event.preventDefault();event.currentTarget.focus();event.currentTarget.setPointerCapture(event.pointerId);}}
      onPointerMove={event=>{if(active&&!effectiveFlattened&&event.currentTarget.hasPointerCapture(event.pointerId)){const r=pair.current?.getBoundingClientRect();if(r)resize((compact?r.bottom-event.clientY:r.right-event.clientX)-LIVE_SPLITTER_SIZE/2);}}}
      onPointerUp={event=>{if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);}}
      onPointerCancel={event=>{if(event.currentTarget.hasPointerCapture(event.pointerId))event.currentTarget.releasePointerCapture(event.pointerId);}}
      onKeyDown={event=>{if(!active||effectiveFlattened||event.altKey||event.ctrlKey||event.metaKey)return;const next=liveSplitKey(event.key,split.pixels,split.min,split.max,compact);if(next!==null){event.preventDefault();resize(next);}}}/>:null}
    <div ref={chat} className="platform-live-chat soop-original-chat" aria-label="SOOP 원본 채팅">{privacyMode?<span>프라이버시 모드</span>:null}</div>
  </div>;
}
