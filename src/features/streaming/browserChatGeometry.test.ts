import { expect, it } from 'vitest';
import adapter from '../../../src-tauri/src/streaming/browser_multiview.js?raw';

const fsName='node:fs',pathName='node:path',osName='node:os',childName='node:child_process',urlName='node:url',processName='node:process';
const fs=await import(fsName) as {existsSync(path:string):boolean;mkdtempSync(prefix:string):string;writeFileSync(path:string,contents:string):void;realpathSync(path:string):string;rmSync(path:string,options:{recursive:boolean;force:boolean;maxRetries:number;retryDelay:number}):void};
const path=await import(pathName) as {join(...parts:string[]):string;relative(from:string,to:string):string;isAbsolute(path:string):boolean};
const {tmpdir}=await import(osName) as {tmpdir():string};
const {pathToFileURL}=await import(urlName) as {pathToFileURL(path:string):{href:string}};
const {execFile}=await import(childName) as {execFile(file:string,args:string[],options:{timeout:number;maxBuffer:number;encoding:'utf8';windowsHide:boolean},callback:(error:Error|null,stdout:string,stderr:string)=>void):void};
const {env}=await import(processName) as {env:Record<string,string|undefined>};
const browser=[env.ATSUMI_TEST_BROWSER??'','C:/Program Files/Google/Chrome/Application/chrome.exe','C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe','C:/Program Files/Microsoft/Edge/Application/msedge.exe'].find(file=>fs.existsSync(file));
type Bounds={left:number;top:number;right:number;bottom:number;width:number;height:number};
type Scene={
  width:number;zoom:number;lines:number;fit:boolean;compact:boolean;viewport:number;scrollWidth:number;scrollHeight:number;height:number;
  aside:Bounds;header:Bounds;input:Bounds;controls:Record<string,Bounds>;title:Bounds;label:Bounds;
  originalNodesPreserved:boolean;inputPreserved:boolean;scaleCount:number;fonts:Record<string,string>;
  headerHeight:number;areaPadding:string;toolsWrap:string;actionPadding:string;dividerDisplay:string;
  actionClicks:number[];scale:string;messagesHeight:number;inputScrollHeight:number;inputClientHeight:number;inputOverflow:string;
};

// Minimum layout-bearing rules read from the public official stylesheet on
// 2026-10-17, not a replacement implementation of the official chat page:
// https://ssl.pstatic.net/static/nng/glive/resource/p/static/css/index-nP-g7x0q.css
// Global reset, header, popup and footer class names match the live DevTools
// evidence in docs/requirements/chzzk-narrow-chat-20261017.json. No remote fonts,
// SVG artwork, account data, messages or network requests are used. The neutral
// 24px SVG slots are explicit synthetic control fixtures. The send button uses
// the actual small solid button classes and their published typography/padding.
const css=`
  html,body{margin:0;min-height:100vh}body{position:relative}
  body,h2,button,textarea{margin:0;padding:0}
  body,button,textarea{font-family:"Malgun Gothic",Arial,sans-serif;font-size:14px;line-height:1.2}
  *,*::before,*::after{box-sizing:border-box}button,textarea{background:none;border:0;border-radius:0}svg{vertical-align:top}
  ._container_1gtzb_1{width:100%;min-width:353px;height:100vh}
  ._container_wj4te_1{transform:translate(0,0);width:100%;height:100%;display:flex}
  ._wrapper_wj4te_16{width:100%;display:flex}
  ._container_nu1j9_2{word-break:break-all;word-wrap:break-word;display:flex;flex-direction:column;position:relative}
  ._container_nu1j9_2._is_popup_chat_nu1j9_13{width:100%;height:100vh}
  ._container_1e2su_2{height:44px;position:relative}
  ._title_1e2su_12{height:100%;border:1px solid transparent;border-width:1px 0;display:flex;justify-content:center;align-items:center;padding:0 44px;font-family:"Malgun Gothic",Arial,sans-serif;font-size:15px;font-weight:400;position:relative}
  ._wrapper_1e2su_28{z-index:1001;position:absolute;top:0}
  ._wrapper_1e2su_28._menu_1e2su_36{right:0}
  ._wrapper_1e2su_28._scale_1e2su_39{display:flex;align-items:center;top:10px;right:40px}
  ._scale_text_1e2su_46{font-variant-numeric:tabular-nums;margin:0 6px;font-size:12px;font-weight:700;line-height:14px}
  ._scale_button_1e2su_55{width:25px;height:25px;border-radius:8px}
  ._button_1e2su_69{padding:8px}
  .messages{flex:1;min-height:0;overflow:auto}
  ._area_nu1j9_49{z-index:1000;border-top:1px solid transparent;flex:none;min-height:102px;padding:10px 20px;position:relative}
  ._container_19u4u_2{border-radius:8px;align-items:center;width:100%;padding:6px;display:flex;position:relative}
  ._setting_button_19u4u_31{border-radius:50%;flex:none;margin-top:auto;padding:1px 0;position:relative}
  ._input_19u4u_59{white-space:normal;width:100%;min-height:20px;max-height:60px;resize:none;outline:none;margin-left:7px;line-height:20px;position:relative;overflow-y:auto}
  ._input_button_19u4u_90{flex:none;align-self:flex-end;margin-left:8px;position:relative}
  ._tools_19u4u_125{align-items:center;margin-top:10px;display:flex;position:relative}
  ._donation_19u4u_132{align-items:center;display:flex;position:relative}
  ._donation_text_19u4u_137{height:28px;flex:none;font-size:13px;line-height:24px}
  ._donation_button_19u4u_151+._donation_button_19u4u_151{margin-left:5px}
  ._icon_cheese_19u4u_155{margin-right:5px}
  ._action_19u4u_140{flex:none;padding-left:17px;position:relative}
  ._action_19u4u_140:before{content:"";width:1px;position:absolute;top:6px;bottom:6px;left:10px}
  ._send_button_19u4u_176{flex:none;margin-left:auto}
  ._container_vgt54_2{vertical-align:top;outline:none;justify-content:center;align-items:center;font-weight:700;display:inline-flex;position:relative}
  ._container_vgt54_2._font_bold_vgt54_15{letter-spacing:0;font-family:"Malgun Gothic",Arial,sans-serif;font-weight:400;line-height:18px}
  ._solid_vgt54_26._small_vgt54_221{height:32px;padding:0 11px;font-size:13px}
`;

