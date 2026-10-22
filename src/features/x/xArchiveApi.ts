import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type XArchivePane = "feed" | "preview";
export type XArchiveIntent = "archive" | "media";
export type XCaptureMedia = { kind:string; url:string; alt:string; poster?:string|null };
export type XCaptureCounts = {replies:string;reposts:string;likes:string;bookmarks:string;views:string};
export type XCaptureQuotePresentation = {handle:string;displayName:string;verified:boolean|null;body:string;timestamp:string;timestampLabel?:string;avatarUrl?:string;mediaUrls?:string[]};
export type XCaptureQuote = XCaptureQuotePresentation & {id:string;url:string};
export type XCapturePresentation = {handle:string;displayName:string;verified:boolean|null;counts:XCaptureCounts;quote?:XCaptureQuote|null;quotePresentation?:XCaptureQuotePresentation|null;mediaBeforeQuote?:boolean;relation:"root"|"reply"|"unknown";conversationId?:string;order:number;avatarUrl?:string;mediaUrls?:string[];timestampLabel?:string};
export type XCapturePost = { id:string; url:string; author:string; body:string; timestamp:string; counts:string; context:string; quotes:string[]; media:XCaptureMedia[]; truncated:boolean; photoOnly?:boolean;presentation?:XCapturePresentation|null };
export type XCaptureDocument = { version:number; owner:string; sourceUrl:string; capturedAt:string; coverage:"loaded_dom_only"; records:XCapturePost[]; limitReached:boolean };
export type XArchiveDraft = { captureId:string; accountId:string; document:XCaptureDocument };
export type XArchiveSaveRequest = { requestId:string; captureId:string; selectedIds:string[]; intent:XArchiveIntent; includeMedia:boolean };
export type XArchiveAsset = { name:string; mime:string; bytes:number; sha256:string; postId:string; kind:string; sourceUrl:string; downloadUrl?:string };
export type XArchiveIssue = { postId:string; kind:string; reason:string };
export type XArchiveRecord = { version:number; id:string; captureId:string; accountId:string; savedAt:number; intent:XArchiveIntent; document:XCaptureDocument; assets:XArchiveAsset[]; issues:XArchiveIssue[]; selectedContentComplete:boolean; mediaRequested:boolean; htmlSha256:string };
export type XArchiveSummary = { id:string; accountId?:string; sourceUrl:string; savedAt:number; capturedAt:string; posts:number; assets:number; issues:number; intent:XArchiveIntent; selectedContentComplete:boolean };
export type XArchiveProgress = {requestId:string;captureId:string;accountId:string;sequence:number;stage:"queued"|"preparing"|"downloading"|"writing"|"complete"|"failed"|"cancelled";processedMedia:number;totalMedia:number;downloadedBytes:number;totalBytes:number|null;partial:boolean;currentTransfer:{receivedBytes:number;totalBytes:number|null}|null};
export function isXArchiveProgress(value:unknown):value is XArchiveProgress {
  if(!value||typeof value!=="object")return false;
  const item=value as Record<string,unknown>;
  const keys=["requestId","captureId","accountId","sequence","stage","processedMedia","totalMedia","downloadedBytes","totalBytes","partial","currentTransfer"];
  const transfer=item.currentTransfer as Record<string,unknown>|null;
  return !Array.isArray(value)&&Object.keys(item).length===keys.length&&keys.every(key=>Object.hasOwn(item,key))
    &&typeof item.requestId==="string"&&validXArchiveId(item.requestId)&&typeof item.captureId==="string"&&validXArchiveId(item.captureId)
    &&typeof item.accountId==="string"&&item.accountId.length>0&&item.accountId.length<=160
    &&typeof item.stage==="string"&&["queued","preparing","downloading","writing","complete","failed","cancelled"].includes(item.stage)
    &&[item.sequence,item.processedMedia,item.totalMedia,item.downloadedBytes].every(number=>typeof number==="number"&&Number.isSafeInteger(number)&&number>=0)
    &&(item.processedMedia as number)<=(item.totalMedia as number)
    &&(item.totalMedia as number)<=4096
    &&(item.totalBytes===null||typeof item.totalBytes==="number"&&Number.isSafeInteger(item.totalBytes)&&item.totalBytes>=Number(item.downloadedBytes))
    &&(transfer===null||typeof transfer==="object"&&!Array.isArray(transfer)&&Object.keys(transfer).length===2
      &&Object.hasOwn(transfer,"receivedBytes")&&Object.hasOwn(transfer,"totalBytes")
      &&typeof transfer.receivedBytes==="number"&&Number.isSafeInteger(transfer.receivedBytes)&&transfer.receivedBytes>=0&&transfer.receivedBytes<=Number(item.downloadedBytes)
      &&(transfer.totalBytes===null||typeof transfer.totalBytes==="number"&&Number.isSafeInteger(transfer.totalBytes)&&transfer.totalBytes>=transfer.receivedBytes))
    &&typeof item.partial==="boolean";
}
export type XArchiveApi = {
  inspect(pane:XArchivePane,generation:number):Promise<XArchiveDraft>;
  release(captureId:string,accountId:string):Promise<boolean>;
  save(request:XArchiveSaveRequest):Promise<XArchiveRecord>;
  cancel(requestId:string):Promise<boolean>;
  list():Promise<XArchiveSummary[]>;
  read(archiveId:string):Promise<XArchiveRecord>;
  openFolder(archiveId:string):Promise<void>;
  openSource(archiveId:string,postId:string):Promise<void>;
  mediaUrl(archiveId:string,assetName:string):string;
  subscribeProgress?(handler:(value:XArchiveProgress)=>void):Promise<()=>void>;
};

