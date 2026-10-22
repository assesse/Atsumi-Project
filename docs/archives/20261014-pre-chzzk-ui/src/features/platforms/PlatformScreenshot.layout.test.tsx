import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import { PlatformLivePlayer } from './PlatformLivePlayer';
import css from './PlatformLivePlayer.css?raw';
import baseCss from '../../styles.css?raw';
import inkCss from '../../atsumi-ink.css?raw';

const fsName='node:fs', pathName='node:path', osName='node:os', childName='node:child_process', urlName='node:url';
const fs=await import(fsName) as { existsSync(path:string):boolean; mkdtempSync(prefix:string):string; writeFileSync(path:string,contents:string):void; realpathSync(path:string):string; rmSync(path:string,options:{recursive:boolean;force:boolean;maxRetries:number;retryDelay:number}):void };
const path=await import(pathName) as {join(...parts:string[]):string;relative(from:string,to:string):string;isAbsolute(path:string):boolean};
const os=await import(osName) as {tmpdir():string};
const {pathToFileURL}=await import(urlName) as {pathToFileURL(path:string):{href:string}};
const {execFile}=await import(childName) as {execFile(file:string,args:string[],options:{timeout:number;maxBuffer:number;encoding:'utf8';windowsHide:boolean},callback:(error:Error|null,stdout:string,stderr:string)=>void):void};
const browser=['C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe','C:/Program Files/Microsoft/Edge/Application/msedge.exe'].find(file=>fs.existsSync(file));
const markup=renderToStaticMarkup(<PlatformLivePlayer reference={{provider:'youtube',kind:'live',externalId:'abcdefghijk'}} title="Offline geometry fixture" privacyMode={false}/>);
type Box={x:number;y:number;width:number;height:number};
it.skipIf(!browser)('keeps screenshot controls and review inside 240, 480 and 960px without moving the video',async()=>{
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'atsumi-capture-layout-'));
  const review='<div class="platform-live-capture-preview"><img alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="><div class="platform-live-capture-actions"><span>스크린샷 저장됨</span><button>저장 취소</button></div></div>';
  const script=`(()=>{const output=[],box=node=>{const b=node.getBoundingClientRect();return{x:b.x,y:b.y,width:b.width,height:b.height}};for(const width of [240,480,960])for(const height of [140,320]){const host=document.getElementById('fixture');host.style.width=width+'px';host.style.height=height+'px';host.innerHTML=${JSON.stringify(markup)};const pair=host.querySelector('.platform-live-pair');pair.querySelector('.platform-live-chat').remove();pair.querySelector('.platform-live-resize').remove();pair.classList.toggle('is-compact',width<580);const player=pair.querySelector('.platform-live-player'),footer=player.querySelector('footer'),video=player.querySelector('video');player.classList.add('show-controls');const before=box(video);player.insertAdjacentHTML('beforeend',${JSON.stringify(review)});const preview=player.querySelector('.platform-live-capture-preview');const button=preview.querySelector('button');const shown=[getComputedStyle(player.querySelector('header')).opacity,getComputedStyle(footer).opacity];player.classList.remove('show-controls');const hidden=[getComputedStyle(player.querySelector('header')).opacity,getComputedStyle(footer).opacity];output.push({width,height,player:box(player),before,after:box(video),preview:box(preview),undo:box(button),controls:[...footer.children].map(box),last:footer.lastElementChild.getAttribute('aria-label'),shown,hidden,scrollWidth:player.scrollWidth});}document.getElementById('metrics').textContent=JSON.stringify(output);})()`;
  try{
    const input=path.join(directory,'fixture.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';img-src data:;style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\nbody{margin:0}#fixture>.platform-live-pair{height:100%;min-height:0;max-height:none}*,*:before,*:after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const stdout=await new Promise<string>((resolve,reject)=>execFile(browser!,['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND',`--user-data-dir=${path.join(directory,'profile')}`,'--window-size=1280,800','--virtual-time-budget=1500','--dump-dom',pathToFileURL(input).href],{timeout:45000,maxBuffer:3*1024**2,encoding:'utf8',windowsHide:true},(error,out,stderr)=>error?reject(new Error(stderr||error.message)):resolve(out)));
    const json=/<pre id="metrics">([^<]+)<\/pre>/.exec(stdout)?.[1];expect(json).toBeTruthy();
    const scenes=JSON.parse(json!) as Array<{width:number;height:number;player:Box;before:Box;after:Box;preview:Box;undo:Box;controls:Box[];last:string;shown:string[];hidden:string[];scrollWidth:number}>;
    expect(scenes).toHaveLength(6);
    for(const scene of scenes){
      expect(scene.after).toEqual(scene.before);expect(scene.last).toBe('전체화면');expect(scene.shown).toEqual(['1','1']);expect(scene.hidden).toEqual(['0','0']);
      expect(scene.scrollWidth).toBeLessThanOrEqual(scene.player.width+1);expect(scene.preview.width).toBeLessThanOrEqual(480);
      expect(scene.preview.x).toBeGreaterThanOrEqual(scene.player.x);expect(scene.preview.y).toBeGreaterThanOrEqual(scene.player.y);expect(scene.preview.x+scene.preview.width).toBeLessThanOrEqual(scene.player.x+scene.player.width+1);expect(scene.preview.y+scene.preview.height).toBeLessThanOrEqual(scene.player.y+scene.player.height+1);
      expect(scene.undo.height).toBeGreaterThanOrEqual(24);expect(scene.undo.y+scene.undo.height).toBeLessThanOrEqual(scene.preview.y+scene.preview.height+1);
      scene.controls.forEach((control,index)=>{expect(control.width).toBeGreaterThan(0);expect(control.x).toBeGreaterThanOrEqual(scene.player.x);expect(control.x+control.width).toBeLessThanOrEqual(scene.player.x+scene.player.width+1);if(index)expect(control.x).toBeGreaterThanOrEqual(scene.controls[index-1]!.x+scene.controls[index-1]!.width);});
    }
  }finally{
    const relative=path.relative(fs.realpathSync(os.tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-capture-layout-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
},60000);
