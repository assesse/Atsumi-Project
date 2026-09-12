/** Development-only, generated-tone AAC fallback for the isolated encoded probe.
 * No microphone, user media, URL, network, profile, storage or app recording API.
 * This uses the browser's real AAC encoder, then copies its actual AAC samples.
 * Recording chunks are joined before parsing, as individual timeslice Blobs need
 * not be independently playable: https://w3c.github.io/mediacapture-record/
 * Fragment addressing: https://www.w3.org/TR/mse-byte-stream-format-isobmff/
 */
export type ProbeAacSample = { bytes: Uint8Array; timestamp: number; key: boolean };
export type ProbeAacResult = { audio: ProbeAacSample[]; asc: Uint8Array };

const MAX_BYTES = 8 * 1024 * 1024;
const MAX_SAMPLES = 2048;
type Box = { type: string; start: number; data: number; end: number };
const invalid = (detail: string): never => { throw new Error(`Synthetic MediaRecorder AAC: ${detail}`); };

/** Only this probe's in-memory, bounded audio-only fragmented MP4 is accepted. */
function extractAac(bytes: Uint8Array, sampleRate: number): ProbeAacResult {
  if (!bytes.length || bytes.length > MAX_BYTES) invalid("invalid MP4 size");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u32 = (at: number, end = bytes.length): number => {
    if (!Number.isSafeInteger(at) || at < 0 || at + 4 > end) return invalid("truncated MP4 integer");
    return view.getUint32(at);
  };
  const u64 = (at: number, end: number): number => {
    const value = u32(at, end) * 4294967296 + u32(at + 4, end);
    if (!Number.isSafeInteger(value)) return invalid("MP4 integer exceeds safe range");
    return value;
  };
  const boxes = (start: number, end: number): Box[] => {
    const result: Box[] = [];
    for (let at = start; at < end;) {
      if (result.length >= 4096 || at + 8 > end) invalid("too many or truncated MP4 boxes");
      const shortSize = u32(at, end);
      const type = String.fromCharCode(...bytes.subarray(at + 4, at + 8));
      const header = shortSize === 1 ? 16 : 8;
      const size = shortSize === 1 ? u64(at + 8, end) : shortSize === 0 ? end - at : shortSize;
      if (size < header || size > MAX_BYTES || at + size > end) invalid("invalid MP4 box extent");
      if (shortSize === 0 && type !== "mdat") invalid("unsupported open-ended MP4 box");
      result.push({ type, start: at, data: at + header, end: at + size });
      at += size;
    }
    return result;
  };
  const one = (list: Box[], type: string): Box => {
    const selected = list.filter((box) => box.type === type);
    if (selected.length !== 1) return invalid(`expected one ${type} box`);
    return selected[0]!;
  };
  const child = (box: Box, type: string) => one(boxes(box.data, box.end), type);
  const full = (box: Box): { version: number; flags: number } => {
    const value = u32(box.data, box.end);
    return { version: value >>> 24, flags: value & 0xffffff };
  };
  const top = boxes(0, bytes.length);
  one(top, "ftyp");
  const movie = one(top, "moov");
  const track = child(movie, "trak");
  const tkhd = child(track, "tkhd"), tkhdVersion = full(tkhd).version;
  if (tkhdVersion > 1) invalid("unsupported tkhd version");
  const trackId = u32(tkhd.data + (tkhdVersion === 1 ? 20 : 12), tkhd.end);
  const media = child(track, "mdia");
  const handler = child(media, "hdlr");
  if (String.fromCharCode(...bytes.subarray(handler.data + 8, handler.data + 12)) !== "soun") invalid("MP4 is not audio-only");
  const mdhd = child(media, "mdhd"), mdhdVersion = full(mdhd).version;
  if (mdhdVersion > 1) invalid("unsupported mdhd version");
  const timescale = u32(mdhd.data + (mdhdVersion === 1 ? 20 : 12), mdhd.end);
  if (!timescale || timescale > 1_000_000_000) invalid("invalid audio timescale");
  const stbl = child(child(media, "minf"), "stbl");
  const stsd = child(stbl, "stsd");
  if (full(stsd).version !== 0 || u32(stsd.data + 4, stsd.end) !== 1) invalid("multiple audio sample descriptions");
  const entry = one(boxes(stsd.data + 8, stsd.end), "mp4a");
  if (entry.end - entry.data < 28 || u32(entry.data + 8, entry.end) !== 0) invalid("unsupported MP4 audio sample entry");
  const esds = one(boxes(entry.data + 28, entry.end), "esds");
  if (full(esds).version !== 0) invalid("unsupported AAC descriptor version");
  const descriptor = (at: number, end: number): { tag: number; data: number; end: number } => {
    if (at >= end) return invalid("missing AAC descriptor");
    const tag = bytes[at++]!;
    let length = 0, complete = false;
    for (let index = 0; index < 4; index++) {
      if (at >= end) invalid("truncated AAC descriptor length");
      const byte = bytes[at++]!; length = length * 128 + (byte & 127);
      if (!(byte & 128)) { complete = true; break; }
    }
    if (!complete || length > 4096 || at + length > end) return invalid("invalid AAC descriptor length");
    return { tag, data: at, end: at + length };
  };
  const es = descriptor(esds.data + 4, esds.end);
  if (es.tag !== 3 || es.end - es.data < 3 || (bytes[es.data + 2]! & 0xe0)) invalid("unsupported AAC ES descriptor");
  const decoder = descriptor(es.data + 3, es.end);
  if (decoder.tag !== 4 || decoder.end - decoder.data < 13 || bytes[decoder.data] !== 0x40 || ((bytes[decoder.data + 1]! >>> 2) & 63) !== 5) invalid("not MPEG-4 AAC audio");
  const specific = descriptor(decoder.data + 13, decoder.end);
  if (specific.tag !== 5 || specific.end - specific.data < 2 || specific.end - specific.data > 8) invalid("missing AudioSpecificConfig");
  const asc = bytes.slice(specific.data, specific.end);
  const rates = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
  if ((asc[0]! >>> 3) !== 2 || rates[((asc[0]! & 7) << 1) | (asc[1]! >>> 7)] !== sampleRate || ((asc[1]! >>> 3) & 15) !== 1 || (asc[1]! & 4)) invalid("expected AAC-LC mono with 1024-sample frames at requested rate");
  const trex = child(child(movie, "mvex"), "trex");
  if (full(trex).version !== 0 || u32(trex.data + 4, trex.end) !== trackId || u32(trex.data + 8, trex.end) !== 1) invalid("invalid AAC track defaults");
  const defaultDuration = u32(trex.data + 12, trex.end), defaultSize = u32(trex.data + 16, trex.end);
  const mdats = top.filter((box) => box.type === "mdat");
  const occupied: Array<{ start: number; end: number }> = [];
  const audio: ProbeAacSample[] = [];
  let firstDts: number | undefined, nextDts: number | undefined;
  for (const moof of top.filter((box) => box.type === "moof")) {
    const traf = child(moof, "traf");
    const children = boxes(traf.data, traf.end);
    if (children.some((box) => !["tfhd", "tfdt", "trun", "sdtp"].includes(box.type))) invalid("unsupported or encrypted AAC fragment metadata");
    const tfhd = one(children, "tfhd"), header = full(tfhd);
    if (header.version !== 0 || (header.flags & ~0x02003b) || u32(tfhd.data + 4, tfhd.end) !== trackId) invalid("unsupported AAC fragment header");
    let at = tfhd.data + 8;
    let base = moof.start;
    if (header.flags & 1) { base = u64(at, tfhd.end); at += 8; }
    if (header.flags & 2) { if (u32(at, tfhd.end) !== 1) invalid("AAC sample description changed"); at += 4; }
    let duration = defaultDuration, size = defaultSize;
    if (header.flags & 8) { duration = u32(at, tfhd.end); at += 4; }
    if (header.flags & 16) { size = u32(at, tfhd.end); at += 4; }
    if (header.flags & 32) at += 4;
    if (at !== tfhd.end) invalid("invalid AAC fragment header length");
    const tfdt = one(children, "tfdt"), clock = full(tfdt);
    if (clock.version > 1 || clock.flags) invalid("unsupported AAC decode clock");
    let dts = clock.version === 1 ? u64(tfdt.data + 4, tfdt.end) : u32(tfdt.data + 4, tfdt.end);
    if (nextDts !== undefined && dts !== nextDts) invalid("AAC decode clock contains a gap or overlap");
    firstDts ??= dts;
    let position: number | undefined;
    const runs = children.filter((box) => box.type === "trun");
    if (!runs.length) invalid("AAC sample run is missing");
    for (const run of runs) {
      const flags = full(run);
      if (flags.version > 1 || (flags.flags & ~0xf05)) invalid("unsupported AAC sample run");
      const count = u32(run.data + 4, run.end);
      if (!count || audio.length + count > MAX_SAMPLES) invalid("AAC sample count exceeds probe bound");
      at = run.data + 8;
      if (flags.flags & 1) { const offset = u32(at, run.end) | 0; at += 4; position = base + offset; }
      if (position === undefined) invalid("AAC first run has no explicit data offset");
      if (flags.flags & 4) at += 4;
      for (let index = 0; index < count; index++) {
        let sampleDuration = duration, sampleSize = size;
        if (flags.flags & 0x100) { sampleDuration = u32(at, run.end); at += 4; }
        if (flags.flags & 0x200) { sampleSize = u32(at, run.end); at += 4; }
        if (flags.flags & 0x400) at += 4;
        if (flags.flags & 0x800) { if (u32(at, run.end) !== 0) invalid("AAC composition offset is nonzero"); at += 4; }
        // MediaRecorder adjusts container durations at timeslice boundaries
        // (observed 1008/1023 ticks at 48 kHz). This is a fixture constructor,
        // not production capture: ASC proves 1024-sample AAC access units, and
        // the caller assigns those unmodified units their exact codec clock.
        // Still reject missing or grossly inconsistent input timing.
        if (!sampleDuration || sampleDuration * sampleRate > 2048 * timescale)
          invalid(`invalid AAC container duration=${sampleDuration}, timescale=${timescale}`);
        if (!sampleSize || sampleSize > 65536 || !Number.isSafeInteger(position) || !mdats.some((mdat) => position! >= mdat.data && position! + sampleSize <= mdat.end)) invalid("AAC sample points outside its mdat");
        const end = position! + sampleSize;
        if (occupied.some((range) => position! < range.end && end > range.start)) invalid("AAC sample payload overlaps another sample");
        occupied.push({ start: position!, end });
        audio.push({ bytes: bytes.slice(position, end), timestamp: Math.round((dts - firstDts) * 1000000 / timescale), key: true });
        position = end; dts += sampleDuration;
        if (!Number.isSafeInteger(dts)) invalid("AAC decode clock exceeds safe range");
      }
      if (at !== run.end) invalid("AAC sample run length does not match its samples");
    }
    nextDts = dts;
  }
  if (!audio.length) invalid("MediaRecorder did not produce fragmented AAC MP4");
  return { audio, asc };
}

