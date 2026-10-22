import { afterEach, beforeEach, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/youtube/original_capture.js?raw";
import rawFraming from "../../../src-tauri/src/youtube/original_raw_framing.js?raw";

const vmName = "node:vm";
const { runInNewContext } = await import(vmName) as { runInNewContext(source: string, context: Record<string, unknown>): unknown };
const CHANNEL = "AbCdEf123_-";
const REQUEST = "30000000-0000-4000-8000-000000000001";
const RECORDING = "40000000-0000-4000-8000-000000000001";
const SUCCESSOR = "40000000-0000-4000-8000-000000000002";
const CONTINUATION = "50000000-0000-4000-8000-000000000001";
type Message = { kind: string; [key: string]: unknown };
type Status = { active: boolean; starting: boolean; stopping: boolean; recordingId: string | null; ready: boolean; detail: string; rateControlAllowed: boolean };
type Bridge = { canStart(video: unknown): boolean; readyForAutomatic(video?: unknown): boolean; canChangePlaybackRate(video: unknown): boolean; getStatus(): Status;
  configure(config:{rawCaptureEnabled?:boolean}):void;
  getDiagnostics(): { reason: string; appendCount: number; queuedBytes:number; sources: unknown[]; lastStart: unknown; lastTransportFault: unknown; latestRecord: {tracks:Array<{trackIndex:number;init:unknown;completeMoofs:number;completeMdats:number;acknowledgedAppends:number;first:unknown;last:unknown}>}|null };
  getReplayClock(video: unknown): null | { clock: string; sourceId: string; sourceTimeSeconds: number };
  cancelPending(requestId:string):boolean;
  start(command: Record<string, unknown>, options: Record<string, unknown>): Promise<unknown>; stop(reason?: string, interrupted?: boolean): Promise<unknown> };
class Target {
  listeners = new Map<string, Set<(event: Record<string, unknown>) => void>>();
  addEventListener(name: string, callback: (event: Record<string, unknown>) => void) { if (!this.listeners.has(name)) this.listeners.set(name, new Set()); this.listeners.get(name)!.add(callback); }
  removeEventListener(name: string, callback: (event: Record<string, unknown>) => void) { this.listeners.get(name)?.delete(callback); }
  dispatch(name: string, fields: Record<string, unknown> = {}) { for (const callback of [...this.listeners.get(name) ?? []]) callback({ target: this, ...fields }); }
}
const concat = (...parts: Uint8Array[]) => { const result = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let at = 0; for (const part of parts) { result.set(part, at); at += part.length; } return result; };
const box = (kind: string, payload = new Uint8Array()) => {
  const result = new Uint8Array(payload.length + 8); new DataView(result.buffer).setUint32(0, result.length);
  for (let i = 0; i < 4; i++) result[4 + i] = kind.charCodeAt(i); result.set(payload, 8); return result;
};
const init = () => concat(box("ftyp", new Uint8Array([1,2,3,4])), box("moov", new Uint8Array([5,6,7,8])));
const media = (size = 16) => concat(box("moof", new Uint8Array([1,2,3,4])), box("mdat", new Uint8Array(size).fill(9)));
const rawInit = () => Uint8Array.from([0x1a,0x45,0xdf,0xa3,0x81,1,0x18,0x53,0x80,0x67,0xff,0x15,0x49,0xa9,0x66,0x80,0x16,0x54,0xae,0x6b,0x81,2]);
const rawMedia = () => Uint8Array.from([0x1f,0x43,0xb6,0x75,0xff,0xa3,0x83,3,4,5]);
const rawUnit = () => concat(rawInit(), rawMedia());
const fragment = (trackId = 7, descriptionIndex = 1) => {
  const tfhd = new Uint8Array(12), tfhdView = new DataView(tfhd.buffer);
  tfhdView.setUint32(0, 0x020002); tfhdView.setUint32(4, trackId); tfhdView.setUint32(8, descriptionIndex);
  const trun = new Uint8Array(12), trunView = new DataView(trun.buffer);
  trunView.setUint32(0, 1); trunView.setUint32(4, 0xffffffff); trunView.setInt32(8, -24);
  return box('moof', box('traf', concat(box('tfhd', tfhd), box('trun', trun))));
};
const timingInit = (trackId:number,kind:'vide'|'soun',scale:number,flags=0x01010000) => {
 const tkhd=new Uint8Array(20),mdhd=new Uint8Array(20),hdlr=new Uint8Array(12),trex=new Uint8Array(24);
 new DataView(tkhd.buffer).setUint32(12,trackId);new DataView(mdhd.buffer).setUint32(12,scale);
 for(let i=0;i<4;i++)hdlr[8+i]=kind.charCodeAt(i);
 const defaults=new DataView(trex.buffer);defaults.setUint32(4,trackId);defaults.setUint32(8,1);defaults.setUint32(12,512);defaults.setUint32(16,128);defaults.setUint32(20,flags);
 return concat(box('ftyp',new Uint8Array(4)),box('moov',concat(box('trak',concat(box('tkhd',tkhd),box('mdia',concat(box('mdhd',mdhd),box('hdlr',hdlr))))),box('mvex',box('trex',trex)))));
};
const shapedInit = (options:{width?:number;height?:number;profile?:number;bitrate?:number;timescale?:number;extra?:Uint8Array;audio?:boolean;sampleRate?:number}={}) => {
 const audio=options.audio===true,tkhd=new Uint8Array(20),mdhd=new Uint8Array(20),hdlr=new Uint8Array(12),trex=new Uint8Array(24);
 new DataView(tkhd.buffer).setUint32(12,1);new DataView(mdhd.buffer).setUint32(12,options.timescale??(audio?48000:90000));
 for(let i=0;i<4;i++)hdlr[8+i]=(audio?'soun':'vide').charCodeAt(i);
 const defaults=new DataView(trex.buffer);defaults.setUint32(4,1);defaults.setUint32(8,1);
 const header=new Uint8Array(audio?28:78),sample=new DataView(header.buffer),bitrate=new Uint8Array(12);
 sample.setUint16(6,1);new DataView(bitrate.buffer).setUint32(8,options.bitrate??1000);
 if(audio){sample.setUint16(16,2);sample.setUint32(24,(options.sampleRate??48000)*65536);}else{sample.setUint16(24,options.width??1280);sample.setUint16(26,options.height??720);}
 const codec=audio?box('esds',new Uint8Array([0,0,0,0,3,3,0,0,0])):box('avcC',new Uint8Array([1,options.profile??100,0,31,255,225,0,0]));
 const stsd=box('stsd',concat(new Uint8Array([0,0,0,0,0,0,0,1]),box(audio?'mp4a':'avc1',concat(header,codec,box('btrt',bitrate)))));
 return concat(box('ftyp',new Uint8Array(4)),box('moov',concat(box('trak',concat(box('tkhd',tkhd),box('mdia',concat(box('mdhd',mdhd),box('hdlr',hdlr),box('minf',box('stbl',stsd)))))),box('mvex',box('trex',trex)),options.extra??new Uint8Array())));
};
const timedFragment = (trackId:number,time:number,options:{version?:number;tfhdFlags?:number;firstFlags?:number;sampleFlags?:number;cts?:number;trunVersion?:number}={}) => {
 const tfhd=new Uint8Array(options.tfhdFlags===undefined?12:24),head=new DataView(tfhd.buffer);
 head.setUint32(0,0x020002|(options.tfhdFlags===undefined?0:0x38));head.setUint32(4,trackId);head.setUint32(8,1);
 if(options.tfhdFlags!==undefined){head.setUint32(12,1024);head.setUint32(16,256);head.setUint32(20,options.tfhdFlags);}
 const tfdt=new Uint8Array(options.version===0?8:12),clock=new DataView(tfdt.buffer);clock.setUint32(0,options.version===0?0:0x01000000);
 if(options.version===0)clock.setUint32(4,time);else{clock.setUint32(4,Math.floor(time/0x100000000));clock.setUint32(8,time%0x100000000);}
 const flags=1|(options.firstFlags===undefined?0:4)|(options.sampleFlags===undefined?0:0x700)|(options.cts===undefined?0:0x800);
 const trun=new Uint8Array(12+(options.firstFlags===undefined?0:4)+(options.sampleFlags===undefined?0:12)+(options.cts===undefined?0:4)),run=new DataView(trun.buffer);
 run.setUint32(0,flags|((options.trunVersion??0)<<24));run.setUint32(4,1);run.setInt32(8,96);let at=12;
 if(options.firstFlags!==undefined){run.setUint32(at,options.firstFlags);at+=4;}
 if(options.sampleFlags!==undefined){run.setUint32(at,900);run.setUint32(at+4,333);run.setUint32(at+8,options.sampleFlags);at+=12;}
 if(options.cts!==undefined)run.setInt32(at,options.cts);
 return concat(box('moof',box('traf',concat(box('tfhd',tfhd),box('tfdt',tfdt),box('trun',trun)))),box('mdat',new Uint8Array(16).fill(55)));
};
const flush = async () => { for (let i = 0; i < 80; i++) await Promise.resolve(); };

function fixture(options: { url?: string; iframe?: boolean; queueBytes?: number; continuityBytes?:number; continuityPieces?:number; continuityFragments?:number; supported?: boolean; rollover?: boolean; stableRollover?:boolean; endingReason?: string; raw?:boolean; enableRaw?:boolean; ackDelayMs?:number; failRawGrowth?:boolean } = {}) {
  const nativeCalls: unknown[] = [];
  class Buffer extends Target {
    mode = "segments"; timestampOffset = 0; appendWindowStart = 0; appendWindowEnd = Infinity;
    throwNext: Error | null = null;
    appendBuffer(bytes: ArrayBuffer | ArrayBufferView) { if (this.throwNext) { const error = this.throwNext; this.throwNext = null; throw error; } nativeCalls.push({ buffer: this, bytes }); return "native-result"; }
    changeType(_mime: string) { return "native-change"; }
    abort() { return "native-abort"; }
    remove(_start: number, _end: number) { return "native-remove"; }
  }
  class MediaSource extends Target {
    sourceBuffers: Buffer[] = [];
    addSourceBuffer(_mime: string) { const buffer = new Buffer(); this.sourceBuffers.push(buffer); return buffer; }
    removeSourceBuffer(buffer: Buffer) { this.sourceBuffers = this.sourceBuffers.filter((b) => b !== buffer); }
  }
  class URLFactory { static createObjectURL(_value: unknown) { return `blob:fixture-${nativeCalls.length}`; } }
  const video = Object.assign(new Target(), { isConnected: true, tagName: "VIDEO", readyState: 4, ended: false, paused: false, seeking: false,
    videoWidth: 1280, videoHeight: 720, mediaKeys: null as unknown, srcObject: null as unknown,
    src: "", currentSrc: "", currentTime: 100, playbackRate: 1, getBoundingClientRect: () => ({ width: 960, height: 540 }) });
  const videos = [video];
  const window = Object.assign(new Target(), { location: new URL(options.url ?? `https://www.youtube.com/watch?v=${CHANNEL}`), top: null as unknown,
    URL: URLFactory, MediaSource: options.supported === false ? undefined : MediaSource, SourceBuffer: Buffer, __atsumiYouTubeEncodedCapture: undefined as Bridge | undefined });
  window.top = options.iframe ? {} : window;
  const document = { title: "Synthetic fixture", ad:false, querySelectorAll: (name: string) => name === "video" ? videos : [], querySelector: (name: string) => name === "#movie_player video" ? video : name.includes("ad-showing") && document.ad ? {} : null, get cookie(): never { throw new Error("No cookie reads"); } };
  let counter = 0;
  const context = { window, document, crypto: { randomUUID: () => `10000000-0000-4000-8000-${String(++counter).padStart(12, "0")}` },
    URLSearchParams, Uint8Array, ArrayBuffer, DataView, btoa, SharedArrayBuffer, Date, setInterval, clearInterval, setTimeout, clearTimeout };
  let script = options.queueBytes ? source.replace("const MAX_QUEUE = 64 * 1024 * 1024;", `const MAX_QUEUE = ${options.queueBytes};`) : source;
  if(options.continuityBytes)script=script.replace('const MAX_CONTINUITY = 32 * 1024 * 1024;',`const MAX_CONTINUITY = ${options.continuityBytes};`);
  if(options.continuityPieces)script=script.replace('const MAX_CONTINUITY_PIECES = 4096;',`const MAX_CONTINUITY_PIECES = ${options.continuityPieces};`);
  if(options.continuityFragments)script=script.replace('const MAX_CONTINUITY_FRAGMENTS = 128;',`const MAX_CONTINUITY_FRAGMENTS = ${options.continuityFragments};`);
  if(options.failRawGrowth)script=script.replace('new Uint8Array(bucket.size + size)','(() => { throw new Error("fixture allocation failure"); })()');
  if (options.raw) runInNewContext(rawFraming, context);
  runInNewContext(script, context);
  if (options.enableRaw) window.__atsumiYouTubeEncodedCapture?.configure({rawCaptureEnabled:true});
  const ms = new MediaSource(); video.src = URLFactory.createObjectURL(ms); video.currentSrc = video.src;
  const v = ms.addSourceBuffer(options.raw ? 'video/webm;codecs="vp09.00.51.08.01.01.01.01.00"' : "video/mp4; codecs=\"avc1.42E01E\"");
  const a = ms.addSourceBuffer("audio/mp4; codecs=\"mp4a.40.2\"");
  const messages: Message[] = []; const pending: Array<{ message: Message; resolve(value: unknown): void; reject(error: Error): void }> = [];
  const hold = new Set<string>(); const reject = new Set<string>(); const notices: string[] = [];
  const response = (message: Message) => message.kind === "encoded_begin" ? { id: message.requestId===CONTINUATION?SUCCESSOR:RECORDING, mode: "encoded", nativeApproved: true, captureChat: true, ...(options.raw?{format:'youtubeVp9AacRawV1'}:{}) } :
    message.kind === 'encoded_raw_init' ? {accepted:true,changed:false} :
    message.kind === "encoded_finish" ? { stopped: true, interrupted: message.interrupted, endingReason: options.endingReason,
      ...(options.rollover ? { continuation: { requestId: CONTINUATION, channelId: CHANNEL, rightsAcknowledged: true,automatic:options.stableRollover===true } } : {}) } : {};
  const request = (kind: string, fields: Record<string, unknown>) => {
    const message = { kind, ...fields }; messages.push(message);
    if (reject.has(kind)) return Promise.reject(new Error("native failure"));
    if (hold.has(kind)) return new Promise((resolve, reject) => pending.push({ message, resolve, reject }));
    if (kind === 'encoded_append' && options.ackDelayMs) return new Promise(resolve => setTimeout(() => resolve(response(message)), options.ackDelayMs));
    return Promise.resolve(response(message));
  };
  const bridge = () => window.__atsumiYouTubeEncodedCapture!;
  const load = () => { v.appendBuffer(options.raw ? rawUnit() : init()); a.appendBuffer(options.raw ? concat(init(),media()) : init()); };
  const start = (fields: Record<string, unknown> = {}, extras: Record<string, unknown> = {}) => bridge().start({ channelId: CHANNEL, rightsAcknowledged: true, requestId: REQUEST, ...fields },
    { request, post: vi.fn(), onStatus: (detail: string) => notices.push(detail), ...extras });
  const release = () => { for (const item of pending.splice(0)) item.resolve(response(item.message)); };
  return { window, document, video, ms, v, a, bridge, messages, nativeCalls, notices, hold, reject, pending, release, load, start,
    ofKind: (kind: string) => messages.filter((message) => message.kind === kind), again: () => runInNewContext(script, context) };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); });
