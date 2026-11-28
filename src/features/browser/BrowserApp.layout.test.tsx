import {act} from "react";
import {createRoot} from "react-dom/client";
import {describe,expect,it,vi} from "vitest";
import {BrowserApp,browserApi,initialBrowserState,type BrowserSnapshot} from "./BrowserApp";
import {auditLayouts,layoutBrowser,type LayoutScene} from "../../test/offlineLayoutBrowser";
import css from "./BrowserApp.css?raw";
import scrollbars from "../../scrollbars.css?raw";
import logo from "../../assets/atsumi.svg?raw";
type Box={x:number;y:number;width:number;height:number;fontSize:number};
async function scenes(){
  const output:LayoutScene[]=[];
  for(const scene of ["home","setup","connected","error-many-tabs","history","preferences"]){
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    const connected=scene==="connected",failed=scene==="error-many-tabs";
    const snapshot:BrowserSnapshot={...initialBrowserState,tabs:Array.from({length:failed?14:2},(_,i)=>({id:String(i),url:i?"https://example.org/":"about:blank",title:i?"길어지는 웹페이지 제목을 가진 탭 "+i:"새 탭",loading:i===1})),active:"0",profiles:connected||failed?[{id:"jp",name:"Atsumi_Browser_JP-JP-FREE-1",countryHint:"JP"},{id:"us",name:"Atsumi_Browser_US-US-FREE-1",countryHint:"US"}]:[],network:{...initialBrowserState.network,enabled:connected||failed,phase:connected?"connected":failed?"error":"direct",profileId:connected||failed?"jp":null,profile:connected||failed?"Atsumi-Browser-JP-FREE-1-long-profile-name":null,country:connected?"JP":null,ip:connected?"203.0.113.1":null,error:failed?"서버에 연결하지 못했습니다. 이 브라우저의 연결을 차단했습니다. 다시 연결하거나 설정 파일을 확인해 주세요.":null}};
    const api={window:vi.fn().mockResolvedValue(undefined),preferences:vi.fn().mockResolvedValue(undefined),libraryQuery:vi.fn().mockResolvedValue({items:[],hasMore:false,nextOffset:0}),libraryUpdate:vi.fn().mockResolvedValue(false),snapshot:async()=>snapshot,subscribe:async()=>()=>{},shortcuts:async()=>()=>{},tab:vi.fn(),bounds:vi.fn().mockResolvedValue(undefined),network:vi.fn(),importConfig:vi.fn()} as typeof browserApi;
    await act(async()=>root.render(<BrowserApp api={api}/>));
    if(!["home","history","preferences"].includes(scene))await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="브라우저 연결 설정"]')!.click());
    if(scene==="history"||scene==="preferences"){
      api.libraryQuery=vi.fn().mockResolvedValue({items:[{id:"one",title:"아주 긴 제목과 여러 단어가 있는 페이지 제목을 읽을 수 있는 방문 기록",url:"https://example.com/very-long-path/to/a-page-with-a-long-address",visitedAt:1794750000000}],hasMore:true,nextOffset:1});
      await act(async()=>container.querySelector<HTMLButtonElement>(scene==="history"?'[aria-label="방문 기록"]':'[aria-label="브라우저 설정"]')!.click());
      await act(async()=>new Promise(resolve=>setTimeout(resolve,15)));
    }
    container.querySelector(".browser-tabbar")?.setAttribute("data-layout-probe","titlebar");
    for(const [i,button] of [...container.querySelectorAll(".browser-navigation .browser-icon-button,.browser-toolbar > .browser-icon-button,.browser-connection")].entries())button.setAttribute("data-layout-probe","toolbar-"+i);
    for(const [i,node] of [...container.querySelectorAll(".browser-address,.browser-settings-grid,.browser-network-card,.browser-home-search,.browser-panel-heading,.browser-country-picker select")].entries())node.setAttribute("data-layout-probe","layout-"+i);
    // innerHTML does not serialize the live selected property of controlled selects.
    for(const select of container.querySelectorAll("select"))for(const option of select.options)option.toggleAttribute("selected",option.value===select.value);
    for(const image of container.querySelectorAll("img"))image.src="data:image/svg+xml;charset=utf-8,"+encodeURIComponent(logo);
    output.push({name:scene,html:'<div id="root">'+container.innerHTML+'</div>'});
    await act(async()=>root.unmount());container.remove();
  }
  return output;
}
describe.skipIf(!layoutBrowser)("Atsumi browser real layout",()=>{
  it.each([[1200,850],[960,640],[720,500]])("keeps toolbar aligned and setup readable at %ix%i",async(width,height)=>{
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("ResizeObserver",class{observe(){}disconnect(){}});
    try{
      const results=await auditLayouts(await scenes(),css+"\n"+scrollbars,width,height,"browser-basics");
      expect(results.flatMap(r=>r.issues.map(i=>r.name+": "+i))).toEqual([]);
      for(const result of results){
        const probes=(result.geometry as {probes:Record<string,Box>}).probes;
        const buttons=Object.entries(probes).filter(([key])=>key.startsWith("toolbar-")).map(([,box])=>box);
        expect(probes.titlebar!.height).toBe(38);
        expect(buttons).toHaveLength(6);
        for(const box of buttons){expect(box.height).toBe(40);expect(box.y).toBe(buttons[0]!.y);expect(box.width).toBeGreaterThanOrEqual(40);}
      }
    }finally{vi.unstubAllGlobals();}
  },120000);
});
