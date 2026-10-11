import * as fs from "node:fs";
import * as path from "node:path";
import { tmpdir } from "node:os";
import { pathToFileURL } from "node:url";
import { spawn } from "node:child_process";
const { scenes, css, width, height, browser, artifacts, captureAll } = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const directory = fs.mkdtempSync(path.join(tmpdir(), "atsumi-global-layout-browser-")), profile = path.join(directory, "profile");
if (artifacts) fs.mkdirSync(artifacts, { recursive: true });
const child = spawn(browser, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND", "--user-data-dir="+profile, "--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "about:blank"], { windowsHide: true, stdio: "ignore" });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
let socket, closeBrowser; const pending = new Map();
const script = `
    (async()=>{const results=[];
    for(const scene of scenes){
      document.body.innerHTML=scene.html;
      for(const dialog of document.querySelectorAll('dialog[open]')){dialog.removeAttribute('open');dialog.showModal();}
      await new Promise(resolve=>setTimeout(resolve,50));
      const issues=[];
      const visible=el=>{const s=getComputedStyle(el);return el.getClientRects().length&&s.visibility!=='hidden'&&s.opacity!=='0'&&!el.closest('[hidden]');};
      const label=el=>(el.getAttribute('aria-label')||el.textContent||el.className).toString().trim().slice(0,65);
      const bounds=el=>el.getBoundingClientRect();
      const modal=[...document.querySelectorAll('dialog[open]')].find(visible);
      const scope=modal||document;
      for(const el of scope.querySelectorAll('button,input,select,textarea,[role="tab"],summary')){
        if(!visible(el)||el.matches('[type="checkbox"],[type="radio"],.sr-only'))continue;
        const rect=bounds(el);if(rect.width<=1||rect.height<=1)continue;
        let xScrollable=false,yScrollable=false,hiddenX=false,hiddenY=false;
        for(let p=el.parentElement;p&&p!==document.body;p=p.parentElement){
          const s=getComputedStyle(p),r=bounds(p);
          if(['auto','scroll'].includes(s.overflowX))xScrollable=true;
          if(['auto','scroll'].includes(s.overflowY))yScrollable=true;
          if(!xScrollable&&['hidden','clip'].includes(s.overflowX)&&(rect.left<r.left-2||rect.right>r.right+2))hiddenX=true;
          if(!yScrollable&&['hidden','clip'].includes(s.overflowY)&&(rect.top<r.top-2||rect.bottom>r.bottom+2))hiddenY=true;
        }
        if(hiddenX||hiddenY||(!xScrollable&&(rect.left< -2||rect.right>innerWidth+2))||(!yScrollable&&(rect.top< -2||rect.bottom>innerHeight+2)))issues.push('clipped control: '+label(el));
      }
      for(const el of document.querySelectorAll('.app-shell,dialog[open],.detail-workspace,.activity-panel,.workspace,.community-shell,.personal-library,.official-browser-library-layout')){
        if(!visible(el))continue;const r=bounds(el);
        if(r.left< -2||r.right>innerWidth+2)issues.push('outside viewport: '+el.className);
        if(el.matches('dialog,.detail-workspace,.activity-panel')&&(r.top< -2||r.bottom>innerHeight+2))issues.push('outside height: '+el.className);
        if(el.scrollWidth>el.clientWidth+2)issues.push('horizontal overflow: '+el.className+' '+el.scrollWidth+'/'+el.clientWidth);
      }
      for(const el of document.querySelectorAll('.gallery-viewport')){
        if(!modal&&visible(el)&&bounds(el).height<100)issues.push('results viewport collapsed: '+Math.round(bounds(el).height));
      }
      for(const card of document.querySelectorAll('.download-artist-folder-card')){
        const button=card.querySelector('.download-artist-folder-button'),stack=card.querySelector('.download-artist-folder-preview-stack'),cover=card.querySelector('.download-artist-folder-preview');
        if(!button||!visible(card))continue;
        const r=bounds(button);
        for(const tag of card.querySelectorAll('.download-artist-folder-tag')){const t=bounds(tag);if(t.top<r.top||t.bottom>r.bottom-1||t.right>r.right)issues.push('clipped artist folder tag: '+label(tag));}
        if(stack&&cover){const a=bounds(stack),b=bounds(cover);if(Math.abs(a.left-b.left)>1||Math.abs(a.top-b.top)>1||Math.abs(a.width-1-b.width)>2||Math.abs(a.height-b.height)>1)issues.push('artist folder cover does not fill frame');}
      }
      for(const el of document.querySelectorAll('.download-artist-folder-window')){const r=bounds(el);if(r.left<0||r.top<0||r.right>innerWidth||r.bottom>innerHeight||el.scrollWidth>el.clientWidth+2)issues.push('artist folder window outside viewport');}
      for(const el of scope.querySelectorAll('.connection-channel-add,.connection-channel-clear,.connection-channel-favorite,.live-channel-actions>button,.auto-record-manager-close,.nav-live-disclosure')){
        const icon=el.querySelector('.fluent,svg');if(!visible(el)||!icon)continue;
        const a=bounds(el),b=bounds(icon);
        if(Math.abs(a.x+a.width/2-b.x-b.width/2)>1||Math.abs(a.y+a.height/2-b.y-b.height/2)>1)issues.push('off-center live control: '+label(el));
        if(el.matches('.connection-channel-clear')){
          const input=el.parentElement.querySelector('input'),r=bounds(input),style=getComputedStyle(el);
          if(a.left<r.left||a.right>r.right||a.top<r.top||a.bottom>r.bottom)issues.push('channel clear outside input');
          if(parseFloat(style.borderTopWidth)!==0||style.backgroundColor!=='rgba(0, 0, 0, 0)')issues.push('channel clear must be borderless');
        }
      }
      for(const item of scope.querySelectorAll('.nav-live-item')){
        const main=item.querySelector('.nav-item'),arrow=item.querySelector('.nav-live-disclosure');
        if(!main||!arrow||!visible(item))continue;
        for(const el of [main,arrow]){const r=bounds(el),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);if(!el.contains(hit))issues.push('live navigation hitbox obscured: '+label(el));}
        const r=bounds(item),a=bounds(arrow);if(a.left<r.left||a.right>r.right||a.top<r.top||a.bottom>r.bottom)issues.push('live arrow outside navigation row');
      }
      for(const list of scope.querySelectorAll('.live-channel-list')){
        const rows=[...list.querySelectorAll('.live-channel-row')];if(!rows.length||!visible(list))continue;
        const rowHeight=bounds(rows[0]).height,gap=parseFloat(getComputedStyle(list).rowGap),columns=rows.filter(row=>Math.abs(bounds(row).top-bounds(rows[0]).top)<1).length;
        const neededRows=Math.ceil(rows.length/columns),expected=Math.min(4,neededRows)*rowHeight+(Math.min(4,neededRows)-1)*gap;
        if(Math.abs(list.clientHeight-expected)>2)issues.push('favorite channels must grow to exactly four rows');
        if(neededRows>4&&list.scrollHeight<=list.clientHeight)issues.push('favorite channels after row four must scroll');
        if(neededRows<=4&&list.scrollHeight>list.clientHeight+2)issues.push('favorite channels scroll before row four');
        for(const row of rows){const r=bounds(row);for(const el of row.querySelectorAll('.live-channel-identity,.live-channel-actions')){const b=bounds(el);if(b.top<r.top||b.bottom>r.bottom||b.left<r.left||b.right>r.right)issues.push('favorite channel content outside row');}}
      }
      for(const row of scope.querySelectorAll('.connection-actions.is-unified')){
        if(!visible(row))continue;
        const modes=row.querySelector('.mado-layout-options'),actions=row.querySelector('.connection-session-actions');
        if(modes&&actions){const a=bounds(modes),b=bounds(actions),r=bounds(row);if(a.right>b.left+1||Math.abs(b.right-r.right)>2||a.top>b.bottom||b.top>a.bottom)issues.push('live mode and session actions must share a row with actions right-aligned');}
      }
      for(const el of scope.querySelectorAll('.live-edge-panel.is-enabled .live-edge-overlay .brand,.live-edge-panel.is-enabled .live-edge-overlay .nav-item,.live-edge-panel.is-enabled .live-edge-overlay .mado-toolbar button')){
        if(!visible(el))continue;const r=bounds(el),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
        if(!el.contains(hit))issues.push('covered live edge control: '+label(el));
      }
      results.push({name:scene.name,width:innerWidth,height:innerHeight,issues:[...new Set(issues)]});
    }
    return results;})();
  `;