it('raw readiness is disabled by default and exact configuration/roles are required',()=>{
 const f=fixture({raw:true});f.load();expect(f.bridge().canStart(f.video)).toBe(false);
 f.bridge().configure({rawCaptureEnabled:true});expect(f.bridge().canStart(f.video)).toBe(true);
 expect(f.bridge().getDiagnostics().reason).toBe('ready');
 f.a.timestampOffset=0.25;expect(f.bridge().canStart(f.video)).toBe(false);
 f.a.timestampOffset=0;f.video.mediaKeys={};expect(f.bridge().canStart(f.video)).toBe(false);
});
it('raw begins only at newly started init after native approval and preserves exact bytes',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();f.hold.add('encoded_begin');const start=f.start();await flush();
 f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));expect(f.ofKind('encoded_append')).toHaveLength(0);
 f.release();await start;await flush();expect(f.bridge().getStatus().starting).toBe(true);
 const video=concat(rawUnit(),rawUnit()),audio=concat(init(),box('skip',new Uint8Array([5,4])),media());
 f.v.appendBuffer(video);f.a.appendBuffer(audio);video.fill(0);audio.fill(0);await vi.advanceTimersByTimeAsync(250);
 const decode=(track:number)=>concat(...f.ofKind('encoded_append').filter(m=>m.trackIndex===track).map(m=>Uint8Array.from(atob(m.data as string),c=>c.charCodeAt(0))));
 expect(decode(0)).toEqual(concat(rawUnit(),rawUnit()));expect(decode(1)).toEqual(concat(init(),box('skip',new Uint8Array([5,4])),media()));
 expect(f.ofKind('encoded_begin')[0]).toMatchObject({format:'youtubeVp9AacRawV1',rawMode:'segments',rawVideoTimestampOffset:0,rawAudioTimestampOffset:0,rawAppendWindow:'default'});
 expect(f.ofKind('encoded_raw_init').map(m=>[m.trackIndex,m.unitIndex,m.startOffset])).toEqual([[0,0,0],[0,1,rawUnit().length],[1,0,0]]);
 expect(f.ofKind('encoded_append').filter(m=>m.rawFirstUnitEnd!==undefined).map(m=>m.rawFirstUnitEnd)).toEqual([rawUnit().length]);
 expect(f.bridge().getStatus().starting).toBe(false);expect(f.bridge().canChangePlaybackRate(f.video)).toBe(false);expect(f.bridge().getReplayClock(f.video)).toBeNull();
});
it('raw serial admission holds data and all later-track work behind the exact init ACK',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.hold.add('encoded_raw_init');f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));await flush();
 expect(f.ofKind('encoded_raw_init')).toHaveLength(1);expect(f.ofKind('encoded_append')).toHaveLength(0);
 f.hold.delete('encoded_raw_init');f.release();await vi.advanceTimersByTimeAsync(250);expect(f.ofKind('encoded_append')).toHaveLength(2);
 expect(f.messages.map(m=>m.kind)).toEqual(['encoded_begin','encoded_raw_init','encoded_append','encoded_raw_init','encoded_append']);
});
it('raw changed init rejects new bytes and preserves old exact prefix without AVC continuation',async()=>{
 const f=fixture({raw:true,enableRaw:true,rollover:true});f.load();await f.start();f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));await flush();
 f.hold.add('encoded_raw_init');f.v.appendBuffer(rawUnit());await flush();expect(f.pending).toHaveLength(1);
 f.pending.splice(0)[0]!.resolve({accepted:false,changed:true});await flush();
 expect(f.ofKind('encoded_append').filter(m=>m.trackIndex===0)).toHaveLength(1);
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'init_changed'});
 vi.advanceTimersByTime(1000);await flush();expect(f.ofKind('encoded_begin')).toHaveLength(1);
});
it('raw stop before delayed approval never arms or queues post-stop media',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();f.hold.add('encoded_begin');const start=f.start();await flush();const stopped=f.bridge().stop();f.release();await start;await stopped;
 f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));await flush();expect(f.ofKind('encoded_append')).toHaveLength(0);expect(f.bridge().getStatus().active).toBe(false);
});
it('raw cancelled pending request cannot arm after a delayed native approval',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();f.hold.add('encoded_begin');const start=f.start();await flush();expect(f.bridge().cancelPending(REQUEST)).toBe(true);f.release();await start;await flush();
 f.v.appendBuffer(rawUnit());await flush();expect(f.ofKind('encoded_append')).toHaveLength(0);expect(f.ofKind('encoded_finish')).toHaveLength(1);
});
it('raw init-only or one-track data does not reset the fixed startup deadline',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());await flush();
 vi.advanceTimersByTime(14500);f.a.appendBuffer(init());await flush();expect(f.bridge().getStatus().starting).toBe(true);
 vi.advanceTimersByTime(500);await flush();expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'first_append_timeout'});
});
it('raw boundaries splitting caller appends are not confused with preapproval header bytes',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();f.v.appendBuffer(rawUnit().subarray(0,2));await f.start();f.v.appendBuffer(rawUnit().subarray(2));await flush();expect(f.ofKind('encoded_append')).toHaveLength(0);
 for(const byte of rawUnit()) f.v.appendBuffer(Uint8Array.of(byte));await vi.advanceTimersByTimeAsync(250);
 const all=concat(...f.ofKind('encoded_append').map(m=>Uint8Array.from(atob(m.data as string),c=>c.charCodeAt(0))));expect(all).toEqual(rawUnit());
});
it('raw automatic stability uses complete structural units without byte-identical padding assumptions',()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();expect(f.bridge().readyForAutomatic()).toBe(false);
 for(let n=0;n<2;n++){f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));}
 f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(true);expect(f.ofKind('encoded_begin')).toHaveLength(0);
});
it('raw queue byte bounds preserve original append success and produce explicit interrupted finish',async()=>{
 const f=fixture({raw:true,enableRaw:true,queueBytes:32});f.load();await f.start();expect(f.v.appendBuffer(rawUnit())).toBe('native-result');await flush();
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'queue_overflow'});
});
it('raw observer never captures a failed original append or mutates its exception',()=>{
 const f=fixture({raw:true,enableRaw:true});const error=new Error('original');f.v.throwNext=error;expect(()=>f.v.appendBuffer(rawUnit())).toThrow(error);
 expect(f.bridge().getDiagnostics().appendCount).toBe(0);expect(f.messages).toHaveLength(0);
});
it('raw immediate ACK retry repeats the exact chunk and one boundary hint without advancing offsets',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());f.a.appendBuffer(concat(init(),media()));await vi.advanceTimersByTimeAsync(250);
 f.hold.add('encoded_append');f.v.appendBuffer(rawUnit());await flush();
 const first=f.pending.splice(0)[0]!;expect(first.message.rawFirstUnitEnd).toBe(rawUnit().length);
 first.reject(Object.assign(new Error('busy'),{code:'BRIDGE_BUSY'}));await flush();vi.advanceTimersByTime(100);await flush();
 expect(f.pending[0]?.message).toEqual(first.message);f.hold.delete('encoded_append');f.release();await flush();
 f.v.appendBuffer(rawUnit());await vi.advanceTimersByTimeAsync(250);expect(f.ofKind('encoded_append').filter(m=>m.rawFirstUnitEnd!==undefined)).toHaveLength(2);
 expect(f.ofKind('encoded_append').at(-1)).toMatchObject({appendIndex:2});expect(f.ofKind('encoded_append').at(-1)?.rawFirstUnitEnd).toBeUndefined();
});
it('raw rejects unknown native format ACK and never authorizes a legacy fallback',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();
 await expect(f.start({}, {request:()=>Promise.reject(Object.assign(new Error('unsupported'),{code:'ENCODED_UNSUPPORTED'}))})).rejects.not.toHaveProperty('allowLegacyFallback');
 const wrong=fixture({raw:true,enableRaw:true});wrong.load();await expect(wrong.start({}, {request:()=>Promise.resolve({id:RECORDING,mode:'encoded',nativeApproved:true,format:'other'})})).rejects.toThrow('encoded_not_approved');
});
it.each(['advertisement','source','offset'] as const)('raw stops on %s without another source or new archive',async(kind:'advertisement'|'source'|'offset')=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());await flush();
 if(kind==='advertisement')f.document.ad=true;else if(kind==='source')f.video.src=f.video.currentSrc='blob:replacement';else f.a.timestampOffset=1;
 vi.advanceTimersByTime(500);await flush();expect(f.ofKind('encoded_finish')[0]?.interrupted).toBe(true);expect(f.ofKind('encoded_begin')).toHaveLength(1);
});
it('raw pending metadata task count remains bounded across repeated structural units',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.hold.add('encoded_raw_init');f.v.appendBuffer(rawUnit());await flush();
 for(let n=0;n<520;n++)f.v.appendBuffer(rawUnit());
 f.hold.delete('encoded_raw_init');f.release();await f.bridge().stop();
 expect((f.bridge().getDiagnostics() as unknown as {lastStop:{reason:string}}).lastStop.reason).toBe('queue_overflow');
});
it('raw sustained same-track bursts remain bounded under unchanged limits and delayed durable ACKs',async()=>{
 const f=fixture({raw:true,enableRaw:true,ackDelayMs:80});f.load();await f.start();
 const expectedVideo=[concat(rawUnit(),rawUnit())],expectedAudio=[concat(init(),media())];
 f.v.appendBuffer(expectedVideo[0]!);f.a.appendBuffer(expectedAudio[0]!);await vi.advanceTimersByTimeAsync(1000);
 expect(f.bridge().getStatus().starting).toBe(false);
 for(let n=0;n<1125;n++){
  const audio=n%9===8,bytes=audio?media():Uint8Array.of(0xa3,0x83,3,4,5);
  (audio?expectedAudio:expectedVideo).push(bytes.slice());(audio?f.a:f.v).appendBuffer(bytes);
  bytes.fill(0);await vi.advanceTimersByTimeAsync(20);
 }
 expect((f.bridge().getDiagnostics() as unknown as {lastStop:unknown}).lastStop).toBeNull();
 expect(f.bridge().getStatus()).toMatchObject({active:true,stopping:false});
 const stopped=f.bridge().stop();await vi.advanceTimersByTimeAsync(100000);await stopped;
 const sent=(track:number)=>concat(...f.ofKind('encoded_append').filter(m=>m.trackIndex===track).map(m=>Uint8Array.from(atob(m.data as string),c=>c.charCodeAt(0))));
 expect(sent(0)).toEqual(concat(...expectedVideo));expect(sent(1)).toEqual(concat(...expectedAudio));
 expect(f.ofKind('encoded_append')).toHaveLength(253);
 expect(f.ofKind('encoded_raw_init').map(m=>[m.trackIndex,m.unitIndex,m.startOffset])).toEqual([[0,0,0],[0,1,rawUnit().length],[1,0,0]]);
 expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.ofKind('encoded_finish')).toHaveLength(1);
});
it('raw staging deadline never slides and owns caller bytes until a single exact flush',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();
 const first=rawUnit(),next=Uint8Array.of(0xa3,0x83,3,4,5),expected=concat(first,next);
 f.v.appendBuffer(first);first.fill(0);await vi.advanceTimersByTimeAsync(200);
 f.v.appendBuffer(next);next.fill(0);await vi.advanceTimersByTimeAsync(49);
 expect(f.ofKind('encoded_append')).toHaveLength(0);expect(f.bridge().getDiagnostics().queuedBytes).toBe(expected.length);
 await vi.advanceTimersByTimeAsync(1);expect(f.ofKind('encoded_append')).toHaveLength(1);
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(expected);expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
 await f.bridge().stop();await vi.advanceTimersByTimeAsync(1000);expect(f.ofKind('encoded_append')).toHaveLength(1);
});
it('raw track and init barriers preserve order while the first boundary hint is immediate',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());await flush();
 expect(f.ofKind('encoded_append')).toHaveLength(0);f.a.appendBuffer(concat(init(),media()));await flush();
 expect(f.messages.map(m=>m.kind)).toEqual(['encoded_begin','encoded_raw_init','encoded_append','encoded_raw_init']);
 f.v.appendBuffer(rawUnit());await flush();
 expect(f.messages.map(m=>[m.kind,m.trackIndex])).toEqual([
  ['encoded_begin',undefined],['encoded_raw_init',0],['encoded_append',0],['encoded_raw_init',1],
  ['encoded_append',1],['encoded_raw_init',0],['encoded_append',0],
 ]);
 expect(f.ofKind('encoded_append').at(-1)).toMatchObject({appendIndex:1,rawFirstUnitEnd:rawUnit().length});
 expect(f.bridge().getStatus().starting).toBe(false);await f.bridge().stop();
});
it('raw exact target flushes without delay and large appends keep one logical index',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();const target=128*1024;
 f.a.appendBuffer(init());f.a.appendBuffer(media(target-init().length-20));await flush();
 expect(f.ofKind('encoded_append')).toHaveLength(1);expect(f.ofKind('encoded_append')[0]).toMatchObject({appendIndex:0,chunkIndex:0,finalChunk:true});
 const large=media(target*3);f.a.appendBuffer(large);await flush();
 const chunks=f.ofKind('encoded_append').slice(1);expect(chunks).toHaveLength(4);
 expect(chunks.map(m=>[m.appendIndex,m.chunkIndex,m.finalChunk])).toEqual([[1,0,false],[1,1,false],[1,2,false],[1,3,true]]);
 expect(recordedBytes(f.messages,RECORDING,1)).toEqual(concat(init(),media(target-init().length-20),large));
 expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);await f.bridge().stop();
});
it('raw stop seals one partially filled bucket and drains it before one finish',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.hold.add('encoded_append');f.v.appendBuffer(rawUnit());await flush();
 const one=f.bridge().stop(),two=f.bridge().stop();await flush();
 expect(f.pending).toHaveLength(1);expect(f.ofKind('encoded_finish')).toHaveLength(0);
 expect(f.bridge().getDiagnostics().queuedBytes).toBe(rawUnit().length);
 f.v.appendBuffer(rawUnit());f.hold.delete('encoded_append');f.release();await one;await two;await vi.advanceTimersByTimeAsync(1000);
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(rawUnit());expect(f.ofKind('encoded_finish')).toHaveLength(1);
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:false,reason:'user_stop'});expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
});
it('raw local budget failure drains the already reserved bucket without reentrant overflow',async()=>{
 const f=fixture({raw:true,enableRaw:true,queueBytes:64});f.load();await f.start();
 const tiny=Uint8Array.of(0xa3,0x83,3,4,5);f.v.appendBuffer(rawUnit());f.v.appendBuffer(tiny);f.v.appendBuffer(tiny);f.v.appendBuffer(tiny);
 await f.bridge().stop();await vi.advanceTimersByTimeAsync(1000);
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(concat(rawUnit(),tiny,tiny));
 expect(f.ofKind('encoded_append')).toHaveLength(1);expect(f.ofKind('encoded_finish')).toHaveLength(1);
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'queue_overflow'});expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
});
it('raw allocation failure preserves the prior owned bytes and rejects only new input',async()=>{
 const f=fixture({raw:true,enableRaw:true,failRawGrowth:true});f.load();await f.start();f.v.appendBuffer(rawUnit());
 expect(f.v.appendBuffer(Uint8Array.of(0xa3,0x83,3,4,5))).toBe('native-result');await f.bridge().stop();
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(rawUnit());
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'queue_overflow'});expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
});
it('raw terminal native rejection discards an unsent bucket and never rearms its timer',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.reject.add('encoded_raw_init');f.v.appendBuffer(rawUnit());await flush();
 expect(f.ofKind('encoded_append')).toHaveLength(0);expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'native_rejected'});
 f.v.appendBuffer(rawUnit());await vi.advanceTimersByTimeAsync(1000);expect(f.ofKind('encoded_append')).toHaveLength(0);expect(f.ofKind('encoded_finish')).toHaveLength(1);
 expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
});
it('raw stopped bucket timer cannot flush a successor recording before its own deadline',async()=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());await vi.advanceTimersByTimeAsync(100);await f.bridge().stop();
 await f.start({requestId:CONTINUATION});f.v.appendBuffer(rawUnit());await vi.advanceTimersByTimeAsync(249);
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(rawUnit());expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
 await vi.advanceTimersByTimeAsync(1);expect(recordedBytes(f.messages,SUCCESSOR,0)).toEqual(rawUnit());await f.bridge().stop();
 expect(f.ofKind('encoded_finish')).toHaveLength(2);
});
it.each(['pagehide','stop','source'] as const)('raw %s invalidation drains only its own accepted prefix once',async(cause)=>{
 const f=fixture({raw:true,enableRaw:true});f.load();await f.start();f.v.appendBuffer(rawUnit());
 if(cause==='pagehide')f.window.dispatch('pagehide');else if(cause==='stop')void f.bridge().stop();
 else f.video.src=f.video.currentSrc='blob:replacement';
 await vi.advanceTimersByTimeAsync(1000);f.v.appendBuffer(rawUnit());await vi.advanceTimersByTimeAsync(1000);
 expect(recordedBytes(f.messages,RECORDING,0)).toEqual(rawUnit());expect(f.ofKind('encoded_finish')).toHaveLength(1);expect(f.bridge().getStatus().active).toBe(false);
});
it('raw strict alternating input still hits the unchanged task bound honestly',async()=>{
 const f=fixture({raw:true,enableRaw:true,ackDelayMs:80});f.load();await f.start();
 f.v.appendBuffer(concat(rawUnit(),rawUnit()));f.a.appendBuffer(concat(init(),media()));await vi.advanceTimersByTimeAsync(1000);
 for(let n=0;n<1125;n++){(n%2?f.a:f.v).appendBuffer(n%2?media():Uint8Array.of(0xa3,0x83,3,4,5));await vi.advanceTimersByTimeAsync(20);}
 const stopped=f.bridge().stop();await vi.advanceTimersByTimeAsync(100000);await stopped;
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'queue_overflow'});
 expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.ofKind('encoded_finish')).toHaveLength(1);expect(f.bridge().getDiagnostics().queuedBytes).toBe(0);
});
it('requires five stable seconds and two complete fragments per track for automatic start only',()=>{
 const f=fixture();f.load();expect(f.bridge().canStart(f.video)).toBe(true);expect(f.bridge().readyForAutomatic()).toBe(false);
 f.v.appendBuffer(media());f.v.appendBuffer(media());f.a.appendBuffer(media());f.video.currentTime+=2;
 vi.advanceTimersByTime(5000);expect(f.bridge().readyForAutomatic()).toBe(false);
 f.a.appendBuffer(media());expect(f.bridge().readyForAutomatic()).toBe(true);
 expect(f.ofKind('encoded_begin')).toHaveLength(0);
});
it('resets automatic readiness on changed init while identical init retains stable observation',()=>{
 const f=fixture();f.load();f.bridge().readyForAutomatic();
 for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 vi.advanceTimersByTime(5000);f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(true);
 f.v.appendBuffer(init());expect(f.bridge().readyForAutomatic()).toBe(true);
 const changed=init();changed[changed.length-1]=(changed.at(-1) ?? 0)^1;f.v.appendBuffer(changed);
 expect(f.bridge().canStart(f.video)).toBe(true);expect(f.bridge().readyForAutomatic()).toBe(false);
 for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 vi.advanceTimersByTime(4999);f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(false);
 vi.advanceTimersByTime(1);expect(f.bridge().readyForAutomatic()).toBe(true);
});
it('requires advancing playback and recent per-track ingress, never a paused buffered frame',()=>{
 const f=fixture();f.load();f.bridge().readyForAutomatic();
 for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 vi.advanceTimersByTime(5000);expect(f.bridge().readyForAutomatic()).toBe(false);
 f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(true);
 f.video.paused=true;expect(f.bridge().readyForAutomatic()).toBe(false);
 f.video.paused=false;expect(f.bridge().readyForAutomatic()).toBe(false);
 f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(true);
 vi.advanceTimersByTime(5001);f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(false);
 f.v.appendBuffer(media());expect(f.bridge().readyForAutomatic()).toBe(false);
 f.a.appendBuffer(media());expect(f.bridge().readyForAutomatic()).toBe(true);
 f.video.seeking=true;expect(f.bridge().readyForAutomatic()).toBe(false);
});
it('pairs safe resolution and AVC configuration facts only on the first changed init',async()=>{
 const f=fixture();const initial=shapedInit(),changed=shapedInit({width:1920,height:1080,profile:77});
 f.v.appendBuffer(initial);f.a.appendBuffer(init());await f.start();
 f.v.appendBuffer(concat(changed,shapedInit({width:640,height:360})));
 await flush();const finish=f.ofKind('encoded_finish');expect(finish).toHaveLength(1);
 expect(finish[0]).toMatchObject({interrupted:true,reason:'init_changed',initChange:{version:1,trackIndex:0,changeMask:3,
  previous:{initBytes:initial.length,tracks:[{kind:1,codecFamily:1,width:1280,height:720,timescale:90000,codecProfile:100,codecLevel:31}]},
  next:{initBytes:changed.length,tracks:[{kind:1,codecFamily:1,width:1920,height:1080,timescale:90000,codecProfile:77,codecLevel:31}]}}});
 const evidence=JSON.stringify(finish[0]!.initChange);expect(evidence).not.toMatch(/ascConfigHex|base64|rawBytes|ranges|structure|blob:|requestId|channelId/);
 expect(f.ofKind('encoded_append')).toHaveLength(0);expect(Array.from(initial)).toEqual(Array.from(shapedInit()));
});
it('distinguishes bitrate and timing header differences without relaxing init_changed rejection',async()=>{
 for(const [options,mask]of [[{bitrate:2000},16],[{timescale:60000},8]] as const){
  const f=fixture();f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.v.appendBuffer(shapedInit(options));await flush();
  expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'init_changed',initChange:{changeMask:mask,previous:{tracks:[{width:1280,height:720}]},next:{tracks:[{width:1280,height:720}]}}});
 }
});
it('projects numeric audio headers without exporting the esds descriptor or arbitrary metadata',async()=>{
 const f=fixture();f.v.appendBuffer(init());f.a.appendBuffer(shapedInit({audio:true}));await f.start();
 f.a.appendBuffer(shapedInit({audio:true,sampleRate:44100,extra:box('free',new TextEncoder().encode('private-init-metadata'))}));await flush();
 const result=f.ofKind('encoded_finish')[0];expect(result).toMatchObject({reason:'init_changed',initChange:{trackIndex:1,changeMask:97,
  previous:{tracks:[{kind:2,codecFamily:3,audioChannels:2,audioSampleRate:48000,width:0,height:0,codecProfile:0}]},next:{tracks:[{audioSampleRate:44100}]}}});
 expect(JSON.stringify(result)).not.toContain('private-init-metadata');expect(JSON.stringify(result)).not.toContain('esds');
});
it('does not emit paired diagnostics for identical init or ordinary manual stop',async()=>{
 const f=fixture();f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.v.appendBuffer(shapedInit());await flush();
 expect(f.ofKind('encoded_finish')).toHaveLength(0);await f.bridge().stop();expect(f.ofKind('encoded_finish')[0]).not.toHaveProperty('initChange');
});
it('bounds malformed or excessive init diagnostics while still finalizing the original failure',async()=>{
 for(const [bytes,dropped]of [[init(),'invalid'],[shapedInit({extra:concat(...Array.from({length:96},()=>box('free')))}),'structure_limit']] as const){
  const f=fixture();f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.v.appendBuffer(bytes);await flush();
  expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'init_changed',initChange:{version:1,trackIndex:0,dropped}});
  expect(Object.keys(f.ofKind('encoded_finish')[0]!.initChange as object)).toEqual(['version','trackIndex','dropped']);
 }
});
it('queues old media before the changed init and never mixes the new media into that job',async()=>{
 const f=fixture();f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();const old=media(24),next=shapedInit({width:854,height:480});
 f.v.appendBuffer(concat(old,next,media(48)));await flush();
 const chunks=f.ofKind('encoded_append').map(message=>Uint8Array.from(atob(message.data as string),value=>value.charCodeAt(0)));
 expect(Array.from(concat(...chunks))).toEqual(Array.from(old));expect(f.messages.map(message=>message.kind)).toEqual(['encoded_begin','encoded_append','encoded_finish']);
 expect(f.ofKind('encoded_finish')[0]).toMatchObject({reason:'init_changed',interrupted:true});
});
it('uses a fresh arm after stable same-receiver continuation and lets exact pending cancel win',async()=>{
 const f=fixture({rollover:true,stableRollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();
 f.v.appendBuffer(shapedInit({width:854,height:480}));await flush();expect(f.bridge().getStatus().starting).toBe(true);
 vi.advanceTimersByTime(250);for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 f.video.currentTime+=2;vi.advanceTimersByTime(4749);await flush();expect(f.ofKind('encoded_begin')).toHaveLength(1);
 vi.advanceTimersByTime(251);await flush();expect(f.ofKind('encoded_begin')[1]).toMatchObject({requestId:CONTINUATION});
 f.v.appendBuffer(shapedInit({width:640,height:360}));await flush();expect(f.bridge().cancelPending(REQUEST)).toBe(false);
 expect(f.bridge().cancelPending(CONTINUATION)).toBe(true);for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 f.video.currentTime+=3;vi.advanceTimersByTime(6000);await flush();expect(f.ofKind('encoded_begin')).toHaveLength(2);
});
it('rechecks automatic stability at start while preserving manual initial readiness',async()=>{
 const f=fixture();f.load();f.bridge().readyForAutomatic();for(let n=0;n<2;n++){f.v.appendBuffer(media());f.a.appendBuffer(media());}
 vi.advanceTimersByTime(5000);f.video.currentTime+=2;expect(f.bridge().readyForAutomatic()).toBe(true);
 f.v.appendBuffer(shapedInit());await expect(f.start({automatic:true})).rejects.toThrow('encoded_unavailable');expect(f.ofKind('encoded_begin')).toHaveLength(0);
 await f.start();expect(f.ofKind('encoded_begin')).toHaveLength(1);await f.bridge().stop();
});
it('remembers exact pending cancellation that arrives before the finish reply',async()=>{
 const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add('encoded_finish');
 f.v.appendBuffer(shapedInit({width:854,height:480}));await flush();expect(f.bridge().cancelPending(CONTINUATION)).toBe(false);
 f.release();await flush();vi.advanceTimersByTime(1000);await flush();expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.bridge().getStatus().starting).toBe(false);
});

const recordedBytes = (messages:Message[],recordingId:string,trackIndex:number) => concat(...messages
 .filter(message=>message.kind==='encoded_append'&&message.recordingId===recordingId&&message.trackIndex===trackIndex)
 .map(message=>Uint8Array.from(atob(message.data as string),value=>value.charCodeAt(0))));
it('buffers new complete A/V through finish and stability, then flushes once only after fresh native approval',async()=>{
 const f=fixture({rollover:true,stableRollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(shapedInit({audio:true}));await f.start();
 const oldAudio=media(31),v1=media(41),v2=media(42),a1=media(51),a2=media(52);
 f.a.appendBuffer(oldAudio);await flush();f.hold.add('encoded_finish');
 const nextInit=shapedInit({width:854,height:480});f.v.appendBuffer(concat(nextInit,v1));f.a.appendBuffer(a1);
 f.v.appendBuffer(v2.subarray(0,3));f.v.appendBuffer(v2.subarray(3,21));f.v.appendBuffer(v2.subarray(21));f.a.appendBuffer(a2);
 await flush();expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
 f.bridge().readyForAutomatic();f.video.currentTime+=2;vi.advanceTimersByTime(5000);
 f.hold.add('encoded_begin');f.hold.delete('encoded_finish');f.release();await flush();vi.advanceTimersByTime(250);await flush();
 expect(f.ofKind('encoded_begin')[1]).toMatchObject({requestId:CONTINUATION,tracks:expect.arrayContaining([expect.objectContaining({init:btoa(String.fromCharCode(...nextInit))})])});
 expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
 f.release();await flush();expect(Array.from(recordedBytes(f.messages,SUCCESSOR,0))).toEqual(Array.from(concat(v1,v2)));
 expect(Array.from(recordedBytes(f.messages,SUCCESSOR,1))).toEqual(Array.from(concat(oldAudio,a1,a2)));
 await f.bridge().stop();
});
it('hands an overlapping partial audio fragment to the new file exactly once, then returns to live forwarding',async()=>{
 const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(shapedInit({audio:true}));await f.start();
 const audio=media(80),nextAudio=media(37),video=media(35);const split=29;
 f.a.appendBuffer(audio.subarray(0,split));f.v.appendBuffer(concat(shapedInit({width:854,height:480}),video));await flush();
 vi.advanceTimersByTime(250);await flush();expect(f.ofKind('encoded_begin')).toHaveLength(2);
 expect(recordedBytes(f.messages,SUCCESSOR,1)).toHaveLength(0);
 f.a.appendBuffer(concat(audio.subarray(split),nextAudio));await flush();
 expect(Array.from(recordedBytes(f.messages,SUCCESSOR,1))).toEqual(Array.from(concat(audio,nextAudio)));
 expect(Array.from(recordedBytes(f.messages,SUCCESSOR,0))).toEqual(Array.from(video));await f.bridge().stop();
});
it('keeps split headers and byte-identical repeated init safe without retaining caller-owned slices',async()=>{
 const f=fixture({rollover:true});const nextInit=shapedInit({width:854,height:480});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();
 f.hold.add('encoded_finish');const bytes=media(43),expected=bytes.slice();
 f.v.appendBuffer(nextInit);f.v.appendBuffer(bytes.subarray(0,2));f.v.appendBuffer(bytes.subarray(2));bytes.fill(77);
 f.v.appendBuffer(nextInit);await flush();f.hold.delete('encoded_finish');f.release();await flush();vi.advanceTimersByTime(250);await flush();
 expect(Array.from(recordedBytes(f.messages,SUCCESSOR,0))).toEqual(Array.from(expected));await f.bridge().stop();
});
it('does not silently drop a second new configuration already buffered during the same transition',async()=>{
 const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add('encoded_finish');
 f.v.appendBuffer(concat(shapedInit({width:854,height:480}),media(25)));f.v.appendBuffer(shapedInit({width:640,height:360}));await flush();
 f.release();await flush();vi.advanceTimersByTime(6000);await flush();
 expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.notices).toContain('encoded_interrupted');expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
});
it('bounds transition bytes, entry count and tiny split-piece allocations instead of silently evicting',async()=>{
 for(const limits of [{continuityBytes:64},{continuityFragments:2},{continuityPieces:8}]){
  const f=fixture({rollover:true,...limits});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add('encoded_finish');
  f.v.appendBuffer(shapedInit({width:854,height:480}));
  if(limits.continuityPieces){const bytes=media(32);for(const value of bytes)f.v.appendBuffer(new Uint8Array([value]));}
  else for(let n=0;n<4;n++)f.v.appendBuffer(media(32));
  await flush();f.release();await flush();vi.advanceTimersByTime(1000);await flush();
  expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.notices).toContain('encoded_interrupted');expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
 }
});
it('limits the entire transition lifetime including a delayed finish reply and delayed successor approval',async()=>{
 for(const delayed of ['encoded_finish','encoded_begin']){
  const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add(delayed);
  f.v.appendBuffer(concat(shapedInit({width:854,height:480}),media(32)));await flush();vi.advanceTimersByTime(250);await flush();
  vi.advanceTimersByTime(15000);await flush();f.release();await flush();
  expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);expect(f.notices).toContain('encoded_interrupted');
  expect(f.ofKind('encoded_begin')).toHaveLength(delayed==='encoded_begin'?2:1);
 }
});
it('clears complete and partial pocket data on stop, pagehide, ad, encryption and receiver replacement',async()=>{
 for(const cause of ['stop','pagehide','ad','encrypted','source']){
  const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add('encoded_finish');
  f.v.appendBuffer(concat(shapedInit({width:854,height:480}),media(32)));f.a.appendBuffer(media(80).subarray(0,30));await flush();
  if(cause==='stop')void f.bridge().stop();else if(cause==='pagehide')f.window.dispatch('pagehide');else if(cause==='ad')f.document.ad=true;
  else if(cause==='encrypted')f.video.mediaKeys={};else f.video.currentSrc='blob:replacement';
  vi.advanceTimersByTime(500);await flush();f.release();await flush();vi.advanceTimersByTime(1000);await flush();
  expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);
 }
});
it('does not create continuation scratch before the initial native approval or replay pre-start media',async()=>{
 const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());const before=media(93);f.v.appendBuffer(before);
 f.hold.add('encoded_begin');const pending=f.start();await flush();
 f.v.appendBuffer(shapedInit({width:854,height:480}));f.v.appendBuffer(media(64));f.release();await pending;await flush();vi.advanceTimersByTime(1000);await flush();
 expect(f.ofKind('encoded_begin')).toHaveLength(1);expect(f.ofKind('encoded_append')).toHaveLength(0);
});
it('keeps a synchronous init snapshot and never flushes held successor bytes after rejected approval',async()=>{
 const f=fixture({rollover:true});f.v.appendBuffer(shapedInit());f.a.appendBuffer(init());await f.start();f.hold.add('encoded_begin');
 const nextInit=shapedInit({width:854,height:480});f.v.appendBuffer(concat(nextInit,media(32)));await flush();vi.advanceTimersByTime(250);await flush();
 const begin=f.ofKind('encoded_begin')[1];expect(begin).toMatchObject({tracks:expect.arrayContaining([expect.objectContaining({init:btoa(String.fromCharCode(...nextInit))})])});
 f.pending[0]!.reject(new Error('denied'));await flush();expect(recordedBytes(f.messages,SUCCESSOR,0)).toHaveLength(0);expect(f.notices).toContain('native_rejected');
});

