import {useEffect,useRef,useState} from "react";
import Hls from "hls.js";
import {type XApi,type XPost,xError} from "./api";
import {useXImage} from "./XMedia";
import {useCenteredVideo} from "./videoFocus";

type Props={api:XApi;post:XPost;generation:number;active:boolean;autoPlayAllowed?:boolean;priority?:number};
const time=(value:number)=>{
  const n=Number.isFinite(value)?Math.max(0,value):0;
  return Math.floor(n/60)+":"+String(Math.floor(n%60)).padStart(2,'0');
};
const localVideo=/^http:\/\/x-video\.localhost\/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;

// A different account/post never inherits old intent or pending source work.
export function XVideo(props:Props){return <XVideoPlayer key={props.generation+':'+props.post.id} {...props}/>;}

function XVideoPlayer({api,post,generation,active,autoPlayAllowed=true,priority=0}:Props){
  const posterIndex=post.media.findIndex(u=>/_video_thumb\//.test(u));
  const poster=useXImage<HTMLDivElement>({api,post,generation,active},posterIndex<0?post.media.length:posterIndex);
  const video=useRef<HTMLVideoElement>(null),intent=useRef(false),hlsCurrent=useRef<Hls|null>(null),failCurrent=useRef<((message:string)=>void)|null>(null);
  const [manual,setManual]=useState<'auto'|'play'|'pause'>('auto');
  const [started,setStarted]=useState(false),[attempt,setAttempt]=useState(0),[playSignal,setPlaySignal]=useState(0);
  // Removing a user-paused centered card would immediately start its neighbor.
  const centered=useCenteredVideo(poster.ref,active&&autoPlayAllowed,priority+(manual==='play'?10:0),()=>video.current?.pause());
  const [error,setError]=useState<string|null>(null),[loading,setLoading]=useState(false),[sourceReady,setSourceReady]=useState(false);
  const [paused,setPaused]=useState(true),[muted,setMuted]=useState(true),[volume,setVolume]=useState(1);
  const [position,setPosition]=useState(0),[duration,setDuration]=useState(0),[fullscreen,setFullscreen]=useState(false);
  const wantsPlay=active&&autoPlayAllowed&&manual!=='pause'&&centered&&!error;
  intent.current=wantsPlay;

  useEffect(()=>{
    if(wantsPlay)setStarted(true);
    else if(!sourceReady||!active){setStarted(false);setLoading(false);if(!active)setSourceReady(false);}
  },[wantsPlay,sourceReady,active]);

  useEffect(()=>{
    if(!started||!active||error||!video.current)return;
    const element=video.current,requestId=crypto.randomUUID();
    let closed=false,cancelled=false,hls:Hls|undefined;
    setLoading(true);setError(null);setSourceReady(false);setPosition(0);setDuration(0);
    const cancel=()=>{if(!cancelled){cancelled=true;void api.cancel?.(requestId).catch(()=>{});}};
    const stop=()=>{if(hlsCurrent.current===hls)hlsCurrent.current=null;hls?.destroy();hls=undefined;element.pause();element.removeAttribute('src');element.load();};
    const fail=(message:string)=>{
      if(closed)return;
      closed=true;intent.current=false;setPaused(true);clearTimeout(timeout);cancel();stop();setLoading(false);setSourceReady(false);setError(message);
    };
    const timeout=setTimeout(()=>fail('동영상 응답이 지연되었습니다. 다시 시도해 주세요.'),35000);
    failCurrent.current=fail;
    const request=api.video?api.video(post.id,generation,requestId):Promise.reject(new Error('동영상은 데스크톱 앱에서 재생할 수 있습니다.'));
    void request.then(result=>{
      if(closed)return;
      if(!localVideo.test(result.url))throw new Error('동영상 응답 주소를 검증하지 못했습니다.');
      if(result.hls&&Hls.isSupported()){
        const player=new Hls({autoStartLoad:false,maxBufferLength:20,maxMaxBufferLength:35,backBufferLength:10,enableWorker:true,manifestLoadingMaxRetry:1,fragLoadingMaxRetry:1});
        hls=player;hlsCurrent.current=player;
        player.on(Hls.Events.ERROR,(_,data)=>{if(data.fatal)fail('동영상 데이터를 재생하지 못했습니다. 다시 시도해 주세요.');});
        player.on(Hls.Events.MANIFEST_PARSED,()=>{if(!closed){clearTimeout(timeout);setSourceReady(true);}});
        player.loadSource(result.url);player.attachMedia(element);
      }else if(!result.hls||element.canPlayType('application/vnd.apple.mpegurl')){
        clearTimeout(timeout);element.src=result.url;setSourceReady(true);
      }else fail('이 환경에서 이 동영상 형식을 재생할 수 없습니다.');
    }).catch(cause=>fail(xError(cause)));
    return()=>{
      closed=true;clearTimeout(timeout);cancel();stop();
      if(failCurrent.current===fail)failCurrent.current=null;
    };
  },[api,post.id,generation,started,active,attempt,error]);

  useEffect(()=>{
    const element=video.current;
    if(!element)return;
    let revoked=false;
    if(!wantsPlay||error){setLoading(false);hlsCurrent.current?.stopLoad();element.pause();return;}
    if(sourceReady){
      hlsCurrent.current?.startLoad(-1);
      setLoading(true);
      void element.play().then(()=>{
        if(revoked){if(!intent.current)element.pause();return;}
        if(!intent.current){element.pause();return;}
        setLoading(false);
      }).catch(cause=>{
        if(revoked||!intent.current)return;
        if(cause instanceof Error&&cause.name==='NotAllowedError'){
          setLoading(false);setManual('pause');setPaused(true);
        }else failCurrent.current?.('동영상을 재생하지 못했습니다. 다시 시도해 주세요.');
      });
    }
    return()=>{revoked=true;hlsCurrent.current?.stopLoad();element.pause();};
  },[wantsPlay,sourceReady,error,playSignal]);

  // canplay does not prove play() resolved or frames are advancing.
  useEffect(()=>{
    if(!loading||!sourceReady||!wantsPlay)return;
    const timeout=setTimeout(()=>failCurrent.current?.('영상 재생이 지연되었습니다. 다시 시도해 주세요.'),15000);
    return()=>clearTimeout(timeout);
  },[loading,sourceReady,wantsPlay]);

  useEffect(()=>{
    const update=()=>setFullscreen(document.fullscreenElement===poster.ref.current);
    document.addEventListener('fullscreenchange',update);
    return()=>document.removeEventListener('fullscreenchange',update);
  },[poster.ref]);

  const playingIntent=!error&&wantsPlay&&(loading||!paused);
  const play=()=>{if(error)return;setManual('play');setStarted(true);setPlaySignal(value=>value+1);};
  const pause=()=>{setManual('pause');setLoading(false);setPaused(true);video.current?.pause();};
  const toggle=()=>{if(playingIntent)pause();else play();};
  const toggleMute=()=>{
    if(muted&&volume===0){setVolume(1);if(video.current)video.current.volume=1;}
    setMuted(value=>!value);
  };
  const seek=(value:number)=>{
    if(!sourceReady||!video.current||!Number.isFinite(duration)||duration<=0)return;
    const next=Math.max(0,Math.min(duration,value));
    try{video.current.currentTime=next;setPosition(next);}catch{/* An expired source may stop accepting seeks. */}
  };
  const toggleFullscreen=()=>{
    const element=poster.ref.current;
    if(!element)return;
    void (document.fullscreenElement===element?document.exitFullscreen():element.requestFullscreen())?.catch(()=>{});
  };
  const playbackEvent=(ready:boolean)=>{
    if(!intent.current){video.current?.pause();setPaused(true);return;}
    setPaused(false);
    // play signals intent, not readiness; keep the deadline until playing.
    if(ready)setLoading(false);
  };
  const finiteDuration=Number.isFinite(duration)&&duration>0?duration:0;

  return <div className="x-inline-video" ref={poster.ref} role="group" aria-label={post.gif?'게시물 GIF':'게시물 동영상'}>
    {/* Own controls and alerts remain accessible; an unloaded native media node announces a false error. */}
    <video ref={video} poster={poster.data??undefined} playsInline preload="none" muted={muted} loop={!!post.gif} aria-hidden="true"
      onClick={toggle} onDoubleClick={toggleFullscreen} onPlay={()=>playbackEvent(false)} onPlaying={()=>playbackEvent(true)} onPause={()=>setPaused(true)}
      onLoadedMetadata={event=>setDuration(event.currentTarget.duration)} onDurationChange={event=>setDuration(event.currentTarget.duration)}
      onTimeUpdate={event=>setPosition(event.currentTarget.currentTime)} onWaiting={()=>{if(intent.current)setLoading(true);}}
      onEnded={()=>{if(!post.gif)pause();}} onError={()=>failCurrent.current?.('동영상을 재생하지 못했습니다. 다시 시도해 주세요.')}/>
    {!started&&!error?<button type="button" className="x-video-overlay" onClick={play}>동영상 재생</button>:wantsPlay&&loading&&!error?<span className="x-video-overlay" role="status">동영상 불러오는 중</span>:null}
    {error?<div className="x-video-overlay" role="alert"><span>{error}</span><button type="button" onClick={()=>{setError(null);setManual('play');setStarted(true);setAttempt(value=>value+1);}}>다시 시도</button></div>:null}
    <div className="x-video-controls" role="group" aria-label="동영상 조작">
      <button type="button" aria-label={playingIntent?'일시 정지':'재생'} disabled={!!error} onClick={toggle}><svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d={playingIntent?'M6 4h4v16H6zM14 4h4v16h-4z':'M7 4v16l13-8Z'}/></svg></button>
      <input type="range" aria-label="재생 위치" min="0" max={finiteDuration} step="0.1" value={Math.max(0,Math.min(position,finiteDuration))} disabled={!sourceReady||finiteDuration===0} onChange={event=>seek(Number(event.target.value))}/>
      <span className="x-video-time">{time(position)} / {time(duration)}</span>
      <button type="button" aria-label={muted?'소리 켜기':'음소거'} aria-pressed={muted} onClick={toggleMute}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true"><path d="M3 9h4l5-5v16l-5-5H3Z"/><path d={muted?'m16 9 6 6m0-6-6 6':'M16 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14'}/></svg></button>
      <input className="x-volume" type="range" aria-label="음량" min="0" max="1" step="0.05" value={muted?0:volume} onChange={event=>{const value=Number(event.target.value);setVolume(value);setMuted(value===0);if(video.current)video.current.volume=value;}}/>
      <button type="button" aria-label={fullscreen?'동영상 전체 화면 해제':'동영상 전체 화면'} onClick={toggleFullscreen}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true"><path d="M3 9V3h6m6 0h6v6m0 6v6h-6m-6 0H3v-6"/></svg></button>
    </div>
  </div>;
}
