// Original SDK only: no CHZZK application, account, advertising or live API code.
import originalVodTemplate from './accepted-vod-template.js';
const CHANNEL='atsumi-replay-player-v1', nonce=location.hash.slice(1);
// This public file must never initialize in a privileged app page or a
// same-origin frame, even if someone opens its URL directly.
if(window===window.top||window.origin!=='null'||!/^[a-f0-9]{32}$/.test(nonce))throw new Error('Original player requires an opaque, scoped frame');
const parents=new Set(['http://tauri.localhost','https://tauri.localhost','tauri://localhost','http://127.0.0.1:1420']);
let player=null,privateMode=false,disposed=false,metrics=null,lastState='',clock=0,presentation={wide:false,fullscreen:false},recordingHeader=null;
// The SDK expects Web Storage even for local playback. Opaque frames intentionally
// have none: provide a bounded, recording-local memory store, never browser cookies/storage.
function memoryStorage(){const entries=new Map();return {get length(){return entries.size;},key:index=>[...entries.keys()][index]??null,getItem:key=>entries.get(String(key))??null,setItem(key,value){key=String(key);value=String(value);if(key.length<=256&&value.length<=65536&&(entries.has(key)||entries.size<100))entries.set(key,value);},removeItem:key=>entries.delete(String(key)),clear:()=>entries.clear()};}
for(const name of ['localStorage','sessionStorage'])Object.defineProperty(window,name,{value:memoryStorage(),configurable:false});
const sdk=await import('./player-vendor-BYg0wCyN.js');
const metadata=await import('./recording-metadata.js');
const localSource=await import('./recording-source.js');
let recordingSource=null,sourceUrl='';
const emit=(type,data)=>{if(!disposed)parent.postMessage({channel:CHANNEL,nonce,type,data},'*');};
const finite=(n,min,max)=>typeof n==='number'&&Number.isFinite(n)&&n>=min&&n<=max;
function mediaAllowed(value,parentOrigin){try{
 const url=new URL(value);
 if(url.username||url.password||url.search||url.hash)return false;
 if((['http:','https:'].includes(url.protocol)&&url.hostname==='atsumi-replay.localhost'&&!url.port||url.protocol==='atsumi-replay:'&&url.hostname==='localhost')&&/^\/[a-f0-9]{32}$/.test(url.pathname))return true;
 return parentOrigin==='http://127.0.0.1:1420'&&url.origin===parentOrigin&&['/.runtime/chzzk-original-player/synthetic.mp4','/.runtime/progressive-preview'].includes(url.pathname);
}catch{return false;}}
function state(){
 if(!player||disposed)return;const v=document.querySelector('video');
 const data={time:Math.floor(Math.max(0,player.currentTime||0)*10)/10,duration:Number.isFinite(player.duration)?player.duration:0,paused:!!player.paused,seeking:!!v?.seeking,aspect:v?.videoWidth&&v?.videoHeight?v.videoWidth/v.videoHeight:16/9};
 const encoded=JSON.stringify(data);if(encoded!==lastState){lastState=encoded;emit('state',data);}
}
async function leavePip(){try{if(document.pictureInPictureElement)await document.exitPictureInPicture();}catch{}}
function privacy(value){privateMode=value;if(value){player?.pause();void leavePip();}state();}
function stop(){if(disposed)return;disposed=true;privateMode=true;clearInterval(clock);clock=0;metrics=null;recordingSource?.dispose();recordingHeader?.dispose();recordingHeader=null;try{player?.pause();}catch{}void leavePip();try{if(player)player.srcObject=null;}catch{}}
function applyPresentation(){
 if(!player||disposed)return;
 // These are the SDK's reflected visual properties, not its native fullscreen
 // action. Skip unchanged values: the official viewmode setter emits change.
 const wide=player.querySelector('pzp-pc-viewmode-button');if(wide&&wide.checked!==presentation.wide)wide.checked=presentation.wide;
 const fullscreen=player.querySelector('pzp-fullscreen-button');if(fullscreen&&fullscreen.fullscreen!==presentation.fullscreen)fullscreen.fullscreen=presentation.fullscreen;
}
function mount(data,parentOrigin){
 const source=localSource.recordingSource(data,url=>mediaAllowed(url,parentOrigin));if(!source)return;
 if(player){if(source.url!==sourceUrl){recordingSource.update(source);sourceUrl=source.url;emit('source',sourceUrl);state();}return;}
 privateMode=!!data.privacy;
 document.documentElement.className='theme_dark';
 // Static, hash-pinned markup mechanically extracted from CHZZK's actual VOD component.
 document.body.innerHTML=originalVodTemplate;
 const P=sdk.S();player=P.default.upgrade(document.querySelector('pzp-pc-layout'));
 player.language='ko';player.querySelector('pzp-pc-layout').sizeType='large';
 player.querySelector('pzp-pc-setting-playbackrate-pane').playbackRates=[.25,.5,.75,1,1.25,1.5,1.75,2];
 recordingHeader=metadata.mountRecordingMetadata(player,data.recording,()=>{if(!privateMode&&!disposed)emit('channel');});
 recordingSource=localSource.attachRecordingSource(document.querySelector('video'),source,()=>emit('tail'));
 sourceUrl=source.url;
 for(const name of ['loadedmetadata','playing','pause','seeking','seeked','timeupdate','durationchange','ended','ratechange'])player.addEventListener(name,state);
 player.addEventListener('play',()=>{if(privateMode||disposed)player.pause();});player.addEventListener('error',()=>emit('error'));
 // The decoded file supplies dimensions through loadedmetadata. Do not label
 // portrait/ultrawide recordings with the synthetic fixture's 1280x720 size.
 player.srcObject=new P.DataProvider({videoTracks:[{src:source.parts[0].url,id:'local',label:'원본',duration:source.duration,selected:true}],textTracks:[]});
 emit('source',sourceUrl);
 applyPresentation();
 player.shadowRoot.addEventListener('click',event=>{const button=event.target.closest('.pzp-pc__fullscreen-button,.pzp-pc__viewmode-button');if(button){event.preventDefault();event.stopImmediatePropagation();emit(button.classList.contains('pzp-pc__viewmode-button')?'wide':'fullscreen');}},true);
 document.addEventListener('enterpictureinpicture',()=>{if(privateMode||disposed)void leavePip();},true);
 for(const name of ['loadedmetadata','durationchange'])player.addEventListener(name,drawMetrics);
 attachMetrics();clock=setInterval(state,100);state();
}
function partialViewer(bucket,duration){return bucket?.viewerCount!=null&&finite(bucket.viewerCoverageSeconds,0,604800)&&bucket.viewerCoverageSeconds<Math.min(metrics.bucketSeconds,duration-bucket.startSeconds)-.01;}
function attachMetrics(){
 const slider=document.querySelector('.pzp-pc__progress-slider');if(!slider)return;
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('class','atsumi-replay-metrics');svg.setAttribute('viewBox','0 0 1000 32');svg.setAttribute('preserveAspectRatio','none');svg.setAttribute('aria-hidden','true');slider.prepend(svg);
 const tip=document.createElement('div');tip.className='atsumi-replay-metric-tip';tip.hidden=true;slider.append(tip);
 slider.addEventListener('pointermove',event=>{if(!metrics||!player||!finite(player.duration,.001,604800))return;const rect=slider.getBoundingClientRect();if(rect.width<=0)return;const fraction=Math.min(1,Math.max(0,(event.clientX-rect.left)/rect.width));const seconds=Math.min(player.duration-.001,fraction*player.duration);const bucket=metrics.buckets.find(b=>seconds>=b.startSeconds&&seconds<b.startSeconds+metrics.bucketSeconds);const count=(v,unit='명')=>!finite(v,0,Number.MAX_SAFE_INTEGER)?'기록 없음':Math.round(v).toLocaleString('ko-KR')+unit;const chat=metrics.participantCounts?`채팅 참여자 ${count(bucket?.uniqueSenderCount)}`:`채팅 수 ${count(bucket?.chatCount,'개')}`;tip.textContent=`${chat}\n평균 시청자 ${count(bucket?.viewerCount)}${partialViewer(bucket,player.duration)?' · 일부 기록':''}\n${metrics.bucketSeconds}초 구간 · 각 곡선은 자체 최대값 기준`;tip.style.left=Math.max(0,Math.min(rect.width-220,event.clientX-rect.left-110))+'px';tip.hidden=false;});
 slider.addEventListener('pointerleave',()=>{tip.hidden=true;});drawMetrics();
}
function drawMetrics(){
 const svg=document.querySelector('.atsumi-replay-metrics');if(!svg)return;svg.replaceChildren();if(!metrics)return;
 for(const [key,cls]of[['viewerPaths','viewers'],['chatPaths','chat']])for(const d of metrics[key]||[]){if(typeof d!=='string'||d.length>250000||!/^[MLCZ0-9., \-]+$/.test(d))continue;const path=document.createElementNS('http://www.w3.org/2000/svg','path');path.setAttribute('d',d);path.setAttribute('class',cls);svg.append(path);}
 const duration=player?.duration;if(!finite(duration,.001,604800))return;
 const buckets=metrics.buckets.filter(b=>finite(b.startSeconds,0,duration)&&b.startSeconds<duration&&finite(b.viewerCount,0,Number.MAX_SAFE_INTEGER));
 const peak=Math.max(1,...buckets.map(b=>b.viewerCount));
 for(const bucket of buckets){if(!partialViewer(bucket,duration))continue;const circle=document.createElementNS('http://www.w3.org/2000/svg','circle');circle.setAttribute('class','viewer-partial');circle.setAttribute('cx',String((bucket.startSeconds+Math.min(metrics.bucketSeconds,duration-bucket.startSeconds)/2)/duration*1000));circle.setAttribute('cy',String(32-bucket.viewerCount/peak*28));circle.setAttribute('r','2');svg.append(circle);}
}
window.addEventListener('message',event=>{
 const m=event.data;if(event.source!==parent||!parents.has(event.origin)||!m||m.channel!==CHANNEL||m.nonce!==nonce||disposed)return;
 try{switch(m.type){
  case'init':mount(m.data,event.origin);break;
  case'privacy':if(typeof m.data==='boolean')privacy(m.data);break;
  case'dispose':stop();break;
  case'seek':if(player&&finite(m.data,0,604800)){player.currentTime=Math.min(player.duration,m.data);state();}break;
  case'toggle':if(player&&!privateMode){if(player.paused)Promise.resolve(player.play()).catch(()=>emit('error'));else player.pause();}break;
  case'mute':if(player)player.muted=!player.muted;break;
  case'presentation':if(m.data&&typeof m.data.wide==='boolean'&&typeof m.data.fullscreen==='boolean'){presentation={wide:m.data.wide,fullscreen:m.data.fullscreen};applyPresentation();}break;
  case'metadata':recordingHeader?.update(m.data);break;
  case'metrics':if(m.data===null||m.data&&Array.isArray(m.data.buckets)&&m.data.buckets.length<=2000&&finite(m.data.bucketSeconds,1,604800)){metrics=m.data;drawMetrics();}break;
 }}catch{emit('error');}
});
document.addEventListener('keydown',event=>{if(event.altKey||event.ctrlKey||event.metaKey||event.target.closest('input,textarea,select,[contenteditable="true"]'))return;const key=event.key.toLowerCase();if(['f','t','escape'].includes(key)){event.preventDefault();event.stopImmediatePropagation();emit(key==='f'?'fullscreen':key==='t'?'wide':'close');}},true);
window.addEventListener('pagehide',stop);
if(/^[a-f0-9]{32}$/.test(nonce))emit('ready');
