import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type XMediaSearchCandidate = {title:string;creator:string|null;similarity:number|null;links:{label:string;url:string}[];thumbnailData:string|null;sourceMetadata:{label:string;value:string}[]};
export type XArchiveSearchArgs={id:string;archiveId:string;postId:string;assetName:string;localContext:string;kind:"image"|"thumbnail"};
export type XCreatorLinks={profileUrl:string;observedAt:string;links:{label:string;url:string}[];message:string};
export type XSearchProvider="saucenao"|"lens";
export type XPageEvidence={requestedUrl:string;url:string;observedAt:string;fields:{label:string;value:string;source:string}[];limited:boolean;ambiguous:boolean};
type WorkField={label:string;value:string;source:string};
export type XWorkContext={group:number;status:"candidate"|"supported"|"source_link"|"conflict"|"page";kind:"page"|"work"|"product"|"upload";association:"unverified"|"same_page"|"source_link"|"identifier"|"explicit_link"|"conflict";pageStatus:"not_read"|"unavailable"|"read";pageTitle:string|null;claims:{provider:XSearchProvider;position:number;title:string;url:string;creator:string|null;similarity:number|null;metadata:{label:string;value:string}[]}[];reasons:string[]};
export type XWorkInfo={title:string|null;titleEvidence?:WorkField;url:string;rootUrl:string;origin:string;observedAt:string;codes:WorkField[];sale:WorkField[];ambiguous:boolean;context?:XWorkContext};
export type XWorkEvidence={reportId:string;works:XWorkInfo[];examined:number;unavailable:number;limited:boolean;sourceStatus?:"complete"|"partial"|"unavailable"};
export type XProviderResult={provider:XSearchProvider;phase:"searching"|"verification"|"complete"|"failed";message:string;candidates:XMediaSearchCandidate[];originalPageState:"available"|"expired"|"closed"|"unavailable"};
export type XSearchSnapshot = {
  id:string;source:"surface"|"archive";archiveId:string|null;assetName:string|null;localContext:string|null;
  generation:number|null;accountId:string|null;feed:string|null;pane:"feed"|"preview"|"archive";
  kind:"image"|"video"|"thumbnail";provider:"saucenao"|"lens";phase:"consent"|"searching"|"verification"|"complete"|"failed";
  imageUrl:string|null;previewData:string|null;message:string;candidates:XMediaSearchCandidate[];
  originalPageState:"available"|"expired"|"closed"|"unavailable";
  sourcePostUrl:string|null;
  providers?:XProviderResult[];
};
export type XMediaSearchApi = {
  status():Promise<XSearchSnapshot|null>;
  subscribe(handler:(value:XSearchSnapshot|null)=>void):Promise<()=>void>;
  confirm(id:string,remember:boolean):Promise<XSearchSnapshot|null>;
  resume(id:string,provider?:XSearchProvider):Promise<XSearchSnapshot|null>;
  cancel(id:string):Promise<void>;
  resetConsent():Promise<void>;
  openResult(id:string,candidateIndex:number,linkIndex:number,provider?:XSearchProvider):Promise<void>;
  pageEvidence?(id:string,candidateIndex:number,linkIndex:number,provider?:XSearchProvider):Promise<XPageEvidence>;
  workEvidence?(id:string):Promise<XWorkEvidence>;
  openWorkEvidence?(id:string,reportId:string,index:number,origin?:boolean):Promise<void>;
  openOriginal(id:string,provider?:XSearchProvider):Promise<void>;
  openLensSettings(id:string):Promise<void>;
  refreshLensResults(id:string):Promise<XSearchSnapshot>;
  startArchive(args:XArchiveSearchArgs):Promise<void>;
  creatorLinks(id:string,candidateIndex?:number,linkIndex?:number):Promise<XCreatorLinks>;
  openCreatorLink(id:string,linkIndex:number):Promise<void>;
};
const uuid=/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/;
const fields=(value:unknown,keys:string[]):value is Record<string,unknown>=>!!value&&typeof value==="object"&&!Array.isArray(value)
  &&Object.keys(value).length===keys.length&&keys.every(key=>Object.hasOwn(value,key));
