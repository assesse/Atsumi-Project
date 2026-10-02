import { afterEach,describe,expect,it,vi } from "vitest";
import { DownloadEventPoller } from "./downloadEventPoller";
afterEach(()=>vi.useRealTimers());
describe("bounded download UI delivery",()=>{
  it("has only one batch in flight even if IPC never responds",async()=>{
    vi.useFakeTimers();
    const call=vi.fn(async(name:string)=>name==="ui_diagnostics_session" ? "epoch" : new Promise(()=>undefined));
    const poller=new DownloadEventPoller(call as never); const stop=poller.subscribe("download:changed",vi.fn());
    await vi.advanceTimersByTimeAsync(60_000);
    expect(call.mock.calls.filter(([n])=>n==="download_events_take")).toHaveLength(1); stop();
  });
  it("delivers one batch to all consumers and requests DB resync on overflow",async()=>{
    vi.useFakeTimers();
    const call=vi.fn(async(name:string)=>name==="ui_diagnostics_session" ? "epoch" : {items:[{job:{jobId:"job"},download:{entryId:"entry"}}],resync:true});
    const poller=new DownloadEventPoller(call as never); const a=vi.fn(),b=vi.fn(),resync=vi.fn();
    const stops=[poller.subscribe("download:changed",a),poller.subscribe("job:changed",b),poller.subscribe("download:resync",resync)];
    await vi.advanceTimersByTimeAsync(250); expect(a).toHaveBeenCalledOnce(); expect(b).toHaveBeenCalledOnce(); expect(resync).toHaveBeenCalledWith(true);
    stops.forEach((stop)=>stop()); const count=call.mock.calls.length; await vi.advanceTimersByTimeAsync(2000); expect(call).toHaveBeenCalledTimes(count);
  });
});
