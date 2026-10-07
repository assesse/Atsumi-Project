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
      results.push({name:scene.name,width:innerWidth,height:innerHeight,issues:[...new Set(issues)]});
    }
    return results;})();
  `;
const html = frames => '<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'; img-src data:"><style>'+css+'\n*,*::before,*::after{animation:none!important;transition:none!important}</style></head><body>'+(frames[0]?.html??"")+'</body></html>';
try {
  const portFile = path.join(profile, "DevToolsActivePort");
  for(let attempt=0;!fs.existsSync(portFile);attempt++){if(attempt>=100||child.exitCode!==null)throw Error("Offline browser did not start");await pause(100);}
  const [port,endpoint]=fs.readFileSync(portFile,"utf8").trim().split(/\r?\n/);
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
