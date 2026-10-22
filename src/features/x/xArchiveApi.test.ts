import {beforeEach,describe,expect,it,vi} from "vitest";
import {invoke} from "@tauri-apps/api/core";
import {createXArchiveApi,isXArchiveLocalUrl,type XArchiveAsset} from "./xArchiveApi";

vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn(),convertFileSrc:(path:string,protocol:string)=>`http://${protocol}.localhost/${encodeURIComponent(path)}`}));
const ID="11111111-1111-4111-8111-111111111111";
const NAME=`asset-${ID}.png`;
const SOURCE="https://pbs.twimg.com/media/observed?format=png&name=small";
const ORIGINAL="https://pbs.twimg.com/media/observed?format=png&name=orig";
const asset=(downloadUrl?:string):XArchiveAsset=>({name:NAME,mime:"image/png",bytes:10,sha256:"a".repeat(64),postId:"123",kind:"image",sourceUrl:SOURCE,...(downloadUrl?{downloadUrl}:{})});
beforeEach(()=>vi.clearAllMocks());

describe("original image provenance does not become a reader network URL",()=>{
  it("passes legacy native manifests through without inventing quality metadata",async()=>{
    const legacy={id:ID,assets:[asset()]};vi.mocked(invoke).mockResolvedValue(legacy);
    const read=await createXArchiveApi("tauri").read(ID);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("x_archive_read",{archiveId:ID});
    expect(read).toBe(legacy);expect(read.assets[0]).not.toHaveProperty("downloadUrl");
  });
  it("retains separate observed and request URLs from a validated native manifest",async()=>{
    const saved={id:ID,assets:[asset(ORIGINAL)]};vi.mocked(invoke).mockResolvedValue(saved);
    const read=await createXArchiveApi("tauri").read(ID);
    expect(read.assets[0]?.sourceUrl).toBe(SOURCE);expect(read.assets[0]?.downloadUrl).toBe(ORIGINAL);
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("still builds reader URLs exclusively from immutable archive and asset names",()=>{
    const api=createXArchiveApi("tauri"),saved=asset(ORIGINAL);
    expect(api.mediaUrl(ID,saved.name)).toBe(`http://x-archive.localhost/${ID}/${NAME}`);
    expect(api.mediaUrl(ID,saved.sourceUrl)).toBe("");expect(api.mediaUrl(ID,saved.downloadUrl!)).toBe("");
    expect(invoke).not.toHaveBeenCalled();
  });
  it("rejects observed/original remote URLs as local assets even if provenance contains them",()=>{
    const names=new Set([NAME]);
    expect(isXArchiveLocalUrl(`http://x-archive.localhost/${ID}/${NAME}`,ID,names)).toBe(true);
    for(const value of[SOURCE,ORIGINAL,`http://x-archive.localhost/${ID}/${NAME}?downloadUrl=${ORIGINAL}`]){
      expect(isXArchiveLocalUrl(value,ID,names)).toBe(false);
    }
  });
});
