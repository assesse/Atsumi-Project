import { afterEach, expect, it, vi } from "vitest";
import script from "../../../src-tauri/src/x_surface_ads.js?raw";

const identity='<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
const post=(badge="",body="Ordinary post",extra="")=>`<article data-testid="tweet"><div class="header"><div data-testid="User-Name"><a href="/creator">Creator</a><a href="/creator/status/123456"><time>now</time></a></div><span class="badge">${badge}</span><button data-testid="caret">More</button></div><p data-testid="tweetText">${body}</p>${extra}<div role="group"><button data-testid="like">Like</button></div></article>`;
const outer=(html:string)=>`<div data-testid="placementTracking">${html}</div>`;
const cell=(html:string)=>`<div data-testid="cellInnerDiv">${html}<div data-testid="placementTracking"></div></div>`;
type HeaderShape={inspected:number;namesMissing:number;namesMultiple:number;stoppedContent:number;stoppedRoleLink:number;roleLinkInName:number;stoppedContext:number;caretMissing:number;caretMultiple:number;candidateLimit:number;depthLimit:number;labelAbsent:number;labelAuthor:number;labelControl:number;labelScope:number;labelNonLeaf:number;labelHidden:number;outerLabel:number};
type Shape={ready:boolean;articles:number;outerPlacements:number;headerBadges:number;promotedIndicators:number;hidden:number;ambiguous:number;limited:boolean;multiplePlacements:number;sharedPlacements:number;structureLimited:number;metadataMissing:number;headerShape:HeaderShape};
type Api={update(ready:boolean):void;blocked(node:Element):boolean;clear():void;diagnostic():Shape|null};
type Change={type:string;target:Node;attributeName?:string;oldValue?:string|null;addedNodes?:Node[];removedNodes?:Node[]};
function adapter(html=outer(post("광고")),options:{host?:string;frame?:boolean;identity?:string;path?:string;diagnostics?:boolean}={}) {
  const doc=new DOMParser().parseFromString((options.identity??identity)+'<main data-testid="primaryColumn">'+html+'</main>',"text/html");
  Object.defineProperty(doc,"hidden",{value:false,configurable:true});
  const origin="https://"+(options.host??"x.com"),location={origin,pathname:options.path??"/home"};
  const win={top:null as unknown,__atsumiXAds:undefined as Api|undefined};win.top=options.frame?{}:win;
  const timers:Array<()=>void>=[],observers:Array<{callback:(records:Change[])=>void;active:boolean;options?:MutationObserverInit}>=[];
  class Observer {
    entry:{callback:(records:Change[])=>void;active:boolean;options?:MutationObserverInit};
    constructor(callback:(records:Change[])=>void){this.entry={callback,active:false};observers.push(this.entry);}
    observe(_node:Node,options:MutationObserverInit){this.entry.active=true;this.entry.options=options;}
    disconnect(){this.entry.active=false;}
  }
  const computed=(node:HTMLElement)=>({
    display:node.getAttribute("data-atsumi-x-ad-hidden")==="promoted-v1"?"none":node.style.display||"block",
    visibility:node.classList.contains("source-hidden")?"hidden":node.style.visibility||"visible",
    opacity:node.style.opacity||"1",
  });
  new Function("window","document","location","URL","Element","getComputedStyle","MutationObserver","setTimeout",
    script.replace("__ATSUMI_X_ADS__",JSON.stringify({owner:"alice",diagnostics:options.diagnostics??true})))(
    win,doc,location,URL,Element,computed,Observer,(fn:()=>void)=>{timers.push(fn);return timers.length;});
  const update=()=>win.__atsumiXAds?.update(true);
  const article=()=>doc.querySelector<HTMLElement>("article")!;
  const notify=(changes:Change[])=>observers.filter(value=>value.active).forEach(value=>value.callback(changes));
  const flush=()=>{const jobs=timers.splice(0);jobs.forEach(fn=>fn());};
  const changed=(target:Node,type="childList",attributeName?:string,oldValue?:string|null)=>{
    notify([{target,type,attributeName,oldValue,addedNodes:[],removedNodes:[]}]);flush();
  };
  const hidden=()=>doc.querySelectorAll('[data-atsumi-x-ad-hidden="promoted-v1"]').length;
  return {doc,win,location,timers,observers,update,article,notify,flush,changed,hidden};
}
afterEach(()=>vi.restoreAllMocks());
it.each(["Ad","Promoted","광고"])("hides only an exclusive outer placement with its own header %s badge",badge=>{
  const a=adapter(outer(post(badge))+post("", "Preserved normal",'<img src="https://pbs.twimg.com/media/Normal"><video src="https://video.twimg.com/normal.mp4"></video>'));
  const image=a.doc.querySelector("img")!,video=a.doc.querySelector("video")!,normal=a.doc.querySelectorAll("article")[1]!;
  const html=normal.outerHTML;
  a.update();expect(a.hidden()).toBe(1);expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(true);
  expect(a.win.__atsumiXAds!.blocked(normal)).toBe(false);expect(normal.outerHTML).toBe(html);
  expect(a.doc.querySelector("img")).toBe(image);expect(a.doc.querySelector("video")).toBe(video);
});
it("recognizes an outside-article sibling placement only within its one-post cell",()=>{
  const a=adapter(cell(post("Ad")));a.update();
  expect(a.hidden()).toBe(1);expect(a.doc.querySelector('[data-testid="cellInnerDiv"]')!.getAttribute("data-atsumi-x-ad-hidden")).toBe("promoted-v1");
});
it.each([
  ["no badge",outer(post())],
  ["badge without placement",post("Ad")],
  ["post body",outer(post("","Ad Promoted 광고"))],
  ["author name",outer(post().replace(">Creator<",">Ad<"))],
  ["paid partnership",outer(post("Paid partnership"))],
  ["inner ordinary video tracking",post("Ad","Body",'<div data-testid="videoPlayer"><div data-testid="placementTracking"><video></video></div></div>')],
  ["quoted promotion",outer(post("","Body",'<div data-testid="quoteTweet"><div data-testid="promotedIndicator">Ad</div>'+post("Ad")+'</div>'))],
  ["repost context",outer(post().replace('<p data-testid="tweetText">','<div data-testid="socialContext">Ad reposted</div><p data-testid="tweetText">'))],
  ["repost indicator",post("","Body",'<div data-testid="socialContext"><span data-testid="promotedIndicator">Ad</span></div>')],
  ["many posts",outer(post("Ad")+post())],
  ["many sibling posts",cell(post("Ad")+post())],
  ["many placements without a shared exclusive cell",outer(outer(post("Ad")))],
  ["hidden badge",outer(post("Ad").replace('class="badge"','class="badge" hidden'))],
  ["button label",outer(post().replace('>More<','>Ad<'))],
  ["body spoofed indicator",outer(post("","<span data-testid=\"promotedIndicator\">Ad</span>"))],
  ["unknown disclosure",outer(post("Sponsored partnership"))],
])("fails open for %s",(_name,html)=>{
  const a=adapter(html);a.update();expect(a.hidden()).toBe(0);
  expect([...a.doc.querySelectorAll("article")].every(node=>!a.win.__atsumiXAds!.blocked(node))).toBe(true);
});
it("accepts an explicit own promotedIndicator but not one in a quote or author",()=>{
  const a=adapter(post("","Body",'<div data-testid="promotedIndicator">Ad</div>'));a.update();expect(a.hidden()).toBe(1);
  const b=adapter(post().replace("Creator",'<span data-testid="promotedIndicator">Ad</span>'));b.update();expect(b.hidden()).toBe(0);
});
it("restores ad-to-normal source recycling without replacing source nodes or source style",()=>{
  const a=adapter();a.update();const article=a.article(),wrapper=article.parentElement!;
  const image=a.doc.createElement("img");article.append(image);wrapper.setAttribute("style","color:red");
  a.doc.querySelector(".badge")!.textContent="";
  a.changed(a.doc.querySelector(".badge")!.firstChild??a.doc.querySelector(".badge")!,"characterData");
  expect(a.hidden()).toBe(0);expect(a.article()).toBe(article);expect(article.querySelector("img")).toBe(image);
  expect(wrapper.getAttribute("style")).toBe("color:red");expect(a.win.__atsumiXAds!.blocked(article)).toBe(false);
});
it("applies normal-to-ad recycling and clears stale marking during a dispatch-time check",()=>{
  const a=adapter(outer(post()));a.update();expect(a.hidden()).toBe(0);
  a.doc.querySelector(".badge")!.textContent="Ad";
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(true);expect(a.hidden()).toBe(1);
  a.doc.querySelector(".badge")!.textContent="Paid partnership";
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(false);expect(a.hidden()).toBe(0);
});
it("does not inherit a marked wrapper after replacement with a different ordinary article",()=>{
  const a=adapter();a.update();const old=a.article(),wrapper=old.parentElement!;
  const replacement=new DOMParser().parseFromString(post(),"text/html").querySelector("article")!;
  wrapper.replaceChild(a.doc.importNode(replacement,true),old);
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(false);expect(a.hidden()).toBe(0);
  expect(a.win.__atsumiXAds!.blocked(old)).toBe(false);
});
it("restores on class/style-only source metadata hiding and rechecks later hydration",()=>{
  const a=adapter();a.update();const badge=a.doc.querySelector<HTMLElement>(".badge")!;
  badge.classList.add("source-hidden");a.changed(badge,"attributes","class","badge");
  expect(a.hidden()).toBe(0);badge.classList.remove("source-hidden");a.changed(badge,"attributes","class","badge source-hidden");
  expect(a.hidden()).toBe(1);badge.style.display="none";a.changed(badge,"attributes","style",null);expect(a.hidden()).toBe(0);
});
it.each(["inert","type","autocomplete"])("observes source %s changes that revoke eligibility",attribute=>{
  const a=adapter();a.update();const node=attribute==="inert"?a.doc.querySelector("main")!:a.doc.createElement("input");
  if(attribute!=="inert")a.doc.body.append(node);
  node.setAttribute(attribute,attribute==="type"?"password":attribute==="autocomplete"?"one-time-code":"");
  expect(a.observers[0]!.options?.attributeFilter).toContain(attribute);
  a.changed(node,"attributes",attribute,null);expect(a.hidden()).toBe(0);
});
it.each(["owner","login","security","photo","video","modal","fullscreen","privacy","primary-hidden"])("revokes suppression for %s without using old readiness",reason=>{
  const a=adapter();a.update();expect(a.hidden()).toBe(1);
  if(reason==="owner")a.doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute("href","/bob");
  if(reason==="login")a.location.pathname="/i/flow/login";
  if(reason==="security")a.doc.body.append(a.doc.createElement("input"));if(reason==="security")a.doc.querySelector("input")!.type="password";
  if(reason==="photo"||reason==="video")a.location.pathname="/creator/status/123456/"+reason+"/1";
  if(reason==="modal")a.doc.body.append(Object.assign(a.doc.createElement("div"),{role:"dialog"}));
  if(reason==="fullscreen")Object.defineProperty(a.doc,"fullscreenElement",{value:a.article()});
  if(reason==="privacy")a.win.__atsumiXAds!.clear();
  if(reason==="primary-hidden")a.doc.querySelector<HTMLElement>("main")!.style.display="none";
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(false);expect(a.hidden()).toBe(0);
});
it.each([{host:"evil.test"},{host:"x.com.evil.test"},{frame:true}])("does not initialize outside the top-level admitted X origin: %j",options=>{
  expect(adapter(undefined,options).win.__atsumiXAds).toBeUndefined();
});
it("requires current exact account/profile evidence, not a post author or cached update argument",()=>{
  for(const identityValue of ["",identity.replace("/alice","/bob"),identity+'<a data-testid="AppTabBar_Profile_Link" href="/bob"></a>']){
    const a=adapter(undefined,{identity:identityValue});a.update();expect(a.hidden()).toBe(0);
  }
});
it("pauses only already-playing HTML media inside a proved ad without rewriting media settings",()=>{
  const html='<video src="https://video.twimg.com/normal.mp4"></video>';
  const a=adapter(outer(post("Ad","Ad media",html))+post("","Normal media",html));
  const videos=[...a.doc.querySelectorAll("video")];
  for(const video of videos)Object.defineProperty(video,"paused",{value:false});
  const pauses=videos.map(video=>vi.spyOn(video,"pause").mockImplementation(()=>{}));
  const before=videos.map(video=>({src:video.src,muted:video.muted,volume:video.volume}));
  a.update();expect(pauses[0]).toHaveBeenCalledOnce();expect(pauses[1]).not.toHaveBeenCalled();
  expect(videos.map(video=>({src:video.src,muted:video.muted,volume:video.volume}))).toEqual(before);
});
it("coalesces source changes but ignores app-owned controls and no-op style/class mutation loops",()=>{
  const a=adapter();a.update();const article=a.article();
  a.notify([{type:"childList",target:article,addedNodes:[],removedNodes:[]}]);
  a.notify([{type:"characterData",target:article}]);expect(a.timers).toHaveLength(1);a.flush();
  a.notify([{type:"attributes",target:article,attributeName:"class",oldValue:article.getAttribute("class")}]);
  const button=a.doc.createElement("button");button.setAttribute("data-atsumi-post-save","true");article.append(button);
  a.notify([{type:"childList",target:article,addedNodes:[button],removedNodes:[]}]);
  a.notify([{type:"attributes",target:button,attributeName:"style",oldValue:null}]);
  expect(a.timers).toHaveLength(0);
  expect(a.observers[0]!.options?.attributeFilter).not.toContain("data-atsumi-x-ad-hidden");
});
it("drops queued work on clear and keeps restore idempotent across detached nodes",()=>{
  const a=adapter();a.update();const wrapper=a.article().parentElement!;
  a.notify([{type:"characterData",target:a.article()}]);wrapper.remove();a.win.__atsumiXAds!.clear();a.flush();
  expect(wrapper.hasAttribute("data-atsumi-x-ad-hidden")).toBe(false);expect(a.observers.every(value=>!value.active)).toBe(true);
  a.win.__atsumiXAds!.clear();expect(a.hidden()).toBe(0);expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject({ready:false,hidden:0});
});
it("bounds source scanning and emits closed counts without text, URLs or identity",()=>{
  const a=adapter(outer(post("Ad")).repeat(161));a.update();
  expect(a.hidden()).toBe(0);expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject({articles:160,limited:true,hidden:0});
  const b=adapter();b.update();const shape=b.win.__atsumiXAds!.diagnostic()!;
  expect(Object.keys(shape).sort()).toEqual(["ambiguous","articles","headerBadges","headerShape","hidden","limited","metadataMissing","multiplePlacements","outerPlacements","promotedIndicators","ready","sharedPlacements","structureLimited"]);
  const {headerShape,...counts}=shape;
  expect(Object.values(counts).every(value=>typeof value==="boolean"||typeof value==="number"&&value>=0&&value<=160)).toBe(true);
  expect(Object.keys(headerShape).sort()).toEqual(["candidateLimit","caretMissing","caretMultiple","depthLimit","inspected","labelAbsent","labelAuthor","labelControl","labelHidden","labelNonLeaf","labelScope","namesMissing","namesMultiple","outerLabel","roleLinkInName","stoppedContent","stoppedContext","stoppedRoleLink"]);
  expect(Object.values(headerShape).every(value=>Number.isInteger(value)&&value>=0&&value<=160)).toBe(true);
  expect(JSON.stringify(shape)).not.toMatch(/alice|creator|https|Ad |광고/);
  const release=adapter(undefined,{diagnostics:false});release.update();expect(release.win.__atsumiXAds!.diagnostic()).toBeNull();
});
it.each<[string,string,Partial<Shape>]>([
  ["multiple single-post markers outside one cell",outer(outer(post("Ad"))),{multiplePlacements:1,sharedPlacements:0,structureLimited:0,metadataMissing:0}],
  ["shared outer placement",outer(post("Ad")+post()),{multiplePlacements:0,sharedPlacements:2,structureLimited:0,metadataMissing:0}],
  ["oversized article",post("Ad","Body","<span></span>".repeat(1025)),{multiplePlacements:0,sharedPlacements:0,structureLimited:1,metadataMissing:0}],
  ["too-deep ancestry","<div>".repeat(33)+outer(post("Ad"))+"</div>".repeat(33),{multiplePlacements:0,sharedPlacements:0,structureLimited:1,metadataMissing:0}],
  ["no metadata",outer(post()),{multiplePlacements:0,sharedPlacements:0,structureLimited:0,metadataMissing:1}],
])("diagnoses %s with closed counts without changing admission",(_reason,html,expected)=>{
  const a=adapter(html);a.update();expect(a.hidden()).toBe(0);
  expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject(expected);
  const before=a.doc.documentElement.outerHTML;
  expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject(expected);
  expect(a.doc.documentElement.outerHTML).toBe(before);
  const release=adapter(html,{diagnostics:false});release.update();
  expect(release.hidden()).toBe(a.hidden());expect(release.win.__atsumiXAds!.diagnostic()).toBeNull();
});
const tracking='<div data-testid="placementTracking"></div>';
const multiCell=(html:string,markers=tracking.repeat(2))=>`<div data-testid="cellInnerDiv">${html}${markers}</div>`;
const diagnosticName='<div data-testid="User-Name"><a href="/creator">Creator</a></div>';
const diagnosticCaret='<button data-testid="caret">More</button>';
const diagnosticArticle=(header:string)=>`<article data-testid="tweet">${header}<p data-testid="tweetText">Private body Ad</p></article>`;
it.each<[string,string,Partial<HeaderShape>]>([
  ["a badge outside the first caret ancestor",`<div><div>${diagnosticName}${diagnosticCaret}</div><span>광고</span></div>`,{labelAbsent:1,outerLabel:1,stoppedContent:0}],
  ["an affiliation role link inside the own name",`<div>${diagnosticName.replace('</div>','<div role="link">Affiliation</div></div>')}<span>Ad</span>${diagnosticCaret}</div>`,{stoppedRoleLink:1,roleLinkInName:1}],
  ["a header control label",`<div>${diagnosticName}<button><span>Ad</span></button>${diagnosticCaret}</div>`,{labelControl:1,labelAbsent:0}],
  ["a hidden badge",`<div>${diagnosticName}<span hidden>Ad</span>${diagnosticCaret}</div>`,{labelHidden:1}],
  ["an author name word",`<div>${diagnosticName.replace('Creator','Ad')}${diagnosticCaret}</div>`,{labelAuthor:1,labelNonLeaf:1}],
  ["missing own name",`<div><span>Ad</span>${diagnosticCaret}</div>`,{namesMissing:1}],
  ["multiple own names",`<div>${diagnosticName.repeat(2)}<span>Ad</span>${diagnosticCaret}</div>`,{namesMultiple:1}],
  ["body contamination",`<div>${diagnosticName}<p data-testid="tweetText">Ad</p>${diagnosticCaret}</div>`,{stoppedContent:1}],
  ["social context contamination",`<div>${diagnosticName}<span data-testid="socialContext">Ad</span>${diagnosticCaret}</div>`,{stoppedContext:1}],
  ["missing caret",`<div>${diagnosticName}<span>Ad</span></div>`,{caretMissing:1}],
  ["multiple carets",`<div>${diagnosticName}<span>Ad</span>${diagnosticCaret.repeat(2)}</div>`,{caretMultiple:1}],
  ["candidate bound",`<div>${diagnosticName}${'<span></span>'.repeat(65)}<span>Ad</span>${diagnosticCaret}</div>`,{candidateLimit:1}],
  ["header depth bound",'<div>'.repeat(9)+diagnosticName+'</div>'.repeat(9),{depthLimit:1}],
])("diagnoses %s without admitting that rejected placement",(_name,header,expected)=>{
  const html=multiCell(diagnosticArticle(header));
  const a=adapter(html),release=adapter(html,{diagnostics:false});a.update();release.update();
  expect(a.hidden()).toBe(0);expect(release.hidden()).toBe(0);
  expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject({metadataMissing:1,headerBadges:0,headerShape:{inspected:1,...expected}});
  expect(a.doc.documentElement.outerHTML).toBe(release.doc.documentElement.outerHTML);
  const before=a.doc.documentElement.outerHTML;
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(false);
  expect(a.doc.documentElement.outerHTML).toBe(before);
});
it("only diagnoses missing metadata in proved placements and bounds aggregate results",()=>{
  const ordinary=adapter(diagnosticArticle(`<div>${diagnosticName}${diagnosticCaret}<span hidden>Ad</span></div>`));ordinary.update();
  expect(ordinary.win.__atsumiXAds!.diagnostic()!.headerShape.inspected).toBe(0);
  const rejected=multiCell(diagnosticArticle(`<div>${diagnosticName}${diagnosticCaret}<span hidden>Ad</span></div>`));
  const bounded=adapter(rejected.repeat(160));bounded.update();
  expect(bounded.win.__atsumiXAds!.diagnostic()!.headerShape).toMatchObject({inspected:160,labelHidden:160});
  expect(Object.values(bounded.win.__atsumiXAds!.diagnostic()!.headerShape).every(value=>value<=160)).toBe(true);
});
it("returns copied closed header counters without leaking text or allowing diagnostic mutation",()=>{
  const a=adapter(multiCell(post()));a.update();
  const shape=a.win.__atsumiXAds!.diagnostic()!;
  expect(JSON.stringify(shape)).not.toMatch(/alice|creator|https|Private|광고/);
  shape.headerShape.inspected=160;
  expect(a.win.__atsumiXAds!.diagnostic()!.headerShape.inspected).toBe(1);
  a.win.__atsumiXAds!.clear();expect(a.win.__atsumiXAds!.diagnostic()!.headerShape.inspected).toBe(0);
});
it.each([
  ["two siblings",multiCell(post("Ad"))],
  ["ancestor and sibling",cell(outer(post("광고")))],
  ["two nested ancestors",`<div data-testid="cellInnerDiv">${outer(outer(post("Promoted")))}</div>`],
  ["sixteen siblings",multiCell(post("Ad"),tracking.repeat(16))],
])("accepts multiple external markers only in one proved own-header ad cell: %s",(_name,html)=>{
  const a=adapter(html+post("","Normal",'<img src="https://pbs.twimg.com/media/Normal"><video></video>'));
  const normal=a.doc.querySelectorAll("article")[1]!,before=normal.outerHTML;
  a.update();expect(a.hidden()).toBe(1);
  expect(a.doc.querySelector('[data-testid="cellInnerDiv"]')!.getAttribute("data-atsumi-x-ad-hidden")).toBe("promoted-v1");
  expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(true);expect(a.win.__atsumiXAds!.blocked(normal)).toBe(false);
  expect(normal.outerHTML).toBe(before);
  expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject({multiplePlacements:1,hidden:1,ambiguous:0,outerPlacements:1});
});
it.each([
  ["missing own badge",multiCell(post())],
  ["indicator does not substitute for own badge",multiCell(post("","Body",'<span data-testid="promotedIndicator">Ad</span>'))],
  ["paid partnership",multiCell(post("Paid partnership"))],
  ["body advertising words",multiCell(post("","Ad Promoted 광고"))],
  ["author advertising words",multiCell(post().replace(">Creator<",">Ad<"))],
  ["quoted advertising words",multiCell(post("","Body",'<div data-testid="quoteTweet"><span>Ad</span></div>'))],
  ["shared ordinary post",multiCell(post("Ad")+post())],
  ["shared quoted article",multiCell(post("Ad","Body",'<div data-testid="quoteTweet">'+post()+'</div>'))],
  ["outside-cell ancestor",outer(cell(post("Ad")))],
  ["nested different cell marker",multiCell(post("Ad"),tracking+`<div data-testid="cellInnerDiv">${tracking}</div>`)],
  ["another cell has its own post",multiCell(post("Ad"),tracking+`<div data-testid="cellInnerDiv">${post()}${tracking}</div>`)],
  ["seventeen markers",multiCell(post("Ad"),tracking.repeat(17))],
])("keeps ambiguous or unproved multiple placements visible: %s",(_name,html)=>{
  const a=adapter(html);a.update();expect(a.hidden()).toBe(0);expect(a.win.__atsumiXAds!.blocked(a.article())).toBe(false);
});
it("ignores ordinary inner-video tracking while proving external markers independently",()=>{
  const video='<div data-testid="videoPlayer">'+tracking+'<video></video></div>';
  const a=adapter(multiCell(post("Ad","Ad",video))+post("","Normal",video));
  a.update();expect(a.hidden()).toBe(1);
  expect(a.win.__atsumiXAds!.blocked(a.doc.querySelectorAll("article")[1]!)).toBe(false);
  const normal=adapter(post("Ad","Normal",video));normal.update();expect(normal.hidden()).toBe(0);
});
it("rechecks one-to-many-to-one marker hydration and restores the same cell after proof disappears",()=>{
  const a=adapter(cell(post("Ad")));a.update();expect(a.hidden()).toBe(1);
  const cellNode=a.doc.querySelector('[data-testid="cellInnerDiv"]')!;
  const second=a.doc.createElement("div");second.setAttribute("data-testid","placementTracking");cellNode.append(second);
  a.changed(cellNode);expect(a.hidden()).toBe(1);expect(a.win.__atsumiXAds!.diagnostic()?.multiplePlacements).toBe(1);
  second.remove();a.changed(cellNode);expect(a.hidden()).toBe(1);
  cellNode.querySelector('[data-testid="placementTracking"]')!.remove();a.changed(cellNode);expect(a.hidden()).toBe(0);
});
it("does not invalidate its own hidden cell or infer advertising from viewport entry and exit",()=>{
  const a=adapter(multiCell(post('Ad'))+post('','Ordinary', '<video></video>'));a.update();
  const ad=a.article(),cellNode=ad.parentElement!,normal=a.doc.querySelectorAll('article')[1]!;
  let y=-500;
  vi.spyOn(ad,'getBoundingClientRect').mockImplementation(()=>({top:y,bottom:y+400,left:0,right:600,width:600,height:400,x:0,y,toJSON:()=>({})}));
  for(let i=0;i<12;i++){
    y=i%2?1000:-500;cellNode.setAttribute('style',`transform:translateY(${y}px)`);a.changed(cellNode,'attributes','style','');
    expect(a.hidden()).toBe(1);expect(a.win.__atsumiXAds!.blocked(ad)).toBe(true);expect(a.win.__atsumiXAds!.blocked(normal)).toBe(false);
    expect(a.win.__atsumiXAds!.diagnostic()).toMatchObject({headerBadges:1,metadataMissing:0,hidden:1});
  }
  a.doc.querySelector('.badge')!.textContent='Ordinary';a.changed(ad);
  expect(a.hidden()).toBe(0);expect(a.win.__atsumiXAds!.blocked(ad)).toBe(false);
});
it("revokes a previously proved multi-marker dispatch when the cell gains another post or loses its header badge",()=>{
  for(const change of ["new post","badge"]){
    const a=adapter(multiCell(post("Ad")));a.update();const original=a.article();
    if(change==="new post"){
      const next=new DOMParser().parseFromString(post(),"text/html").querySelector("article")!;
      original.parentElement!.append(a.doc.importNode(next,true));
    }else a.doc.querySelector(".badge")!.textContent="";
    expect(a.win.__atsumiXAds!.blocked(original)).toBe(false);expect(a.hidden()).toBe(0);
  }
});
it("does not use network/private state, remove source nodes, synthesize source actions or inspect iframe contents",()=>{
  expect(script).not.toMatch(/fetch\(|XMLHttpRequest|document\.cookie|localStorage|sessionStorage|__react|innerHTML|outerHTML|\.click\(|contentWindow|contentDocument/);
  const a=adapter();const buttons=[...a.doc.querySelectorAll("button")].map(button=>vi.spyOn(button,"click"));
  a.update();a.win.__atsumiXAds!.clear();expect(buttons.every(button=>button.mock.calls.length===0)).toBe(true);
  expect(Object.isFrozen(a.win.__atsumiXAds)).toBe(true);
});
