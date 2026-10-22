import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import { PlatformLivePlayer } from './PlatformLivePlayer';
import { liveSplit } from './liveSplit';
import css from './PlatformLivePlayer.css?raw';
import baseCss from '../../styles.css?raw';
import inkCss from '../../atsumi-ink.css?raw';
import workspaceCss from '../youtube/YouTubeWorkspace.css?raw';
import multiCss from '../youtube/YouTubeMultiView.css?raw';
import connectionCss from '../youtube/YouTubeConnection.css?raw';
import { YouTubeViewingControls } from '../youtube/YouTubeViewingControls';
import { YouTubePaneGrid } from '../youtube/YouTubePaneGrid';

const fsName='node:fs', pathName='node:path', osName='node:os', childName='node:child_process', urlName='node:url';
const fs=await import(fsName) as { existsSync(path:string):boolean; mkdtempSync(prefix:string):string; writeFileSync(path:string,contents:string):void; realpathSync(path:string):string; rmSync(path:string,options:{recursive:boolean;force:boolean;maxRetries:number;retryDelay:number}):void };
const path=await import(pathName) as {join(...parts:string[]):string;relative(from:string,to:string):string;isAbsolute(path:string):boolean};
const os=await import(osName) as {tmpdir():string};
const {pathToFileURL}=await import(urlName) as {pathToFileURL(path:string):{href:string}};
const {execFile}=await import(childName) as {execFile(file:string,args:string[],options:{timeout:number;maxBuffer:number;encoding:'utf8';windowsHide:boolean},callback:(error:Error|null,stdout:string,stderr:string)=>void):void};
const browsers=['C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe','C:/Program Files/Microsoft/Edge/Application/msedge.exe','C:/Program Files/Google/Chrome/Application/chrome.exe','C:/Program Files (x86)/Google/Chrome/Application/chrome.exe'].filter(file=>fs.existsSync(file));
const browser=browsers[0];
async function dumpDom(input:string,directory:string,size:string):Promise<string>{
  const failures:string[]=[];
  for(const [index,engine] of browsers.entries()){
    try{
      const result=await new Promise<{out:string;stderr:string}>((resolve,reject)=>execFile(engine,['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND',`--user-data-dir=${path.join(directory,'profile-'+index)}`,'--window-size='+size,'--virtual-time-budget=1500','--dump-dom',pathToFileURL(input).href],{timeout:20000,maxBuffer:3*1024**2,encoding:'utf8',windowsHide:true},(error,out,stderr)=>error?reject(new Error(stderr||error.message)):resolve({out,stderr})));
      if(/<pre id="metrics">[^<]+<\/pre>/.test(result.out))return result.out;
      failures.push(`${engine}: no fixture metrics; stdout ${result.out.length} bytes; ${result.stderr.slice(-500)}`);
    }catch(error){failures.push(`${engine}: ${String(error).slice(-500)}`);}
  }
  throw new Error('No installed browser completed the offline geometry fixture. '+failures.join('\n'));
}
const markup=renderToStaticMarkup(<PlatformLivePlayer reference={{provider:'youtube',kind:'live',externalId:'abcdefghijk'}} title="Offline geometry fixture" privacyMode={false}/>);
type Box={x:number;y:number;width:number;height:number};
type Measurement={name:string;compact:boolean;expected:number;pair:Box;video:Box;chat:Box;separator:Box;display:string;fit:string;scrollWidth:number;scrollHeight:number};

