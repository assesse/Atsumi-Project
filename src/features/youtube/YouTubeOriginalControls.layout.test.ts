import { expect, it } from 'vitest';
import adapter from '../../../src-tauri/src/youtube/original_page.js?raw';

const fsName='node:fs',pathName='node:path',osName='node:os',childName='node:child_process',urlName='node:url',processName='node:process';
const fs=await import(fsName) as {existsSync(path:string):boolean;mkdtempSync(prefix:string):string;writeFileSync(path:string,contents:string):void;realpathSync(path:string):string;rmSync(path:string,options:{recursive:boolean;force:boolean;maxRetries:number;retryDelay:number}):void;mkdirSync(path:string,options:{recursive:boolean}):void};
const path=await import(pathName) as {join(...parts:string[]):string;relative(from:string,to:string):string;isAbsolute(path:string):boolean};
const {tmpdir}=await import(osName) as {tmpdir():string};
const {pathToFileURL}=await import(urlName) as {pathToFileURL(path:string):{href:string}};
const {execFile}=await import(childName) as {execFile(file:string,args:string[],options:{timeout:number;maxBuffer:number;encoding:'utf8';windowsHide:boolean},callback:(error:Error|null,stdout:string,stderr:string)=>void):void};
const {env}=await import(processName) as {env:Record<string,string|undefined>};
const browser=[env.ATSUMI_TEST_BROWSER??'','C:/Program Files/Google/Chrome/Application/chrome.exe','C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe','C:/Program Files/Microsoft/Edge/Application/msedge.exe'].find(file=>fs.existsSync(file));
type Box={x:number;y:number;width:number;height:number};
type Glyph={kind:string;button:Box;svg:Box;padding:string;namespace:string;shapes:Box[];childTags:string[]};
type Scene={width:number;height:number;recording:boolean;glyphs:Glyph[];order:string[];close:{button:Box;svg:Box;padding:string;opacity:string;hidden:boolean};fade:{opacity:string;hidden:boolean};denied:boolean};

