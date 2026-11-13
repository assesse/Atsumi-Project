// Thin interaction layer over X's own video and controls; never a second player.
(() => {
  const config=__ATSUMI_X_VIDEO__;
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname))return;
  const api=()=>window.__atsumiXSurface;
  let probes=0;
  const probe=phase=>{if(config.diagnostics===true&&probes++<40)window.chrome?.webview?.postMessage('ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/'+config.nonce+'?kind=videoProbe&value='+phase);};
  const viewer=()=>/\/status\/[1-9]\d{4,24}\/(?:mediaviewer|mediaViewer|video\/[1-4])$/.test(location.pathname);
  const immersive=()=>viewer()||!!document.fullscreenElement;
  const controls='button,[role="button"],[role="slider"],input,textarea,select,[contenteditable="true"],[role="menu"],[role="listbox"],[data-atsumi-x-action]';
  let lastVideo=null, timer=0;
  const presented=node=>{
    if(!node?.isConnected||node.ownerDocument!==document||node.closest('[hidden],[aria-hidden="true"],[inert]')||window.__atsumiXAds?.blocked(node))return false;
    for(let n=node,depth=0;n&&depth<64;n=n.parentElement,depth++){
      const s=getComputedStyle(n);if(s.display==='none'||s.visibility==='hidden'||s.visibility==='collapse'||s.opacity==='0')return false;
    }
    const b=node.getBoundingClientRect();return b.width>40&&b.height>30&&b.bottom>0&&b.right>0&&b.top<innerHeight&&b.left<innerWidth;
  };
  const scrubber=root=>root.querySelector('[data-testid="scrubber"]')||[...root.querySelectorAll('[role="slider"],input[type="range"]')].find(n=>/^(?:scrubber|seek|seek bar|스크러버|탐색|シークバー)$/i.test(label(n)));
  const focusVideo=video=>{if(!video.hasAttribute('tabindex'))video.setAttribute('tabindex','-1');video.focus({preventScroll:true});};
  let entryFocusUntil=0, entryControl=null;
  const player=node=>{
    const inline=node?.closest('[data-testid="videoPlayer"]');if(!immersive())return inline||null;
    // X unmounts the immersive controls while idle. Video identity must not
    // depend on the scrubber/play button existing at the instant of a click.
    let mediaRoot=null;
    for(let root=node,depth=0;root&&root!==document.body&&depth<64;root=root.parentElement,depth++){
      const count=root.querySelectorAll('video').length;
      if(count>1)break; // Never bind a related-video list to its first controls.
      if(count===1){
        mediaRoot=root;
        if(scrubber(root)&&[...root.querySelectorAll('button,[role="button"]')].some(b=>/^(?:play|pause|play video|pause video|플레이하기|재생|일시\s?정지|再生|一時停止)$/i.test(label(b))))return root;
      }
    }
    return mediaRoot;
  };
  const videos=root=>[...root.querySelectorAll('video')].slice(0,32).filter(presented);
  const pick=target=>{
    if(!immersive()){const root=player(target),found=root? videos(root):[];return found.length===1?found[0]:null;}
    const scope=document.fullscreenElement||[...document.querySelectorAll('[role="dialog"],[data-testid="modalContainer"]')].filter(presented).at(-1)||document;
    // Background feed players remain mounted and can retain keyboard focus.
    // Prefer the actual native viewer cards, never the target's old player.
    const native=viewer()?[...scope.querySelectorAll('[data-testid^="immersive-video-controls-"]')].slice(0,32)
      .filter(n=>/^immersive-video-controls-[1-9]\d{4,24}$/.test(n.getAttribute('data-testid')||''))
      .flatMap(n=>{const root=player(n);return root?videos(root):[];}):[];
    const candidates=native.length?[...new Set(native)]:videos(scope);
    const found=candidates.map(video=>{const b=video.getBoundingClientRect();return{video,area:Math.max(0,Math.min(b.bottom,innerHeight)-Math.max(0,b.top))*Math.max(0,Math.min(b.right,innerWidth)-Math.max(0,b.left))};}).sort((a,b)=>b.area-a.area);
    return found[0]?.video||null;
  };
  const label=node=>(node.getAttribute('aria-label')||node.getAttribute('title')||'').trim();
  const fullButton=target=>{
    const button=target?.closest('button,[role="button"]');if(!button||!player(button))return null;
    return /^(?:full\s?screen|enter full\s?screen|전체\s?화면(?:으로 보기)?|フルスクリーン|全画面表示)$/i.test(label(button))?button:null;
  };
  const ownControl=target=>!!target?.closest(controls+',a[href],[data-testid="scrubber"]');
  const background=(target,event)=>{
    if(!target||api()?.videoSourceTarget?.(target))return null;
    if(!player(target)){if(immersive())probe('click-no-root');return null;}
    // The whole video container can itself have button semantics. Never treat
    // nested source controls, links, attribution, scrubbers or menus as video.
    const control=target.closest(controls+',a[href],[data-testid="scrubber"]');
    const chosen=pick(target);
    if(control && (!chosen || !control.contains(chosen))){if(immersive())probe('click-control');return null;}
    const component=target.closest('[data-testid="videoComponent"]');
    if(!component&&!target.matches('video')){
      if(!immersive()||!chosen||!player(chosen)?.contains(target))return null;
      const box=chosen.getBoundingClientRect();
      if(!Number.isFinite(event?.clientX)||!Number.isFinite(event?.clientY)||
        event.clientX<box.left||event.clientX>box.right||event.clientY<box.top||event.clientY>box.bottom)return null;
    }
    return chosen;
  };
  const stop=event=>{event.preventDefault();event.stopImmediatePropagation();};
  const command=(video,kind)=>{
    const root=player(video);
    const match=kind==='play'?/^(?:play|play video|플레이하기|재생|동영상 재생|再生)$/i:/^(?:pause|pause video|일시\s?정지|동영상 일시\s?정지|一時停止)$/i;
    const button=root&&[...root.querySelectorAll('button,[role="button"]')].find(b=>match.test(label(b)));
    if(button){button.click();return;}
    if(kind==='play'){if(video.ended)video.currentTime=0;const result=video.play();result?.catch(()=>{});}else video.pause();
  };
  const toggle=video=>command(video,video.paused||video.ended?'play':'pause');
  function markControls(video){
    const root=player(video);if(!root)return;
    // Public source scrubber and its control bar only. Never fade captions,
    // the video, source/profile links, social controls or related-video cards.
    let node=scrubber(root);
    for(let depth=0;node&&node!==root&&depth<8;depth++,node=node.parentElement){
      if(node.querySelector('video'))break;
      if([...node.querySelectorAll('button,[role="button"]')].some(b=>/^(?:play|pause|play video|pause video|플레이하기|재생|일시\s?정지|再生|一時停止)$/i.test(label(b)))){node.setAttribute('data-atsumi-video-controls','');break;}
    }
    root.setAttribute('data-atsumi-video-interaction','');
  }
  const hudRequests=new WeakMap();
  function sourceHud(video,show){
    if(!viewer())return false;
    const root=player(video);if(!root||root.querySelectorAll('video').length!==1)return false;
    // Verified original immersive overlay: its sole click action toggles the
    // native HUD. It never navigates, reacts, or changes player/account state.
    const overlays=[...root.querySelectorAll('[data-testid^="immersive-video-controls-"]')];
    if(overlays.length!==1)return false;
    const overlay=overlays[0],id=overlay.getAttribute('data-testid').slice('immersive-video-controls-'.length);
    if(!/^[1-9]\d{4,24}$/.test(id)||overlay.querySelector('button,a,video,input,[role="button"]'))return false;
    const hud=root.querySelector('[data-testid="immersive-tweet-ui-content-container-'+id+'"]');
    let visible=!!hud;
    for(let n=hud;visible&&n&&n!==root;n=n.parentElement){
      const style=getComputedStyle(n);
      if(n.hidden||n.hasAttribute('inert')||n.getAttribute('aria-hidden')==='true'||style.display==='none'||style.visibility==='hidden'||style.opacity==='0')visible=false;
    }
    if(visible===show)return true;
    const now=performance.now(),pending=hudRequests.get(overlay);
    if(pending&&now-pending.at<350)return true; // React fade/mount is asynchronous.
    hudRequests.set(overlay,{at:now});
    overlay.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,button:0}));
    return true;
  }
  let muteRequest=null,muteTimer=0;
  const cancelMute=()=>{clearTimeout(muteTimer);muteRequest=null;};
  function toggleMute(video){
    // X unmounts its HUD while idle. Let the original mute control mount so
    // the source's player context, icon and next related clip stay in sync.
    if(muteRequest?.video===video&&muteRequest.path===location.href){muteRequest.odd=!muteRequest.odd;return;}
    cancelMute();
    const request={video,path:location.href,odd:true,attempts:0};muteRequest=request;
    const attempt=()=>{
      if(muteRequest!==request)return;
      if(!api()?.videoInteractionReady?.()||!immersive()||location.href!==request.path||!presented(video)||pick(null)!==video){cancelMute();return;}
      const root=player(video),button=root&&[...root.querySelectorAll('button,[role="button"]')].find(b=>/^(?:mute|unmute|뮤트|언뮤트|음소거|음소거 해제|ミュート|ミュート解除)$/i.test(label(b)));
      if(button){const odd=request.odd;cancelMute();if(odd)button.click();return;}
      if(!sourceHud(video,true)||request.attempts++>=10){const odd=request.odd;cancelMute();if(odd)video.muted=!video.muted;return;}
      muteTimer=setTimeout(attempt,40);
    };
    attempt();
  }
  function wake(video){
    if(!video||!immersive())return;lastVideo=video;sourceHud(video,true);markControls(video);
    const root=player(video);if(!root)return;
    root.setAttribute('data-atsumi-video-awake','');
    clearTimeout(timer);timer=setTimeout(()=>{
      if(lastVideo!==video||!immersive())return;
      const bar=root.querySelector('[data-atsumi-video-controls]');
      if(bar?.matches(':hover')||bar?.contains(document.activeElement)){wake(video);return;}
      root.removeAttribute('data-atsumi-video-awake');sourceHud(video,false);
    },2400);
  }
  const can=event=>event.isTrusted&&!event.defaultPrevented&&!event.ctrlKey&&!event.altKey&&!event.metaKey&&!event.shiftKey&&api()?.videoInteractionReady?.();
  // Native immersive X puts role=slider on its small thumb, while the
  // painted rail is an ancestor. Inline X can instead label the entire rail.
  // Resolve only the thumb's narrow, single-slider control; never the video,
  // a whole toolbar, another slider, or a guessed strip of screen coordinates.
  const seekTarget=event=>{
    if(!immersive()||!(event.target instanceof Element))return null;
    const video=pick(event.target),root=video&&player(video),slider=root&&scrubber(root);
    if(!slider||!slider.isConnected||slider.getAttribute('aria-disabled')==='true'||slider.disabled
      ||slider.closest('[hidden],[inert],[aria-hidden="true"]'))return null;
    const thumbBox=slider.getBoundingClientRect();
    let track=slider,box=thumbBox,thumb=false;
    if(box.width<=40){
      thumb=true;track=null;
      for(let node=slider.parentElement,depth=0;node&&node!==root&&depth<5;node=node.parentElement,depth++){
        if(node.matches('button,a,input,textarea,select')||node.querySelector('video,button,a,input:not([type="range"]),textarea,select'))break;
        const others=[...node.querySelectorAll('[role="slider"],input[type="range"]')].filter(n=>n!==slider&&!slider.contains(n));
        if(others.length)break;
        const b=node.getBoundingClientRect();
        if(b.height>48)break;
        if(b.width>40&&b.height>0&&thumbBox.left+thumbBox.width/2>=b.left-1&&thumbBox.left+thumbBox.width/2<=b.right+1
          &&thumbBox.top+thumbBox.height/2>=b.top-1&&thumbBox.top+thumbBox.height/2<=b.bottom+1){track=node;box=b;break;}
      }
      if(!track)return null;
    }
    // The rail can be only a few pixels tall. Its native thumb is the
    // visible hit-height reference, including when the pointer hits the
    // transparent player overlay immediately above/below the painted line.
    const thumbHeight=thumb&&thumbBox.height>0&&thumbBox.height<=48;
    const top=thumbHeight?Math.min(box.top,thumbBox.top):box.top;
    const bottom=thumbHeight?Math.max(box.bottom,thumbBox.bottom):box.bottom;
    if(!track.contains(event.target)&&!event.target.contains(track)&&(!thumbHeight||!root.contains(event.target)))return null;
    const control=event.target.closest(controls+',a[href]');
    if(control&&!control.contains(slider)&&!slider.contains(control))return null;
    for(let node=slider;node&&node!==root;node=node.parentElement){
      const style=getComputedStyle(node);if(style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse'||style.opacity==='0')return null;
    }
    if(!(box.width>40&&box.height>0)||!Number.isFinite(event.clientX)||!Number.isFinite(event.clientY)
      ||event.clientX<box.left||event.clientX>box.right||event.clientY<top||event.clientY>bottom)return null;
    return {video,track,box,onThumb:thumb&&slider.contains(event.target)};
  };
  let scrubGesture=null;
  document.addEventListener('pointerdown',event=>{
    scrubGesture=null;
    if(!can(event)||event.button!==0)return;
    const hit=seekTarget(event);
    if(hit)scrubGesture={...hit,path:location.href,x:event.clientX,y:event.clientY,at:performance.now(),dragged:false};
  },true);
  document.addEventListener('pointermove',event=>{
    if(scrubGesture&&event.isTrusted&&Math.hypot(event.clientX-scrubGesture.x,event.clientY-scrubGesture.y)>5)scrubGesture.dragged=true;
  },true);
  document.addEventListener('pointercancel',()=>{if(scrubGesture)scrubGesture.dragged=true;},true);
  function seekClick(event){
    const hit=seekTarget(event),gesture=scrubGesture;scrubGesture=null;
    if(!hit)return false;
    if(hit.onThumb)return true; // Native handle interaction owns its full lifecycle.
    if(gesture&&(gesture.dragged||gesture.video!==hit.video||gesture.track!==hit.track||gesture.path!==location.href||performance.now()-gesture.at>1500))return true;
    if(!Number.isFinite(hit.video.duration)||hit.video.duration<=0)return true;
    if(!api()?.videoInput(event,hit.video))return true;
    const fraction=Math.max(0,Math.min(1,(event.clientX-hit.box.left)/hit.box.width));
    hit.video.currentTime=fraction*hit.video.duration;
    stop(event);wake(hit.video);return true;
  }
  // Hover is a bounded audio intent, not a new player or autoplay request.
  // Native pane arbitration still decides which webview is allowed to sound.
  let hover=null;
  const inlineVideo=target=>{
    if(immersive()||!(target instanceof Element))return null;
    const root=target.closest('[data-testid="videoPlayer"]');
    if(!root)return null;
    const found=videos(root);return found.length===1?found[0]:null;
  };
  const hoverCurrent=()=>hover&&hover.path===location.href&&hover.source===(hover.video.currentSrc||hover.video.src)
    &&hover.video.isConnected&&presented(hover.video)&&!immersive()&&api()?.videoInteractionReady?.();
  function releaseHover(restore=true){
    const previous=hover;hover=null;
    if(restore&&previous?.claimed)previous.video.muted=previous.muted;
  }
  function soundHover(){
    if(!hoverCurrent()){releaseHover();return;}
    if(hover.claimed||hover.manual||hover.video.paused||hover.video.ended)return;
    if(!api()?.videoInput(hover.event,hover.video))return;
    hover.claimed=true;hover.muted=hover.video.muted;hover.video.muted=false;
  }
  const hoverMove=event=>{
    if(!can(event)||event.pointerType==='touch')return;
    const video=inlineVideo(event.target);
    if(video===hover?.video&&hoverCurrent())return;
    releaseHover();
    if(video){hover={video,event,path:location.href,source:video.currentSrc||video.src,claimed:false,manual:false,muted:video.muted};soundHover();}
  };
  document.addEventListener('pointerover',hoverMove,{passive:true});
  document.addEventListener('pointermove',hoverMove,{passive:true});
  document.addEventListener('pointerout',event=>{
    if(event.isTrusted&&hover&&inlineVideo(event.relatedTarget)!==hover.video)releaseHover();
  },{passive:true});
  document.addEventListener('play',event=>{if(event.isTrusted&&event.target===hover?.video)soundHover();},true);
  document.addEventListener('pointerdown',event=>{
    if(!can(event)||event.button!==0||!hover)return;
    const target=event.target instanceof Element?event.target.closest('button,[role="button"],[role="slider"],input[type="range"]'):null;
    if(target&&player(hover.video)?.contains(target)&&/^(?:mute|unmute|volume|뮤트|언뮤트|음소거(?: 해제)?|볼륨|ミュート(?:解除)?|音量)$/i.test(label(target))){
      hover.claimed=false;hover.manual=true; // Explicit source controls now own mute.
    }
  },true);
  window.addEventListener('blur',()=>releaseHover());
  document.addEventListener('visibilitychange',()=>{if(document.hidden)releaseHover();});
  // Prevent the native background's mousedown/up route handler from running.
  // Pointer events still reach the original hover/playback coordinator.
  for(const type of ['mousedown','mouseup'])document.addEventListener(type,event=>{
    if(!can(event)||event.button!==0)return;
    if(type==='mousedown'&&!immersive()){
      const full=fullButton(event.target instanceof Element?event.target:null);
      if(full){entryControl=full;entryFocusUntil=performance.now()+2000;return;}
    }
    if(seekTarget(event)||scrubGesture&&scrubGesture.path===location.href&&scrubGesture.video===pick(event.target))return;
    const video=background(event.target,event);if(video)stop(event);
  },true);
  document.addEventListener('click',event=>{
    if(!can(event)||event.button!==0)return;
    if(seekClick(event))return;
    const target=event.target instanceof Element?event.target:null;
    const full=fullButton(target),video=full?pick(full):background(target,event);
    if(!video){if(immersive())probe('click-no-video');return;}
    if(full){
      // Move the original background entry point to the explicit fullscreen
      // action, retaining X's own route/context and related-video controller.
      if(!immersive()&&api()?.sourceMediaViewerAvailable?.()){
        stop(event);if(!api().videoInput(event,video))return;
        entryFocusUntil=performance.now()+2000;entryControl=full;
        const root=player(video),box=video.getBoundingClientRect();
        const hit=document.elementFromPoint(Math.max(1,Math.min(innerWidth-1,box.left+box.width/2)),Math.max(1,Math.min(innerHeight-1,box.top+box.height/2)));
        if(!hit||!root.contains(hit)||ownControl(hit)&&!hit.closest(controls)?.contains(video)&&hit!==root){full.click();return;}
        const path=location.pathname,options={bubbles:true,cancelable:true,button:0,clientX:box.left+box.width/2,clientY:box.top+box.height/2};
        hit.dispatchEvent(new MouseEvent('mousedown',options));
        hit.dispatchEvent(new MouseEvent('mouseup',options));
        if(location.pathname===path)hit.dispatchEvent(new MouseEvent('click',options));
        return;
      }
      return; // Original browser fullscreen remains available if source drifts.
    }
    stop(event);
    if(api()?.videoInput(event,video)){probe('click-accepted');focusVideo(video);toggle(video);wake(video);}else probe('click-denied');
  },true);
  window.addEventListener('keydown',event=>{
    if(!can(event)){if(event.isTrusted)probe('key-unready');return;}
    if(event.isComposing||!immersive()){probe('key-not-viewer');return;}
    const target=event.target instanceof Element?event.target:null;
    if(target?.closest('input,textarea,select,[contenteditable="true"],[role="slider"],[role="menu"],[role="listbox"]'))return;

    const physical={KeyA:'a',KeyD:'d',KeyM:'m',KeyK:'k'};
    const key=physical[event.code] || event.key.toLowerCase();
    if(!['a','d','arrowleft','arrowright',' ','k','m','arrowup','arrowdown'].includes(key))return;
    const video=pick(target);if(!video){probe('key-no-video');return;}
    const button=target?.closest('button,a[href],[role="button"]');
    // The original entry button can retain focus behind X's new viewer.
    // Preserve real focused controls, but never reactivate that stale entry.
    const staleEntry=viewer()&&button&&(button===entryControl||
      !!button.closest('[data-testid="videoPlayer"]')&&!player(video)?.contains(button)&&
      /^(?:full\s?screen|enter full\s?screen|전체\s?화면(?:으로 보기)?|フルスクリーン|全画面表示)$/i.test(label(button)));
    // The native dialog may autofocus its back button on entry. Space is a
    // playback shortcut there; Enter still activates that explicit control.
    const nativeBack=viewer()&&button&&/^immersive-tweet-back-button-[1-9]\d{4,24}$/.test(button.getAttribute('data-testid')||'');
    if((event.key===' '||event.key==='Enter')&&button&&!button.contains(video)&&!staleEntry&&!nativeBack){
      probe(/full|전체|全画面/i.test(label(button))?'key-control-full':/back|close|뒤로|닫기/i.test(label(button))?'key-control-back':button.matches('a')?'key-control-link':'key-control-other');return;
    }
    if(staleEntry||nativeBack)focusVideo(video);
    if(!api()?.videoInput(event,video)){probe('key-denied');return;}
    probe('key-accepted');
    stop(event);
    if(['a','d','arrowleft','arrowright'].includes(key)){
      if(Number.isFinite(video.duration)&&video.duration>0)video.currentTime=Math.max(0,Math.min(video.duration,video.currentTime+(['a','arrowleft'].includes(key)?-5:5)));
    }else if([' ','k'].includes(key)){if(!event.repeat)toggle(video);}
    else if(key==='m'){
      if(event.repeat)return;
      toggleMute(video);
    }
    else{video.volume=Math.max(0,Math.min(1,video.volume+(key==='arrowup'?.05:-.05)));}
    wake(video);
  },true);
  document.addEventListener('mousemove',event=>{if(event.isTrusted&&api()?.videoInteractionReady?.()&&immersive())wake(pick(event.target));},{passive:true});
  window.__atsumiXVideo=Object.freeze({update(ready){
    if(!ready){releaseHover();scrubGesture=null;clearTimeout(timer);cancelMute();lastVideo=null;return;}
    if(hover&&!hoverCurrent())releaseHover();
    if(!immersive()) for(const root of [...document.querySelectorAll('[data-testid="videoPlayer"]')].slice(0,32)){
      const box=root.getBoundingClientRect();
      for(const button of root.querySelectorAll('button,[role="button"]')){
        if(!/^(?:play|pause|play video|pause video|재생|플레이하기|일시\s?정지|再生|一時停止)$/i.test(label(button)))continue;
        const b=button.getBoundingClientRect();
        if(b.width>=8&&b.width<=32&&b.height>=8&&b.height<=32&&box.bottom-b.bottom>=0&&box.bottom-b.bottom<=40)
          button.setAttribute('data-atsumi-video-idle-play','');
      }
    }
    if(!document.getElementById('atsumi-video-controls-style')){
      const style=document.createElement('style');style.id='atsumi-video-controls-style';
      style.textContent='body:not([data-atsumi-x-video-viewer]) [data-testid="videoPlayer"]:not(:hover):not(:focus-within) [data-atsumi-video-idle-play]{display:none!important}body[data-atsumi-x-video-viewer] [data-atsumi-video-interaction] [data-atsumi-video-controls]{transition:opacity .18s ease;opacity:0!important;pointer-events:none!important}body[data-atsumi-x-video-viewer] [data-atsumi-video-interaction][data-atsumi-video-awake] [data-atsumi-video-controls],body[data-atsumi-x-video-viewer] [data-atsumi-video-controls]:focus-within{opacity:1!important;pointer-events:auto!important}@media(prefers-reduced-motion:reduce){[data-atsumi-video-controls]{transition:none!important}}';
      document.head.append(style);
    }
    if(immersive()){const video=pick(null);if(video&&video!==lastVideo)wake(video);
      if(video&&entryFocusUntil>performance.now()&&document.hasFocus()){entryFocusUntil=0;focusVideo(video);}
    }
    else{clearTimeout(timer);lastVideo=null;}
  }});
})();
