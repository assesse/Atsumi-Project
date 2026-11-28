import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { DetailWorkspace } from "./DetailWorkspace";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient } from "../thumbnail";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import { PersonalLibraryProvider } from "../features/personalLibrary/PersonalLibraryProvider";
import { createBrowserLibraryApi } from "../features/personalLibrary/api";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import previewCss from "./PagePreviewOverlay.css?raw";
const noop=()=>{};
const gallery={...mockGalleries[0]!,title:"페이지 프리뷰 · 폴더 이동 확인",pages:130,download:undefined,pageDimensions:[{sourcePage:11,width:800,height:1200}]};
const client=new ThumbnailClient({resolve:()=>({kind:"missing",reason:"layout fixture"})});
const api=createBrowserLibraryApi({getItem:()=>null,setItem:()=>{}});
describe.skipIf(!layoutBrowser)("persistent reading position layout",()=>{
 it.each([[1280,720],[1920,1080],[480,760]])("keeps counters and controls inside the preview at %ix%i",async(width,height)=>{
  Object.defineProperty(window,"innerWidth",{configurable:true,value:width});
  Object.defineProperty(window,"innerHeight",{configurable:true,value:height});
  const markup=renderToStaticMarkup(<PersonalLibraryProvider api={api}><DetailWorkspace tabs={[]} activeId={gallery.id} minimized={false}
    galleries={new Map([[gallery.id,gallery]])} favoriteMetadata={new Set()} thumbnailClient={client}
    previewOnly previewAlbumPosition={{index:10,total:12,label:"작가"}} pageOpenRequest={{galleryId:gallery.id,page:11,sequence:1}}
    onActivate={noop} onClose={noop} onCloseAll={noop} onMinimize={noop} onRestore={noop} onOpenRelated={noop}
    onQueue={noop} onMetadataSearch={noop} onMetadataFavorite={noop}/></PersonalLibraryProvider>);
  const doc=new DOMParser().parseFromString(markup,"text/html");
  const dialog=doc.querySelector<HTMLDialogElement>(".page-preview-dialog")!;dialog.open=true;
  const scenes=[false,true].map(visible=>{
    dialog.dataset.controlsVisible=String(visible);
    return {name:visible?"controls-visible":"counter-only",html:dialog.outerHTML};
  });
  const result=await auditLayouts(scenes,appCss+inkCss+previewCss,width,height,"reading-position-"+width);
  expect(result.flatMap(r=>r.issues.map(issue=>r.name+": "+issue))).toEqual([]);
 },60000);
});