it.skipIf(!browser)('keeps the fading identity header within 240, 480 and 960px with a stable avatar slot',async()=>{
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'atsumi-live-split-'));
  const script=`(()=>{const output=[],box=node=>{const b=node.getBoundingClientRect();return{x:b.x,y:b.y,width:b.width,height:b.height}};for(const width of [240,480,960]){const host=document.getElementById('fixture');host.style.width=width+'px';host.innerHTML=${JSON.stringify(markup)};const pair=host.querySelector('.platform-live-pair');pair.querySelector('.platform-live-chat').remove();pair.querySelector('.platform-live-resize').remove();pair.classList.toggle('is-compact',width<580);const player=pair.querySelector('.platform-live-player'),header=player.querySelector('header'),avatar=header.querySelector('.platform-live-avatar'),channel=header.querySelector('.platform-live-channel'),title=header.querySelector('.platform-live-title'),badge=header.querySelector('.platform-live-badge'),footer=player.querySelector('footer');channel.textContent='Verified long public channel '.repeat(8);title.textContent='Verified long broadcast title '.repeat(10);badge.textContent='LIVE';player.classList.add('show-controls');const before={avatar:box(avatar),channel:box(channel),title:box(title),badge:box(badge)};avatar.innerHTML='<img alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=">';const ready={avatar:box(avatar),channel:box(channel),title:box(title),badge:box(badge)},shown=[getComputedStyle(header).opacity,getComputedStyle(footer).opacity];player.classList.remove('show-controls');const hidden=[getComputedStyle(header).opacity,getComputedStyle(footer).opacity];output.push({width,player:box(player),header:box(header),before,ready,shown,hidden,channelOverflow:getComputedStyle(channel).textOverflow,titleOverflow:getComputedStyle(title).textOverflow,scrollWidth:player.scrollWidth,scrollHeight:player.scrollHeight});}document.getElementById('metrics').textContent=JSON.stringify(output);})()`;
  try{
    const input=path.join(directory,'identity.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';img-src data:;style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\nbody{margin:0}#fixture{height:240px}#fixture>.platform-live-pair{height:100%;min-height:0;max-height:none}*,*:before,*:after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const stdout=await dumpDom(input,directory,'1280,800');
    const json=/<pre id="metrics">([^<]+)<\/pre>/.exec(stdout)?.[1];expect(json,stdout.slice(-1500)).toBeTruthy();
    type Parts={avatar:Box;channel:Box;title:Box;badge:Box};
    const results=JSON.parse(json!) as Array<{width:number;player:Box;header:Box;before:Parts;ready:Parts;shown:string[];hidden:string[];channelOverflow:string;titleOverflow:string;scrollWidth:number;scrollHeight:number}>;
    expect(results).toHaveLength(3);
    for(const scene of results){
      expect(scene.ready).toEqual(scene.before);expect(scene.ready.avatar.width).toBe(36);expect(scene.ready.avatar.height).toBe(36);
      expect(scene.channelOverflow).toBe('ellipsis');expect(scene.titleOverflow).toBe('ellipsis');expect(scene.shown).toEqual(['1','1']);expect(scene.hidden).toEqual(['0','0']);
      expect(scene.ready.channel.x+scene.ready.channel.width).toBeLessThanOrEqual(scene.ready.badge.x);
      expect(scene.ready.title.x+scene.ready.title.width).toBeLessThanOrEqual(scene.ready.badge.x);
      expect(scene.ready.badge.x+scene.ready.badge.width).toBeLessThanOrEqual(scene.player.x+scene.player.width);
      expect(scene.header.width).toBeCloseTo(scene.player.width,1);expect(scene.scrollWidth).toBeLessThanOrEqual(scene.player.width+1);expect(scene.scrollHeight).toBeLessThanOrEqual(scene.player.height+1);
    }
  }finally{
    const relative=path.relative(fs.realpathSync(os.tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-live-split-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    try{fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});}catch(error){if(!['EPERM','EBUSY','ENOTEMPTY'].includes((error as {code?:string}).code??''))throw error;}
  }
},60000);

