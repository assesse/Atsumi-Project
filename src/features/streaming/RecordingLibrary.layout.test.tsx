import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { RecordingLibrary } from "./RecordingLibrary";
import { groupRecordings } from "./recordingGroups";
import type { BrowserRecording } from "../../api/officialBrowser";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../../test/offlineLayoutBrowser";
import baseCss from "../../styles.css?raw";
import css from "./RecordingLibrary.css?raw";
import channelCss from "./RecordedChannel.css?raw";
vi.mock("../../api/recordingProfile",()=>({recordingProfile:async()=>({name:"녹화 채널 이름이 길어지는 경우",image:null}),openRecordingChannel:vi.fn()}));
type Box={x:number;y:number;width:number;height:number;position:string;fontSize:number};
const item=(id:string,title:string,patch:Partial<BrowserRecording>={}):BrowserRecording=>({id,title,channelId:id,startedAt:1800000000000,updatedAt:1800000001000,status:"stopped",mimeType:"video/webm",outputDir:"fixture",segmentCount:2,bytesWritten:27200000000,durationSeconds:3600,lastError:null,segments:[],...patch});
const records=[item("one","짧은 제목"),item("long","아주 긴 방송 제목이 여러 줄로 표시되더라도 주변 카드와 정렬을 유지하는 녹화 · 기념 방송",{status:"failed",lastError:"fixture"}),...['part1','part2','part3'].map(id=>item(id,"여러 파일이 있는 방송",{channelId:'shared',broadcastKey:'id:123'})),item('last','마지막 방송')];
async function frames(){
  const result:LayoutScene[]=[];
  for(const index of [-1,0,2]){
    const groups=groupRecordings(records);
    localStorage.setItem('atsumi.recording-library.expanded.v1',JSON.stringify(index<0?{}:{[groups[index]!.key]:true}));
    const container=document.createElement('div');document.body.append(container);const root=createRoot(container);
    await act(async()=>root.render(<RecordingLibrary recordings={records} selectedId={null} privacy={false} disabled={false} retrying={false} openingFolder={false} onSelect={()=>{}} onFolder={()=>{}} onReplay={()=>{}} onRetryMerge={()=>{}} onDelete={async()=>({deletedIds:[],failures:[]})}/>));
    for(const [i,node] of [...container.querySelectorAll('.recording-broadcast-group')].entries()){
      node.setAttribute('data-layout-probe','group-'+i);
      for(const selector of ['.recording-broadcast-heading','.recording-broadcast-files','.recording-broadcast-meta','.recording-broadcast-status'])node.querySelector(selector)?.setAttribute('data-layout-probe',selector.slice(1)+'-'+i);
    }
    result.push({name:index<0?'collapsed':index===0?'single-overlay':'multi-overlay',html:'<main class="streaming-workspace" style="padding:20px;container-type:inline-size;height:100vh;overflow:auto">'+container.innerHTML+'</main>'});
    await act(async()=>root.unmount());container.remove();
  }
  localStorage.removeItem('atsumi.recording-library.expanded.v1');return result;
}
describe.skipIf(!layoutBrowser)('recording broadcast layout in an isolated browser',()=>{
  it.each([[1280,720],[1920,1080],[960,640]])('keeps collapsed boxes aligned and overlay expansion out of grid flow at %ix%i',async(width,height)=>{
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
    try{
      const scenes=await frames(),results=await auditLayouts(scenes,baseCss+'\n'+channelCss+'\n'+css,width,height,'recording-collections');
      expect(results.flatMap(r=>r.issues.map(i=>r.name+': '+i))).toEqual([]);
      const probes=(n:number)=>(results[n]!.geometry as {probes:Record<string,Box>}).probes;
      const baseline=probes(0),groups=Object.entries(baseline).filter(([key])=>key.startsWith('group-'));
      expect(groups.length).toBe(4);
      for(const [,box] of groups)expect(box.height).toBeCloseTo(groups[0]![1].height,1);
      for(const n of [1,2])for(const [key,box] of groups)for(const axis of ["x","y","width","height"] as const)expect(probes(n)[key]![axis],results[n]!.name+key+axis).toBeCloseTo(box[axis],1);
      for(const [scene,index] of [[1,0],[2,2]])expect(probes(scene!)["recording-broadcast-files-"+index]?.position).toBe('absolute');
      for(const [i,group] of groups.entries()){
        expect(baseline['recording-broadcast-status-'+i]!.y-group[1].y).toBeCloseTo(baseline['recording-broadcast-status-0']!.y-groups[0]![1].y,1);
      }
    }finally{vi.unstubAllGlobals();}
  },60000);
});