const uuid = "[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}";
const archiveIdPattern = new RegExp(`^${uuid}$`);
const assetNamePattern = new RegExp(`^asset-${uuid}\\.(?:jpg|png|webp|gif|mp4|m4s|ts|aac|m3u8|vtt)$`);
export const validXArchiveId = (value:string) => archiveIdPattern.test(value);
export const validXArchiveAsset = (value:string) => assetNamePattern.test(value);
/** Reader URLs can only name this immutable archive's committed asset manifest. */
export function isXArchiveLocalUrl(value:string,archiveId:string,assets:ReadonlySet<string>):boolean {
  if(!validXArchiveId(archiveId)||!value||value.includes("%")||value.includes("\\"))return false;
  const match=value.match(/^(?:https?:\/\/x-archive\.localhost|x-archive:\/\/localhost)\/([^/?#]+)\/([^/?#]+)$/);
  return !!match&&match[1]===archiveId&&validXArchiveAsset(match[2]!)&&assets.has(match[2]!);
}
export function createXArchiveApi(runtime:"tauri"|"browser-mock"):XArchiveApi {
  const unavailable=async():Promise<never>=>{throw new Error("X 보관은 데스크톱 앱에서 사용할 수 있습니다.");};
  return {
    inspect:runtime==="tauri"?(pane,generation)=>invoke("x_archive_inspect",{pane,generation}):unavailable,
    release:runtime==="tauri"?(captureId,accountId)=>invoke("x_archive_release",{captureId,accountId}):unavailable,
    save:runtime==="tauri"?request=>invoke("x_archive_save",{request}):unavailable,
    cancel:runtime==="tauri"?requestId=>invoke("x_archive_cancel",{requestId}):unavailable,
    list:runtime==="tauri"?()=>invoke("x_archive_list"):unavailable,
    read:runtime==="tauri"?archiveId=>invoke("x_archive_read",{archiveId}):unavailable,
    openFolder:runtime==="tauri"?archiveId=>invoke("x_archive_open_folder",{archiveId}):unavailable,
    openSource:runtime==="tauri"?(archiveId,postId)=>invoke("x_archive_open_source",{archiveId,postId}):unavailable,
    subscribeProgress:runtime==="tauri"?handler=>listen<unknown>("x:archive-progress",event=>{if(isXArchiveProgress(event.payload))handler(event.payload);}):undefined,
    mediaUrl:(archiveId,assetName)=>{
      if(!validXArchiveId(archiveId)||!validXArchiveAsset(assetName))return "";
      // Tauri encodes the whole input; keep our two validated protocol segments separate.
      return `${convertFileSrc(archiveId,"x-archive")}/${assetName}`;
    },
  };
}