it.skipIf(!browser)('keeps the real separator and both panes inside small or wide cells with production CSS',async()=>{
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'atsumi-live-split-'));
  const scenes=[[300,180],[477,305],[650,305],[1000,700]].flatMap(([width,height])=>[.25,.44,.7].map(fraction=>{
    const split=liveSplit(width!,height!,fraction*width!,fraction);
    return {name:`${width}x${height}/${fraction}`,width,height,...split};
  }));
  const script=`(()=>{const output=[];for(const scene of ${JSON.stringify(scenes)}){const host=document.getElementById('fixture');host.style.width=scene.width+'px';host.style.height=scene.height+'px';host.innerHTML=${JSON.stringify(markup)};const pair=host.querySelector('.platform-live-pair');pair.classList.toggle('is-compact',scene.compact);pair.style.setProperty('--platform-live-chat-size',scene.pixels+'px');const video=pair.querySelector('.platform-live-player'),chat=pair.querySelector('.platform-live-chat'),separator=pair.querySelector('.platform-live-resize'),box=node=>{const b=node.getBoundingClientRect();return{x:b.x,y:b.y,width:b.width,height:b.height}};output.push({name:scene.name,compact:scene.compact,expected:scene.pixels,pair:box(pair),video:box(video),chat:box(chat),separator:box(separator),display:getComputedStyle(separator).display,fit:getComputedStyle(video.querySelector('video')).objectFit,scrollWidth:pair.scrollWidth,scrollHeight:pair.scrollHeight});}document.getElementById('metrics').textContent=JSON.stringify(output);})()`;
  try {
    const input=path.join(directory,'fixture.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\nbody{margin:0}#fixture{position:relative}#fixture>.platform-live-pair{height:100%;min-height:0;max-height:none;box-sizing:border-box}*,*:before,*:after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const stdout=await dumpDom(input,directory,'1280,800');
    const json=/<pre id="metrics">([^<]+)<\/pre>/.exec(stdout)?.[1];expect(json).toBeTruthy();
    const results=JSON.parse(json!) as Measurement[];expect(results).toHaveLength(12);
    for(const scene of results){
      const {pair,video,chat,separator}=scene;
      expect(scene.display,scene.name).not.toBe('none');expect(scene.fit).toBe('contain');
      expect(scene.scrollWidth,scene.name).toBeLessThanOrEqual(pair.width+1);
      expect(scene.scrollHeight,scene.name).toBeLessThanOrEqual(pair.height+1);
      if(scene.compact){
        expect(separator.height,scene.name).toBeCloseTo(7,1);expect(chat.height).toBeCloseTo(scene.expected,1);
        expect(video.y+video.height).toBeCloseTo(separator.y,1);expect(separator.y+separator.height).toBeCloseTo(chat.y,1);
        expect(chat.y+chat.height).toBeCloseTo(pair.y+pair.height,1);expect(video.width).toBeCloseTo(pair.width,1);
      }else{
        expect(separator.width,scene.name).toBeCloseTo(7,1);expect(chat.width).toBeCloseTo(scene.expected,1);
        expect(video.x+video.width).toBeCloseTo(separator.x,1);expect(separator.x+separator.width).toBeCloseTo(chat.x,1);
        expect(chat.x+chat.width).toBeCloseTo(pair.x+pair.width,1);expect(video.height).toBeCloseTo(pair.height,1);
      }
    }
  }finally{
    const relative=path.relative(fs.realpathSync(os.tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-live-split-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    try{fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});}catch(error){if(!['EPERM','EBUSY','ENOTEMPTY'].includes((error as {code?:string}).code??''))throw error;}
  }
},60000);

