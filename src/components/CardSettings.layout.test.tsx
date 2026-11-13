import {act} from "react";
import {createRoot} from "react-dom/client";
import {describe,it,expect,vi} from "vitest";
import {SettingsDialog} from "./SettingsDialog";
import {GalleryGrid} from "./GalleryGrid";
import {renderToStaticMarkup} from "react-dom/server";
import {backend} from "../api/backend";
import {auditLayouts,layoutBrowser,type LayoutScene} from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import settingsCss from "./SettingsDialog.css?raw";
const noop=()=>{};
async function settingsScene():Promise<LayoutScene>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const desc=Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype,"showModal");
 Object.defineProperty(HTMLDialogElement.prototype,"showModal",{configurable:true,value(){this.setAttribute("open","");}});
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const result=await backend.settingsGet();if(!result.ok)throw Error('offline settings missing');
 try{
  await act(async()=>root.render(<SettingsDialog open settings={result.data} loading={false} error={null} onClose={noop} onSave={async()=>true}
   onLoadStorageUsage={async()=>({ok:false,error:{code:"UNAVAILABLE",message:"offline"}}) as never} onPreviewLayout={noop} onPreviewFolderName={async()=>({ok:true,data:"preview"})}
   onMaintenance={vi.fn()} onCheckForUpdates={vi.fn()} onTagCatalogRefresh={vi.fn()} tagCatalogRefreshing={false}
   onLoadExplorationExclusions={vi.fn()} onRestoreExplorationExclusions={vi.fn()}/>));
  const tabs=[...host.querySelectorAll<HTMLButtonElement>('[role=tab]')];expect(tabs).toHaveLength(9);
  expect(tabs.every(t=>t.querySelector('img[alt=""]'))).toBe(true);
  await act(async()=>tabs.find(t=>t.textContent==='X')!.click());
  expect(host.querySelectorAll('.x-collection-settings input[type=range]')).toHaveLength(1);
  return {name:'x-settings',html:host.innerHTML};
 }finally{await act(async()=>root.unmount());host.remove();if(desc)Object.defineProperty(HTMLDialogElement.prototype,'showModal',desc);else delete(HTMLDialogElement.prototype as {showModal?:unknown}).showModal;vi.unstubAllGlobals();}
}
describe.skipIf(!layoutBrowser)('card settings and centered Hitomi layout',()=>{
 it.each([[1920,1080],[1280,720],[390,800]])('fits icons, labels and X controls at %ix%i',async(width,height)=>{
  const scene=await settingsScene();const results=await auditLayouts([scene],appCss+inkCss+settingsCss,width,height,'card-settings');
  expect(results.flatMap(r=>r.issues)).toEqual([]);
 },60000);
 it.each([600,1000])('centers fixed-size Hitomi cards at %ipx without stretching',async width=>{
  const columns=Math.floor((width+12)/232);
  const html=renderToStaticMarkup(<GalleryGrid columns={columns} previewWidth={220} selectionContext={false} ariaLabel="앨범" displayMode="compact">{Array.from({length:columns},(_,i)=><article className="gallery-card is-compact" key={i} data-layout-probe={'card'+i} style={{height:330}}><div className="cover"/>검증 앨범</article>)}</GalleryGrid>);
  const [r]=await auditLayouts([{name:'centered-hitomi',html}],appCss+inkCss+'html,body{margin:0;padding:0}',width,720,'card-centering');
  const probes=(r!.geometry as {probes:Record<string,{x:number;width:number}>}).probes;
  expect(probes.card0!.width).toBeCloseTo(220,0);
  const last=probes['card'+(columns-1)]!;expect(probes.card0!.x).toBeCloseTo(width-last.x-last.width,0);
 },60000);
});
