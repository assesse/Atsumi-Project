import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type XMediaSearchCandidate = {title:string;creator:string|null;similarity:number|null;links:{label:string;url:string}[]};
export type XSearchSnapshot = {
  id:string;generation:number;accountId:string;feed:string;pane:"feed"|"preview";
  kind:"image"|"video";provider:"saucenao"|"lens";phase:"consent"|"searching"|"verification"|"complete"|"failed";
  imageUrl:string|null;previewData:string|null;message:string;candidates:XMediaSearchCandidate[];
};
export type XMediaSearchApi = {
  status():Promise<XSearchSnapshot|null>;
  subscribe(handler:(value:XSearchSnapshot|null)=>void):Promise<()=>void>;
  confirm(id:string,remember:boolean):Promise<XSearchSnapshot|null>;
  resume(id:string):Promise<XSearchSnapshot|null>;
  cancel(id:string):Promise<void>;
  resetConsent():Promise<void>;
  openResult(id:string,candidateIndex:number,linkIndex:number):Promise<void>;
};
const uuid=/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/;
const fields=(value:unknown,keys:string[]):value is Record<string,unknown>=>!!value&&typeof value==="object"&&!Array.isArray(value)
  &&Object.keys(value).length===keys.length&&keys.every(key=>Object.hasOwn(value,key));
const text=(value:unknown,max:number,empty=true):value is string=>typeof value==="string"&&value.length<=max&&(empty||value.length>0)&&!/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value);
export function isXMediaSearchLink(value:unknown):value is string {
  if(typeof value!=="string"||!value||value.length>2048||/[\s\u0000-\u001f\u007f\\]/u.test(value))return false;
  try{const url=new URL(value);return ["http:","https:"].includes(url.protocol)&&!!url.hostname&&!url.username&&!url.password&&url.href===value;}catch{return false;}
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
function preview(value:unknown):boolean {
  if(typeof value!=="string"||value.length>Math.ceil(4*1024*1024/3)*4+23||!value.startsWith("data:image/jpeg;base64,"))return false;
  const encoded=value.slice(23);
  if(!encoded.length||encoded.length%4!==0||!/^[A-Za-z0-9+/]+={0,2}$/.test(encoded))return false;
  const bytes=encoded.length/4*3-(encoded.endsWith("==")?2:encoded.endsWith("=")?1:0);
  if(bytes<3||bytes>4*1024*1024)return false;
  try{return atob(encoded.slice(0,8)).startsWith("\xff\xd8\xff");}catch{return false;}
}
export function isXSearchSnapshot(value:unknown):value is XSearchSnapshot {
  if(!fields(value,["id","generation","accountId","feed","pane","kind","provider","phase","imageUrl","previewData","message","candidates"]))return false;
  if(typeof value.id!=="string"||!uuid.test(value.id)||!Number.isSafeInteger(value.generation)||Number(value.generation)<0
    ||typeof value.accountId!=="string"||!/^(?:legacy|[a-f0-9]{32})$/.test(value.accountId)
    ||!text(value.feed,160,false)||/[\s\u0000-\u001f\u007f]/u.test(value.feed)
    ||typeof value.pane!=="string"||!["feed","preview"].includes(value.pane)||typeof value.phase!=="string"||!["consent","searching","verification","complete","failed"].includes(value.phase)
    ||!text(value.message,2048)||!Array.isArray(value.candidates)||value.candidates.length>10)return false;
  if(value.kind==="image"){
    if(value.provider!=="saucenao"||!(imageUrl(value.imageUrl)||value.phase==="failed"&&value.imageUrl===null)||value.previewData!==null)return false;
  }else if(value.kind==="video"){
    if(value.provider!=="lens"||value.imageUrl!==null||!(preview(value.previewData)||value.phase==="failed"&&value.previewData===null))return false;
  }else return false;
  return value.candidates.every(candidate=>fields(candidate,["title","creator","similarity","links"])
    &&text(candidate.title,512)&& (candidate.creator===null||text(candidate.creator,256,false))
    &&(candidate.similarity===null||typeof candidate.similarity==="number"&&Number.isFinite(candidate.similarity)&&candidate.similarity>=0&&candidate.similarity<=100)
    &&Array.isArray(candidate.links)&&candidate.links.length<=3&&candidate.links.every(link=>fields(link,["label","url"])&&text(link.label,256)&&isXMediaSearchLink(link.url)));
}
const checked=(value:unknown):XSearchSnapshot|null=>{if(value===null||isXSearchSnapshot(value))return value;throw new Error("검색 응답을 안전하게 확인하지 못했습니다.");};
const checkId=(id:string)=>{if(!uuid.test(id))throw new Error("검색 요청을 확인하지 못했습니다.");};
export function createXMediaSearchApi(runtime:"tauri"|"browser-mock"):XMediaSearchApi {
  if(runtime!=="tauri")return{status:async()=>null,subscribe:async()=>()=>{},confirm:async()=>{throw new Error("검색은 데스크톱 앱에서 사용할 수 있습니다.");},resume:async()=>{throw new Error("검색은 데스크톱 앱에서 사용할 수 있습니다.");},cancel:async()=>{},resetConsent:async()=>{},openResult:async()=>{throw new Error("검색은 데스크톱 앱에서 사용할 수 있습니다.");}};
  return{
    status:async()=>checked(await invoke("x_media_search_status")),
    subscribe:handler=>listen<unknown>("x:media-search",event=>{if(event.payload===null||isXSearchSnapshot(event.payload))handler(event.payload);}),
    confirm:async(id,remember)=>{checkId(id);if(typeof remember!=="boolean")throw new Error("전송 동의를 확인해 주세요.");return checked(await invoke("x_media_search_confirm",{id,remember}));},
    resume:async id=>{checkId(id);return checked(await invoke("x_media_search_resume",{id}));},
    cancel:async id=>{checkId(id);await invoke("x_media_search_cancel",{id});},
    resetConsent:async()=>{await invoke("x_media_search_reset_consent");},
    openResult:async(id,candidateIndex,linkIndex)=>{checkId(id);if(!Number.isInteger(candidateIndex)||candidateIndex<0||candidateIndex>=10||!Number.isInteger(linkIndex)||linkIndex<0||linkIndex>=3)throw new Error("검색 링크를 확인하지 못했습니다.");await invoke("x_media_search_open_result",{id,candidateIndex,linkIndex});},
  };
}