const html = frames => '<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'; img-src data:"><style>'+css+'\n*,*::before,*::after{animation:none!important;transition:none!important}</style></head><body>'+(frames[0]?.html??"")+'</body></html>';
try {
  const portFile = path.join(profile, "DevToolsActivePort");
  let port, endpoint;
  for(let attempt=0;attempt<100;attempt++){
    if(child.exitCode!==null)throw Error("Offline browser did not start");
    try {
      const lines=fs.readFileSync(portFile,"utf8").trim().split(/\r?\n/);
      if(/^\d+$/.test(lines[0]??"")&&lines[1]?.startsWith("/devtools/browser/")){[port,endpoint]=lines;break;}
    } catch(error) {
      // Chrome may have created the file but still hold its Windows write lock.
      if(!["ENOENT","EBUSY","EPERM","EACCES"].includes(error.code))throw error;
    }
    await pause(100);
  }
  if(!port||!endpoint)throw Error("Offline browser did not publish its debug endpoint");
  socket=new WebSocket("ws://127.0.0.1:"+port+endpoint);
  await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error("Offline browser handshake timeout")),10000);socket.onopen=()=>{clearTimeout(timer);resolve();};socket.onerror=()=>{clearTimeout(timer);reject(Error("Offline browser connection failed"));};});
  let id=0;
  socket.onmessage=event=>{const msg=JSON.parse(String(event.data)),call=pending.get(msg.id);if(!call)return;pending.delete(msg.id);clearTimeout(call.timer);if(msg.error)call.reject(Error(JSON.stringify(msg.error)));else call.resolve(msg.result);};
  const send=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);reject(Error("CDP timeout: "+method));},15000);pending.set(next,{resolve,reject,timer});socket.send(JSON.stringify({id:next,method,params,...(sessionId?{sessionId}:{})}));});
  closeBrowser = () => send("Browser.close");
  const target=await send("Target.createTarget",{url:"about:blank"});
  const {sessionId}=await send("Target.attachToTarget",{targetId:target.targetId,flatten:true});
  const page=(method,params={})=>send(method,params,sessionId);
  await page("Page.enable");
  await page("Emulation.setDeviceMetricsOverride",{width,height,deviceScaleFactor:1,mobile:false});
  const fixture=path.join(directory,"fixture.html");
  fs.writeFileSync(fixture,html(scenes));
  await page("Page.navigate",{url:pathToFileURL(fixture).href});await pause(200);
  const evaluate=async frames=>{const result=await page("Runtime.evaluate",{expression:"(()=>{const scenes="+JSON.stringify(frames)+";return "+script.trim()+"})()",awaitPromise:true,returnByValue:true});if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));return result.result.value;};
  const results=await evaluate(scenes);
  if(!results?.length||results.some(r=>r.width!==width||r.height!==height))throw Error("Requested viewport was not applied");
  if(artifacts){
    fs.writeFileSync(path.join(artifacts,width+"x"+height+".json"),JSON.stringify(results,null,2));
    for(const result of results){
      if(!result.issues.length&&!captureAll)continue;
      const frame=scenes.find(s=>s.name===result.name),name=width+"x"+height+"-"+frame.name;
      fs.writeFileSync(path.join(artifacts,name+".html"),html([frame]));await evaluate([frame]);
      const screenshot=await page("Page.captureScreenshot",{format:"png",captureBeyondViewport:false});
      fs.writeFileSync(path.join(artifacts,name+".png"),Buffer.from(screenshot.data,"base64"));
    }
  }
  process.stdout.write(JSON.stringify(results));
} finally {
  // Let Chromium release its profile handles before removing our fixture.
  if (socket?.readyState === WebSocket.OPEN) await closeBrowser?.().catch(() => {});
  for(const call of pending.values())clearTimeout(call.timer);
  socket?.close();
  // Only terminate the disposable browser that this test created.
  if(child.exitCode===null){const exited=new Promise(resolve=>child.once("exit",resolve));await Promise.race([exited,pause(3000)]);if(child.exitCode===null){child.kill();await Promise.race([exited,pause(3000)]);}}
  const target=path.relative(fs.realpathSync(tmpdir()),fs.realpathSync(directory));
  if(!target.startsWith("atsumi-global-layout-browser-")||target.includes("..")||path.isAbsolute(target))throw Error("Unexpected fixture cleanup target");
  try {
    fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  } catch (error) {
    // Antivirus/Chromium may retain a Windows handle after process exit. Do not
    // turn a valid measurement into a layout failure or touch another profile.
    if (!["EPERM", "EBUSY", "ENOTEMPTY"].includes(error.code)) throw error;
    process.stderr.write(`Retained locked disposable QA profile: ${directory}\n`);
  }
}
