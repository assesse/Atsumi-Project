// SOOP-owned original document adapter. No media requests or account secrets.
(() => {
  "use strict";
  if (window.top !== window || location.origin !== "https://play.sooplive.com" || window.__atsumiSoopOriginal) return;
  const match = /^\/([A-Za-z0-9_-]{1,40})\/(?:([0-9]+)\/)?embed\/?$/.exec(location.pathname);
  if (!match) return;
  const channel = match[1].toLowerCase();
  let epoch = 0, booted = false, nativeReady = false, layout = null;
  let controlsObserver = null, controlsFrame = 0, controlsUntil = 0, lastOpacity = -1;
  let nativeFullscreen = false;
  let ui=null,uiPending=false,pointerDiagnostics=0;
  const icons={record:'<circle cx="12" cy="12" r="8" stroke-width="1.4"/><circle cx="12" cy="12" r="4.5"/>',stop:'<circle cx="12" cy="12" r="8" stroke-width="1.4"/><rect x="8" y="8" width="8" height="8" rx="1" fill="currentColor" stroke="none"/>',camera:'<path d="M3 7h5l2-3h4l2 3h5v13H3z"/><circle cx="12" cy="13" r="4"/>',close:'<path d="m6 6 12 12M18 6 6 18"/>'};
  function renderControls(opacity=lastOpacity){
    if(!(epoch&&bound())||!ui)return;
    const player=document.getElementById('player'),anchor=player?.querySelector('.btn_fullScreen_mode');if(!player||!anchor?.parentElement)return;
    for(const kind of ['record','camera','close']){
      let button=player.querySelector(`[data-atsumi-live-action="${kind}"]`);
      if(!button){button=document.createElement('button');button.type='button';button.dataset.atsumiLiveAction=kind;
        button.addEventListener('pointerdown',event=>{if(!event.isTrusted||event.button!==0||button.disabled||button.hidden||!layout?.visible||!(epoch&&bound())||uiPending)return;event.preventDefault();event.stopImmediatePropagation();uiPending=true;renderControls();
          const action=kind==='close'?'viewer_close':kind==='camera'?'screenshot':ui.recording?'record_stop':'record_start';void requests('ui_intent',{action,input:'pointer'}).then(()=>{if(kind==='record')ui.recordBusy=true;}).catch(()=>{}).finally(()=>{uiPending=false;renderControls();});});
        button.addEventListener('click',event=>{event.preventDefault();event.stopImmediatePropagation();});
        if(kind==='close'){button.style.cssText='position:absolute;top:12px;left:50%;transform:translateX(-50%);width:72px;height:72px;box-sizing:border-box;padding:0;border:0;border-radius:50%;background:rgba(0,0,0,.52);color:#fff;align-items:center;justify-content:center;z-index:60;cursor:pointer';player.appendChild(button);}
        else{button.className='atsumi-live-control';button.style.cssText='width:40px;height:40px;padding:8px;border:0;background:transparent;color:#fff;display:inline-flex;align-items:center;justify-content:center;vertical-align:top;cursor:pointer';anchor.parentElement.insertBefore(button,anchor);}
      }
      const name=kind==='close'?'방송 시청 종료':kind==='camera'?'스크린샷 저장 (S)':ui.recording?'녹화 중지':'녹화';button.setAttribute('aria-label',name);button.title=name;
      const icon=kind==='close'?'close':kind==='camera'?'camera':ui.recording?'stop':'record';if(button.dataset.icon!==icon){button.dataset.icon=icon;button.innerHTML=`<svg viewBox="0 0 24 24" width="${kind==='close'?40:24}" height="${kind==='close'?40:24}" fill="none" stroke="currentColor" stroke-width="${kind==='close'?2:kind==='record'?1.8:1.7}" stroke-linecap="round" aria-hidden="true" style="pointer-events:none;${kind==='close'?'max-width:60%;max-height:60%':''}">${icons[icon]}</svg>`;}
      button.disabled=uiPending||(kind==='record'?(ui.recordBusy||!ui.canRecord&&!ui.recording):kind==='camera'?ui.captureBusy:!ui.canClose);
      if(kind==='record'){button.setAttribute('aria-pressed',String(ui.recording));button.style.color=ui.recording?'#ff5b70':'#fff';}
      if(kind==='close'){const r=player.getBoundingClientRect(),size=Math.max(0,Math.min(72,r.width-24,r.height-24));button.style.width=button.style.height=`${size}px`;button.hidden=!ui.canClose||opacity<=0||!Number.isFinite(size)||size<=0;button.style.display=button.hidden?'none':'flex';button.style.opacity=String(Math.max(0,opacity));button.style.pointerEvents=button.hidden?'none':'auto';}
    }
  }
  const pending = new Map();
  const binding=()=>{try{const value=window.livePlayer?.liveAPI?.getResponse?.()?.CHANNEL;return {actualChannel:typeof value?.BJID==="string"?value.BJID.toLowerCase():null,actualBroadcast:value?.BNO===undefined?null:String(value.BNO)};}catch{return {actualChannel:null,actualBroadcast:null};}};
  const bound=()=>{const value=binding();return value.actualChannel===channel&&value.actualBroadcast===match[2];};
  const requests = (type, fields = {}) => new Promise((resolve, reject) => {
    if (!epoch || pending.size >= 8 || !window.chrome?.webview?.postMessage) return reject(Object.assign(new Error("bridge_unavailable"), {code:"BRIDGE_BUSY"}));
    const id = crypto.randomUUID();
    const timer = setTimeout(() => { pending.delete(id); reject(Object.assign(new Error("ack_timeout"),{code:"BROWSER_ACK_TIMEOUT"})); }, 12_000);
    pending.set(id, {resolve,reject,timer});
    try { window.chrome.webview.postMessage("ATSUMI_SOOP_V1:" + JSON.stringify({id,epoch,type,...fields,...binding()})); }
    catch { clearTimeout(timer);pending.delete(id);reject(Object.assign(new Error("bridge_post_failed"),{code:"BRIDGE_POST_FAILED"})); }
  });
  const notify = (type, fields = {}) => { if(epoch) try { window.chrome.webview.postMessage("ATSUMI_SOOP_V1:"+JSON.stringify({id:crypto.randomUUID(),epoch,type,...fields})); } catch {} };
  const css = () => {
    let style=document.getElementById("atsumi-soop-original-layout");
    if(!style){style=document.createElement("style");style.id="atsumi-soop-original-layout";(document.head||document.documentElement).appendChild(style);}
    style.textContent=`html,body,#webplayer{margin:0!important;padding:0!important;width:100%!important;height:100%!important;min-width:0!important;min-height:0!important;overflow:hidden!important;background:#000!important}#player,#chatting_area{position:fixed!important;box-sizing:border-box!important;margin:0!important;min-width:0!important;min-height:0!important;max-width:none!important;max-height:none!important;transform:none!important}#player{left:var(--atsumi-player-x)!important;top:var(--atsumi-player-y)!important;width:var(--atsumi-player-w)!important;height:var(--atsumi-player-h)!important;display:var(--atsumi-player-display)!important}#chatting_area{left:var(--atsumi-chat-x)!important;top:var(--atsumi-chat-y)!important;width:var(--atsumi-chat-w)!important;height:var(--atsumi-chat-h)!important;display:var(--atsumi-chat-display)!important}#chatbox{height:100%!important;width:100%!important;min-width:0!important}#player .player_ratio{padding:0!important;height:100%!important}#player .video_area,#player .video_wrap{width:100%!important;height:100%!important}video#livePlayer{object-fit:contain!important}`;
  };
  // Public embed markup: only the chat area's header hide action, not dialogs.
  function ensureControlsStyle(){
    const parent=document.head||document.documentElement;
    // DocumentCreated precedes HTML parsing; do not abort adapter installation.
    if(!parent||document.getElementById('atsumi-soop-live-controls'))return;
    const style=document.createElement('style');style.id='atsumi-soop-live-controls';
    style.textContent='#chatting_area #chatbox > .area_header > .chat_title #setbox_close{display:none!important}[data-atsumi-live-action]:disabled{opacity:.45;cursor:default}[data-atsumi-live-action]:focus-visible{outline:2px solid white;outline-offset:2px}';parent.appendChild(style);
  }
  ensureControlsStyle();
  const applyLayout = () => {
    ensureControlsStyle();
    if(!layout||!document.documentElement)return;
    css();
    for(const name of ["player","chat"]){const r=layout[name];for(const [key,value] of Object.entries({x:r?.x??0,y:r?.y??0,w:r?.width??0,h:r?.height??0}))document.documentElement.style.setProperty(`--atsumi-${name}-${key}`,`${value}px`);document.documentElement.style.setProperty(`--atsumi-${name}-display`,r?"block":"none");}
  };
  async function boot() {
    if(booted||!nativeReady||!window.livePlayer||!window.liveView)return;
    booted=true;
    try {
      await window.liveView.isLoaded();
      // The official fromApi document intentionally has not called loadBroad.
      // This is its own supported message method, not a replacement receiver.
      window.postMessage({cmd:"Pload",id:channel,no:Number(match[2]||0),showChat:true,autoPlay:true,mutePlay:false,isAdShow:true,showQualityBox:true,fromApi:1},location.origin);
      applyLayout();
      const player=document.getElementById("player"), controls=player?.querySelector(".player_ctrlBox");
      if(player&&controls&&!controlsObserver){
        const sample=()=>{controlsFrame=0;let opacity=1;for(let node=controls;node&&node!==document.documentElement;node=node.parentElement){const style=getComputedStyle(node);if(style.display==="none"||style.visibility==="hidden"){opacity=0;break;}opacity*=Number(style.opacity)||0;}
          // The official controls slide below #player with opacity still 1.
          // Follow the actual original button's visible fraction, not a new idle timer.
          const button=controls.querySelector('.btn_fullScreen_mode'),r=button?.getBoundingClientRect(),p=player.getBoundingClientRect();
          if(!r||![r.left,r.top,r.right,r.bottom,r.width,r.height,p.left,p.top,p.right,p.bottom,p.width,p.height].every(Number.isFinite)||r.width<=0||r.height<=0||p.width<=0||p.height<=0)opacity=0;
          else opacity*=Math.max(0,Math.min(r.right,p.right)-Math.max(r.left,p.left))/r.width*Math.max(0,Math.min(r.bottom,p.bottom)-Math.max(r.top,p.top))/r.height;
          opacity=Math.round(Math.min(1,Math.max(0,opacity))*100)/100;renderControls(opacity);if(opacity!==lastOpacity){lastOpacity=opacity;notify("presentation",{opacity});}if(performance.now()<controlsUntil)controlsFrame=requestAnimationFrame(sample);};
        const follow=()=>{controlsUntil=performance.now()+600;if(!controlsFrame)controlsFrame=requestAnimationFrame(sample);};
        controlsObserver=new MutationObserver(follow);for(const node of [player,controls,document.body])if(node)controlsObserver.observe(node,{attributes:true,attributeFilter:["class","style"]});
        player.addEventListener("pointermove",follow,{passive:true});follow();
      }
    } catch { booted=false;notify("status",{state:"error",error:"SOOP 원본 플레이어를 준비하지 못했습니다."}); }
  }
  function status() {
    const video=document.getElementById("livePlayer"),capture=window.__atsumiSoopEncodedCapture;
    const active=video?.tagName==="VIDEO"&&video.readyState>=2&&video.videoWidth>0;
    let authenticated=null;try{authenticated=typeof window.liveView?.isLogin==="function"?Boolean(window.liveView.isLogin()):null;}catch{}
    const chat=document.getElementById("chatting_area"),rect=chat?.getBoundingClientRect();
    return {state:active?(video.ended?"ended":"playing"):booted?"loading":"preparing",authenticated,chatVisible:Boolean(rect&&rect.width>0&&rect.height>0),captureReady:bound()&&!!capture?.getStatus?.().ready,recording:!!capture?.getStatus?.().recording,recordingId:capture?.getStatus?.().recordingId??null,...binding()};
  }
  const heartbeat=setInterval(()=>{void boot();notify("status",status());},1500);
  const playerInput = event => event.isTrusted && event.target?.closest?.("#player") && !event.target?.closest?.('#chatting_area,input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]');
  document.addEventListener("pointerdown", event => {
    if(pointerDiagnostics<16&&event.isTrusted&&event.button===0&&epoch&&bound()&&event.target?.closest?.('#player')){
      const kind=event.target.closest?.('[data-atsumi-live-action]')?.dataset?.atsumiLiveAction;
      const target=['record','camera','close'].includes(kind)?kind:'player_other';pointerDiagnostics++;notify('ui_pointer',{target});
    }
    if(event.button!==0 || !playerInput(event) || !event.target?.closest?.(".btn_fullScreen_mode"))return;
    event.preventDefault();event.stopImmediatePropagation();notify("fullscreen",{enabled:!nativeFullscreen,input:"pointer"});
  },true);
  document.addEventListener("click", event => {if(playerInput(event)&&event.target?.closest?.(".btn_fullScreen_mode")){event.preventDefault();event.stopImmediatePropagation();}},true);
  document.addEventListener("keydown", event => {
    if(!playerInput(event)||event.defaultPrevented||event.repeat||event.isComposing||event.keyCode===229||event.altKey||event.ctrlKey||event.metaKey||event.shiftKey)return;
    const key=String(event.key).toLowerCase();
    if(key==="f"||key==="escape"&&nativeFullscreen){event.preventDefault();event.stopImmediatePropagation();notify("fullscreen",{enabled:key==="escape"?false:!nativeFullscreen,input:key});}
    else if(key==="s"&&!event.target?.closest?.("button,a")){event.preventDefault();event.stopImmediatePropagation();notify("screenshot_shortcut",{input:"s"});}
  },true);
  window.addEventListener("pagehide",()=>{clearInterval(heartbeat);controlsObserver?.disconnect();if(controlsFrame)cancelAnimationFrame(controlsFrame);for(const item of pending.values()){clearTimeout(item.timer);item.reject(new Error("page_changed"));}pending.clear();});
  Object.defineProperty(window,"__atsumiSoopOriginal",{value:Object.freeze({
    configure(value){if(!value||!Number.isSafeInteger(value.epoch)||value.epoch<1||value.channel!==channel)return false;epoch=value.epoch;nativeReady=true;void boot();return true;},
    layout(value){layout=value;applyLayout();},
    controls(value){if(!epoch||value?.epoch!==epoch)return false;ui={canClose:value.canClose===true,captureBusy:value.captureBusy===true,canRecord:value.canRecord===true,recording:value.recording===true,recordBusy:value.recordBusy===true};renderControls();return true;},
    resume(value){if(value!==epoch||!bound()||!layout?.visible||!layout.player)return false;const video=document.getElementById('livePlayer');if(video?.tagName==='VIDEO'&&video.paused&&!video.ended)void video.play().catch(()=>notify('status',{...status(),error:'원본 플레이어에서 재생을 눌러 주세요.'}));return true;},
    fullscreen(enabled){nativeFullscreen=enabled===true;const button=document.querySelector?.(".btn_fullScreen_mode");if(button){button.setAttribute("aria-pressed",String(nativeFullscreen));button.setAttribute("title",nativeFullscreen?"전체화면 종료 (F)":"전체화면 (F)");}},
    reply(id,result){const item=pending.get(id);if(!item)return;pending.delete(id);clearTimeout(item.timer);if(result?.ok)item.resolve(result.value);else item.reject(Object.assign(new Error(result?.error||"native_rejected"),{code:result?.code||"NATIVE_REJECTED"}));},
    start(requestId){if(!bound())return Promise.reject(new Error("broadcast_changed"));const video=document.getElementById("livePlayer");return window.__atsumiSoopEncodedCapture?.start({requestId,channelId:channel,rightsAcknowledged:true,video},{request:requests,onStatus:()=>notify("status",status())}).catch(()=>notify("status",{...status(),error:"원본 영상 저장을 시작하지 못했습니다."}));},
    stop(){return window.__atsumiSoopEncodedCapture?.stop("user_stop",false);},
    status,
  }),configurable:false});
  document.addEventListener("DOMContentLoaded",()=>{applyLayout();void boot();},{once:true});
})();
