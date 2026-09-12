/** Development-only synthetic encoder fixture. Never imported by the app.
 * No fetch, media URL, camera/microphone, account, recording API or filesystem.
 * The isolated native harness explicitly calls __atsumiEncodedProbeRun().
 * Bundle as browser IIFE, defining __USE_M2TS_ADVANCED_CODECS__=false.
 * Raw formats: https://www.w3.org/TR/webcodecs-avc-codec-registration/
 *             https://www.w3.org/TR/webcodecs-aac-codec-registration/
 */
import MP4, { type TrackFragmentSample } from "../node_modules/hls.js/src/remux/mp4-generator";
import type { DemuxedAAC, DemuxedAVC1 } from "../node_modules/hls.js/src/types/demuxer";
import { encodeAacViaMediaRecorder } from "./chzzk-aac-probe";

type Chunk = { byteLength: number; timestamp: number; duration?: number | null; type: string; copyTo(destination: Uint8Array): void };
type Description = ArrayBuffer | ArrayBufferView;
type CodecMetadata = { decoderConfig?: { codec?: string; description?: Description } };
type Encoder = EventTarget & { encodeQueueSize: number; state: string; configure(config: object): void; encode(data: unknown, options?: object): void; flush(): Promise<void>; close(): void };
type EncoderClass = { new(init: { output(chunk: Chunk, metadata?: CodecMetadata): void; error(error: DOMException): void }): Encoder; isConfigSupported(config: object): Promise<{ supported?: boolean }> };
type AudioFrame = { close(): void };
type ProbeWindow = Window & {
  AudioEncoder?: EncoderClass;
  AudioData?: { new(init: { format: string; sampleRate: number; numberOfFrames: number; numberOfChannels: number; timestamp: number; data: Float32Array }): AudioFrame };
  chrome?: { webview?: { postMessage(message: string): void } };
  __atsumiEncodedProbeRun?: () => Promise<void>;
};
type Encoded = { bytes: Uint8Array; timestamp: number; key: boolean };
type ProbeMessage = { kind: "init" | "append" | "done" | "error"; trackIndex: 0 | 1; mimeType?: string; data?: string; error?: string };
const probeWindow = window as ProbeWindow;
const seconds = 16, fps = 30, sampleRate = 48000;
const prefix = "ATSUMI_ENCODED_PROBE:";
const join = (parts: Uint8Array[]) => { const output = new Uint8Array(parts.reduce((size, part) => size + part.byteLength, 0)); let offset = 0; for (const part of parts) { output.set(part, offset); offset += part.byteLength; } return output; };
const descriptionBytes = (value: Description) => value instanceof ArrayBuffer ? new Uint8Array(value.slice(0)) : new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 16384) binary += String.fromCharCode(...bytes.subarray(offset, offset + 16384));
  return btoa(binary);
}
function send(message: ProbeMessage): Promise<void> {
  const bridge = probeWindow.chrome?.webview;
  if (!bridge) return Promise.reject(new Error("Isolated native probe bridge is missing"));
  const id = crypto.randomUUID();
  return new Promise((resolve, reject) => {
    const cleanup = () => { clearTimeout(timer); window.removeEventListener("atsumi-encoded-probe-reply", reply); };
    const reply = (event: Event) => {
      const value = (event as CustomEvent<{ id?: string; ok?: boolean }>).detail;
      if (value?.id !== id) return;
      cleanup(); value.ok === true ? resolve() : reject(new Error("Native fixture ACK rejected"));
    };
    const timer = setTimeout(() => { cleanup(); reject(new Error("Native fixture ACK timed out")); }, 10000);
    window.addEventListener("atsumi-encoded-probe-reply", reply);
    try { bridge.postMessage(prefix + JSON.stringify({ id, ...message })); } catch (error) { cleanup(); reject(error); }
  });
}
async function drain(encoder: Encoder): Promise<void> {
  if (encoder.encodeQueueSize <= 16) return;
  await new Promise<void>((resolve, reject) => {
    const cleanup = () => { clearTimeout(timer); encoder.removeEventListener("dequeue", check); };
    const check = () => { if (encoder.encodeQueueSize <= 16) { cleanup(); resolve(); } };
    const timer = setTimeout(() => { cleanup(); reject(new Error("Synthetic encoder queue stalled")); }, 10000);
    encoder.addEventListener("dequeue", check); check();
  });
}
function avcParameters(config: Uint8Array): { sps: Uint8Array[]; pps: Uint8Array[] } {
  if (config.length < 7 || config[0] !== 1 || (config[4]! & 3) !== 3) throw new Error("AVCC configuration with four-byte NAL lengths is required");
  let offset = 6;
  const read = (count: number) => {
    const values: Uint8Array[] = [];
    for (let index = 0; index < count; index++) {
      if (offset + 2 > config.length) throw new Error("Truncated AVC parameter length");
      const size = config[offset]! * 256 + config[offset + 1]!; offset += 2;
      if (!size || offset + size > config.length) throw new Error("Truncated AVC parameter data");
      values.push(config.slice(offset, offset + size)); offset += size;
    }
    return values;
  };
  const sps = read(config[5]! & 31);
  if (offset >= config.length) throw new Error("AVCC PPS count is missing");
  const pps = read(config[offset++]!);
  if (!sps.length || !pps.length) throw new Error("AVCC SPS/PPS are missing");
  return { sps, pps };
}