const icon='<svg width="24" height="24" viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="5" width="14" height="14"/></svg>';
const control=(id:string,className:string,label:string)=>`<button id="${id}" class="${className}" type="button" aria-label="${label}">${icon}</button>`;
const popup=`<div id="root"><div class="_container_1gtzb_1"><section class="_container_wj4te_1"><div class="_wrapper_wj4te_16">
  <aside id="aside-chatting" class="_container_nu1j9_2 _is_popup_chat_nu1j9_13">
    <header class="_container_1e2su_2"><h2 class="_title_1e2su_12">채팅</h2>
      <div class="_wrapper_1e2su_28 _scale_1e2su_39"><button class="_scale_button_1e2su_55">−</button><span class="_scale_text_1e2su_46">100%</span><button class="_scale_button_1e2su_55">+</button></div>
      <div class="_wrapper_1e2su_28 _menu_1e2su_36">${control('menu','_button_1e2su_69','채팅 메뉴')}</div>
    </header><div class="messages"></div>
    <div class="_area_nu1j9_49">
      <div class="_container_19u4u_2">${control('profile','_setting_button_19u4u_31','프로필')}<textarea id="input" class="_input_19u4u_59" rows="1"></textarea>${control('emote','_input_button_19u4u_90','이모티콘')}</div>
      <div class="_tools_19u4u_125"><div class="_donation_19u4u_132">
        <button id="donation" class="_donation_text_19u4u_137"><svg class="_icon_cheese_19u4u_155" width="24" height="24" aria-hidden="true"></svg>후원하기</button>
        <div class="_action_19u4u_140">${control('video','_donation_button_19u4u_151','영상 후원')}${control('mission','_donation_button_19u4u_151','미션')}${control('target','_donation_button_19u4u_151','목표')}</div>
      </div><button id="send_chat_or_donate" class="_container_vgt54_2 _primary_vgt54_30 _solid_vgt54_26 _small_vgt54_221 _font_bold_vgt54_15 _send_button_19u4u_176">전송</button></div>
    </div>
  </aside></div></section></div></div>`;

const inside=(rect:Bounds,width:number,height:number)=>rect.width>0&&rect.height>0&&rect.left>=-0.1&&rect.top>=-0.1&&rect.right<=width+0.1&&rect.bottom<=height+0.1;
const overlaps=(a:Bounds,b:Bounds)=>Math.min(a.right,b.right)-Math.max(a.left,b.left)>0.5&&Math.min(a.bottom,b.bottom)-Math.max(a.top,b.top)>0.5;

