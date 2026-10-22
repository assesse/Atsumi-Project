// Offline synthetic mockups only. No connection to user browser, account or app.
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const source = 'C:/Users/JJH/.codex/visualizations/2026/09/10/01a089ee-56ed-7322-a750-c034423826c2/x-feed-layouts.html';
const fragment = await fs.readFile(source, 'utf8');
assert.ok(!fragment.includes('\\"') && !fragment.includes('\\n'));
assert.ok(!/<(?:html|body|head)\b|<!doctype/i.test(fragment));
for(const script of fragment.matchAll(/<script>([\s\S]*?)<\/script>/g)) new Function(script[1]);
const artifacts = path.resolve('.runtime/pinterest-x-20261018/mockups', String(Date.now()));
await fs.mkdir(artifacts, { recursive: true });
const server = http.createServer((req,res)=>{ res.setHeader('Content-Type','text/html;charset=utf-8');res.setHeader('Content-Security-Policy',"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'");res.end(`<html style="color-scheme:dark"><head><style>body{margin:0} [hidden]{display:none!important} i[data-lucide]{display:inline-block;width:16px;height:16px;border:1px solid currentColor;border-radius:4px;opacity:.6}</style></head><body>${fragment}</body></html>`); });
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const directory=await fs.mkdtemp(path.join(os.tmpdir(),'atsumi-x-mockup-qa-')), profile=path.join(directory,'profile');
const child=spawn('C:/Program Files/Google/Chrome/Application/chrome.exe',['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1',`--user-data-dir=${profile}`,'--remote-debugging-port=0','--remote-debugging-address=127.0.0.1','about:blank'],{windowsHide:true,stdio:'ignore'});
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms)), pending=new Map(), report={scenes:[],errors:[],passed:false,icons:'Host-supplied Lucide absent offline;16px placeholders used for geometry. Inline conversation uses real host icons.'};
let socket,send,page;
try{
  let port,endpoint;for(let i=0;i<100;i++){try{[port,endpoint]=(await fs.readFile(path.join(profile,'DevToolsActivePort'),'utf8')).trim().split(/\r?\n/);if(port&&endpoint)break;}catch{}if(child.exitCode!==null)throw Error('Offline browser failed');await pause(100);}
  assert.match(port??'',/^\d+$/);socket=new WebSocket(`ws://127.0.0.1:${port}${endpoint}`);await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});let id=0;
  socket.onmessage=e=>{const m=JSON.parse(e.data),call=pending.get(m.id);if(!call)return;clearTimeout(call.timer);pending.delete(m.id);m.error?call.reject(Error(JSON.stringify(m.error))):call.resolve(m.result);};
  send=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);reject(Error(`Timeout: ${method}`));},15000);pending.set(next,{resolve,reject,timer});socket.send(JSON.stringify({id:next,method,params,...(sessionId?{sessionId}:{})}));});
  const {targetId}=await send('Target.createTarget',{url:'about:blank'}),{sessionId}=await send('Target.attachToTarget',{targetId,flatten:true});page=(method,params={})=>send(method,params,sessionId);
  const evaluate=async expression=>{const r=await page('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  await page('Page.enable');await page('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`});
  for(let i=0;i<100;i++){if(await evaluate("document.querySelectorAll('.ax-app').length===5"))break;await pause(50);}assert.equal(await evaluate("document.querySelectorAll('.ax-app').length"),5);
  for(const width of [1024,736,320])for(const theme of ['dark','light']){
    await page('Emulation.setDeviceMetricsOverride',{width,height:2600,deviceScaleFactor:1,mobile:false});await evaluate(`document.documentElement.style.colorScheme=${JSON.stringify(theme)}`);
    for(let index=0;index<5;index++){
      const result=await evaluate(`(()=>{const sections=[...document.querySelectorAll('#atsumi-x-designs > section')];sections.forEach((s,i)=>s.hidden=i!==${index});const s=sections[${index}], app=s.querySelector('.ax-app'), issues=[];for(const e of s.querySelectorAll('button,input')){if(!e.getClientRects().length||e.closest('[hidden]'))continue;const r=e.getBoundingClientRect();if(r.left<0||r.right>innerWidth+1)issues.push('outside:'+e.textContent);for(let p=e.parentElement;p&&p!==s;p=p.parentElement){const b=p.getBoundingClientRect();if(['hidden','clip'].includes(getComputedStyle(p).overflowX)&&(r.left<b.left-1||r.right>b.right+1))issues.push('clipped:'+e.textContent);}}return {name:s.dataset.variant,height:app.getBoundingClientRect().height,scroll:app.scrollWidth,client:app.clientWidth,issues};})()`);
      report.scenes.push({width,theme,...result});const capture=await page('Page.captureScreenshot',{format:'png',clip:{x:0,y:0,width,height:Math.ceil(result.height),scale:1},captureBeyondViewport:true});await fs.writeFile(path.join(artifacts,`${width}-${theme}-${index+1}.png`),Buffer.from(capture.data,'base64'));assert.deepEqual(result.issues,[]);assert.ok(result.scroll<=result.client+1);
    }
  }
  assert.equal(await evaluate("(()=>{const s=document.querySelectorAll('#atsumi-x-designs > section')[3];s.querySelector('[data-post=\"1\"]').click();return s.querySelector('.ax-reader .ax-name').textContent;})()"),'윤슬');
  assert.equal(await evaluate("(()=>{const s=document.querySelectorAll('#atsumi-x-designs > section')[0],b=s.querySelector('.ax-save');b.click();return b.getAttribute('aria-pressed');})()"),'true');
  assert.equal(await evaluate("(()=>{const s=document.querySelectorAll('#atsumi-x-designs > section')[4];s.querySelector('[data-creator=\"2\"]').click();return s.querySelector('.ax-collection-header .ax-name').textContent;})()"),'이든');
  report.passed=true;
}catch(error){report.errors.push(String(error));process.exitCode=1;if(page){try{const c=await page('Page.captureScreenshot',{format:'png'});await fs.writeFile(path.join(artifacts,'failure.png'),Buffer.from(c.data,'base64'));}catch{}}}
finally{await fs.writeFile(path.join(artifacts,'verification.json'),JSON.stringify(report,null,2));if(socket?.readyState===WebSocket.OPEN)await send('Browser.close').catch(()=>{});for(const call of pending.values())clearTimeout(call.timer);socket?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));console.log(JSON.stringify({passed:report.passed,artifacts,errors:report.errors,heights:report.scenes.filter(s=>s.theme==='dark').map(s=>[s.width,s.name,s.height])}));}