it.skipIf(!browser)('keeps ordinary floating live details stable through measured compact layout updates',async()=>{
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'atsumi-live-split-'));
  const scenes=[300,900].flatMap(width=>[1,2,4].map(count=>({width,count})));
  const script=`(async()=>{const output=[],box=node=>{const b=node.getBoundingClientRect();return{x:b.x,y:b.y,width:b.width,height:b.height}};for(const scene of ${JSON.stringify(scenes)}){const host=document.getElementById('fixture');host.innerHTML='<div class="youtube-detail-backdrop"><div class="youtube-detail" style="width:'+scene.width+'px"><header><div class="youtube-detail-tabs"><div class="is-active"><button>Offline live fixture</button><button>×</button></div></div><button class="youtube-detail-minimize">−</button></header><div class="youtube-detail-actions"><strong>Offline live fixture</strong><button>☆</button><button>다운로드</button><button>시청 종료</button></div><div class="youtube-playing-grid'+(scene.count>1?' is-multiple':'')+'" data-count="'+scene.count+'" data-mode="all">'+Array.from({length:scene.count},()=>'<div class="youtube-playing-cell">'+${JSON.stringify(markup)}+'</div>').join('')+'</div></div></div>';const pairs=[...host.querySelectorAll('.platform-live-pair')],trace=[];for(let turn=0;turn<12;turn++){for(const pair of pairs){const b=pair.getBoundingClientRect(),compact=b.width<580,available=Math.max(0,(compact?b.height:b.width)-7),min=Math.min(96,available*.35),max=Math.max(min,Math.min(available-min,compact?available:available*.6));pair.classList.toggle('is-compact',compact);pair.style.setProperty('--platform-live-chat-size',Math.max(min,Math.min(max,compact?available*.44:240))+'px');}trace.push(pairs.map(pair=>box(pair).height));}const detail=host.querySelector('.youtube-detail');output.push({name:scene.width+'/'+scene.count,detail:box(detail),scrollWidth:detail.scrollWidth,trace,pairs:pairs.map(pair=>({pair:box(pair),video:box(pair.querySelector('.platform-live-player')),chat:box(pair.querySelector('.platform-live-chat')),separator:box(pair.querySelector('.platform-live-resize'))}))});}document.getElementById('metrics').textContent=JSON.stringify(output);})()`;
  try{
    const input=path.join(directory,'ordinary-detail.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\n${workspaceCss}\n${multiCss}\nbody{margin:0}*,*:before,*:after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const stdout=await dumpDom(input,directory,'960,640');
    const json=/<pre id="metrics">([^<]+)<\/pre>/.exec(stdout)?.[1];expect(json).toBeTruthy();
    const results=JSON.parse(json!) as Array<{name:string;detail:Box;scrollWidth:number;trace:number[][];pairs:Array<{pair:Box;video:Box;chat:Box;separator:Box}>}>;
    expect(results).toHaveLength(6);
    for(const scene of results){
      expect(scene.scrollWidth,scene.name).toBeLessThanOrEqual(scene.detail.width+1);
      expect(scene.detail.height,scene.name).toBeLessThanOrEqual(582);
      expect(scene.trace.at(-1),scene.name).toEqual(scene.trace.at(-2));
      for(const pair of scene.pairs){
        expect(pair.pair.height,scene.name).toBeGreaterThanOrEqual(220);
        expect(pair.video.height,scene.name).toBeGreaterThanOrEqual(64);
        expect(pair.chat.height,scene.name).toBeGreaterThanOrEqual(64);
        expect(Math.min(pair.separator.height,pair.separator.width),scene.name).toBeCloseTo(7,1);
      }
    }
  }finally{
    const relative=path.relative(fs.realpathSync(os.tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-live-split-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    try{fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});}catch(error){if(!['EPERM','EBUSY','ENOTEMPTY'].includes((error as {code?:string}).code??''))throw error;}
  }
},60000);