export async function encodeAacViaMediaRecorder(seconds: number, sampleRate = 48000): Promise<ProbeAacResult> {
  if (!Number.isFinite(seconds) || seconds < 1 || seconds > 30 || sampleRate !== 48000) invalid("probe accepts only 1–30 seconds of 48 kHz audio");
  if (typeof MediaRecorder === "undefined" || typeof AudioContext === "undefined") invalid("MediaRecorder or WebAudio is unavailable");
  const mimeType = ['audio/mp4;codecs="mp4a.40.2"', "audio/mp4;codecs=mp4a.40.2"].find((mime) => MediaRecorder.isTypeSupported(mime));
  if (!mimeType) invalid("this WebView cannot encode MediaRecorder AAC-LC MP4");
  const context = new AudioContext({ sampleRate });
  const destination = context.createMediaStreamDestination();
  destination.channelCount = 1; destination.channelCountMode = "explicit";
  const oscillator = context.createOscillator(), gain = context.createGain();
  oscillator.frequency.value = 440; oscillator.type = "sine"; gain.gain.value = 0.2;
  oscillator.connect(gain); gain.connect(destination);
  let recorder: MediaRecorder | undefined;
  let stopTimer: ReturnType<typeof setTimeout> | undefined;
  let deadline: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([context.resume(), new Promise<never>((_, reject) => {
      deadline = setTimeout(() => reject(new Error("Synthetic AAC AudioContext resume timed out")), 3000);
    })]);
    clearTimeout(deadline); deadline = undefined;
    if (context.state !== "running" || context.sampleRate !== sampleRate || destination.stream.getAudioTracks().length !== 1) invalid("synthetic audio context did not start at the requested rate");
    recorder = new MediaRecorder(destination.stream, { mimeType, audioBitsPerSecond: 64000 });
    const chunks: Blob[] = [];
    const completed = new Promise<Uint8Array>((resolve, reject) => {
      let size = 0, settled = false;
      const fail = (error: Error) => { if (!settled) { settled = true; reject(error); } };
      recorder!.ondataavailable = (event) => {
        if (settled || !event.data.size) return;
        size += event.data.size;
        if (size > MAX_BYTES || chunks.length >= 64) { fail(new Error("Synthetic AAC recording exceeded its byte/chunk limit")); return; }
        chunks.push(event.data);
      };
      recorder!.onerror = () => fail(new Error("Synthetic MediaRecorder AAC encoder failed"));
      recorder!.onstop = () => {
        if (settled) return;
        settled = true;
        if (!size) { reject(new Error("Synthetic AAC recorder returned no data")); return; }
        void new Blob(chunks, { type: mimeType }).arrayBuffer().then((buffer) => resolve(new Uint8Array(buffer)), reject);
      };
      deadline = setTimeout(() => fail(new Error("Synthetic AAC recorder exceeded its completion deadline")), seconds * 1000 + 6000);
      recorder!.start(1000);
      oscillator.start();
      stopTimer = setTimeout(() => { if (recorder?.state !== "inactive") recorder?.stop(); }, seconds * 1000 + 100);
    });
    const result = extractAac(await completed, sampleRate);
    // A small recorder/encoder delay is permitted; do not fabricate, repeat or
    // trim AAC access units to obtain a particular sample count.
    const duration = result.audio.length * 1024 / sampleRate;
    if (duration < seconds - 0.1 || duration > seconds + 2) invalid("AAC duration falls outside the bounded synthetic capture interval");
    return result;
  } finally {
    clearTimeout(stopTimer); clearTimeout(deadline);
    if (recorder && recorder.state !== "inactive") { try { recorder.stop(); } catch { /* Synthetic resource cleanup only. */ } }
    if (recorder) recorder.ondataavailable = recorder.onstop = recorder.onerror = null;
    try { oscillator.stop(); } catch { /* May not have started if capability failed. */ }
    oscillator.disconnect(); gain.disconnect();
    for (const track of destination.stream.getTracks()) track.stop();
    await context.close().catch(() => {});
  }
}
