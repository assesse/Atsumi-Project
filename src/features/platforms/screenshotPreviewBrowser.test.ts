// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { build, transform } from 'esbuild';

const fsName = 'node:fs', pathName = 'node:path', osName = 'node:os', childName = 'node:child_process', processName = 'node:process';
const fs = await import(fsName) as { existsSync(path: string): boolean; readFileSync(path: string, encoding: 'utf8'): string; mkdtempSync(prefix: string): string; writeFileSync(path: string, content: string): void; realpathSync(path: string): string; rmSync(path: string, options: { recursive: boolean; force: boolean; maxRetries: number; retryDelay: number }): void };
const path = await import(pathName) as { join(...parts: string[]): string; relative(from: string, to: string): string; isAbsolute(path: string): boolean };
const os = await import(osName) as { tmpdir(): string };
const child = await import(childName) as { execFile(file: string, args: string[], options: { timeout: number; maxBuffer: number; encoding: 'utf8'; windowsHide: boolean }, callback: (error: Error | null, stdout: string, stderr: string) => void): void };
const runtime = await import(processName) as { env: Record<string, string | undefined>; execPath: string; cwd(): string };
const browser = [runtime.env.ATSUMI_TEST_BROWSER ?? '', 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe', 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe'].find(candidate => fs.existsSync(candidate));

