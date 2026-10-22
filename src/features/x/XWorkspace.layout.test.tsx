import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { SideRail } from "../../components/SideRail";
import { XPreviewTabs } from "./XPreviewTabs";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../../test/offlineLayoutBrowser";
import css from "./XWorkspace.css?raw";
import archiveCss from "./XArchivePanel.css?raw";
import shellCss from "../../styles.css?raw";
import platformCss from "../platforms/PlatformWorkspace.css?raw";
const icon=(name:"more"|"check"|"close"|"refresh")=>'<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="'+({more:"M5 12h.01M12 12h.01M19 12h.01",check:"m5 12 4 4 10-10",close:"m6 6 12 12M18 6 6 18",refresh:"M20 11a8 8 0 1 0-2.3 6M20 4v7h-7"}[name])+'"/></svg>';
const account=(open=false,local=false)=>'<div class="x-sidebar-controls"><div class="x-floating-account"><button class="x-account-anchor" aria-label="계정"><span class="x-account-avatar">A</span><span class="x-account-copy">@sample_author</span><span class="x-account-more">'+icon("more")+'</span></button>'+(open?'<div class="x-floating-account-menu" data-native-overlay="true" role="menu"><div class="x-saved-accounts">'+["sample_author","second_account","third_account","long_author"].map((handle,index)=>'<button class="x-saved-account" role="menuitemradio" aria-checked="'+(index===0)+'" aria-label="@'+handle+'"><span class="x-account-avatar">A</span><span class="x-account-copy"><strong>표시 이름 '+(index+1)+'</strong><small>@'+handle+'</small></span>'+(index===0?'<span class="x-account-current">'+icon("check")+'</span>':'')+'</button>').join('')+'</div><button role="menuitem">계정 추가</button><button role="menuitem">현재 계정 정보 복구</button>'+(local?'':'<button role="menuitem" aria-label="새로고침">'+icon("refresh")+'새로고침</button>')+'</div>':'')+'</div></div>';
const skeleton='<div class="x-skeleton">'+Array.from({length:4},()=>'<div class="x-skeleton-post"><i></i><div><b></b><span></span><span></span><em></em></div></div>').join('')+'</div>';
const screen=(options:{preview?:boolean;background?:boolean;tabCount?:number;loading?:boolean;error?:string;collapsed?:boolean;menu?:boolean;local?:boolean;withoutAccount?:boolean;view?:"explore"|"likes"|"bookmarks"}={})=>{
  const{preview=false,background=false,tabCount=1,loading=false,error='',collapsed=false,menu=false,local=false,withoutAccount=false,view="explore"}=options;
  const rail=renderToStaticMarkup(<SideRail source="x" view={local?"local":view} collapsed={collapsed} autoFindCount={0} attentionCount={0} onNavigate={()=>{}} onSourceChange={()=>{}} onToggle={()=>{}} onSettings={()=>{}} onPrivacyModeToggle={()=>{}} footer={withoutAccount?undefined:<div className="x-sidebar-slot" dangerouslySetInnerHTML={{__html:account(menu,local)}}/>}/>);
  const tabs=renderToStaticMarkup(<XPreviewTabs tabs={Array.from({length:tabCount},(_,i)=>({id:`tab-${i}`,path:`/sample_author/status/12345${i}`,title:`@sample_author · 게시물 ${i+1}`,state:loading?"loading":"ready"}))} activeId={background?null:`tab-${tabCount-1}`} disabled={false} panelId="preview-panel" onSelect={()=>{}} onClose={()=>{}}/>);
  const body=local?'<section class="x-archive-library-view" aria-label="X 로컬 저장소"><div class="x-archive-timeline-shell"><header class="x-archive-timeline-heading"><div><h2>로컬 저장소</h2><span>0개 보관</span></div><button>새로고침</button></header><div class="x-archive-timeline" role="feed"><p class="x-archive-empty">보관된 항목이 없습니다.</p></div></div></section>':'<div class="x-body"><div class="x-native-feed">'+(loading?skeleton:'')+'</div><aside class="x-preview'+(preview||background?' has-selection':'')+'">'+(preview||background?tabs:'')+'<div class="x-native-preview" id="preview-panel">'+(preview?(loading?skeleton:''):'<p class="x-preview-empty">게시물을 선택하면 여기에 표시됩니다.</p>')+'</div></aside></div>';
  return '<div class="app-shell platform-shell'+(collapsed?' sidebar-collapsed':'')+'" data-x-sidebar-audit>'+rail+'<main class="platform-main"><section class="x-workspace x-official-workspace">'+(error?'<p class="x-notice" role="alert">'+error+'</p>':'')+body+'</section></main></div>';
};
describe.skipIf(!layoutBrowser)('X official surface shell geometry (not authenticated X acceptance)',()=>{
  // The native window has a 960px minimum width; narrower checks below isolate
  // the tab strip instead of pretending the whole app supports a 640px window.
  it.each([[960,640],[1920,1080]])('keeps sidebar and floating account controls reachable and icons centered at %ix%i',async(width,height)=>{
    const scenes:LayoutScene[]=[
      {name:'original-common-footer',html:screen({withoutAccount:true})},
      {name:'unselected',html:screen()},
      {name:'likes',html:screen({view:"likes"})},
      {name:'bookmarks',html:screen({view:"bookmarks"})},
      {name:'loading',html:screen({loading:true})},
      {name:'native-preview-slot',html:screen({preview:true})},
      {name:'three-retained-previews',html:screen({preview:true,tabCount:3})},
      {name:'sixteen-retained-previews',html:screen({preview:true,tabCount:16})},
      {name:'first-background-preview',html:screen({background:true})},
      {name:'preview-loading',html:screen({preview:true,loading:true})},
      {name:'account-mismatch',html:screen({error:'X 화면의 계정이 선택한 계정과 다릅니다. 계정 메뉴에서 현재 계정 정보를 복구해 주세요.'})},
      {name:'account-menu',html:screen({menu:true})},
      {name:'collapsed',html:screen({collapsed:true,preview:true})},
      {name:'collapsed-account-menu',html:screen({collapsed:true,menu:true})},
      {name:'local-storage',html:screen({local:true})},
      {name:'local-account-menu',html:screen({local:true,menu:true})},
    ];
    const results=await auditLayouts(scenes,shellCss+platformCss+archiveCss+css,width,height,"x-native-sidebar-"+width);
    expect(results.flatMap(result=>result.issues.map(issue=>result.name+": "+issue))).toEqual([]);
    type Geometry={xSidebarFooter:Record<string,{x:number;y:number;width:number;height:number}>;account:{x:number;y:number;width:number;height:number}|null;blackSurfaces:Record<string,string>};
    const baseline=results[0]!.geometry as Geometry;
    expect(Object.keys(baseline.xSidebarFooter)).toHaveLength(3);
    for(const result of results){
      const geometry=result.geometry as Geometry;
      for(const [label,expected] of Object.entries(baseline.xSidebarFooter)){
        expect(geometry.xSidebarFooter[label],result.name+": "+label).toBeDefined();
        expect(geometry.xSidebarFooter[label]!.y,result.name+": fixed "+label).toBeCloseTo(expected.y,1);
        expect(geometry.xSidebarFooter[label]!.height,result.name+": fixed height "+label).toBeCloseTo(expected.height,1);
      }
      if(geometry.account)expect(geometry.account.y+geometry.account.height,result.name+": account above feedback").toBeLessThanOrEqual(geometry.xSidebarFooter["오류·의견 보내기"]!.y);
      for(const [selector,color] of Object.entries(geometry.blackSurfaces))expect(color,result.name+": black "+selector).toBe("rgb(0, 0, 0)");
    }
  },60000);
  it.each([320,640])("keeps overflow tabs and their close hitboxes reachable in a %ipx preview pane",async width=>{
    const scenes:LayoutScene[]=[3,16].map(count=>({
      name:`preview-${count}-tabs`,
      html:renderToStaticMarkup(<section className="x-workspace x-official-workspace" data-x-sidebar-audit style={{height:"100vh"}}>
        <XPreviewTabs tabs={Array.from({length:count},(_,i)=>({id:`tab-${i}`,path:`/author/status/12345${i}`,title:`@sample_author · 게시물 ${i+1}`,state:"ready"}))}
          activeId={`tab-${count-1}`} disabled={false} panelId="preview-panel" onSelect={()=>{}} onClose={()=>{}}/>
        <div className="x-native-preview" id="preview-panel"/>
      </section>)
    }));
    const results=await auditLayouts(scenes,css,width,640,`x-preview-strip-${width}`);
    expect(results.flatMap(result=>result.issues.map(issue=>`${result.name}: ${issue}`))).toEqual([]);
  },60000);
});