const text=(value:unknown,max:number,empty=true):value is string=>typeof value==="string"&&value.length<=max&&(empty||value.length>0)&&!/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value);
export function isXMediaSearchLink(value:unknown):value is string {
  if(typeof value!=="string"||!value||value.length>2048||/[\s\u0000-\u001f\u007f\\]/u.test(value))return false;
  try{const url=new URL(value);return ["http:","https:"].includes(url.protocol)&&!!url.hostname&&!url.username&&!url.password&&url.href===value;}catch{return false;}
}
export function isXCreatorPostLink(value:unknown):value is string {
  if(!isXMediaSearchLink(value))return false;
  const url=new URL(value);
  return url.protocol==="https:"&&["x.com","www.x.com","twitter.com","www.twitter.com"].includes(url.hostname)&&!url.port&&!url.search&&!url.hash&&/^\/[A-Za-z0-9_]{1,15}\/status\/\d{1,32}$/.test(url.pathname);
}
export function isXCreatorLinks(value:unknown):value is XCreatorLinks {
  if(!fields(value,["profileUrl","observedAt","links","message"])||!isXMediaSearchLink(value.profileUrl)||!text(value.observedAt,64,false)||!Number.isFinite(Date.parse(value.observedAt))||!text(value.message,1024)||!Array.isArray(value.links)||value.links.length>8)return false;
  const url=new URL(value.profileUrl);
  return url.protocol==="https:"&&url.hostname==="x.com"&&!url.port&&!url.search&&!url.hash&&/^\/[A-Za-z0-9_]{1,15}$/.test(url.pathname)
    &&value.links.every(link=>fields(link,["label","url"])&&text(link.label,128)&&isXMediaSearchLink(link.url));
}
export function isXPageEvidence(value:unknown):value is XPageEvidence {
  return fields(value,["requestedUrl","url","observedAt","fields","limited","ambiguous"])
    &&isXMediaSearchLink(value.requestedUrl)&&isXMediaSearchLink(value.url)
    &&text(value.observedAt,64,false)&&Number.isFinite(Date.parse(value.observedAt))
    &&typeof value.limited==="boolean"&&typeof value.ambiguous==="boolean"
    &&Array.isArray(value.fields)&&value.fields.length<=24
    &&value.fields.every(item=>fields(item,["label","value","source"])&&text(item.label,128,false)&&text(item.value,2048,false)&&text(item.source,128,false));
}
export function isXWorkEvidence(value:unknown):value is XWorkEvidence {
  const count=(v:unknown):v is number=>typeof v==="number"&&Number.isInteger(v)&&v>=0&&v<=24;
  const fact=(v:unknown)=>fields(v,["label","value","source"])&&text(v.label,128,false)&&text(v.value,256,false)&&text(v.source,128,false);
  const titleFact=(v:unknown,title:unknown)=>fields(v,["label","value","source"])&&text(v.label,128,false)&&text(v.value,512,false)&&v.value===title&&text(v.source,128,false);
  const keys=["reportId","works","examined","unavailable","limited"];
  const workKeys=["title","url","rootUrl","origin","observedAt","codes","sale","ambiguous"];
  const context=(v:unknown)=>fields(v,["group","status","kind","association","pageStatus","pageTitle","claims","reasons"])
    &&Number.isInteger(v.group)&&Number(v.group)>=0&&Number(v.group)<48
    &&typeof v.status==="string"&&["candidate","supported","source_link","conflict","page"].includes(v.status)
    &&typeof v.kind==="string"&&["page","work","product","upload"].includes(v.kind)
    &&typeof v.association==="string"&&["unverified","same_page","source_link","identifier","explicit_link","conflict"].includes(v.association)
    &&typeof v.pageStatus==="string"&&["not_read","unavailable","read"].includes(v.pageStatus)
    &&(v.pageTitle===null||text(v.pageTitle,512))&&Array.isArray(v.reasons)&&v.reasons.length<=6&&v.reasons.every(r=>text(r,256,false))
    &&Array.isArray(v.claims)&&v.claims.length<=16&&v.claims.every(c=>fields(c,["provider","position","title","url","creator","similarity","metadata"])
      &&["lens","saucenao"].includes(String(c.provider))&&Number.isInteger(c.position)&&Number(c.position)>0&&Number(c.position)<=40
      &&text(c.title,512)&&isXMediaSearchLink(c.url)&&(c.creator===null||text(c.creator,256))
      &&(c.similarity===null||typeof c.similarity==="number"&&Number.isFinite(c.similarity)&&c.similarity>=0&&c.similarity<=100)
      &&Array.isArray(c.metadata)&&c.metadata.length<=8&&c.metadata.every(m=>fields(m,["label","value"])&&text(m.label,128)&&text(m.value,512)));
  return (fields(value,keys)||fields(value,[...keys,"sourceStatus"]))&&(!Object.hasOwn(value,"sourceStatus")||typeof value.sourceStatus==="string"&&["complete","partial","unavailable"].includes(value.sourceStatus))&&typeof value.reportId==="string"&&uuid.test(value.reportId)
    &&count(value.examined)&&count(value.unavailable)&&value.examined+value.unavailable<=24&&typeof value.limited==="boolean"
    &&Array.isArray(value.works)&&value.works.length<=48&&value.works.every(work=>(fields(work,workKeys)||fields(work,[...workKeys,"titleEvidence"])||fields(work,[...workKeys,"context"])||fields(work,[...workKeys,"context","titleEvidence"]))
      &&(work.title===null||text(work.title,512))&&isXMediaSearchLink(work.url)&&isXMediaSearchLink(work.rootUrl)&&text(work.origin,128,false)&&text(work.observedAt,64,false)&&Number.isFinite(Date.parse(work.observedAt))
      &&typeof work.ambiguous==="boolean"&&(!Object.hasOwn(work,"titleEvidence")||!work.ambiguous&&titleFact(work.titleEvidence,work.title))
      &&(!Object.hasOwn(work,"context")||context(work.context))
      &&Array.isArray(work.codes)&&work.codes.length<=16&&work.codes.every(fact)&&Array.isArray(work.sale)&&work.sale.length<=16&&work.sale.every(fact));
}
function imageUrl(value:unknown):boolean {
  if(!isXMediaSearchLink(value)||value.includes("%"))return false;
  const url=new URL(value);
  if(url.protocol!=="https:"||url.hostname!=="pbs.twimg.com"||url.port||url.hash)return false;
  const match=/^\/media\/([A-Za-z0-9_-]{1,256})(?:\.(jpg|jpeg|png|webp|gif))?$/.exec(url.pathname);if(!match)return false;
  let format:string|null=null,name=false;
  for(const [key,item]of url.searchParams){
    if(key==="format"&&format===null&&/^(?:jpg|jpeg|png|webp|gif)$/.test(item)){if(match[2]&&match[2]!==item)return false;format=item;}
    else if(key==="name"&&!name&&/^[A-Za-z0-9_-]{1,24}$/.test(item))name=true;
    else return false;
  }
  return !!match[2]||format!==null;
}
function preview(value:unknown,maxBytes=4*1024*1024):boolean {
  if(typeof value!=="string"||value.length>Math.ceil(maxBytes/3)*4+23||!value.startsWith("data:image/jpeg;base64,"))return false;
  const encoded=value.slice(23);
  if(!encoded.length||encoded.length%4!==0||!/^[A-Za-z0-9+/]+={0,2}$/.test(encoded))return false;
  const bytes=encoded.length/4*3-(encoded.endsWith("==")?2:encoded.endsWith("=")?1:0);
  if(bytes<3||bytes>maxBytes)return false;
  try{return atob(encoded.slice(0,8)).startsWith("\xff\xd8\xff");}catch{return false;}
}
export function isXSearchSnapshot(value:unknown):value is XSearchSnapshot {
  const keys=["id","source","archiveId","assetName","localContext","generation","accountId","feed","pane","kind","provider","phase","imageUrl","previewData","message","candidates","originalPageState","sourcePostUrl"];
  if(!fields(value,keys)&&!fields(value,[...keys,"providers"]))return false;
  if(typeof value.id!=="string"||!uuid.test(value.id)
    ||typeof value.phase!=="string"||!["consent","searching","verification","complete","failed"].includes(value.phase)
    ||typeof value.originalPageState!=="string"||!["available","expired","closed","unavailable"].includes(value.originalPageState)
    ||!(value.sourcePostUrl===null||isXCreatorPostLink(value.sourcePostUrl))
    ||!text(value.message,2048)||!Array.isArray(value.candidates)||value.candidates.length>10)return false;
  if(value.source==="surface"){
    if(!Number.isSafeInteger(value.generation)||Number(value.generation)<0||typeof value.accountId!=="string"||!/^(?:legacy|[a-f0-9]{32})$/.test(value.accountId)
      ||!text(value.feed,160,false)||/[\s\u0000-\u001f\u007f]/u.test(value.feed)||typeof value.pane!=="string"||!["feed","preview"].includes(value.pane)
      ||value.archiveId!==null||value.assetName!==null||value.localContext!==null)return false;
  }else if(value.source==="archive"){
    if(value.accountId!==null||value.generation!==null||value.feed!==null||value.pane!=="archive"
      ||!text(value.archiveId,128,false)||!/^[a-zA-Z0-9_-]+$/.test(value.archiveId)
      ||!text(value.assetName,256,false)||/[\\/\u0000-\u001f\u007f]/u.test(value.assetName)||value.assetName==="."||value.assetName===".."
      ||typeof value.localContext!=="string"||!uuid.test(value.localContext))return false;
  }else return false;
  if(value.kind==="image"){
    if(value.provider!=="saucenao")return false;
    if(value.source==="surface"){
      if(!(imageUrl(value.imageUrl)||value.phase==="failed"&&value.imageUrl===null)||!(value.previewData===null||Object.hasOwn(value,"providers")&&preview(value.previewData)))return false;
    }else if(value.imageUrl!==null||!(preview(value.previewData)||value.phase==="failed"&&value.previewData===null))return false;
  }else if(value.kind==="video"||value.kind==="thumbnail"){
    if(value.provider!=="lens"||value.imageUrl!==null||!(preview(value.previewData)||value.phase==="failed"&&value.previewData===null))return false;
  }else return false;
  if(!candidates(value.candidates))return false;
  if(!Object.hasOwn(value,"providers"))return true; // Previous native build during a development refresh.
  if(!Array.isArray(value.providers))return false;
  if(value.providers.length===0)return value.phase==="consent"||value.phase==="failed";
  const valid=value.providers.length===2&&value.providers.every((item,index)=>fields(item,["provider","phase","message","candidates","originalPageState"])
    &&item.provider===(index===0?"saucenao":"lens")&&typeof item.phase==="string"&&["searching","verification","complete","failed"].includes(item.phase)
    &&typeof item.originalPageState==="string"&&["available","expired","closed","unavailable"].includes(item.originalPageState)
    &&text(item.message,2048)&&candidates(item.candidates));
  if(!valid)return false;
  const providers=value.providers as XProviderResult[];
  const aggregate=providers.some(item=>item.phase==="searching")?"searching":providers.some(item=>item.phase==="verification")?"verification":providers.every(item=>item.phase==="failed")?"failed":"complete";
  return value.phase===aggregate;
}
function candidates(value:unknown):value is XMediaSearchCandidate[]{
  return Array.isArray(value)&&value.length<=10&&value.every(candidate=>fields(candidate,["title","creator","similarity","links","thumbnailData","sourceMetadata"])
    &&text(candidate.title,512)&& (candidate.creator===null||text(candidate.creator,256,false))
    &&(candidate.similarity===null||typeof candidate.similarity==="number"&&Number.isFinite(candidate.similarity)&&candidate.similarity>=0&&candidate.similarity<=100)
    &&(candidate.thumbnailData===null||preview(candidate.thumbnailData,256*1024))
    &&Array.isArray(candidate.sourceMetadata)&&candidate.sourceMetadata.length<=10&&candidate.sourceMetadata.every(item=>fields(item,["label","value"])&&text(item.label,128,false)&&text(item.value,1024,false))
    &&Array.isArray(candidate.links)&&candidate.links.length<=3&&candidate.links.every(link=>fields(link,["label","url"])&&text(link.label,256)&&isXMediaSearchLink(link.url)));
}
const checked=(value:unknown):XSearchSnapshot|null=>{if(value===null||isXSearchSnapshot(value))return value;throw new Error("검색 응답을 안전하게 확인하지 못했습니다.");};
const checkId=(id:string)=>{if(!uuid.test(id))throw new Error("검색 요청을 확인하지 못했습니다.");};
const providerArgs=(provider:XSearchProvider|undefined)=>{if(provider===undefined)return{};if(provider!=="saucenao"&&provider!=="lens")throw new Error("검색 서비스를 확인하지 못했습니다.");return{provider};};
export function createXMediaSearchApi(runtime:"tauri"|"browser-mock"):XMediaSearchApi {
  const unavailable=async()=>{throw new Error("검색은 데스크톱 앱에서 사용할 수 있습니다.");};
  if(runtime!=="tauri")return{status:async()=>null,subscribe:async()=>()=>{},confirm:unavailable,resume:unavailable,cancel:async()=>{},resetConsent:async()=>{},openResult:unavailable,openOriginal:unavailable,openLensSettings:unavailable,refreshLensResults:unavailable,startArchive:unavailable,creatorLinks:unavailable,openCreatorLink:unavailable};
  return{
    startArchive:async args=>{checkId(args.id);checkId(args.localContext);if(!/^[A-Za-z0-9_-]{1,128}$/.test(args.archiveId)||!/^\d{1,32}$/.test(args.postId)||!text(args.assetName,256,false)||/[\\/\u0000-\u001f\u007f]/u.test(args.assetName)||args.assetName==="."||args.assetName===".."||!["image","thumbnail"].includes(args.kind))throw new Error("저장된 미디어를 확인하지 못했습니다.");await invoke("x_media_search_start_archive",args);},
    status:async()=>checked(await invoke("x_media_search_status")),
    subscribe:handler=>listen<unknown>("x:media-search",event=>{if(event.payload===null||isXSearchSnapshot(event.payload))handler(event.payload);}),
    confirm:async(id,remember)=>{checkId(id);if(typeof remember!=="boolean")throw new Error("전송 동의를 확인해 주세요.");return checked(await invoke("x_media_search_confirm",{id,remember}));},
    resume:async(id,provider)=>{checkId(id);return checked(await invoke("x_media_search_resume",{id,...providerArgs(provider)}));},
    cancel:async id=>{checkId(id);await invoke("x_media_search_cancel",{id});},
    resetConsent:async()=>{await invoke("x_media_search_reset_consent");},
    openOriginal:async(id,provider)=>{checkId(id);await invoke("x_media_search_open_original",{id,...providerArgs(provider)});},
    openLensSettings:async id=>{checkId(id);await invoke("x_media_search_open_lens_settings",{id});},
    refreshLensResults:async id=>{checkId(id);const value=checked(await invoke("x_media_search_refresh_lens_results",{id}));if(!value||value.id!==id)throw new Error("Lens 검색 응답을 확인하지 못했습니다.");return value;},
    creatorLinks:async(id,candidateIndex,linkIndex)=>{checkId(id);if((candidateIndex===undefined)!==(linkIndex===undefined)||candidateIndex!==undefined&&(!Number.isInteger(candidateIndex)||candidateIndex<0||candidateIndex>=10||!Number.isInteger(linkIndex)||linkIndex!<0||linkIndex!>=3))throw new Error("게시자 프로필 근거를 확인하지 못했습니다.");const value=await invoke("x_media_search_creator_links",{id,candidateIndex:candidateIndex??null,linkIndex:linkIndex??null});if(!isXCreatorLinks(value))throw new Error("프로필 링크 응답을 확인하지 못했습니다.");return value;},
    openCreatorLink:async(id,linkIndex)=>{checkId(id);if(!Number.isInteger(linkIndex)||linkIndex<0||linkIndex>=8)throw new Error("프로필 링크를 확인하지 못했습니다.");await invoke("x_media_search_open_creator_link",{id,linkIndex});},
    openResult:async(id,candidateIndex,linkIndex,provider)=>{checkId(id);if(!Number.isInteger(candidateIndex)||candidateIndex<0||candidateIndex>=10||!Number.isInteger(linkIndex)||linkIndex<0||linkIndex>=3)throw new Error("검색 링크를 확인하지 못했습니다.");await invoke("x_media_search_open_result",{id,candidateIndex,linkIndex,...providerArgs(provider)});},
    pageEvidence:async(id,candidateIndex,linkIndex,provider)=>{checkId(id);if(!Number.isInteger(candidateIndex)||candidateIndex<0||candidateIndex>=10||!Number.isInteger(linkIndex)||linkIndex<0||linkIndex>=3)throw new Error("검색 링크를 확인하지 못했습니다.");const value=await invoke("x_media_search_page_evidence",{id,candidateIndex,linkIndex,...providerArgs(provider)});if(!isXPageEvidence(value))throw new Error("페이지 정보 응답을 확인하지 못했습니다.");return value;},
    workEvidence:async id=>{checkId(id);const value=await invoke("x_media_search_work_evidence",{id});if(!isXWorkEvidence(value))throw new Error("작품 정보 응답을 확인하지 못했습니다.");return value;},
    openWorkEvidence:async(id,reportId,index,origin)=>{checkId(id);checkId(reportId);if(!Number.isInteger(index)||index<0||index>=48||origin!==undefined&&typeof origin!=="boolean")throw new Error("작품 출처를 확인하지 못했습니다.");await invoke("x_media_search_open_work_evidence",{id,reportId,index,...(origin?{origin:true}:{})});},
  };
}