function clockExpression(source: string) {
  const call = /const (?:timer|lifetime)\s*=\s*createPreviewLifetime\(\s*/.exec(source);
  if (!call) throw new Error('Production preview timer call missing');
  const start = call.index + call[0].length;
  if (source[start] !== '{') throw new Error('Production preview clock is not an object');
  let depth = 0;
  for (let at = start; at < Math.min(source.length, start + 1024); at++) {
    if (source[at] === '{') depth++;
    if (source[at] === '}' && --depth === 0) return source.slice(start, at + 1);
  }
  throw new Error('Production preview clock exceeds fixture bound');
}

// Native browser globals must run outside jsdom. This constant child uses the
// same offline/temp-profile ownership as tools/layout-browser.mjs, never an app.
const browserChild = String.raw`
import * as fs from 'node:fs';import * as path from 'node:path';import {spawn} from 'node:child_process';
const {browser,expression}=JSON.parse(fs.readFileSync(process.argv[1],'utf8'));
const directory=path.dirname(process.argv[1]),profile=path.join(directory,'profile');
const child=spawn(browser,['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND','--user-data-dir='+profile,'--remote-debugging-port=0','--remote-debugging-address=127.0.0.1','about:blank'],{windowsHide:true,stdio:'ignore'});
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));let socket,send;const pending=new Map();
try{
  let port,endpoint;
  for(let n=0;n<100;n++){
    if(child.exitCode!==null)throw Error('Offline browser did not start');
    try{const lines=fs.readFileSync(path.join(profile,'DevToolsActivePort'),'utf8').trim().split(/\r?\n/);if(/^\d+$/.test(lines[0]??'')&&lines[1]?.startsWith('/devtools/browser/')){[port,endpoint]=lines;break;}}
    catch(error){if(!['ENOENT','EBUSY','EPERM','EACCES'].includes(error.code))throw error;}await pause(100);
  }
  if(!port||!endpoint)throw Error('Offline browser endpoint unavailable');
  socket=new WebSocket('ws://127.0.0.1:'+port+endpoint);
  await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Offline handshake timeout')),10000);socket.onopen=()=>{clearTimeout(timer);resolve();};socket.onerror=()=>{clearTimeout(timer);reject(Error('Offline handshake failed'));};});
  let id=0;socket.onmessage=event=>{const message=JSON.parse(String(event.data)),call=pending.get(message.id);if(!call)return;pending.delete(message.id);clearTimeout(call.timer);if(message.error)call.reject(Error('Offline protocol failure'));else call.resolve(message.result);};
  send=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);reject(Error('Offline protocol timeout'));},15000);pending.set(next,{resolve,reject,timer});socket.send(JSON.stringify({id:next,method,params,...(sessionId?{sessionId}:{})}));});
  const target=await send('Target.createTarget',{url:'about:blank'}),{sessionId}=await send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  const response=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true},sessionId);
  if(response.exceptionDetails)throw Error('Offline fixture expression failed');
  process.stdout.write(JSON.stringify(response.result.value));
}finally{
  if(socket?.readyState===WebSocket.OPEN)await send?.('Browser.close').catch(()=>{});
  for(const call of pending.values())clearTimeout(call.timer);socket?.close();
  if(child.exitCode===null){const exited=new Promise(resolve=>child.once('exit',resolve));await Promise.race([exited,pause(3000)]);if(child.exitCode===null){child.kill();await Promise.race([exited,pause(3000)]);}}
}`;

type Result = { rawSet: boolean; rawClear: boolean; adapters: { name: string; throws: boolean; disposedCallbacks: number }[]; mounted: boolean; undo: number; dismiss: number; errors: number };
async function probe(expression: string): Promise<Result> {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'atsumi-preview-browser-test-'));
  try {
    const fixture = path.join(directory, 'input.json');
    fs.writeFileSync(fixture, JSON.stringify({ browser, expression }));
    const output = await new Promise<string>((resolve, reject) => child.execFile(runtime.execPath, ['--input-type=module', '-e', browserChild, fixture], { timeout: 60000, maxBuffer: 65536, encoding: 'utf8', windowsHide: true }, (error, stdout, stderr) => error ? reject(new Error(stderr || error.message)) : resolve(stdout)));
    return JSON.parse(output) as Result;
  } finally {
    const relative = path.relative(fs.realpathSync(os.tmpdir()), fs.realpathSync(directory));
    if (!relative.startsWith('atsumi-preview-browser-test-') || relative.includes('..') || path.isAbsolute(relative)) throw new Error('Unexpected disposable cleanup target');
    try { fs.rmSync(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
    catch (error) { if (!['EPERM', 'EBUSY', 'ENOTEMPTY'].includes((error as { code?: string }).code ?? '')) throw error; }
  }
}

describe.skipIf(!browser)('screenshot preview native browser timers', () => {
  it('rejects raw native method receivers and mounts both current adapters without changing lifecycle policy', async () => {
    const root = runtime.cwd();
    const component = fs.readFileSync(path.join(root, 'src/features/platforms/ScreenshotPreview.tsx'), 'utf8');
    const injected = fs.readFileSync(path.join(root, 'src-tauri/src/streaming/browser_player_ui.js'), 'utf8');
    const source = fs.readFileSync(path.join(root, 'src/features/platforms/screenshotPreviewLifetime.ts'), 'utf8');
    const lifetime = (await transform(source.replace('export function ', 'function ') + '\nglobalThis.fixtureLifetime = createPreviewLifetime;', { loader: 'ts', format: 'iife' })).code;
    const bundle = (await build({ stdin: { contents: `import React from 'react';import {createRoot} from 'react-dom/client';import {ScreenshotPreview} from './src/features/platforms/ScreenshotPreview';globalThis.fixtureRoot=createRoot(document.getElementById('root'));globalThis.fixtureRoot.render(React.createElement(ScreenshotPreview,{preview:{id:'fixture',url:'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',undoing:false,error:'',undoDeadline:performance.now()+45000},onDismiss:()=>globalThis.fixtureDismiss++,onUndo:()=>globalThis.fixtureUndo++}));`, resolveDir: root, loader: 'tsx' }, bundle: true, write: false, format: 'iife', jsx: 'automatic', define: { 'process.env.NODE_ENV': '"production"' } })).outputFiles[0]!.text;
    const result = await probe(`(async()=>{'use strict';${lifetime}
      const output={rawSet:false,rawClear:false,adapters:[],mounted:false,undo:0,dismiss:0,errors:0};
      const raw={set:setTimeout,clear:clearTimeout},illegal=error=>error instanceof TypeError&&String(error.message).includes('Illegal invocation');
      try{raw.set(()=>{},0);}catch(error){output.rawSet=illegal(error);}
      const token=window.setTimeout(()=>{},1000);try{raw.clear(token);}catch(error){output.rawClear=illegal(error);}finally{window.clearTimeout(token);}
      for(const [name,clock] of [['react',${clockExpression(component)}],['chzzk',${clockExpression(injected)}]]){
        let callbacks=0;try{const timer=fixtureLifetime(clock,performance.now()+45000,{expire(){callbacks++;},dismiss(){callbacks++;}});timer.hover(true);timer.hover(false);timer.focus(true);timer.focus(false);timer.block(true);timer.block(false);timer.dispose();await new Promise(done=>window.setTimeout(done,10));output.adapters.push({name,throws:false,disposedCallbacks:callbacks});}catch{output.adapters.push({name,throws:true,disposedCallbacks:callbacks});}
      }
      document.body.innerHTML='<div id="root" style="position:relative;width:640px;height:400px"></div>';
      window.addEventListener('error',event=>{output.errors++;event.preventDefault();});globalThis.fixtureUndo=0;globalThis.fixtureDismiss=0;
      ${bundle}
      await new Promise(done=>window.setTimeout(done,100));
      output.mounted=!!document.querySelector('[aria-label="방금 저장한 스크린샷"]');document.querySelector('.platform-live-capture-cancel')?.click();document.querySelector('.platform-live-capture-dismiss')?.click();
      output.undo=globalThis.fixtureUndo;output.dismiss=globalThis.fixtureDismiss;globalThis.fixtureRoot.unmount();
      return output;
    })()`);
    expect(result.rawSet).toBe(true);
    expect(result.rawClear).toBe(true);
    expect(result.adapters).toEqual([{ name: 'react', throws: false, disposedCallbacks: 0 }, { name: 'chzzk', throws: false, disposedCallbacks: 0 }]);
    expect(result.mounted).toBe(true);
    expect(result.undo).toBe(1);
    expect(result.dismiss).toBe(1);
    expect(result.errors).toBe(0);
  }, 90000);
});