async function encodeSynthetic(): Promise<{ video: Encoded[]; audio: Encoded[]; avcc: Uint8Array; asc: Uint8Array }> {
  const Video = globalThis.VideoEncoder as unknown as EncoderClass | undefined;
  const Audio = probeWindow.AudioEncoder, AudioDataClass = probeWindow.AudioData;
  if (!Video || typeof VideoFrame === "undefined") throw new Error("This WebView does not expose a video WebCodecs encoder");
  const videoConfig = { codec: "avc1.42001e", width: 320, height: 180, framerate: fps, bitrate: 300000, latencyMode: "realtime", avc: { format: "avc" } };
  const audioConfig = { codec: "mp4a.40.2", sampleRate, numberOfChannels: 1, bitrate: 64000, aac: { format: "aac" } };
  if (!(await Video.isConfigSupported(videoConfig)).supported) throw new Error("AVC baseline encoding is not supported in this WebView");
  const audioWebCodecs = Boolean(Audio && AudioDataClass && (await Audio.isConfigSupported(audioConfig)).supported);
  const video: Encoded[] = [], audio: Encoded[] = [];
  let avcc: Uint8Array | undefined, asc: Uint8Array | undefined, totalBytes = 0, failure: Error | undefined;
  const output = (target: Encoded[], isVideo: boolean) => (chunk: Chunk, metadata?: CodecMetadata) => {
    if (failure) return;
    if (chunk.byteLength < 1 || chunk.byteLength > 2 * 1024 ** 2 || (totalBytes += chunk.byteLength) > 64 * 1024 ** 2) { failure = new Error("Synthetic encoded fixture exceeded its byte limit"); return; }
    const bytes = new Uint8Array(chunk.byteLength); chunk.copyTo(bytes);
    target.push({ bytes, timestamp: chunk.timestamp, key: chunk.type === "key" });
    if (metadata?.decoderConfig?.description) {
      const value = descriptionBytes(metadata.decoderConfig.description), prior = isVideo ? avcc : asc;
      if (prior && (prior.length !== value.length || prior.some((byte, index) => byte !== value[index]))) { failure = new Error("Encoder configuration changed within the synthetic fixture"); return; }
      if (isVideo) avcc = value; else asc = value;
    }
  };
  const fail = (error: DOMException) => { failure = new Error(`Synthetic encoder failed: ${error.name}`); };
  const videoEncoder = new Video({ output: output(video, true), error: fail });
  const audioEncoder = audioWebCodecs ? new Audio!({ output: output(audio, false), error: fail }) : null;
  try {
    videoEncoder.configure(videoConfig); audioEncoder?.configure(audioConfig);
    const canvas = document.createElement("canvas"); canvas.width = 320; canvas.height = 180;
    const context = canvas.getContext("2d", { alpha: false });
    if (!context) throw new Error("Synthetic canvas context is unavailable");
    for (let index = 0; index < seconds * fps; index++) {
      if (failure) throw failure;
      context.fillStyle = `hsl(${index * 2 % 360} 65% 30%)`; context.fillRect(0, 0, 320, 180);
      context.fillStyle = "#fff"; context.fillRect(index * 3 % 296, 30, 24, 110);
      context.font = "18px sans-serif"; context.fillText(`Synthetic ${index + 1}/480`, 10, 22);
      const timestamp = Math.round(index * 1000000 / fps);
      const frame = new VideoFrame(canvas, { timestamp, duration: Math.round((index + 1) * 1000000 / fps) - timestamp });
      try { videoEncoder.encode(frame, { keyFrame: index % (fps * 2) === 0 }); } finally { frame.close(); }
      await drain(videoEncoder);
    }
    for (let offset = 0; audioEncoder && offset < seconds * sampleRate; offset += 1024) {
      if (failure) throw failure;
      const data = new Float32Array(1024);
      for (let index = 0; index < data.length; index++) data[index] = Math.sin(2 * Math.PI * 440 * (offset + index) / sampleRate) * 0.2;
      const frame = new AudioDataClass!({ format: "f32", sampleRate, numberOfChannels: 1, numberOfFrames: data.length, timestamp: Math.round(offset * 1000000 / sampleRate), data });
      try { audioEncoder.encode(frame); } finally { frame.close(); }
      await drain(audioEncoder);
    }
    await Promise.all([videoEncoder.flush(), audioEncoder?.flush()]);
    if (!audioEncoder) {
      const fallback = await encodeAacViaMediaRecorder(seconds, sampleRate);
      audio.push(...fallback.audio); asc = fallback.asc;
    }
    if (failure) throw failure;
    if (video.length !== seconds * fps || audio.length < 748 || audio.length > 800 || !avcc || !asc) throw new Error("Synthetic encoder output or codec configuration is incomplete");
    if (video.some((sample, index) => Math.abs(sample.timestamp - Math.round(index * 1000000 / fps)) > 1) || !video[0]?.key) throw new Error("Synthetic video is not a zero-based no-B-frame sequence");
    if (asc.length < 2 || (asc[0]! >> 3) !== 2 || (((asc[0]! & 7) << 1) | (asc[1]! >> 7)) !== 3 || ((asc[1]! >> 3) & 15) !== 1) throw new Error("Expected AAC-LC 48 kHz mono AudioSpecificConfig");
    return { video, audio, avcc, asc };
  } finally {
    if (videoEncoder.state !== "closed") videoEncoder.close();
    if (audioEncoder && audioEncoder.state !== "closed") audioEncoder.close();
  }
}

