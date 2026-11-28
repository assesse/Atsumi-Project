// Watch-only YouTube document. No recorder, chat receiver, account inputs or media extraction.
(() => {
  'use strict';
  if (window.top !== window || location.origin !== 'https://www.youtube.com' || location.pathname !== '/watch' || window.__atsumiYouTubeVod) return;
  const videoId = new URLSearchParams(location.search).get('v');
  if (!/^[A-Za-z0-9_-]{11}$/.test(videoId ?? '')) return;
  let epoch = 0, layout = null, layoutKey = '', fullscreen = false;
  let endedCancelled = false, observedPlayer = null, endscreenObserver = null, pageGone = false;
  let media = null, mediaSource = '', sourceId = crypto.randomUUID(), sourceGeneration = 1, sequence = 0, seekGeneration = 1;
  let lastSeeking = false, waiting = false, phase = 'unknown', stableSince = 0, closeEnabled = false, closeFrame = 0, closeUntil = 0;
  const notify = (type, fields = {}) => {
    if (!epoch) return;
    try { window.chrome.webview.postMessage('ATSUMI_YOUTUBE_VOD_V1:' + JSON.stringify({ id: crypto.randomUUID(), epoch, type, ...fields })); } catch {}
  };
  function apply() {
    if (!layout || !document.documentElement) return;
    if (!document.getElementById('atsumi-youtube-vod-layout')) {
      const style = document.createElement('style'); style.id = 'atsumi-youtube-vod-layout';
      // Preserve the original VOD player/menus, not the unrelated watch page.
      // Visibility keeps site layout alive without recreating the receiver.
      const watchSiblings = `:is(ytd-watch-flexy #secondary,ytd-watch-flexy #chat,ytd-watch-flexy #below,ytd-watch-flexy ytd-watch-metadata,ytd-masthead):not(:has(#movie_player))`;
      style.textContent = `html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#000!important}${watchSiblings},${watchSiblings} *{visibility:hidden!important;pointer-events:none!important}ytd-app,ytd-page-manager,ytd-watch-flexy,#columns,#primary,#primary-inner,#player-container-outer,#player-container-inner,ytd-player{transform:none!important;contain:none!important;overflow:visible!important}#movie_player{position:fixed!important;left:var(--atsumi-vod-x)!important;top:var(--atsumi-vod-y)!important;width:var(--atsumi-vod-w)!important;height:var(--atsumi-vod-h)!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;background:#000!important;z-index:2147483000!important}#movie_player .html5-video-container{width:100%!important;height:100%!important}#movie_player video.html5-main-video{width:100%!important;height:100%!important;left:0!important;top:0!important;object-fit:contain!important}#movie_player .ytp-chrome-bottom{width:calc(100% - 24px)!important;left:12px!important}#movie_player .ytp-popup{max-width:calc(100% - 24px)!important;max-height:calc(100% - 56px)!important}ytd-popup-container,tp-yt-iron-dropdown,ytd-menu-popup-renderer{z-index:2147483100!important}`;
      style.textContent += `#movie_player [data-atsumi-vod-restart]:hover{border-color:#ffffff8a!important;background:#303336!important}#movie_player [data-atsumi-vod-restart]:focus-visible{outline:2px solid #fff;outline-offset:3px}`;
      (document.head || document.documentElement).appendChild(style);
    }
    const rect = layout.player, key = JSON.stringify(rect);
    if (key === layoutKey) return; layoutKey = key;
    for (const [name, value] of Object.entries({ x: rect?.x ?? 0, y: rect?.y ?? 0, w: rect?.width ?? 0, h: rect?.height ?? 0 })) document.documentElement.style.setProperty('--atsumi-vod-' + name, value + 'px');
    window.dispatchEvent(new Event('resize'));
  }
  function selectedDocument() {
    const query = new URLSearchParams(location.search);
    return location.origin === 'https://www.youtube.com' && location.pathname === '/watch'
      && query.getAll('v').length === 1 && query.get('v') === videoId;
  }
  const mediaEvents = ['seeking','seeked','pause','play','playing','waiting','stalled','ended','ratechange','durationchange','loadedmetadata','emptied'];
  function onMedia(event) {
    if (event.target !== media || pageGone) return;
    if (event.type === 'waiting' || event.type === 'stalled' || event.type === 'emptied') waiting = true;
    if (event.type === 'playing' || event.type === 'seeked' || event.type === 'loadedmetadata') waiting = false;
    publishClock();
  }
  function renewSource() { sourceId = crypto.randomUUID(); sourceGeneration++; stableSince = 0; }
  function sampleClock() {
    const player = document.getElementById('movie_player'), next = player?.querySelector('video');
    let response = null;
    try { response = player?.getPlayerResponse?.() ?? window.ytInitialPlayerResponse; } catch {}
    const nextSource = typeof next?.currentSrc === 'string' ? next.currentSrc : '';
    if (next !== media || nextSource !== mediaSource) {
      for (const name of mediaEvents) media?.removeEventListener?.(name,onMedia);
      media = next || null; mediaSource = nextSource; renewSource(); waiting = false; lastSeeking = false;
      for (const name of mediaEvents) media?.addEventListener?.(name,onMedia);
    }
    const ad = !!player && (player.classList?.contains('ad-showing') || player.classList?.contains('ad-interrupting'));
    const duration = Number(media?.duration), time = Number(media?.currentTime), rate = Number(media?.playbackRate);
    const declared = Number(response?.videoDetails?.lengthSeconds);
    // The selected response ID can remain unchanged during ads. Duration is an
    // extra reject condition, never a substitute for the official ad signals.
    const candidate = !pageGone && selectedDocument() && response?.videoDetails?.videoId === videoId
      && response?.playabilityStatus?.status === 'OK'
      && !!media && media.readyState >= 1 && Number.isFinite(time) && time >= 0
      && Number.isFinite(duration) && duration > 0 && duration <= 604800 && time <= duration + 2
      && Number.isFinite(declared) && declared > 0 && Math.abs(duration-declared) <= 2
      && Number.isFinite(rate) && rate >= .1 && rate <= 16;
    const nextPhase = ad ? 'ad' : candidate ? 'candidate' : 'unknown';
    if (nextPhase !== phase) { phase = nextPhase; renewSource(); }
    if (candidate && !ad && !stableSince) stableSince = Date.now();
    const valid = candidate && !ad && Date.now()-stableSince >= 200;
    const seeking = !!media?.seeking;
    if (seeking && !lastSeeking) seekGeneration++;
    lastSeeking = seeking;
    return {videoId,sourceId,sourceGeneration,sequence:++sequence,seekGeneration,currentTime:valid?time:null,duration:valid?duration:null,
      paused:!media||media.paused===true,seeking,playbackRate:Number.isFinite(rate)&&rate>=.1&&rate<=16?rate:1,
      buffering:!!media&&!media.paused&&!media.ended&&(waiting||media.readyState<3),ended:valid&&media.ended===true,
      adState:ad?'ad':valid?'content':'unknown',valid};
  }
  function publishClock() { if (!pageGone && epoch) { const clock=sampleClock();notify('clock',clock);renderRestart(clock); } }
  // Native archival verification only. No automatic capture and no playback
  // controls: each tiny RAM image and media timestamp come from the same frame.
  let frameSample = null;
  function stopFrameSample(code) {
    const sample=frameSample;if(!sample)return;
    if(sample.callback)sample.video.cancelVideoFrameCallback?.(sample.callback);
    sample.callback=0;clearTimeout(sample.timer);
    if(code){sample.error=code;sample.frames=[];}
    sample.done=true;
    sample.expires=Date.now()+2000;
    setTimeout(()=>{if(frameSample===sample&&sample.done){sample.frames=[];frameSample=null;}},2000);
  }
  function beginFrameSample(value) {
    if(frameSample?.done&&frameSample.expires<=Date.now())frameSample=null;
    if(frameSample || pageGone || !value || typeof value.requestId!=='string'
      || !/^[a-f0-9-]{36}$/.test(value.requestId) || value.documentEpoch!==epoch
      || !Number.isSafeInteger(value.expiresAtMs)||value.expiresAtMs<Date.now()||value.expiresAtMs>Date.now()+3000)return false;
    const first=sampleClock(),v=media;
    const acceptable=c=>c.valid&&c.adState==='content'&&!c.seeking&&!c.buffering&&!c.paused&&!c.ended
      &&c.sourceId===value.sourceId&&c.sourceGeneration===value.sourceGeneration&&c.seekGeneration===value.seekGeneration;
    if(!acceptable(first)||!v||typeof v.requestVideoFrameCallback!=='function'||v.mediaKeys
      ||v.readyState<2||!v.videoWidth||!v.videoHeight)return false;
    const sample={requestId:value.requestId,video:v,documentEpoch:epoch,sourceId:first.sourceId,
      sourceGeneration:first.sourceGeneration,seekGeneration:first.seekGeneration,videoId,frames:[],done:false,error:null,callback:0,timer:0,expires:0};
    const canvas=document.createElement('canvas');canvas.width=32;canvas.height=18;
    const context=canvas.getContext('2d',{willReadFrequently:true});if(!context)return false;
    // Reduce aliasing when comparing these tiny frames with the area-scaled source.
    context.imageSmoothingQuality = 'high';
    frameSample=sample;sample.timer=setTimeout(()=>stopFrameSample('frame_timeout'),6000);
    let lastPresented=0, lastCallbackPts=-1;
    const frame=(now,metadata)=>{
      sample.callback=0;
      if(frameSample!==sample||sample.done)return;
      const before=sampleClock();
      if(pageGone||epoch!==sample.documentEpoch||media!==v||!acceptable(before)){stopFrameSample('frame_source_changed');return;}
      const time=metadata.mediaTime,pts=Math.round(time*90000);
      const shown=metadata.presentedFrames,display=metadata.expectedDisplayTime,presented=metadata.presentationTime;
      // The compositor can present frames between callbacks. Use only observed frames
      // and keep a skipped callback within the matcher's 200 ms observation interval.
      const missedWithLongGap=lastPresented>0&&shown>lastPresented+1&&pts-lastCallbackPts>18000;
      if(!Number.isFinite(time)||!Number.isSafeInteger(pts)||pts<0||time>before.duration+1
        ||Math.abs(time-before.currentTime)>.25||!Number.isSafeInteger(shown)||shown<=lastPresented
        ||pts<=lastCallbackPts||missedWithLongGap||!Number.isFinite(now)||!Number.isFinite(display)
        ||!Number.isFinite(presented)||presented>display+1||Math.abs(now-display)>50){stopFrameSample('frame_clock');return;}
      lastPresented=shown;lastCallbackPts=pts;
      const last=sample.frames.at(-1);
      if(!last||pts-last.pts90k>=7500){
        try{
          context.drawImage(v,0,0,32,18);
          const rgba=context.getImageData(0,0,32,18).data,gray=[];
          for(let i=0;i<rgba.length;i+=4)gray.push(Math.round((299*rgba[i]+587*rgba[i+1]+114*rgba[i+2])/1000));
          const after=sampleClock();
          if(media!==v||epoch!==sample.documentEpoch||!acceptable(after)){stopFrameSample('frame_source_changed');return;}
          // rVFC/canvas synchronization is best effort, not microsecond proof.
          // Include compositor lateness, observed media advance and a 50ms
          // scheduling allowance. Broad uncertainty is rejected by the matcher.
          const uncertaintyTicks=Math.ceil(90000*(.05*after.playbackRate+Math.max(Math.abs(time-before.currentTime),Math.abs(time-after.currentTime))+Math.abs(now-display)*after.playbackRate/1000));
          if(uncertaintyTicks>22500){stopFrameSample('frame_clock');return;}
          sample.frames.push({pts90k:pts,uncertaintyTicks,gray});
        }catch{stopFrameSample('frame_pixels_unavailable');return;}
      }
      if(sample.frames.length>=48||sample.frames.length>=16&&pts-sample.frames[0].pts90k>=180000){stopFrameSample(null);return;}
      sample.callback=v.requestVideoFrameCallback(frame);
    };
    sample.callback=v.requestVideoFrameCallback(frame);return true;
  }
  function takeFrameSample(id,cancel=false){
    const sample=frameSample;if(!sample||sample.requestId!==id)return null;
    if(cancel){stopFrameSample('frame_cancelled');frameSample=null;return null;}
    if(!sample.done)return {pending:true};
    frameSample=null;
    return {requestId:sample.requestId,videoId:sample.videoId,documentEpoch:sample.documentEpoch,
      sourceId:sample.sourceId,sourceGeneration:sample.sourceGeneration,seekGeneration:sample.seekGeneration,
      error:sample.error,frames:sample.frames};
  }
  function control(value) {
    const result = {requestId:value?.requestId,accepted:false};
    if (pageGone || !epoch || value?.documentEpoch !== epoch || !selectedDocument()
      || !Number.isSafeInteger(value.expiresAtMs) || value.expiresAtMs < Date.now() || value.expiresAtMs > Date.now()+3000) return result;
    const clock = sampleClock();
    if (value.sourceId !== clock.sourceId || value.sourceGeneration !== clock.sourceGeneration) return result;
    const player = document.getElementById('movie_player');
    try {
      if (value.action === 'pause' && value.seconds == null && value.rate == null && typeof player?.pauseVideo === 'function') player.pauseVideo();
      else if (value.action === 'resume' && value.seconds == null && value.rate == null && clock.valid && typeof player?.playVideo === 'function') player.playVideo();
      else if (value.action === 'seek' && value.rate == null && clock.valid && Number.isFinite(value.seconds) && value.seconds >= 0 && value.seconds <= clock.duration && typeof player?.seekTo === 'function') player.seekTo(value.seconds,true);
      else if (value.action === 'rate' && value.seconds == null && clock.valid && Number.isFinite(value.rate) && value.rate >= .1 && value.rate <= 16
        && typeof player?.getAvailablePlaybackRates === 'function' && typeof player?.setPlaybackRate === 'function') {
        const supported = player.getAvailablePlaybackRates();
        if (!Array.isArray(supported) || !supported.some(rate=>typeof rate==='number'&&Math.abs(rate-value.rate)<.000001)) return result;
        player.setPlaybackRate(value.rate);
      }
      else return result;
      result.accepted = true; publishClock();
    } catch {}
    return result;
  }
  function restartPlayerVisible(player) {
    if (!player || pageGone || !epoch || layout?.visible !== true || document.hidden === true
      || !(layout.player?.width > 0) || !(layout.player?.height > 0)) return false;
    const rect=player.getBoundingClientRect();
    if (!(rect.width > 0) || !(rect.height > 0)) return false;
    // A transparent/hidden ancestor also conceals the player. Do not infer
    // visibility from the bottom controls, which legitimately fade at EOF.
    for (let node=player;node;node=node.parentElement) {
      if (node.hidden || node.inert || node.getAttribute?.('inert') != null || node.getAttribute?.('aria-hidden') === 'true') return false;
      const style=window.getComputedStyle(node);
      // Unlike display/opacity, visibility can be restored by a child.
      if (!style || style.display === 'none' || node===player&&(style.visibility === 'hidden' || style.visibility === 'collapse')
        || Number(style.opacity) === 0) return false;
    }
    return true;
  }
  function restartLayer(player) {
    // Native hit testing found modern fullscreen grid cards above layer61,
    // while the official popup uses1001. Inspect only those known player
    // regions; never move the end cards or assign a page-wide maximum layer.
    const visible=node=>{
      const rect=node.getBoundingClientRect();if(!(rect.width>0&&rect.height>0))return false;
      if(window.getComputedStyle(node).visibility!=='visible')return false;
      for(let current=node;current&&current!==player;current=current.parentElement){
        const style=window.getComputedStyle(current);
        if(current.hidden||current.inert||current.getAttribute?.('aria-hidden')==='true'||style.display==='none'||Number(style.opacity)===0)return false;
      }
      return true;
    };
    let floor=60,ceiling=1001;
    const menus=player.querySelectorAll('.ytp-popup,.ytp-contextmenu');
    if(menus.length>32)return null;
    for(const menu of menus){
      if(visible(menu))return null;
      const z=Number(window.getComputedStyle(menu).zIndex);
      if(Number.isSafeInteger(z)&&z>0)ceiling=Math.min(ceiling,z);
    }
    const ends=player.querySelectorAll('.ytp-fullscreen-grid-stills-container,.ytp-modern-videowall-still,.html5-endscreen,.ytp-endscreen-content');
    if(ends.length>64)return null;
    for(const end of ends){
      if(!visible(end))continue;
      let node=end,depth=0;
      for(;node&&node!==player&&depth<32;node=node.parentElement,depth++){
        const z=Number(window.getComputedStyle(node).zIndex);
        if(Number.isSafeInteger(z)&&z>=0)floor=Math.max(floor,z);
      }
      if(node!==player)return null;
    }
    return floor+1<ceiling?floor+1:null;
  }
  function renderRestart(clock) {
    const player=document.getElementById('movie_player');
    let button=player?.querySelector('[data-atsumi-vod-restart]');
    const eligible=restartPlayerVisible(player) && (clock ??= sampleClock()).valid && clock.ended && !clock.seeking
      && typeof player.seekTo === 'function' && typeof player.playVideo === 'function';
    const layer=eligible?restartLayer(player):null,shown=eligible&&layer!==null;
    if (!button && shown) {
      button=document.createElement('button');button.type='button';button.setAttribute('data-atsumi-vod-restart','');
      button.setAttribute('aria-label','처음부터 다시보기');button.title='처음부터 다시 재생';
      button.style.cssText='position:absolute;z-index:61;top:50%;left:50%;transform:translate(-50%,-50%);display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;box-sizing:border-box;width:112px;height:112px;margin:0;padding:14px;border:1px solid #ffffff40;border-radius:50%;background:#151719;color:#fff;box-shadow:0 3px 18px #0006;font-family:inherit;font-size:16px;line-height:1.2;cursor:pointer';
      const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
      for(const [key,value] of Object.entries({viewBox:'0 0 24 24',width:'36',height:'36','aria-hidden':'true'}))svg.setAttribute(key,value);
      svg.style.cssText='display:block;flex:none;width:36px;height:36px;pointer-events:none';
      const path=document.createElementNS('http://www.w3.org/2000/svg','path');
      for(const [key,value] of Object.entries({d:'M20 7v5h-5M19.4 12A7.5 7.5 0 1 0 18 17',fill:'none',stroke:'currentColor','stroke-width':'1.8','stroke-linecap':'round','stroke-linejoin':'round'}))path.setAttribute(key,value);
      svg.appendChild(path);button.appendChild(svg);
      const label=document.createElement('span');label.textContent='다시보기';button.appendChild(label);
      button.addEventListener('pointerdown',event=>event.stopPropagation());
      for(const type of ['keydown','keyup'])button.addEventListener(type,event=>{
        // Keep the native button's Enter/Space activation, not player shortcuts.
        if(!event.isComposing&&event.keyCode!==229&&!event.ctrlKey&&!event.altKey&&!event.metaKey&&!event.shiftKey
          &&(event.key==='Enter'||event.key===' '))event.stopImmediatePropagation();
      });
      button.addEventListener('click',event=>{
        event.preventDefault();event.stopImmediatePropagation();
        if(button.hidden || !restartPlayerVisible(player) || document.getElementById('movie_player')!==player || restartLayer(player)===null) return;
        const current=sampleClock();
        if(!current.valid||!current.ended||current.seeking) { renderRestart(current);return; }
        try { player.seekTo(0,true);player.playVideo(); } catch { /* Keep actual clock/state authoritative. */ }
        publishClock();
      });
      player.appendChild(button);
    }
    if(!button)return;
    const rect=player.getBoundingClientRect(),size=Math.max(0,Math.min(112,rect.width-24,rect.height-24));
    button.hidden=!shown||!size;button.disabled=button.hidden;
    if(layer!==null)button.style.zIndex=String(layer);
    button.style.width=button.style.height=size+'px';button.style.display=button.hidden?'none':'flex';
    button.style.pointerEvents=button.hidden?'none':'auto';
  }
  function renderClose() {
    const player = document.getElementById('movie_player'), chrome = player?.querySelector('.ytp-chrome-bottom');
    if (!player || !epoch || pageGone) return;
    let button = player.querySelector('[data-atsumi-vod-close]');
    if (!button && closeEnabled) {
      button = document.createElement('button'); button.type='button'; button.setAttribute('data-atsumi-vod-close','');
      button.setAttribute('aria-label','영상 닫기'); button.title='영상 닫기';
      button.style.cssText='position:absolute;top:12px;left:50%;transform:translateX(-50%);width:72px;height:72px;box-sizing:border-box;padding:0;border:0;border-radius:50%;background:rgba(0,0,0,.52);color:#fff;align-items:center;justify-content:center;z-index:60;cursor:pointer';
      const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
      for (const [key,value] of Object.entries({viewBox:'0 0 24 24',width:'40',height:'40',fill:'none',stroke:'currentColor','stroke-width':'2','stroke-linecap':'round','aria-hidden':'true'})) svg.setAttribute(key,value);
      svg.style.pointerEvents='none';svg.style.maxWidth='60%';svg.style.maxHeight='60%';
      const path=document.createElementNS('http://www.w3.org/2000/svg','path');path.setAttribute('d','m6 6 12 12M18 6 6 18');svg.appendChild(path);button.appendChild(svg);
      button.addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0&&closeEnabled&&!button.hidden&&layout?.visible&&selectedDocument()){event.preventDefault();event.stopImmediatePropagation();notify('viewer_close',{inputKey:1});}});
      button.addEventListener('click',event=>{event.preventDefault();event.stopImmediatePropagation();});player.appendChild(button);
    }
    if (!button) return;
    const computed = chrome ? window.getComputedStyle(chrome) : null;
    const opacity = computed && computed.visibility !== 'hidden' && computed.display !== 'none' ? Number(computed.opacity) : 0;
    const rect=player.getBoundingClientRect(),size=Math.max(0,Math.min(72,rect.width-24,rect.height-24));
    button.hidden=!closeEnabled||!layout?.visible||!Number.isFinite(opacity)||opacity<=0||!size;
    button.style.width=button.style.height=size+'px';button.style.display=button.hidden?'none':'flex';button.style.opacity=String(Math.max(0,opacity));button.style.pointerEvents=button.hidden?'none':'auto';
  }
  function followClose() {
    if (pageGone) return;
    closeUntil=Date.now()+5000;
    if (closeFrame || typeof requestAnimationFrame !== 'function') { renderClose();return; }
    const frame=()=>{closeFrame=0;renderClose();if(!pageGone&&Date.now()<closeUntil)closeFrame=requestAnimationFrame(frame);};
    closeFrame=requestAnimationFrame(frame);
  }
  function retainSelectedEndscreen(player, video, response) {
    if (pageGone || !epoch || !selectedDocument() || response?.videoDetails?.videoId !== videoId || !video) return;
    if (!video.ended) { endedCancelled = false; return; }
    if (endedCancelled || player.classList?.contains('ad-showing') || player.classList?.contains('ad-interrupting')) return;
    const cancel = player.querySelector('.ytp-autonav-endscreen-upnext-cancel-button');
    if (!cancel || cancel.disabled || cancel.getAttribute('aria-disabled') === 'true'
      || cancel.closest('[hidden],[aria-hidden="true"]') || !cancel.getClientRects().length) return;
    const style = window.getComputedStyle(cancel);
    if (style.display === 'none' || style.visibility !== 'visible' || Number(style.opacity) === 0) return;
    // Official one-shot endscreen cancellation, not the persisted autoplay toggle.
    // Do not construct/execute the site's optional cancelCommand ourselves.
    endedCancelled = true;
    try { cancel.click(); } catch { /* Native identity fencing remains the fail-closed fallback. */ }
  }
  function observeEndscreen(player) {
    if (pageGone || player === observedPlayer) return;
    endscreenObserver?.disconnect(); endscreenObserver = null; observedPlayer = player;
    if (player && typeof MutationObserver === 'function') {
      endscreenObserver = new MutationObserver(() => status());
      // Only new/removed nodes inside the original player; no full-page/style observer.
      endscreenObserver.observe(player, { childList: true, subtree: true });
    }
  }
  function status() {
    const player = document.getElementById('movie_player'), video = player?.querySelector('video');
    let response = null, authenticated = null;
    try { response = player?.getPlayerResponse?.() ?? window.ytInitialPlayerResponse; } catch {}
    try { const known = window.ytcfg?.get?.('LOGGED_IN'); if (typeof known === 'boolean') authenticated = known; } catch {}
    observeEndscreen(player);
    retainSelectedEndscreen(player, video, response);
    renderRestart();
    const correct = selectedDocument() && response?.videoDetails?.videoId === videoId;
    const unavailable = ['ERROR', 'UNPLAYABLE', 'LOGIN_REQUIRED', 'LIVE_STREAM_OFFLINE'].includes(response?.playabilityStatus?.status);
    const ready = correct && video?.readyState >= 2 && video.videoWidth > 0;
    // End is a presentation state, not teardown. The official replay and seek controls remain active.
    return { state: ready ? video.ended ? 'ended' : 'playing' : unavailable ? 'error' : 'loading', authenticated, unavailable,
      playability: unavailable ? response?.playabilityStatus?.status === 'LOGIN_REQUIRED' ? 'LOGIN_REQUIRED' : 'UNAVAILABLE' : null };
  }
  const timer = setInterval(() => notify('status', status()), 1500);
  const clockTimer = setInterval(() => {publishClock();renderClose();},250);
  document.addEventListener('pointermove',followClose,true);
  document.addEventListener('pointerleave',followClose,true);
  document.addEventListener('ended', () => status(), true);
  document.addEventListener('playing', () => status(), true);
  document.addEventListener('seeked', () => status(), true);
  document.addEventListener('visibilitychange', () => renderRestart(), true);
  document.addEventListener('pointerdown', event => {
    if (event.isTrusted && event.button === 0 && event.target?.closest?.('.ytp-fullscreen-button')) notify('fullscreen', { enabled: !fullscreen, inputKey: 1 });
  }, true);
  document.addEventListener('click', event => {
    if (event.isTrusted && event.target?.closest?.('.ytp-fullscreen-button')) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  document.addEventListener('keydown', event => {
    if (!event.isTrusted || event.repeat || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey || event.isComposing || event.keyCode === 229 || event.target?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]')) return;
    if (event.key === 'Escape' && fullscreen || event.key.toLowerCase() === 'f') {
      event.preventDefault(); event.stopImmediatePropagation();
      notify('fullscreen', { enabled: event.key === 'Escape' ? false : !fullscreen, inputKey: event.key === 'Escape' ? 0x1B : 0x46 });
    }
  }, true);
  window.addEventListener('pagehide', () => { pageGone = true;stopFrameSample('frame_page_closed');frameSample=null;renderRestart(); clearInterval(timer);clearInterval(clockTimer);if(closeFrame&&typeof cancelAnimationFrame==='function')cancelAnimationFrame(closeFrame);for(const name of mediaEvents)media?.removeEventListener?.(name,onMedia); endscreenObserver?.disconnect(); endscreenObserver = null; observedPlayer = null; }, { once: true });
  Object.defineProperty(window, '__atsumiYouTubeVod', { value: Object.freeze({
    configure(value) { if (!value || !Number.isSafeInteger(value.epoch) || value.epoch < 1 || value.videoId !== videoId) return false; if(epoch!==value.epoch){epoch=value.epoch;sequence=0;renewSource();} fullscreen = value.fullscreen === true; notify('status', status());publishClock(); return true; },
    layout(value) { layout = value; apply();renderClose();renderRestart(); },
    controls(value) {if(value?.epoch!==epoch||pageGone)return false;closeEnabled=value.canClose===true;renderClose();return true;},
    control,
    beginFrameSample,
    takeFrameSample,
    fullscreen(enabled) { fullscreen = enabled === true; const button = document.querySelector('.ytp-fullscreen-button'); if (button) { button.setAttribute('aria-label', fullscreen ? '전체화면 종료 (f)' : '전체화면 (f)'); button.setAttribute('aria-pressed', String(fullscreen)); } },
    status,
  }), configurable: false });
  document.addEventListener('DOMContentLoaded', apply, { once: true });
})();