it.skipIf(!browser)('keeps all official popup controls inside narrow panes without overlap, lost input or automatic font scaling',async()=>{
  const directory=fs.mkdtempSync(path.join(tmpdir(),'atsumi-chat-layout-'));
  // Frame widths below Chromium's top-level window minimum are needed here.
  // Only this fixture substitutes the URL/top-level checks; real guard cases
  // are exercised without substitutions in browserMultiviewBridge.test.ts.
  const fixtureAdapter=adapter.replaceAll('window.location','fixtureLocation').replace('window.top !== window','false');
  const scenes=[160,171,175,219,299,300,333,440].flatMap(width=>[100,50].flatMap(zoom=>[200,600].flatMap(height=>[1,2,3].map(lines=>({width,zoom,height,lines})))));
  const documents=scenes.map(({width,zoom,lines})=>`<!doctype html><meta charset="utf-8"><style>${css}</style>${popup}<script>
    const draft=${JSON.stringify(['작성 중 ㄱ','둘째 줄','셋째 줄'].slice(0,lines).join('\n'))};
    const input=document.getElementById('input');input.value=draft;input.style.height='${lines*20}px';input.setSelectionRange(5,5);
    const ids=['menu','profile','emote','donation','video','mission','target','send_chat_or_donate'];
    const originals=ids.map(id=>document.getElementById(id)),parents=originals.map(node=>node.parentElement),inputParent=input.parentElement;
    const actionClicks=ids.map(()=>0);originals.forEach((node,index)=>node.addEventListener('click',()=>actionClicks[index]++));
    {const fixtureLocation=new URL('https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/chat');${fixtureAdapter}}
    window.__atsumiMultiView.configureChat({channelId:'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',number:1,channelName:'매우 긴 테스트 채널 이름'});
    ${zoom===50?"for(let i=0;i<5;i++)document.querySelector('.atsumi-chat-scale button').click();":''}
    window.measure=()=>{
      const bounds=node=>{const r=node.getBoundingClientRect();return{left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height}};
      const aside=document.getElementById('aside-chatting'),header=aside.querySelector('header'),heading=header.querySelector('h2');
      const label=heading.querySelector('.atsumi-mado-chat-channel'),scale=header.querySelector('.atsumi-chat-scale');
      const range=document.createRange();range.selectNodeContents(heading.firstChild);
      const controls=Object.fromEntries(ids.map(id=>[id,bounds(document.getElementById(id))]));
      controls.minus=bounds(scale.firstElementChild);controls.scale=bounds(scale.querySelector('output'));controls.plus=bounds(scale.lastElementChild);
      originals.forEach(node=>node.click());
      const font=node=>getComputedStyle(node).fontSize;
      return{width:${width},zoom:${zoom},lines:${lines},fit:document.body.hasAttribute('data-atsumi-chat-fit'),compact:document.body.hasAttribute('data-atsumi-chat-compact'),viewport:innerWidth,height:innerHeight,scrollWidth:document.documentElement.scrollWidth,scrollHeight:document.documentElement.scrollHeight,
        aside:bounds(aside),header:bounds(header),input:bounds(input),title:bounds(range),label:bounds(label),controls,
        originalNodesPreserved:originals.every((node,i)=>node===document.getElementById(ids[i])&&node.parentElement===parents[i]),inputPreserved:document.getElementById('input')===input&&input.parentElement===inputParent&&input.value===draft&&input.selectionStart===5&&input.selectionEnd===5,
        scaleCount:document.querySelectorAll('.atsumi-chat-scale').length,fonts:{heading:font(heading),label:font(label),input:font(input),donation:font(document.getElementById('donation')),scale:font(scale.querySelector('output'))},
        headerHeight:header.clientHeight,areaPadding:getComputedStyle(aside.querySelector('._area_nu1j9_49')).padding,toolsWrap:getComputedStyle(aside.querySelector('._tools_19u4u_125')).flexWrap,actionPadding:getComputedStyle(aside.querySelector('._action_19u4u_140')).paddingLeft,dividerDisplay:getComputedStyle(aside.querySelector('._action_19u4u_140'),'::before').display,
        actionClicks,scale:scale.querySelector('output').textContent,messagesHeight:aside.querySelector('.messages').getBoundingClientRect().height,inputScrollHeight:input.scrollHeight,inputClientHeight:input.clientHeight,inputOverflow:getComputedStyle(input).overflowY};
    };<\/script>`);
  try{
    const input=path.join(directory,'fixture.html');
    fs.writeFileSync(input,`<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none';style-src 'unsafe-inline';script-src 'unsafe-inline';frame-src 'self' about:"><pre id="metrics" hidden></pre><script>const documents=${JSON.stringify(documents).replaceAll('<','\\u003c')};const scenes=${JSON.stringify(scenes)};for(let i=0;i<documents.length;i++){const frame=document.createElement('iframe');frame.style.cssText='border:0;width:'+scenes[i].width+'px;height:'+scenes[i].height+'px';frame.srcdoc=documents[i];document.body.appendChild(frame)}setTimeout(()=>document.getElementById('metrics').textContent=JSON.stringify([...document.querySelectorAll('iframe')].map(frame=>frame.contentWindow.measure())),500);<\/script>`);
    const stdout=await new Promise<string>((resolve,reject)=>execFile(browser!,['--headless','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--host-resolver-rules=MAP * ~NOTFOUND',`--user-data-dir=${path.join(directory,'profile')}`,'--window-size=1000,800','--virtual-time-budget=1500','--dump-dom',pathToFileURL(input).href],{timeout:45000,maxBuffer:20*1024**2,encoding:'utf8',windowsHide:true},(error,out,stderr)=>error?reject(new Error(stderr||error.message)):resolve(out)));
    const json=/<pre id="metrics" hidden="">([^<]+)<\/pre>/.exec(stdout)?.[1];
    expect(json,'Offline Chromium must return all narrow-pane measurements').toBeTruthy();
    const result=JSON.parse(json!) as Scene[];expect(result).toHaveLength(scenes.length);
    const obscured=result.filter(scene=>scene.messagesHeight<=0||Object.values(scene.controls).some(rect=>!inside(rect,scene.width,scene.height))).map(scene=>({
      width:scene.width,height:scene.height,zoom:scene.zoom,lines:scene.lines,headerHeight:scene.headerHeight,messagesHeight:scene.messagesHeight,
      input:scene.input,lowestControlBottom:Math.max(...Object.values(scene.controls).map(rect=>rect.bottom)),
      clippedControls:Object.entries(scene.controls).filter(([,rect])=>!inside(rect,scene.width,scene.height)).map(([name])=>name),
    }));
    expect(obscured,'The minimum pane must retain message space and all native controls, including with a three-line draft').toEqual([]);
    for(const scene of result){
      const label=`${scene.width}×${scene.height}px / ${scene.zoom}% / ${scene.lines} lines`,compact=scene.width/(scene.zoom/100)<300;
      expect(scene.fit).toBe(true);expect(scene.viewport).toBe(scene.width);expect(scene.scrollWidth).toBe(scene.width);expect(scene.scrollHeight).toBe(scene.height);
      expect(scene.aside.width).toBeCloseTo(scene.width,1);expect(scene.aside.height).toBeCloseTo(scene.height,1);
      expect(scene.compact,label).toBe(compact);expect(scene.headerHeight,label).toBe(compact?72:44);
      expect(scene.areaPadding,label).toBe(compact?'6px 8px':'10px 20px');expect(scene.toolsWrap,label).toBe(compact?'wrap':'nowrap');
      expect(scene.actionPadding,label).toBe(compact?'0px':'17px');expect(scene.dividerDisplay,label).toBe(compact?'none':'block');
      const visible={...scene.controls,input:scene.input,title:scene.title,label:scene.label};
      for(const [name,bounds] of Object.entries(visible))expect(inside(bounds,scene.width,scene.height),`${label}: ${name} within viewport ${JSON.stringify(bounds)}`).toBe(true);
      const entries=Object.entries(visible);
      for(let i=0;i<entries.length;i++)for(let j=i+1;j<entries.length;j++){
        const [a,rectA]=entries[i]!,[b,rectB]=entries[j]!;
        expect(overlaps(rectA,rectB),`${label}: ${a} overlaps ${b} ${JSON.stringify([rectA,rectB])}`).toBe(false);
      }
      expect(scene.fonts,label).toEqual({heading:'15px',label:'15px',input:'14px',donation:'13px',scale:'12px'});
      expect(scene.originalNodesPreserved,label).toBe(true);expect(scene.actionClicks,label).toEqual(Array(8).fill(1));
      expect(scene.scale,label).toBe(`${scene.zoom}%`);expect(scene.messagesHeight,label).toBeGreaterThan(0);
      const factor=scene.zoom/100,maxInputHeight=compact?Math.max(20,Math.min(60,scene.height/factor-180)):60;
      expect(scene.input.height,label).toBeCloseTo(Math.min(scene.lines*20,maxInputHeight)*factor,1);
      expect(scene.inputOverflow,label).toBe('auto');
      if(scene.lines*20>maxInputHeight)expect(scene.inputScrollHeight,label).toBeGreaterThan(scene.inputClientHeight);
      expect(scene.inputPreserved).toBe(true);expect(scene.scaleCount).toBe(1);
    }
  }finally{
    const relative=path.relative(fs.realpathSync(tmpdir()),fs.realpathSync(directory));
    if(!relative.startsWith('atsumi-chat-layout-')||relative.includes('..')||path.isAbsolute(relative))throw new Error('Unexpected disposable fixture path');
    fs.rmSync(directory,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
},60000);