function fragments(samples: Encoded[], type: "video" | "audio", id: number, perFragment: number, duration: number, timescale: number) {
  const result: { trackIndex: 0 | 1; time: number; data: Uint8Array }[] = [];
  for (let start = 0, sequence = 1; start < samples.length; start += perFragment, sequence++) {
    const group = samples.slice(start, start + perFragment);
    const fragmentSamples: TrackFragmentSample[] = group.map((sample) => ({ size: sample.bytes.length, duration, cts: 0,
      flags: { isLeading: 0, dependsOn: sample.key ? 2 : 1, isDependedOn: 0, hasRedundancy: 0, paddingValue: 0, isNonSync: sample.key ? 0 : 1, degradPrio: 0 } }));
    const moof = MP4.moof(sequence, start * duration, { type, id, samples: fragmentSamples });
    result.push({ trackIndex: type === "video" ? 0 : 1, time: start * duration / timescale, data: join([moof, MP4.mdat(join(group.map((sample) => sample.bytes)))]) });
  }
  return result;
}

let started = false;
probeWindow.__atsumiEncodedProbeRun = async () => {
  if (started) return;
  started = true;
  try {
    const encoded = await encodeSynthetic();
    const parameters = avcParameters(encoded.avcc);
    if (encoded.avcc[1] !== 0x42 || encoded.video.some((sample, index) => index % (fps * 2) === 0 && !sample.key)) throw new Error("Expected baseline AVC with a random access frame every two seconds");
    const codec = `avc1.${[...encoded.avcc.subarray(1, 4)].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
    const common = { pid: -1, inputTimeScale: 90000, sequenceNumber: 0, dropped: 0 };
    const videoTrack: DemuxedAVC1 = { ...common, type: "video", id: 1, timescale: 90000, duration: seconds, segmentCodec: "avc", codec, samples: [], width: 320, height: 180, pixelRatio: [1, 1], ...parameters };
    // AAC encoder priming/padding is retained, not edited or silently discarded.
    const audioTrack: DemuxedAAC = { ...common, type: "audio", id: 2, inputTimeScale: sampleRate, timescale: sampleRate, duration: encoded.audio.length * 1024 / sampleRate, segmentCodec: "aac", codec: "mp4a.40.2", config: encoded.asc, samples: [], samplerate: sampleRate, channelCount: 1 };
    const videoInit = MP4.initSegment([videoTrack]), audioInit = MP4.initSegment([audioTrack]);
    if (videoInit.length > 65536 || audioInit.length > 65536) throw new Error("Synthetic initialization exceeds the probe init limit");
    await send({ kind: "init", trackIndex: 0, mimeType: `video/mp4; codecs="${codec}"`, data: base64(videoInit) });
    await send({ kind: "init", trackIndex: 1, mimeType: 'audio/mp4; codecs="mp4a.40.2"', data: base64(audioInit) });
    const media = [
      ...fragments(encoded.video, "video", 1, fps * 2, 3000, 90000),
      ...fragments(encoded.audio, "audio", 2, 94, 1024, sampleRate),
    ].sort((left, right) => left.time - right.time || left.trackIndex - right.trackIndex);
    for (const fragment of media) {
      // Complete moof+mdat when possible; the native fixture reassembles splits.
      for (let offset = 0; offset < fragment.data.length; offset += 128 * 1024)
        await send({ kind: "append", trackIndex: fragment.trackIndex, data: base64(fragment.data.subarray(offset, offset + 128 * 1024)) });
    }
    await send({ kind: "done", trackIndex: 0 });
  } catch (error) {
    const message = (error instanceof Error ? error.message : String(error)).replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 220);
    try { await send({ kind: "error", trackIndex: 0, error: message }); } catch { /* The harness may already have terminated after a failed ACK. */ }
  }
};
