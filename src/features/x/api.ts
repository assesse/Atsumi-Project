import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { NativeViewport } from "../platforms/nativePlayerGeometry";
export type XFeed = "foryou" | "following" | "communities" | "likes" | "bookmarks" | `community:${string}` | `post:${string}` | `profile:${string}`;
export type XStats = {replies:string;reposts:string;likes:string;views:string;bookmarks:string;liked:boolean;reposted:boolean;bookmarked:boolean;canReply:boolean;canLike:boolean;canRepost:boolean;canBookmark:boolean};
export type XQuotePreview = {handle:string;name:string;text:string;time:string;media:string[];avatar?:string|null;video:boolean};
export type XPost = { id:string; handle:string; name:string; text:string; time:string; media:string[]; avatar?:string|null; video:boolean; gif?:boolean; relation?:"reply"|"related"|"unknown"|null; truncated:boolean; pinned?:boolean; repost?:string|null; quote?:XPost|null;quotePreview?:XQuotePreview|null; stats?:XStats };
export type XCommunity = {id:string;name:string;path:string};
export type XPage = { posts:XPost[]; cursor:string|null; generation:number; status:"signedIn"|"signedOut"|"unknown"; limited:boolean; communities?:XCommunity[] };
export type XAccount = {id:string;handle:string};
export type XAccountPresentation = {handle:string;name:string;avatarData:string|null};
export type XSnapshot = { status:XPage["status"]; generation:number; open:boolean; accounts?:XAccount[];active?:string;error?:string|null;presentations?:Record<string,XAccountPresentation> };
export type XApi = {
  load(feed:XFeed, cursor:string|null, requestId:string):Promise<XPage>;
  cancel(requestId:string):Promise<void>;
  suspend():Promise<void>;
  account(add?:boolean):Promise<XSnapshot>;
  select(accountId:string):Promise<XSnapshot>;
  status():Promise<XSnapshot>;
  image(postId:string,index:number,generation:number):Promise<string>;
  open(postId:string):Promise<void>;
  video?(postId:string,generation:number,requestId?:string):Promise<{url:string;hls:boolean}>;
  action?(postId:string,action:"like"|"bookmark",generation:number,expected:boolean):Promise<XPost>;
};
export const xError = (error:unknown) => error instanceof Error ? error.message : typeof error === "string" ? error : "X 요청을 완료하지 못했습니다.";
export function createXApi(runtime:"tauri"|"browser-mock"):XApi {
  const unavailable=async():Promise<never>=>{throw new Error("X 피드는 데스크톱 앱에서 사용할 수 있습니다.");};
  if(runtime!=="tauri")return{load:unavailable,cancel:async()=>{},suspend:async()=>{},account:unavailable,select:unavailable,status:async()=>({status:"unknown",generation:0,open:false}),image:unavailable,open:unavailable};
  return{
    load:(feed,cursor,requestId)=>invoke("x_feed_load",{feed,cursor,requestId}), cancel:requestId=>invoke("x_feed_cancel",{requestId}),suspend:()=>invoke("x_feed_suspend"),
    account:(add=false)=>invoke("x_account_open",{add}),select:accountId=>invoke("x_account_select",{accountId}),status:()=>invoke("x_account_status"),image:(postId,index,generation)=>invoke("x_image_get",{postId,index,generation}),open:postId=>invoke("x_post_open",{postId}),
    video:(postId,generation,requestId)=>invoke("x_video_open",{postId,generation,requestId}),
    action:(postId,action,generation,expected)=>invoke("x_post_action",{postId,action,generation,expected,text:""}),
  };
}
export type XPreviewTab = {id:string;path:string;title:string;state:string};
export type XSurfaceSnapshot = {
  generation:number; feed:string; state:string; preview:boolean; previewState:string; error:string|null;
  previews:XPreviewTab[];activePreviewId:string|null;
};
/** Intent only: content is read independently from the same visible native pane. */
export type XArchiveChooseRequest = {id:string;generation:number;accountId:string;feed:string;pane:"feed"|"preview";path:string;sourceUrl:string};
export function isXArchiveChooseRequest(value:unknown):value is XArchiveChooseRequest {
  if(!value||typeof value!=="object")return false;
  const item=value as Partial<XArchiveChooseRequest>;
  if(typeof item.id!=="string"||!/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(item.id)
    ||!Number.isSafeInteger(item.generation)||item.generation!<0
    ||typeof item.accountId!=="string"||!item.accountId||item.accountId.length>100
    ||typeof item.feed!=="string"||!item.feed||item.feed.length>100
    ||!["feed","preview"].includes(item.pane??"")
    ||typeof item.path!=="string"||!/^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/[1-9]\d{4,24}$/.test(item.path)
    ||typeof item.sourceUrl!=="string"||item.sourceUrl.length>2048)return false;
  try{
    const source=new URL(item.sourceUrl);
    return source.protocol==="https:"&&["x.com","www.x.com"].includes(source.hostname)&&!source.port&&!source.username&&!source.password
      &&!source.search&&!source.hash&&!source.pathname.startsWith("/__atsumi_surface__/")&&source.href===item.sourceUrl;
  }catch{return false;}
}
export type XSurfaceApi = Pick<XApi,"account"|"select"|"status"> & {
  suspend(retainDocuments?:boolean):Promise<void>;
  surface(feed:XFeed,order:number,refresh:boolean):Promise<XSurfaceSnapshot>;
  surfaceStatus():Promise<XSurfaceSnapshot>;
  viewport(generation:number,viewport:NativeViewport):Promise<void>;
  selectPreview(generation:number,previewId:string):Promise<XSurfaceSnapshot>;
  closePreview(generation:number,previewId?:string):Promise<XSurfaceSnapshot>;
  dismissMenus(generation:number):Promise<void>;
  subscribe?(changed:()=>void):Promise<()=>void>;
  subscribeArchiveChoose?(chosen:(request:XArchiveChooseRequest)=>void):Promise<()=>void>;
};
export function createXSurfaceApi(runtime:"tauri"|"browser-mock"):XSurfaceApi {
  const account=createXApi(runtime);
  const unavailable=async():Promise<never>=>{throw new Error("X 피드는 데스크톱 앱에서 사용할 수 있습니다.");};
  return {account:account.account,select:account.select,status:account.status,
    suspend:retainDocuments=>retainDocuments===true
      ?runtime==='tauri'?invoke("x_surface_suspend"):Promise.resolve()
      :account.suspend(),
    surface:runtime==='tauri'?(feed,order,refresh)=>invoke("x_surface_open",{feed,order,refresh}):unavailable,
    surfaceStatus:runtime==='tauri'?()=>invoke("x_surface_status"):unavailable,
    viewport:runtime==='tauri'?(generation,viewport)=>invoke("x_surface_viewport",{generation,viewport}):async()=>{},
    selectPreview:runtime==='tauri'?(generation,previewId)=>invoke("x_surface_select_preview",{generation,previewId}):unavailable,
    closePreview:runtime==='tauri'?(generation,previewId)=>invoke("x_surface_close_preview",{generation,...(previewId===undefined?{}:{previewId})}):unavailable,
    dismissMenus:runtime==='tauri'?generation=>invoke("x_surface_dismiss_menus",{generation}):async()=>{},
    subscribe:runtime==='tauri'?changed=>listen("x:surface-changed",()=>changed()):async()=>()=>{},
    subscribeArchiveChoose:runtime==='tauri'?chosen=>listen<unknown>("x:archive-choose",event=>{if(isXArchiveChooseRequest(event.payload))chosen(event.payload);}):async()=>()=>{},
  };
}
