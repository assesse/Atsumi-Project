// YouTube-owned layout and bounded capture bridge. The site remains the player.
(() => {
  "use strict";
  if (window.top !== window || location.origin !== "https://www.youtube.com" || window.__atsumiYouTubeOriginal) return;
  const isChat = location.pathname === "/live_chat";
  if (!isChat && location.pathname !== "/watch") return;
  const channel = new URLSearchParams(location.search).get("v");
  if (!/^[A-Za-z0-9_-]{11}$/.test(channel ?? "")) return;
  let epoch = 0, layout = null, lastLayout = "", nativeFullscreen = false;
  // Pixel visibility is native-owned. A hidden recording receiver must not
  // become a 0x0 player and trigger a different adaptive rendition on reattach.
  let receiverRect = { x: 0, y: 0, width: 1280, height: 720 };
  let lastOpacity = -1, frame = 0, observeUntil = 0;
  let startFailure = null;
  let ui=null,uiPending=false;
  const svgNamespace='http://www.w3.org/2000/svg';
  const icons={
    record:[['circle',{cx:'12',cy:'12',r:'6',fill:'currentColor'}]],
    stop:[['rect',{x:'6',y:'6',width:'12',height:'12',rx:'1',fill:'currentColor'}]],
    camera:[['path',{d:'M3 7h5l2-3h4l2 3h5v13H3z'}],['circle',{cx:'12',cy:'13',r:'4'}]],
    close:[['path',{d:'m6 6 12 12M18 6 6 18'}]],
  };
  function createControlIcon(icon,large){
    // Fixed SVG DOM nodes work with the site's Trusted Types policy unchanged.
    const svg=document.createElementNS(svgNamespace,'svg');
    for(const [name,value] of Object.entries({viewBox:'0 0 24 24',width:large?'40':'24',height:large?'40':'24',fill:'none',stroke:'currentColor','stroke-width':large?'2':'1.7','stroke-linecap':'round','aria-hidden':'true'}))svg.setAttribute(name,value);
    svg.style.pointerEvents='none';
    if(large){svg.style.maxWidth='60%';svg.style.maxHeight='60%';}
    else{svg.style.padding='0';}
    for(const [tag,attributes] of icons[icon]){
      const shape=document.createElementNS(svgNamespace,tag);
      for(const [name,value] of Object.entries(attributes))shape.setAttribute(name,value);
      svg.appendChild(shape);
    }
    return svg;
  }
  function renderControls(opacity=lastOpacity){
    if(!(!isChat&&epoch)||!ui)return;
    const player=document.getElementById('movie_player'),anchor=player?.querySelector('.ytp-fullscreen-button');if(!player||!anchor?.parentElement)return;
    for(const kind of ['record','camera','close']){
      let button=player.querySelector(`[data-atsumi-live-action="${kind}"]`);
      if(!button){button=document.createElement('button');button.type='button';button.dataset.atsumiLiveAction=kind;
        button.addEventListener('pointerdown',event=>{if(!event.isTrusted||event.button!==0||button.disabled||button.hidden||!layout?.visible||!(!isChat&&epoch)||uiPending)return;event.preventDefault();event.stopImmediatePropagation();uiPending=true;renderControls();
          const action=kind==='close'?'viewer_close':kind==='camera'?'screenshot':ui.recording?'record_stop':'record_start';void request('ui_intent',{action,input:'pointer'}).then(()=>{if(kind==='record')ui.recordBusy=true;}).catch(()=>{}).finally(()=>{uiPending=false;renderControls();});});
        button.addEventListener('click',event=>{event.preventDefault();event.stopImmediatePropagation();});
        if(kind==='close'){button.style.cssText='position:absolute;top:12px;left:50%;transform:translateX(-50%);width:72px;height:72px;box-sizing:border-box;padding:0;border:0;border-radius:50%;background:rgba(0,0,0,.52);color:#fff;align-items:center;justify-content:center;z-index:60;cursor:pointer';player.appendChild(button);}
        else{button.className='ytp-button';button.style.cssText='width:40px;padding:8px;border:0;background:transparent;color:#fff;display:inline-flex;align-items:center;justify-content:center;vertical-align:top;cursor:pointer';anchor.parentElement.insertBefore(button,anchor);}
      }
      const name=kind==='close'?'방송 시청 종료':kind==='camera'?'스크린샷 저장 (S)':ui.recording?'녹화 중지':'녹화';button.setAttribute('aria-label',name);button.title=kind==='record'&&!ui.canRecord&&!ui.recording?(ui.captureReason||'원본 영상이 준비되면 녹화할 수 있습니다.'):name;
      const icon=kind==='close'?'close':kind==='camera'?'camera':ui.recording?'stop':'record';if(button.dataset.icon!==icon||!button.querySelector('svg')){button.replaceChildren(createControlIcon(icon,kind==='close'));button.dataset.icon=icon;}
      button.disabled=uiPending||(kind==='record'?(ui.recordBusy||!ui.canRecord&&!ui.recording):kind==='camera'?ui.captureBusy:!ui.canClose);
      if(kind==='record'){button.setAttribute('aria-pressed',String(ui.recording));button.style.color=ui.recording?'#ff4e45':'#fff';}
      if(kind==='close'){const r=player.getBoundingClientRect(),size=Math.max(0,Math.min(72,r.width-24,r.height-24));button.style.width=button.style.height=`${size}px`;button.hidden=!ui.canClose||opacity<=0||!Number.isFinite(size)||size<=0;button.style.display=button.hidden?'none':'flex';button.style.opacity=String(Math.max(0,opacity));button.style.pointerEvents=button.hidden?'none':'auto';}
    }
  }
  const pending = new Map();
  const notify = (type, fields = {}) => { if (epoch && (!isChat || type === "chat_status")) try { window.chrome.webview.postMessage("ATSUMI_YOUTUBE_V1:" + JSON.stringify({ id: crypto.randomUUID(), epoch, type, ...fields })); } catch {} };
  const request = (type, fields = {}) => new Promise((resolve, reject) => {
    if (!epoch || pending.size >= 8 || !window.chrome?.webview?.postMessage || isChat) return reject(Object.assign(new Error("bridge_unavailable"), { code: "BRIDGE_BUSY" }));
    const id = crypto.randomUUID();
    const timer = setTimeout(() => { pending.delete(id); reject(Object.assign(new Error("ack_timeout"), { code: "BROWSER_ACK_TIMEOUT" })); }, 12000);
    pending.set(id, { resolve, reject, timer });
    try { window.chrome.webview.postMessage("ATSUMI_YOUTUBE_V1:" + JSON.stringify({ id, epoch, type, ...fields })); }
    catch { clearTimeout(timer); pending.delete(id); reject(Object.assign(new Error("bridge_post_failed"), { code: "BRIDGE_POST_FAILED" })); }
  });
  const ensureStyle = () => {
    let style = document.getElementById("atsumi-youtube-original-layout");
    if (style) return;
    style = document.createElement("style"); style.id = "atsumi-youtube-original-layout";
    // Keep the original receiver/layout alive, but not the unrelated watch page
    // behind it. Never hide a region containing the player or force its faded
    // controls/menus visible. Paired live_chat has its own unchanged stylesheet.
    const watchSiblings = `:is(ytd-watch-flexy #secondary,ytd-watch-flexy #chat,ytd-watch-flexy #below,ytd-watch-flexy ytd-watch-metadata,ytd-masthead):not(:has(#movie_player))`;
    style.textContent = isChat ? `html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#0f0f0f!important}yt-live-chat-app{position:fixed!important;left:var(--atsumi-youtube-x)!important;top:var(--atsumi-youtube-y)!important;width:var(--atsumi-youtube-w)!important;height:var(--atsumi-youtube-h)!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important}` : `html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#000!important}${watchSiblings},${watchSiblings} *{visibility:hidden!important;pointer-events:none!important}ytd-app,ytd-page-manager,ytd-watch-flexy,#columns,#primary,#primary-inner,#player-container-outer,#player-container-inner,ytd-player{transform:none!important;contain:none!important;overflow:visible!important}#movie_player{position:fixed!important;left:var(--atsumi-youtube-x)!important;top:var(--atsumi-youtube-y)!important;width:var(--atsumi-youtube-w)!important;height:var(--atsumi-youtube-h)!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;background:#000!important;z-index:2147483000!important}#movie_player .html5-video-container{width:100%!important;height:100%!important}#movie_player video.html5-main-video{width:100%!important;height:100%!important;left:0!important;top:0!important;object-fit:contain!important}#movie_player .ytp-chrome-bottom{width:calc(100% - 24px)!important;left:12px!important}#movie_player .ytp-popup{max-width:calc(100% - 24px)!important;max-height:calc(100% - 56px)!important}ytd-popup-container,tp-yt-iron-dropdown,ytd-menu-popup-renderer{z-index:2147483100!important}`;
    (document.head || document.documentElement)?.appendChild(style);
  };
  function applyLayout() {
    if (!layout || !document.documentElement) return;
    ensureStyle();
    if(isChat){let closeStyle=document.getElementById('atsumi-live-chat-header');if(!closeStyle){closeStyle=document.createElement('style');closeStyle.id='atsumi-live-chat-header';closeStyle.textContent='yt-live-chat-header-renderer #close-button{display:none!important}';(document.head||document.documentElement).appendChild(closeStyle);}}
    const requested = isChat ? layout.chat : layout.player;
    if (!isChat && requested && [requested.x, requested.y, requested.width, requested.height].every(Number.isFinite)
        && requested.width > 0 && requested.height > 0) receiverRect = { ...requested };
    const r = isChat ? requested : receiverRect;
    const key = JSON.stringify(r); if (key === lastLayout) return; lastLayout = key;
    for (const [name, value] of Object.entries({ x: r?.x ?? 0, y: r?.y ?? 0, w: r?.width ?? 0, h: r?.height ?? 0 })) document.documentElement.style.setProperty(`--atsumi-youtube-${name}`, `${value}px`);
    window.dispatchEvent(new Event("resize"));
  }
  function chatAvailability(watchData) {
    const renderer = watchData?.conversationBar?.liveChatRenderer;
    // An incomplete watch response is not affirmative unavailability. Only
    // these observed official system notices establish a negative observation.
    const value = watchData?.conversationBar?.conversationBarRenderer?.availabilityMessage?.messageRenderer?.text;
    let notice = null;
    if (typeof value?.simpleText === "string" && value.simpleText.length <= 256) notice = value.simpleText;
    else if (Array.isArray(value?.runs) && value.runs.length > 0 && value.runs.length <= 8
        && value.runs.every(run => typeof run?.text === "string" && run.text.length <= 256)) {
      const joined = value.runs.map(run => run.text).join("");
      if (joined.length <= 256) notice = joined;
    }
    if (["Chat is disabled for this live stream.", "채팅을 사용할 수 없는 실시간 스트림입니다."].includes(notice)) return false;
    // A matching iframe may display that same disabled notice. Its presence
    // or visibility alone therefore cannot certify available chat.
    return renderer && typeof renderer === "object" && !Array.isArray(renderer) ? true : null;
  }
  function status() {
    const player = document.getElementById("movie_player"), video = player?.querySelector("video");
    let response = null, authenticated = null;
    try { response = player?.getPlayerResponse?.() ?? window.ytInitialPlayerResponse; } catch {}
    try { const value = window.ytcfg?.get?.("LOGGED_IN"); if (typeof value === "boolean") authenticated = value; } catch {}
    const details = response?.videoDetails;
    const watchData = window.ytInitialData?.contents?.twoColumnWatchNextResults;
    const descriptor = details?.videoId === channel && /^UC[A-Za-z0-9_-]{22}$/.test(details.channelId ?? "") && details.isLive === true && watchData ? {
      videoId: channel, channelId: details.channelId, isLive: true,
      chatAvailable: chatAvailability(watchData),
    } : null;
    const capture = window.__atsumiYouTubeEncodedCapture;
    const advertisement = Boolean(document.querySelector("#movie_player.ad-showing,#movie_player.ad-interrupting"));
    const active = video && video.readyState >= 2 && video.videoWidth > 0;
    const playability = response?.playabilityStatus?.status;
    return { state: active ? video.ended ? "ended" : "playing" : ["ERROR", "UNPLAYABLE", "LOGIN_REQUIRED", "LIVE_STREAM_OFFLINE"].includes(playability) ? "error" : "loading",
      authenticated, chatVisible: descriptor?.chatAvailable === true, captureReady: !advertisement && !!capture?.getStatus?.().ready,
      captureStable: !advertisement && capture?.readyForAutomatic?.(video) === true,
      captureReason: advertisement ? "advertisement" : capture?.getDiagnostics?.().reason ?? "waiting_video",
      chatDescriptor: descriptor, captureStartError: startFailure, error: startFailure ? "capture_start_failed" : ["ERROR", "UNPLAYABLE", "LOGIN_REQUIRED", "LIVE_STREAM_OFFLINE"].includes(playability) ? "site_playback_unavailable" : null };
  }
  function presentation() {
    const chrome=document.querySelector("#movie_player .ytp-chrome-bottom");
    const style=chrome?getComputedStyle(chrome):null;
    const opacity=style&&style.display!=="none"&&style.visibility!=="hidden"?Math.min(1,Math.max(0,Number(style.opacity)||0)):0;
    renderControls(opacity);
    if(opacity!==lastOpacity){lastOpacity=opacity;notify("presentation",{opacity});}
  }
  function tick(){frame=0;presentation();if(performance.now()<observeUntil)frame=requestAnimationFrame(tick);}
  function motion(){if(isChat)return;observeUntil=performance.now()+4000;if(!frame)frame=requestAnimationFrame(tick);}
  document.addEventListener("mousemove",motion,true);document.addEventListener("mouseleave",motion,true);
  document.addEventListener("focusin",motion,true);document.addEventListener("keydown",motion,true);
  function chatStatus(){const app=document.querySelector('yt-live-chat-app'),r=app?.getBoundingClientRect(),style=app?getComputedStyle(app):null;notify('chat_status',{visible:!!r&&r.width>0&&r.height>0&&style?.display!=='none'&&style?.visibility!=='hidden'});}
  const timer = setInterval(() => {if(isChat)chatStatus();else{notify("status", status());presentation();}}, 1500);
  // Keep the original button/icon. Only the outer window layout action is owned
  // by Atsumi, so native panes and the paired chat expand together.
  document.addEventListener("pointerdown", event => {
    if(!isChat&&event.isTrusted&&event.button===0&&event.target?.closest?.('.ytp-fullscreen-button'))notify('fullscreen',{enabled:!nativeFullscreen,inputKey:1});
  },true);
  document.addEventListener("click", event => {
    if (!isChat && event.isTrusted && event.target?.closest?.(".ytp-fullscreen-button")) {
      event.preventDefault(); event.stopImmediatePropagation();
    }
  }, true);
  document.addEventListener("keydown", event => {
    if (isChat || !event.isTrusted || event.repeat || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey || event.isComposing || event.keyCode===229 || event.target?.closest?.('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]')) return;
    if (event.key === "Escape" && nativeFullscreen || event.key.toLowerCase() === "f") { event.preventDefault(); event.stopImmediatePropagation(); notify("fullscreen", { enabled: event.key === "Escape" ? false : !nativeFullscreen,inputKey:event.key==='Escape'?0x1B:0x46 }); }
    else if(event.code==='KeyS'||event.key.toLowerCase()==='s'){event.preventDefault();event.stopImmediatePropagation();notify('screenshot',{inputKey:0x53});}
  }, true);
  window.addEventListener("pagehide", () => { if (timer) clearInterval(timer);if(frame)cancelAnimationFrame(frame); for (const item of pending.values()) { clearTimeout(item.timer); item.reject(new Error("page_changed")); } pending.clear(); });
  Object.defineProperty(window, "__atsumiYouTubeOriginal", { value: Object.freeze({
    configure(value) { if (!value || !Number.isSafeInteger(value.epoch) || value.epoch < 1 || value.channel !== channel) return false; epoch = value.epoch; nativeFullscreen = value.fullscreen === true; if (isChat)chatStatus();else { window.__atsumiYouTubeEncodedCapture?.configure?.({ rawCaptureEnabled: value.rawCaptureEnabled === true }); notify("status", status()); } return true; },
    layout(value) { layout = value; applyLayout(); },
    controls(value) { if(isChat||!epoch||value?.epoch!==epoch)return false;ui={canClose:value.canClose===true,captureBusy:value.captureBusy===true,canRecord:value.canRecord===true,captureReason:typeof value.captureReason==='string'?value.captureReason.slice(0,300):null,recording:value.recording===true,recordBusy:value.recordBusy===true};renderControls();return true; },
    resume(value) { if(isChat||value!==epoch||!layout?.visible||!layout.player)return false;let response;try{response=document.getElementById('movie_player')?.getPlayerResponse?.()??window.ytInitialPlayerResponse;}catch{}if(response?.videoDetails?.videoId!==channel||response?.videoDetails?.isLive!==true)return false;const video=document.querySelector('#movie_player video');if(video?.paused&&!video.ended)void video.play().catch(()=>{startFailure='RESUME_FAILED';notify('status',status());});return true; },
    fullscreen(enabled) { nativeFullscreen = enabled === true; const button = document.querySelector(".ytp-fullscreen-button"); if (button) { button.setAttribute("aria-label", nativeFullscreen ? "전체화면 종료 (f)" : "전체화면 (f)"); button.setAttribute("aria-pressed", String(nativeFullscreen)); } },
    reply(id, result) { const item = pending.get(id); if (!item) return; pending.delete(id); clearTimeout(item.timer); if (result?.ok) item.resolve(result.value); else item.reject(Object.assign(new Error(result?.error || "native_rejected"), { code: result?.code || "NATIVE_REJECTED", diagnosticMessage: result?.diagnosticMessage })); },
    start(requestId, automatic = false) { startFailure = null; const video = document.querySelector("#movie_player video"); return window.__atsumiYouTubeEncodedCapture?.start({ requestId, channelId: channel, rightsAcknowledged: true, video, automatic: automatic === true }, { request, onStatus: () => notify("status", status()) }).catch(error => { startFailure = ["NATIVE_REJECTED", "BROWSER_ACK_TIMEOUT", "BRIDGE_BUSY", "BRIDGE_POST_FAILED", "ENCODED_UNSUPPORTED"].includes(error?.code) ? error.code : "START_FAILED"; notify("status", status()); }); },
    stop() { return window.__atsumiYouTubeEncodedCapture?.stop("user_stop", false); }, status,
  }), configurable: false });
  document.addEventListener("DOMContentLoaded", applyLayout, { once: true });
})();
