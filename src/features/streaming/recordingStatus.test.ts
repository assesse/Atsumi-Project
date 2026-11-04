import { describe,it,expect } from "vitest";
import { recordingStatus,recordingWorkLabel } from "./recordingStatus";
import type {BrowserRecording} from "../../api/officialBrowser";
const record=(patch:Partial<BrowserRecording>={}):BrowserRecording=>({id:"a",channelId:"b",title:"synthetic",startedAt:1,updatedAt:1,status:"stopped",mimeType:"video/mp4",outputDir:"synthetic",segmentCount:1,bytesWritten:100,durationSeconds:10,lastError:null,segments:[],...patch});
describe("truthful recording work status",()=>{
 it("shows observed work ahead of the historical capture error",()=>{
   const r=record({status:"interrupted",workProgress:{phase:"verifying",completed:3,total:10,unit:"seconds",updatedAt:1}});
   expect(recordingStatus(r)).toBe("재생 검증 30%");
 });
 it("does not invent percent when total is absent",()=>{
   expect(recordingWorkLabel(record({workProgress:{phase:"finalizing",completed:3,total:null,unit:"files",updatedAt:1}}))).toBe("검증 결과 저장 중");
   expect(recordingWorkLabel(record({workProgress:{phase:"final_verification",completed:12345,total:null,unit:"bytes",updatedAt:1}}))).toBe("최종 파일 확인 중");
 });
 it("keeps queued and failed distinct from running",()=>{
   expect(recordingStatus(record({merge:{status:"queued",segmentCount:1,updatedAt:1}}))).toBe("병합 대기");
   expect(recordingStatus(record({status:"interrupted",merge:{status:"failed",segmentCount:1,updatedAt:1}}))).toBe("처리 실패");
 });
 it("does not erase interruption history when a durable partial broadcast is playable",()=>{
   const token="a".repeat(32);
   const r=record({status:"interrupted",merge:{status:"complete",segmentCount:1,updatedAt:1,file:"merged-"+token+".mp4",timelineFile:"merged-"+token+".timeline.jsonl",bytes:99,durationSeconds:10}});
   expect(recordingStatus(r)).toBe("저장된 구간 재생 가능");
   expect(r.status).toBe("interrupted");
   const archived = {status:"blocked" as const,sourceDir:"source",destinationRoot:"target",token:"token",retryAt:0,lastError:"copy failed"};
   expect(recordingStatus({...r,status:"stopped",archive:archived})).toBe("이동 재시도 대기");
   expect(recordingStatus({...r,status:"stopped",archive:{...archived,status:"pending",lastError:null}})).toBe("파일 이동 대기");
 });
});