it('preserves original append calls and exact case-sensitive identity without an extra receiver',async()=>{
 const f=fixture();f.load();expect(f.bridge().canStart(f.video)).toBe(true);expect(f.messages).toHaveLength(0);await f.start();const data=media();expect(f.v.appendBuffer(data)).toBe('native-result');await flush();expect(f.ofKind('encoded_append')).toHaveLength(1);await f.bridge().stop();expect(f.ofKind('encoded_finish')).toHaveLength(1);
});
it('latches safe start errors without retaining arbitrary error text and clears on explicit retry',async()=>{
 const f=fixture();f.load();await expect(f.start({}, {request:async()=>{throw Object.assign(new Error('private-url-or-account'),{code:'BROWSER_ACK_TIMEOUT'});}})).rejects.toThrow();
 expect(f.bridge().getDiagnostics().lastStart).toEqual({code:'BROWSER_ACK_TIMEOUT',message:null});expect(JSON.stringify(f.bridge().getDiagnostics())).not.toContain('private-url');f.v.appendBuffer(media());expect(f.bridge().getDiagnostics().lastStart).toEqual({code:'BROWSER_ACK_TIMEOUT',message:null});await f.start();expect(f.bridge().getDiagnostics().lastStart).toBeNull();await f.bridge().stop();
});
it('retains fixed native append rejection without exposing the raw exception message',async()=>{
 const f=fixture();f.load();await f.start({}, {request:async(kind:string)=>{if(kind==='encoded_begin')return{id:RECORDING,mode:'encoded',nativeApproved:true,captureChat:true};if(kind==='encoded_append')throw Object.assign(new Error('private-original-error'),{code:'NATIVE_REJECTED',diagnosticMessage:'지원하지 않는 trun flags입니다.'});return{stopped:true};}});f.v.appendBuffer(media());await flush();expect(f.bridge().getDiagnostics().lastTransportFault).toMatchObject({code:'NATIVE_REJECTED',message:'지원하지 않는 trun flags입니다.',attempt:0});expect(JSON.stringify(f.bridge().getDiagnostics())).not.toContain('private-original-error');
});
it('correlates numeric moof facts with the original rejected append even after later fragments arrive',async()=>{
 const f=fixture();f.load();await f.start();f.hold.add('encoded_append');
 const first=concat(fragment(7,1),box('mdat',new TextEncoder().encode('private-media-sample'))),original=Array.from(first);
 f.v.appendBuffer(first);await flush();f.v.appendBuffer(concat(fragment(999,2),box('mdat',new Uint8Array(10))));
 f.pending[0]!.reject(Object.assign(new Error('private-error'),{code:'NATIVE_REJECTED',diagnosticMessage:'절대 offset 또는 지원하지 않는 tfhd입니다.'}));await flush();
 expect(f.bridge().getDiagnostics().lastTransportFault).toMatchObject({appendIndex:0,fragments:[{moofBytes:56,trafCount:1,trafs:[{tfhd:{version:0,flags:0x020002,trackId:7,descriptionIndex:1,baseOffsetPresent:false,defaultBase:true},truns:[{version:0,flags:1,sampleCount:0xffffffff,dataOffset:-24}]}]}]});
 expect(Array.from(first)).toEqual(original);const diag=JSON.stringify(f.bridge().getDiagnostics());expect(diag).not.toContain('private-media-sample');expect(JSON.stringify(f.bridge().getDiagnostics().lastTransportFault)).not.toContain('999');expect(f.ofKind('encoded_append')[0]).not.toHaveProperty('fragments');
});
it('retains successful per-record timing summaries after stop and resets on the next recording',async()=>{
 const f=fixture();f.v.appendBuffer(timingInit(7,'vide',90000));f.a.appendBuffer(timingInit(9,'soun',48000));await f.start();
 f.v.appendBuffer(timedFragment(7,0x100000000+12,{sampleFlags:0x02000000,tfhdFlags:0x01010000,cts:-3,trunVersion:1}));f.a.appendBuffer(timedFragment(9,240000,{version:0}));await flush();await f.bridge().stop();
 const record=f.bridge().getDiagnostics().latestRecord!;expect(record.tracks[0]).toMatchObject({trackIndex:0,completeMoofs:1,completeMdats:1,acknowledgedAppends:1,init:{tracks:[{trackId:7,kind:'video',timescale:90000,defaultDuration:512,defaultSize:128,defaultFlags:0x01010000}]},first:{trafs:[{timescale:90000,kind:'video',tfdt:{version:1,baseDecodeTime:0x100000000+12},truns:[{firstSample:{duration:900,size:333,flags:0x02000000,compositionOffset:-3,effectiveFlags:0x02000000,randomAccess:true}}]}]}});
 expect(record.tracks[1]).toMatchObject({completeMoofs:1,completeMdats:1,acknowledgedAppends:1,last:{trafs:[{tfdt:{version:0,baseDecodeTime:240000},kind:'audio',timescale:48000}]}});expect(f.bridge().getDiagnostics().lastTransportFault).toBeNull();
 f.v.appendBuffer(timedFragment(7,0x100000100));expect(record.tracks[0]!.completeMoofs).toBe(1);await f.start();expect(f.bridge().getDiagnostics().latestRecord!.tracks[0]).toMatchObject({completeMoofs:0,completeMdats:0,acknowledgedAppends:0,first:null,last:null});await f.bridge().stop();
});
it('uses first-sample flag precedence and retains only first and last complete fragment summaries',async()=>{
 const f=fixture();f.v.appendBuffer(timingInit(7,'vide',90000));f.a.appendBuffer(timingInit(9,'soun',48000));await f.start();
 const cases=[{sampleFlags:0x02000000,tfhdFlags:0x01010000},{firstFlags:0x02000000,tfhdFlags:0x01010000},{tfhdFlags:0x02000000},{}];
 for(let i=0;i<cases.length;i++){f.v.appendBuffer(timedFragment(7,100+i,cases[i]));await flush();const track=f.bridge().getDiagnostics().latestRecord!.tracks[0]!;expect(track.last).toMatchObject({trafs:[{truns:[{firstSample:{effectiveFlags:i<3?0x02000000:0x01010000,randomAccess:i<3}}]}]});}
 const track=f.bridge().getDiagnostics().latestRecord!.tracks[0]!;expect(track).toMatchObject({completeMoofs:4,completeMdats:4,acknowledgedAppends:4,first:{trafs:[{tfdt:{baseDecodeTime:100}}]},last:{trafs:[{tfdt:{baseDecodeTime:103}}]}});expect(Object.keys(track)).toEqual(['trackIndex','init','completeMoofs','completeMdats','acknowledgedAppends','first','last']);await f.bridge().stop();
});
it('counts split moof/mdat completions once and saturates diagnostic counters',async()=>{
 const f=fixture();f.load();await f.start();const bytes=timedFragment(7,100),moofBytes=new DataView(bytes.buffer).getUint32(0);
 f.v.appendBuffer(bytes.subarray(0,13));await flush();expect(f.bridge().getDiagnostics().latestRecord!.tracks[0]).toMatchObject({completeMoofs:0,completeMdats:0});
 f.v.appendBuffer(bytes.subarray(13,moofBytes+9));await flush();expect(f.bridge().getDiagnostics().latestRecord!.tracks[0]).toMatchObject({completeMoofs:1,completeMdats:0,first:null});
 f.v.appendBuffer(bytes.subarray(moofBytes+9));await flush();const track=f.bridge().getDiagnostics().latestRecord!.tracks[0]!;expect(track).toMatchObject({completeMoofs:1,completeMdats:1,acknowledgedAppends:3});
 track.completeMoofs=track.completeMdats=track.acknowledgedAppends=0xffffffff;f.v.appendBuffer(bytes);await flush();expect(track).toMatchObject({completeMoofs:0xffffffff,completeMdats:0xffffffff,acknowledgedAppends:0xffffffff});await f.bridge().stop();
});
it('keeps unsafe or truncated timing diagnostics static while preserving forwarded bytes',async()=>{
 const f=fixture();f.load();await f.start();const bytes=timedFragment(7,Number.MAX_SAFE_INTEGER+1);const before=Array.from(bytes);f.v.appendBuffer(bytes);await flush();expect(f.bridge().getDiagnostics().latestRecord!.tracks[0]).toMatchObject({completeMoofs:1,completeMdats:1,last:{dropped:'tfdt_unsafe_integer'}});expect(Array.from(bytes)).toEqual(before);expect(f.ofKind('encoded_append')).toHaveLength(1);expect(f.bridge().canStart(f.video)).toBe(true);await f.bridge().stop();
});
it('keeps completed numeric moof facts across split appends and a separate mdat continuation',async()=>{
 const f=fixture();f.load();await f.start();const moof=fragment(12,1);
 for(const bytes of [moof.subarray(0,5),moof.subarray(5,21),moof.subarray(21)]){f.v.appendBuffer(bytes);await flush();}
 f.reject.add('encoded_append');f.v.appendBuffer(box('mdat',new Uint8Array(4)));await flush();
 expect(f.bridge().getDiagnostics().lastTransportFault).toMatchObject({appendIndex:2,fragments:[{trafs:[{tfhd:{trackId:12,descriptionIndex:1}}]}]});
});
it('captures each adjacent fragment once and bounds diagnostic fragment count without changing forwarded bytes',async()=>{
 const f=fixture();f.load();await f.start();f.reject.add('encoded_append');const bytes=concat(...[1,2,3,4,5,6].map(id=>concat(fragment(id),box('mdat',new Uint8Array(1)))));
 f.v.appendBuffer(bytes);await flush();const fault=f.bridge().getDiagnostics().lastTransportFault as {fragments:Array<{trafs?:Array<{tfhd:{trackId:number}}> ;dropped?:string}>};
 expect(fault.fragments).toHaveLength(5);expect(fault.fragments.slice(0,4).map(item=>item.trafs![0]!.tfhd.trackId)).toEqual([1,2,3,4]);expect(fault.fragments[4]).toEqual({dropped:'append_fragment_limit'});
 const sent=Uint8Array.from(atob(f.ofKind('encoded_append')[0]!.data as string),c=>c.charCodeAt(0));expect(Array.from(sent)).toEqual(Array.from(bytes));
});
it('drops malformed oversized or excessive moof metadata without scanning sample arrays or changing admission',async()=>{
 const malformed=new Uint8Array(8);new DataView(malformed.buffer).setUint32(0,2000);
 const cases:Array<[Uint8Array,string]>=[
  [box('moof',malformed),'box_size_invalid'],[box('moof',new Uint8Array(65536)),'size_limit'],
  [box('moof',concat(...new Array(64).fill(null).map(()=>box('free')))),'node_limit'],
  [box('moof',concat(box('traf'),box('traf'),box('traf'))),'traf_limit'],
  [box('moof',box('traf',box('tfhd',new Uint8Array(7)))),'tfhd_truncated'],
  [box('moof',box('traf',box('trun',new Uint8Array(7)))),'trun_truncated']
 ];
 for(const [moof,dropped]of cases){const f=fixture();f.load();await f.start();f.reject.add('encoded_append');const bytes=concat(moof,box('mdat',new Uint8Array(1)));expect(f.v.appendBuffer(bytes)).toBe('native-result');await flush();expect(f.bridge().getDiagnostics().lastTransportFault).toMatchObject({fragments:[{moofBytes:moof.length,dropped}]});expect(f.ofKind('encoded_append')).toHaveLength(1);}
});
it('reports bounded init box shape without payload values',()=>{
 const f=fixture();const bytes=concat(box('ftyp',new Uint8Array(4)),box('moov',concat(box('trak',concat(box('tkhd'),box('edts',new Uint8Array([123,45,67])))),box('mvex',box('trex')))));f.v.appendBuffer(bytes);f.a.appendBuffer(bytes);
 const tracks=(f.bridge().getDiagnostics().sources as Array<{tracks:Array<{initTree:unknown}>}>)[0]!.tracks;expect(tracks[0]!.initTree).toEqual([{type:'ftyp',size:12},{type:'moov',size:51,children:[{type:'trak',size:27,children:[{type:'tkhd',size:8},{type:'edts',size:11}]},{type:'mvex',size:16,children:[{type:'trex',size:8}]}]}]);expect(Array.from(bytes)).toContain(123);
});
it('reports only bounded AAC configuration fields, not media sample bytes',()=>{
 const f=fixture();const desc=(tag:number,bytes:Uint8Array)=>concat(new Uint8Array([tag,bytes.length]),bytes);const config=new Uint8Array([0x12,0x10,...new Array(38).fill(0)]);const decoder=new Uint8Array(13);decoder[0]=0x40;decoder[1]=0x15;
 const esds=box('esds',concat(new Uint8Array(4),desc(3,concat(new Uint8Array(3),desc(4,concat(decoder,desc(5,config)))))));
 const bytes=concat(box('ftyp',new Uint8Array(4)),box('moov',box('trak',box('mdia',box('minf',box('stbl',box('stsd',concat(new Uint8Array(8),box('mp4a',concat(new Uint8Array(28),esds))))))))));f.v.appendBuffer(bytes);f.a.appendBuffer(bytes);
 const tree=(f.bridge().getDiagnostics().sources as Array<{tracks:Array<{initTree:unknown}>}>)[0]!.tracks[0]!.initTree;const find=(value:unknown):Record<string,unknown>|undefined=>{if(Array.isArray(value)){for(const child of value){const result=find(child);if(result)return result;}}else if(value&&typeof value==='object'){const object=value as Record<string,unknown>;if(object.type==='esds')return object.aac as Record<string,unknown>;return find(object.children);}return undefined;};
 expect(find(tree)).toMatchObject({esTag:3,decoderTag:4,objectType:64,streamType:5,ascTag:5,ascBytes:40,audioObjectType:2,frequencyIndex:4,channelConfig:2});expect((find(tree)!.ascConfigHex as string)).toHaveLength(64);
});
it('retains bounded unsupported FourCC facts without exposing incoming sample bytes',()=>{
 const f=fixture();f.load();const payload=box('test',new Uint8Array([1,2,3,4]));f.v.appendBuffer(payload);const result=f.bridge().getDiagnostics();expect(result.reason).toBe('container_unsupported');const value=result.sources as Array<{tracks:Array<{unsupportedBox:unknown}>}>;expect(value[0]!.tracks[0]!.unsupportedBox).toEqual({type:'test',size:12,appendBytes:12,offset:0});expect(JSON.stringify(result)).not.toContain('blob:');expect(f.bridge().canStart(f.video)).toBe(false);
});
it('still drains encoded writes before finish and respects init fragments',async()=>{
 const f=fixture();const bytes=init();f.v.appendBuffer(bytes.subarray(0,5));f.v.appendBuffer(bytes.subarray(5));f.a.appendBuffer(bytes);await f.start();f.hold.add('encoded_append');f.v.appendBuffer(media());await flush();const stopped=f.bridge().stop();await flush();expect(f.ofKind('encoded_finish')).toHaveLength(0);f.release();await stopped;expect(f.ofKind('encoded_finish')).toHaveLength(1);
});
it('accepts observed YouTube init-prefix skip padding without forwarding it or touching native input',async()=>{
 const f=fixture();const videoInit=concat(init(),box('skip',new Uint8Array(286)));const audioInit=concat(init(),box('skip',new Uint8Array(325)));expect(f.v.appendBuffer(videoInit)).toBe('native-result');f.a.appendBuffer(audioInit);expect(f.bridge().canStart(f.video)).toBe(true);
 await f.start();const original=concat(box('skip',new Uint8Array(286)),media(),box('skip',new Uint8Array(325)));const expected=Array.from(original);f.v.appendBuffer(original);await flush();expect(Array.from(original)).toEqual(expected);expect(f.ofKind('encoded_append')).toHaveLength(1);const sent=Uint8Array.from(atob(f.ofKind('encoded_append')[0]!.data as string),c=>c.charCodeAt(0));expect(Array.from(sent)).toEqual(Array.from(media()));await f.bridge().stop();
});
it('handles split skip header/body outside fragments while still refusing unknown boxes',async()=>{
 const f=fixture();f.load();const padding=box('skip',new Uint8Array(325));for(const bytes of [padding.subarray(0,3),padding.subarray(3,17),padding.subarray(17)])f.v.appendBuffer(bytes);expect(f.bridge().canStart(f.video)).toBe(true);f.v.appendBuffer(box('uuid'));expect(f.bridge().canStart(f.video)).toBe(false);expect(f.bridge().getDiagnostics().reason).toBe('container_unsupported');
});
it('rejects skip within moof/mdat rather than silently shifting sample offsets',async()=>{
 const f=fixture();f.load();await f.start();f.v.appendBuffer(concat(box('moof',new Uint8Array(4)),box('skip',new Uint8Array(8)),box('mdat',new Uint8Array(16))));await flush();expect(f.bridge().canStart(f.video)).toBe(false);expect(f.ofKind('encoded_finish')[0]).toMatchObject({interrupted:true,reason:'container_unsupported'});
});
it('drops only complete out-of-fragment padding across adjacent fragments and unfinished appends',async()=>{
 const f=fixture();f.load();await f.start();const padding=box('skip',new Uint8Array(325));f.v.appendBuffer(concat(padding,media(),padding,media(),padding.subarray(0,17)));f.v.appendBuffer(padding.subarray(17));await flush();const sent=f.ofKind('encoded_append').map(message=>Uint8Array.from(atob(message.data as string),c=>c.charCodeAt(0)));expect(Array.from(concat(...sent))).toEqual(Array.from(concat(media(),media())));expect(f.bridge().canStart(f.video)).toBe(true);await f.bridge().stop();
});