it.skipIf(!browser)('keeps owned SVG viewports and shapes visible under the measured YouTube padding cascade in offline Chrome',async()=>{
  const directory=fs.mkdtempSync(path.join(tmpdir(),'atsumi-youtube-control-layout-'));
  // Minimal synthetic cascade reproducing the computed styles measured in the
  // real player on2026-10-17; not a downloaded copy of the official stylesheet.
  const css=`html,body{margin:0;background:#000;color:#fff}#movie_player{overflow:hidden}.ytp-chrome-bottom{position:absolute;bottom:0;height:40px}.control-row{display:flex;justify-content:flex-end;height:40px}.ytp-button{box-sizing:border-box;width:40px;height:40px;padding:0;border:0}.ytp-button svg{padding:8px 12px;box-sizing:content-box;width:100%;height:100%}`;
  const fixtureScript=`(()=>{
    const callbacks=[],errors=[];window.addEventListener('error',event=>errors.push(event.message));
    // Only the adapter's URL guard and timer are supplied by this offline
    // fixture. No real account, YouTube request, media or native bridge exists.
    {const location=new URL('https://www.youtube.com/watch?v=abcdefghijk');const setInterval=callback=>{callbacks.push(callback);return 0;};${adapter}}
    const bridge=window.__atsumiYouTubeOriginal,player=document.getElementById('movie_player'),chrome=player.querySelector('.ytp-chrome-bottom');
    const box=node=>{const r=node.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height};};
    bridge.configure({epoch:1,channel:'abcdefghijk'});
    const controls=(patch={})=>bridge.controls({epoch:1,canClose:true,canRecord:true,captureBusy:false,recordBusy:false,recording:false,...patch});
    const sample=()=>callbacks.forEach(callback=>callback());const scenes=[];
    for(const [width,height] of [[800,450],[240,140],[240,64]])for(const recording of [false,true]){
      bridge.layout({visible:true,player:{x:0,y:0,width,height},chat:null});chrome.style.opacity='1';controls({recording});sample();
      const glyphs=['record','camera'].map(kind=>{const button=player.querySelector('[data-atsumi-live-action="'+kind+'"]'),svg=button.querySelector('svg');return{kind,button:box(button),svg:box(svg),padding:getComputedStyle(svg).padding,namespace:svg.namespaceURI,shapes:[...svg.children].map(box),childTags:[...svg.children].map(child=>child.localName)};});
      const close=player.querySelector('[data-atsumi-live-action="close"]'),closeSvg=close.querySelector('svg');
      const shown={button:box(close),svg:box(closeSvg),padding:getComputedStyle(closeSvg).padding,opacity:getComputedStyle(close).opacity,hidden:close.hidden};
      chrome.style.opacity='0';sample();const fade={opacity:getComputedStyle(close).opacity,hidden:close.hidden};
      chrome.style.opacity='1';sample();controls({canClose:false});
      scenes.push({width,height,recording,glyphs,order:[...player.querySelector('.control-row').children].map(node=>node.dataset.atsumiLiveAction||'fullscreen'),close:shown,fade,denied:close.hidden});
    }
    bridge.layout({visible:true,player:{x:0,y:0,width:800,height:450},chat:null});chrome.style.opacity='1';controls();sample();
    document.getElementById('metrics').textContent=JSON.stringify({errors,scenes});
  })();`;
  try{
    const input=path.join(directory,'fixture.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';style-src 'unsafe-inline';script-src 'unsafe-inline';require-trusted-types-for 'script';trusted-types 'none'"><style>${css}</style><div id="movie_player"><video></video><div class="ytp-chrome-bottom" style="opacity:1"><div class="control-row"><button class="ytp-button ytp-fullscreen-button" aria-label="Fixture fullscreen">□</button></div></div></div><pre id="metrics" hidden></pre><script>${fixtureScript}</script>`);
    const screenshot:string[]=[];
    if(env.ATSUMI_LAYOUT_ARTIFACTS){fs.mkdirSync(env.ATSUMI_LAYOUT_ARTIFACTS,{recursive:true});screenshot.push(`--screenshot=${path.join(env.ATSUMI_LAYOUT_ARTIFACTS,'youtube-owned-controls-offline.png')}`);}
    const stdout=await new Promise<string>((resolve,reject)=>execFile(browser!,['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND',`--user-data-dir=${path.join(directory,'profile')}`,'--window-size=1000,600','--virtual-time-budget=1000',...screenshot,'--dump-dom',pathToFileURL(input).href],{timeout:45000,maxBuffer:3*1024**2,encoding:'utf8',windowsHide:true},(error,out,stderr)=>error?reject(new Error(stderr||error.message)):resolve(out)));
    const json=/<pre id="metrics" hidden="">([^<]+)<\/pre>/.exec(stdout)?.[1];
    expect(json,`Offline browser did not produce layout metrics (browser: ${browser}, stdout length: ${stdout.length}). Set ATSUMI_TEST_BROWSER to a working headless Chromium binary.`).toBeTruthy();
    const result=JSON.parse(json!) as {errors:string[];scenes:Scene[]};expect(result.errors).toEqual([]);expect(result.scenes).toHaveLength(6);
    for(const scene of result.scenes){
      expect(scene.order).toEqual(['record','camera','fullscreen']);
      for(const glyph of scene.glyphs){
        expect(glyph.button.width).toBe(40);expect(glyph.button.height).toBe(40);expect(glyph.svg.width).toBe(24);expect(glyph.svg.height).toBe(24);expect(glyph.padding).toBe('0px');expect(glyph.namespace).toBe('http://www.w3.org/2000/svg');
        expect(glyph.svg.x-glyph.button.x).toBe(8);expect(glyph.svg.y-glyph.button.y).toBe(8);
        for(const shape of glyph.shapes){expect(shape.width).toBeGreaterThan(0);expect(shape.height).toBeGreaterThan(0);expect(shape.x).toBeGreaterThanOrEqual(glyph.svg.x);expect(shape.y).toBeGreaterThanOrEqual(glyph.svg.y);expect(shape.x+shape.width).toBeLessThanOrEqual(glyph.svg.x+glyph.svg.width);expect(shape.y+shape.height).toBeLessThanOrEqual(glyph.svg.y+glyph.svg.height);}
        expect(glyph.childTags).toEqual(glyph.kind==='camera'?['path','circle']:scene.recording?['rect']:['circle']);
      }
      const closeSize=Math.min(72,scene.width-24,scene.height-24);
      expect(scene.close.button.width).toBe(closeSize);expect(scene.close.button.height).toBe(closeSize);expect(scene.close.button.x+closeSize/2).toBe(scene.width/2);expect(scene.close.button.y).toBe(12);expect(scene.close.padding).toBe('0px');
      expect(scene.close.svg.width).toBeCloseTo(Math.min(40,closeSize*.6),2);expect(scene.close.svg.height).toBeCloseTo(Math.min(40,closeSize*.6),2);expect(scene.close.hidden).toBe(false);expect(scene.close.opacity).toBe('1');expect(scene.fade).toEqual({opacity:'0',hidden:true});expect(scene.denied).toBe(true);
    }
  }finally{
    const relative=path.relative(fs.realpathSync(tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-youtube-control-layout-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
},60000);