it.skipIf(!browser)('fits the viewing shell grid and overlay sheet without moving its centered handle',async()=>{
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'atsumi-live-split-'));
  const noop=()=>{};
  const controls=renderToStaticMarkup(<YouTubeViewingControls active open privacyMode={false} busy={false} items={Array.from({length:4},(_,i)=>({kind:'video' as const,id:`fixture000${i}`,url:'https://www.youtube.com/',title:`Offline fixture ${i}`,author:`Channel ${i}`,isLive:true}))} playingIds={[]} favoriteIds={[]} mode="all" lead="" onOpenChange={noop} onApply={noop} onStop={noop} onExplore={noop} onCloseItem={noop} onFavorite={noop} onDownload={noop}/>);
  const scenes=[650,1000,1600].flatMap(width=>[...[1,2,4].map(count=>({width,count,mode:'all' as 'all'|'selected'})),{width,count:4,mode:'selected' as 'all'|'selected'}]).map(scene=>({ ...scene,
    markup: renderToStaticMarkup(<YouTubePaneGrid kind="pair" className="youtube-playing-grid" mode={scene.mode} live onResizing={noop} accessory={<div className="youtube-chat-tabs"><button>채팅</button></div>}>{Array.from({length:scene.count},(_,i)=><div className="youtube-playing-cell" key={i} data-video-visible={scene.mode==='all'||i===0} data-chat-active={i===0}><PlatformLivePlayer reference={{provider:'youtube',kind:'live',externalId:'abcdefghijk'}} title="Offline geometry fixture" privacyMode={false}/></div>)}</YouTubePaneGrid>)
  }));
  const script=`(()=>{try{const output=[],box=node=>{if(!node)throw new Error('missing geometry node');const b=node.getBoundingClientRect();return{x:b.x,y:b.y,width:b.width,height:b.height}};for(const scene of ${JSON.stringify(scenes)}){const host=document.getElementById('fixture');host.style.width=scene.width+'px';host.style.height='600px';host.innerHTML='<div class="youtube-surface"><div class="youtube-detail-backdrop youtube-viewing"><div class="youtube-detail">'+scene.markup+'</div></div>'+${JSON.stringify(controls)}+'</div>';const grid=host.querySelector('.youtube-playing-grid');for(const [i,pair] of [...host.querySelectorAll('.platform-live-pair')].entries()){if(scene.mode==='selected'&&i>0){pair.classList.add('chat-only');pair.querySelector('.platform-live-resize').remove();}else if(getComputedStyle(pair).display!=='contents'){const b=pair.getBoundingClientRect(),compact=b.width<580;pair.classList.toggle('is-compact',compact);const available=(compact?b.height:b.width)-7,min=Math.min(96,available*.35),max=Math.max(min,Math.min(available-min,compact?available:available*.6));pair.style.setProperty('--platform-live-chat-size',Math.max(min,Math.min(max,compact?available*.44:240))+'px');}}const panel=host.querySelector('.youtube-connection-panel'),handle=host.querySelector('.youtube-bottom-handle');output.push({name:scene.width+'/'+scene.count+'/'+scene.mode,host:box(host),grid:box(grid),panel:box(panel),handle:box(handle),scrollWidth:host.scrollWidth,scrollHeight:host.scrollHeight,panes:[...grid.querySelectorAll('.platform-live-player,.platform-live-chat')].filter(node=>getComputedStyle(node).display!=='none').map(box)});}document.getElementById('metrics').textContent=JSON.stringify(output);}catch(error){document.getElementById('metrics').textContent=JSON.stringify({fixtureError:String(error)});}})()`;
  try{
    const input=path.join(directory,'shell.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';style-src 'unsafe-inline';script-src 'unsafe-inline'"><style>${baseCss}\n${inkCss}\n${css}\n${workspaceCss}\n${multiCss}\n${connectionCss}\nbody{margin:0}#fixture{position:relative}*,*:before,*:after{animation:none!important;transition:none!important}</style><main id="fixture"></main><pre id="metrics"></pre><script>${script}</script>`);
    const stdout=await dumpDom(input,directory,'1800,800');
    const json=/<pre id="metrics">([^<]+)<\/pre>/.exec(stdout)?.[1];expect(json).toBeTruthy();
    const results=JSON.parse(json!) as Array<{name:string;host:Box;grid:Box;panel:Box;handle:Box;scrollWidth:number;scrollHeight:number;panes:Box[]}>;
    expect(results).toHaveLength(12);
    for(const scene of results){
      expect(scene.scrollWidth,scene.name).toBeLessThanOrEqual(scene.host.width+1);expect(scene.scrollHeight,scene.name).toBeLessThanOrEqual(scene.host.height+1);
      expect(scene.grid.height,scene.name).toBeCloseTo(scene.host.height-22,1);
      expect(scene.panel.x+scene.panel.width/2,scene.name).toBeCloseTo(scene.handle.x+scene.handle.width/2,1);
      expect(scene.handle.y,scene.name).toBeGreaterThanOrEqual(scene.host.y);expect(scene.panel.y,scene.name).toBeCloseTo(scene.handle.y+scene.handle.height,1);expect(scene.panel.y+scene.panel.height,scene.name).toBeCloseTo(scene.host.y+scene.host.height,1);
      expect(scene.panes.length,scene.name).toBeGreaterThanOrEqual(2);
      for(const pane of scene.panes){expect(pane.width,scene.name).toBeGreaterThan(0);expect(pane.height,scene.name).toBeGreaterThan(0);expect(pane.x,scene.name).toBeGreaterThanOrEqual(scene.grid.x-1);expect(pane.y,scene.name).toBeGreaterThanOrEqual(scene.grid.y-1);expect(pane.x+pane.width,scene.name).toBeLessThanOrEqual(scene.grid.x+scene.grid.width+1);expect(pane.y+pane.height,scene.name).toBeLessThanOrEqual(scene.grid.y+scene.grid.height+1);}
    }
  }finally{
    const relative=path.relative(fs.realpathSync(os.tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-live-split-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    try{fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});}catch(error){if(!['EPERM','EBUSY','ENOTEMPTY'].includes((error as {code?:string}).code??''))throw error;}
  }
},60000);
