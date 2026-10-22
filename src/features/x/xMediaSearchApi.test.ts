import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createXMediaSearchApi, isXMediaSearchLink, isXSearchSnapshot, type XSearchSnapshot } from "./xMediaSearchApi";
vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn()}));
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn()}));
const id="11111111-1111-4111-8111-111111111111";
const snapshot=(change:Partial<XSearchSnapshot>={}):XSearchSnapshot=>({id,generation:7,accountId:"legacy",feed:"foryou",pane:"feed",kind:"image",provider:"saucenao",phase:"consent",imageUrl:"https://pbs.twimg.com/media/Observed?format=jpg&name=small",previewData:null,message:"",candidates:[],...change});
const candidate={title:"Observed result",creator:"Named creator",similarity:85.25,links:[{label:"Source",url:"https://example.org/art?id=3"}]};
beforeEach(()=>vi.clearAllMocks());
it("accepts only closed bounded native image/frame snapshots and explicit creator/similarity fields",()=>{
  expect(isXSearchSnapshot(snapshot())).toBe(true);
  expect(isXSearchSnapshot(snapshot({phase:"complete",candidates:[candidate]}))).toBe(true);
  expect(isXSearchSnapshot(snapshot({kind:"video",provider:"lens",imageUrl:null,previewData:"data:image/jpeg;base64,/9j/2Q=="}))).toBe(true);
  expect(isXSearchSnapshot(snapshot({phase:"failed",imageUrl:null,message:"인증이 필요합니다."}))).toBe(true);
  expect(isXSearchSnapshot(snapshot({phase:"verification",message:"서비스 창에서 직접 확인해 주세요."}))).toBe(true);
});
it.each([
  {extra:"hidden"},{id:"../outside"},{generation:-1},{generation:Infinity},{generation:1.5},{accountId:"../profile"},{accountId:""},
  {feed:"a".repeat(161)},{feed:"home\nother"},{pane:["feed"]},{phase:["consent"]},{phase:"verified"},{phase:["verification"]},{provider:"lens"},{kind:"audio"},
  {phase:"verification",imageUrl:null},
  {imageUrl:null},{imageUrl:"https://pbs.twimg.com/profile_images/a.jpg"},{imageUrl:"https://pbs.twimg.com/media/a.jpg?token=secret"},
  {imageUrl:"https://pbs.twimg.com/media/a?format=jpg&format=jpg"},{imageUrl:"https://pbs.twimg.com/media/a.jpg?format=png"},
  {imageUrl:"https://pbs.twimg.com/media/a%2ejpg"},{previewData:"data:image/jpeg;base64,/9j/2Q=="},{message:"a".repeat(2049)},
  {candidates:Array.from({length:11},()=>candidate)},{candidates:[{...candidate,extra:"private"}]},{candidates:[{...candidate,title:"a".repeat(513)}]},
  {candidates:[{...candidate,creator:"a".repeat(257)}]},{candidates:[{...candidate,similarity:NaN}]},{candidates:[{...candidate,similarity:100.1}]},
  {candidates:[{...candidate,links:Array.from({length:4},()=>candidate.links[0])}]},{candidates:[{...candidate,links:[{label:"bad",url:"javascript:alert(1)"}]}]},
])("rejects malformed or extra snapshot fields: %j",change=>{expect(isXSearchSnapshot({...snapshot(),...change})).toBe(false);});
it.each(["data:image/png;base64,iVBORw==","data:image/jpeg;base64,AAAA","data:image/jpeg;base64,/9j/2Q=","data:image/jpeg;base64,/9j/"+"AAAA".repeat(1398102),"https://example.org/frame.jpg"])("never renders an unsafe/oversized frame preview: %s",previewData=>{
  expect(isXSearchSnapshot(snapshot({kind:"video",provider:"lens",imageUrl:null,previewData}))).toBe(false);
});
it.each(["file:///C:/secret","javascript:alert(1)","https://user:pass@example.org/x","https://example.org/a b","https://example.org/\\x","https://example.org/\u0000","data:text/html,unsafe"])("rejects unsafe result links: %s",url=>{expect(isXMediaSearchLink(url)).toBe(false);});
it("passes only IDs, consent and validated indices to native commands",async()=>{
  vi.mocked(invoke).mockResolvedValue(snapshot());const api=createXMediaSearchApi("tauri");
  await expect(api.status()).resolves.toEqual(snapshot());await expect(api.confirm(id,false)).resolves.toEqual(snapshot());
  await api.cancel(id);await api.resetConsent();await api.openResult(id,1,2);await expect(api.resume(id)).resolves.toEqual(snapshot());
  expect(invoke).toHaveBeenNthCalledWith(1,"x_media_search_status");expect(invoke).toHaveBeenNthCalledWith(2,"x_media_search_confirm",{id,remember:false});
  expect(invoke).toHaveBeenNthCalledWith(3,"x_media_search_cancel",{id});expect(invoke).toHaveBeenNthCalledWith(4,"x_media_search_reset_consent");
  expect(invoke).toHaveBeenNthCalledWith(5,"x_media_search_open_result",{id,candidateIndex:1,linkIndex:2});
  expect(invoke).toHaveBeenNthCalledWith(6,"x_media_search_resume",{id});
  for(const operation of [()=>api.confirm("wrong",true),()=>api.resume("wrong"),()=>api.cancel("wrong"),()=>api.openResult(id,10,0),()=>api.openResult(id,0,3),()=>api.openResult(id,NaN,0)])await expect(operation()).rejects.toThrow();
  expect(invoke).toHaveBeenCalledTimes(6);
});
it("validates status/confirm results and filters malformed events while allowing native null dismissal",async()=>{
  const stop=vi.fn();vi.mocked(listen).mockResolvedValue(stop);const api=createXMediaSearchApi("tauri"),handler=vi.fn();
  expect(await api.subscribe(handler)).toBe(stop);expect(listen).toHaveBeenCalledWith("x:media-search",expect.any(Function));
  const receive=vi.mocked(listen).mock.calls[0]![1] as (event:{payload:unknown})=>void;
  receive({payload:snapshot()});receive({payload:snapshot({phase:"verification"})});receive({payload:{...snapshot(),extra:1}});receive({payload:null});
  expect(handler.mock.calls).toEqual([[snapshot()],[snapshot({phase:"verification"})],[null]]);
  vi.mocked(invoke).mockResolvedValue({...snapshot(),imageUrl:"https://evil.test/image.jpg"});
  await expect(api.status()).rejects.toThrow("안전하게");await expect(api.confirm(id,true)).rejects.toThrow("안전하게");await expect(api.resume(id)).rejects.toThrow("안전하게");
  vi.mocked(invoke).mockResolvedValue(null);await expect(api.resume(id)).resolves.toBeNull();
});
it("does not issue any native or provider operation in browser-mock mode",async()=>{
  const api=createXMediaSearchApi("browser-mock");expect(await api.status()).toBeNull();(await api.subscribe(vi.fn()))();
  await api.cancel(id);await api.resetConsent();await expect(api.confirm(id,false)).rejects.toThrow("데스크톱");await expect(api.resume(id)).rejects.toThrow("데스크톱");await expect(api.openResult(id,0,0)).rejects.toThrow("데스크톱");
  expect(invoke).not.toHaveBeenCalled();expect(listen).not.toHaveBeenCalled();
});
