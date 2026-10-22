// Isolated public QA observer only. No production import, payload storage,
// request interception, codec selection or playback changes.
(() => {
  'use strict';
  const id = window.__atsumiYouTubeWebmShapeProbeId;
  const canonical = () => window.top === window && location.origin === 'https://www.youtube.com' &&
    location.pathname === '/watch' && new URLSearchParams(location.search).getAll('v').length === 1 && new URLSearchParams(location.search).get('v') === id;
  if (typeof id !== 'string' || !/^[\w-]{11}$/.test(id) || !canonical() || window.__atsumiYouTubeWebmShapeProbe) return;
  const LIMIT = { bytes: 32 * 1024 * 1024, total: 64 * 1024 * 1024, append: 16 * 1024 * 1024, scratch: 65536, appends: 128, elements: 16384, perAppend: 4096 };
  const numeric = 'timestampScale trackNumber defaultDurationNs width height trackScale codecDelayNs flagLacing vp9Profile vp9Width vp9Height vp9BitDepth vp9ColorSpace vp9FullRange firstPtsSeconds lastPtsSeconds minCadenceSeconds maxCadenceSeconds audioTimescale firstDtsSeconds lastEndSeconds'.split(' ');
  const counts = 'ebmlHeaders segments segmentUnknown infoCount tracksCount trackEntries videoTracks codecVp9 codecOther codecPrivate contentEncodings compression encryption clusters unknownClusters closedClusters simpleBlocks blockGroups blocks lacingNone lacingXiph lacingFixed lacingEbml lacedFrames referenceBlocks blockDurations containerKeys otherTrackBlocks vp9Keys vp9Inter vp9Invisible vp9ShowExisting vp9MarkerErrors vp9HeaderUnknown superframes superframeFrames superframeInvalid mp4Boxes moofs mdats audioSamples'.split(' ');
  const stats = () => Object.assign(Object.fromEntries(numeric.map(k => [k, null])), Object.fromEntries(counts.map(k => [k, 0])), { completeInit: false, partialHeader: false, openUnknownCluster: false, partialBlock: false });
  const start = performance.now();
  const sources = [], sourceMap = new WeakMap(), bufferMap = new WeakMap(), urls = new Map();
  let totalBytes = 0, stopped = false, observerError = false;
  const number = (value, signed = false) => typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= 1e12 && (signed || value >= 0) ? value : null;
  const sourceTime = value => number(value) !== null && value <= 1e9 ? value : null;
  const offsetTime = value => number(value, true) !== null && Math.abs(value) <= 1e9 ? value : null;
  const fail = reason => { throw reason; };
  const safeMime = raw => {
    if (typeof raw !== 'string' || raw.length > 160) return 'other';
    const type = raw.toLowerCase().replace(/[\s"]/g, '');
    return /^(?:video\/webm;codecs=(?:vp9|vp09(?:\.\d{2}){3,8})|audio\/mp4;codecs=mp4a\.40\.2|video\/mp4;codecs=avc[13]\.[a-f0-9]{6})$/.test(type) ? type : 'other';
  };
  const bytesOf = value => {
    if (ArrayBuffer.isView(value)) {
      if (typeof SharedArrayBuffer !== 'undefined' && value.buffer instanceof SharedArrayBuffer) return null;
      return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    }
    return value instanceof ArrayBuffer ? new Uint8Array(value) : null;
  };
  const currentVideo = () => document.querySelector('#movie_player video');
  const currentSource = video => video && !video.mediaKeys && !video.srcObject ? urls.get(video.currentSrc || video.src) ?? null : null;
  const excluded = () => Boolean(document.querySelector('#movie_player.ad-showing, #movie_player.ad-interrupting') || currentVideo()?.mediaKeys || currentVideo()?.srcObject);
  const clock = (track, begin, end) => {
    const startSeconds = sourceTime(begin), endSeconds = sourceTime(end);
    if (startSeconds === null) return;
    track.clock.push({ atMs: Math.max(0, Math.round(performance.now() - start)), startSeconds, endSeconds, offsetSeconds: offsetTime(track.buffer.timestampOffset) });
    if (track.clock.length > 8) track.clock.shift();
  };
  const vint = (bytes, at, isId = false) => {
    if (at >= bytes.length) return null;
    const first = bytes[at]; let width = 1, mask = 128;
    while (mask && !(first & mask)) { mask >>= 1; width++; }
    if (!mask || width > (isId ? 4 : 8)) fail('malformed');
    if (at + width > bytes.length) return null;
    let value = BigInt(isId ? first : first & (mask - 1));
    for (let i = 1; i < width; i++) value = (value << 8n) | BigInt(bytes[at + i]);
    return { width, value, unknown: !isId && value === (1n << BigInt(width * 7)) - 1n };
  };
  const uint = bytes => { if (!bytes.length || bytes.length > 8) fail('malformed'); let value = 0n; for (const b of bytes) value = value * 256n + BigInt(b); if (value > 1000000000000n) fail('limit'); return Number(value); };
  const sampleHeader = prefix => {
    let bit = 0;
    const bits = n => { if (bit + n > prefix.length * 8) fail('unsupported'); let value = 0; for (let i = 0; i < n; i++, bit++) value = value * 2 + ((prefix[bit >> 3] >> (7 - (bit & 7))) & 1); return value; };
    const marker = bits(2); const profile = bits(1) + bits(1) * 2;
    if (profile === 3 && bits(1)) return { invalid: true };
    if (marker !== 2) return { invalid: true };
    if (bits(1)) return { profile, existing: true, key: false, show: true };
    const key = bits(1) === 0, show = bits(1) === 1; bits(1);
    if (key && bits(24) !== 0x498342) return { invalid: true };
    const result = { profile, existing: false, key, show };
    if (key) {
      result.bitDepth = profile >= 2 ? (bits(1) ? 12 : 10) : 8;
      result.colorSpace = bits(3);
      if (result.colorSpace !== 7) { result.fullRange = bits(1); if (profile === 1 || profile === 3) { bits(1); bits(1); if (bits(1)) return { invalid: true }; } }
      else { result.fullRange = 1; if (profile !== 1 && profile !== 3 || bits(1)) return { invalid: true }; }
      result.width = bits(16) + 1; result.height = bits(16) + 1;
    }
    return result;
  };
  const blockHeader = bytes => {
    const track = vint(bytes, 0); if (!track) return null;
    if (track.unknown || track.value === 0n || track.value > 1_000_000n) fail('malformed');
    const base = track.width; if (bytes.length < base + 3) return null;
    const raw = bytes[base] * 256 + bytes[base + 1], relative = raw >= 32768 ? raw - 65536 : raw;
    const flags = bytes[base + 2], lace = (flags >> 1) & 3;
    let at = base + 3, frames = 1, sizes = [];
    if (lace) {
      if (bytes.length <= at) return null;
      frames = bytes[at++] + 1;
      if (frames < 2) fail('malformed');
      if (lace === 1) {
        for (let i = 0; i < frames - 1; i++) { let size = 0, b; do { if (at >= bytes.length) return null; b = bytes[at++]; size += b; } while (b === 255); if (!size) fail('malformed'); sizes.push(size); }
      } else if (lace === 3) {
        let item = vint(bytes, at); if (!item) return null; if (item.unknown) fail('malformed'); at += item.width;
        let size = Number(item.value); if (!Number.isSafeInteger(size) || size <= 0) fail('malformed'); sizes.push(size);
        for (let i = 1; i < frames - 1; i++) { item = vint(bytes, at); if (!item) return null; if (item.unknown) fail('malformed'); at += item.width; const delta = item.value - ((1n << BigInt(7 * item.width - 1)) - 1n); const next = BigInt(size) + delta; if (next <= 0n || next > BigInt(LIMIT.append)) fail('malformed'); size = Number(next); sizes.push(size); }
      }
    }
    return { track: Number(track.value), relative, flags, lace, frames, sizes, bytes: at };
  };
  const makeBlock = size => ({ size, headerBytes: [], header: null, prefix: new Uint8Array(32), prefixSize: 0, tail: new Uint8Array(34), tailSize: 0, payloadBytes: 0 });
  const feedBlock = (block, bytes) => {
    let at = 0;
    while (!block.header && at < bytes.length) {
      if (block.headerBytes.length >= 4096) fail('limit');
      block.headerBytes.push(bytes[at++]); block.header = blockHeader(block.headerBytes);
    }
    if (!block.header || at === bytes.length) return;
    const count = bytes.length - at, prefixCount = Math.min(32 - block.prefixSize, count);
    block.prefix.set(bytes.subarray(at, at + prefixCount), block.prefixSize); block.prefixSize += prefixCount;
    if (count >= 34) { block.tail.set(bytes.subarray(bytes.length - 34)); block.tailSize = 34; }
    else { const keep = Math.min(block.tailSize, 34 - count); block.tail.copyWithin(0, block.tailSize - keep, block.tailSize); block.tail.set(bytes.subarray(at), keep); block.tailSize = keep + count; }
    block.payloadBytes += count;
  };
  const finishBlock = block => {
    const h = block.header;
    if (!h || h.bytes + block.payloadBytes !== block.size || !block.payloadBytes) fail('malformed');
    if (h.lace === 2 && block.payloadBytes % h.frames !== 0) fail('malformed');
    if ((h.lace === 1 || h.lace === 3) && h.sizes.reduce((a, b) => a + b, 0) >= block.payloadBytes) fail('malformed');
    let header = null; if (!h.lace) { try { header = sampleHeader(block.prefix.subarray(0, block.prefixSize)); } catch { /* Partial uncompressed header is unknown. */ } }
    let superframe = null;
    if (!h.lace && block.tailSize) {
      const marker = block.tail[block.tailSize - 1];
      if ((marker & 0xe0) === 0xc0) {
        const count = (marker & 7) + 1, magnitude = ((marker >> 3) & 3) + 1, indexSize = 2 + count * magnitude;
        let sum = 0, valid = indexSize <= block.tailSize && block.tail[block.tailSize - indexSize] === marker;
        if (valid) { let at = block.tailSize - indexSize + 1; for (let i = 0; i < count; i++) { let n = 0; for (let j = 0; j < magnitude; j++) n += block.tail[at++] * 2 ** (8 * j); if (!n) valid = false; sum += n; } }
        superframe = { count, valid: valid && sum + indexSize === block.payloadBytes };
      }
    }
    return { ...h, header, superframe };
  };
  const masterLevels = new Map([[0x1a45dfa3,0],[0x18538067,0],[0x1549a966,1],[0x1654ae6b,1],[0x1f43b675,1],[0xae,2],[0xe0,3],[0x55b0,4],[0x6d80,3],[0x6240,4],[0x5034,5],[0x5035,5],[0xa0,2]]);
  const scalarIds = new Set([0x4282,0x2ad7b1,0xd7,0x83,0x86,0x23e383,0x9c,0xb0,0xba,0x23314f,0x56aa,0xe7,0x9b,0xfb]);
  class Webm {
    constructor(track) { this.track = track; this.offset = 0; this.header = []; this.stack = []; this.leaf = null; this.elements = 0; this.sawInfo = false; this.sawSegment = false; }
    entry() { return [...this.stack].reverse().find(s => s.id === 0xae); }
    cluster() { return [...this.stack].reverse().find(s => s.id === 0x1f43b675); }
    group() { return [...this.stack].reverse().find(s => s.id === 0xa0); }
    close() {
      const node = this.stack.pop(), s = this.track.stats;
      if (node.id === 0x1549a966) this.sawInfo = true;
      if (node.id === 0xae) {
        s.trackEntries++;
        if (node.fields.type === 1) {
          s.videoTracks++; if (node.fields.codec === 'vp9') s.codecVp9++; else s.codecOther++;
          if (s.videoTracks === 1) { for (const key of ['trackNumber','defaultDurationNs','width','height','flagLacing','trackScale','codecDelayNs']) s[key] = node.fields[key] ?? ({ flagLacing: 1, trackScale: 1, codecDelayNs: 0 }[key] ?? null); }
        }
      }
      if (node.id === 0x1654ae6b) s.completeInit = this.sawSegment && this.sawInfo && s.videoTracks === 1 && s.codecVp9 === 1;
      if (node.id === 0x1f43b675) s.closedClusters++;
      if (node.id === 0xa0 && node.block) this.record(node.block, node.duration, node.references === 0);
    }
    record(block, durationTicks, key) {
      const s = this.track.stats; s.blocks++; s[['lacingNone','lacingXiph','lacingFixed','lacingEbml'][block.lace]]++; s.lacedFrames += block.lace ? block.frames : 0;
      if (key) s.containerKeys++;
      if (block.track !== s.trackNumber) { s.otherTrackBlocks++; return; }
      if (block.header?.invalid) s.vp9MarkerErrors++;
      else if (block.header) { s.vp9Profile = block.header.profile; if (block.header.existing) s.vp9ShowExisting++; else if (block.header.key) { s.vp9Keys++; s.vp9Width = block.header.width; s.vp9Height = block.header.height; s.vp9BitDepth = block.header.bitDepth; s.vp9ColorSpace = block.header.colorSpace; s.vp9FullRange = block.header.fullRange; } else s.vp9Inter++; if (!block.header.show) s.vp9Invisible++; }
      else s.vp9HeaderUnknown++;
      if (block.superframe) { if (block.superframe.valid) { s.superframes++; s.superframeFrames += block.superframe.count; } else s.superframeInvalid++; }
      const cluster = this.cluster();
      if (cluster?.timestamp === null || cluster?.timestamp === undefined) fail('malformed');
      if (!s.completeInit || s.trackScale !== 1 || s.codecDelayNs !== 0 || !s.timestampScale) return;
      const pts = sourceTime((cluster.timestamp + block.relative) * s.timestampScale / 1e9);
      if (pts === null) return;
      const duration = durationTicks !== undefined ? durationTicks * s.timestampScale / 1e9 : s.defaultDurationNs === null ? null : s.defaultDurationNs * block.frames / 1e9;
      if (s.lastPtsSeconds !== null) { const cadence = pts - s.lastPtsSeconds; if (cadence >= 0) { s.minCadenceSeconds = Math.min(s.minCadenceSeconds ?? cadence, cadence); s.maxCadenceSeconds = Math.max(s.maxCadenceSeconds ?? cadence, cadence); } }
      s.firstPtsSeconds ??= pts; s.lastPtsSeconds = pts;
      clock(this.track, pts, duration === null ? null : pts + duration);
    }
    scalar(id, bytes) {
      const s = this.track.stats, entry = this.entry(), group = this.group(), cluster = this.cluster();
      if (id === 0x4282) { if (String.fromCharCode(...bytes) !== 'webm') fail('unsupported'); return; }
      if (id === 0x86) { if (entry) entry.fields.codec = String.fromCharCode(...bytes) === 'V_VP9' ? 'vp9' : 'other'; return; }
      if (id === 0xfb) { s.referenceBlocks++; if (group) group.references++; return; }
      if (id === 0x23314f) { if (entry && [4,8].includes(bytes.length)) { const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length); entry.fields.trackScale = number(bytes.length === 4 ? view.getFloat32(0) : view.getFloat64(0)); } else fail('malformed'); return; }
      const value = uint(bytes);
      if (id === 0x2ad7b1) { if (!value) fail('malformed'); s.timestampScale = value; }
      else if (id === 0xe7) { if (!cluster) fail('malformed'); cluster.timestamp = value; }
      else if (id === 0x9b) { s.blockDurations++; if (group) group.duration = value; }
      else if (entry) { const key = { 215:'trackNumber',131:'type',2352003:'defaultDurationNs',156:'flagLacing',176:'width',186:'height',22186:'codecDelayNs' }[id]; if (key) entry.fields[key] = value; }
    }
    feed(bytes) {
      let at = 0, elementsThisAppend = 0;
      const s = this.track.stats;
      while (at < bytes.length) {
        if (this.leaf) {
          const leaf = this.leaf, n = Math.min(leaf.remaining, bytes.length - at), part = bytes.subarray(at, at + n);
          if (leaf.block) feedBlock(leaf.block, part);
          else if (leaf.data) { leaf.data.set(part, leaf.written); leaf.written += n; }
          at += n; this.offset += n; leaf.remaining -= n;
          if (leaf.remaining === 0) {
            if (leaf.block) { const block = finishBlock(leaf.block); if (leaf.id === 0xa1) { const group = this.group(); if (!group || group.block) fail('malformed'); group.block = block; } else this.record(block, undefined, Boolean(block.flags & 0x80)); }
            else if (leaf.data) this.scalar(leaf.id, leaf.data);
            this.leaf = null;
          }
          continue;
        }
        if (this.header.length === 0) {
          while (this.stack.length && this.stack.at(-1).end === this.offset) this.close();
          if (this.stack.some(item => item.end !== null && this.offset > item.end)) fail('malformed');
        }
        this.header.push(bytes[at++]); this.offset++;
        const ident = vint(this.header, 0, true); if (!ident) continue;
        const length = vint(this.header, ident.width); if (!length) continue;
        if (++elementsThisAppend > LIMIT.perAppend || ++this.elements > LIMIT.elements) fail('limit');
        const id = Number(ident.value), level = masterLevels.get(id), begin = this.offset - this.header.length;
        if (length.unknown && id !== 0x18538067 && id !== 0x1f43b675) fail('unsupported');
        while (this.stack.length && this.stack.at(-1).end === null && level !== undefined && this.stack.at(-1).level >= level) this.close();
        const size = length.unknown ? null : length.value <= BigInt(Number.MAX_SAFE_INTEGER - this.offset) ? Number(length.value) : fail('limit');
        const end = size === null ? null : this.offset + size;
        if (this.stack.some(item => item.end !== null && (end === null || end > item.end || begin < item.start))) fail('malformed');
        this.header = [];
        if (level !== undefined) {
          if (this.stack.length >= 12) fail('limit');
          if (id === 0x1a45dfa3) {
            if (s.ebmlHeaders) {
              if (this.stack.length) fail('malformed');
              if (this.track.generation >= 8) fail('limit');
              this.track.generation++; Object.assign(s, stats()); this.track.clock = []; this.sawInfo = false; this.sawSegment = false;
            }
            s.ebmlHeaders++;
          }
          if (id === 0x18538067) { this.sawSegment = true; s.segments++; s.segmentUnknown += size === null ? 1 : 0; s.timestampScale ??= 1000000; }
          if (id === 0x1549a966) s.infoCount++;
          if (id === 0x1654ae6b) s.tracksCount++;
          if (id === 0x1f43b675) { if (s.clusters >= 20) fail('limit'); s.clusters++; if (size === null) s.unknownClusters++; }
          if (id === 0xa0) s.blockGroups++;
          if (id === 0x6d80) s.contentEncodings++;
          if (id === 0x5034) s.compression++;
          if (id === 0x5035) s.encryption++;
          this.stack.push({ id, level, start: this.offset, end, fields: {}, timestamp: null, references: 0 });
        } else {
          if (id === 0x63a2) s.codecPrivate++;
          const block = id === 0xa3 || id === 0xa1;
          if (block) { if (!this.cluster() || this.cluster().timestamp === null) fail('malformed'); if (id === 0xa3) s.simpleBlocks++; if (!size || size > LIMIT.append) fail('limit'); }
          const scalar = scalarIds.has(id);
          if (scalar && (!size || size > (id === 0x4282 || id === 0x86 ? 16 : 8))) fail('malformed');
          this.leaf = { id, remaining: size, block: block ? makeBlock(size) : null, data: scalar ? new Uint8Array(size) : null, written: 0 };
          if (size === 0) this.leaf = null;
        }
      }
      while (!this.leaf && !this.header.length && this.stack.length && this.stack.at(-1).end === this.offset) this.close();
    }
    snapshot() { const s = this.track.stats; s.partialHeader = this.header.length > 0; s.partialBlock = Boolean(this.leaf?.block); s.openUnknownCluster = this.stack.some(item => item.id === 0x1f43b675 && item.end === null); }
  }
  const u32 = (b, at) => { if (at < 0 || at + 4 > b.length) fail('malformed'); return new DataView(b.buffer, b.byteOffset, b.length).getUint32(at); };
  const atoms = (b, begin = 0, end = b.length) => {
    const out = []; for (let at = begin; at < end;) { if (out.length >= 4096 || end - at < 8) fail('limit'); const size = u32(b, at); if (size < 8 || size > end - at) fail('malformed'); out.push({ kind: String.fromCharCode(...b.subarray(at + 4, at + 8)), start: at + 8, end: at + size }); at += size; } return out;
  };
  class Mp4 {
    constructor(track) { this.track = track; this.header = []; this.leaf = null; this.defaults = null; this.trackId = null; this.ftyp = false; this.elements = 0; }
    parseMoov(bytes) {
      const s = this.track.stats;
      if (s.completeInit) {
        if (this.track.generation >= 8) fail('limit');
        this.track.generation++; Object.assign(s, stats()); this.track.clock = []; this.defaults = null; this.trackId = null;
      }
      const one = (items, kind) => { const found = items.filter(a => a.kind === kind); if (found.length !== 1) fail('unsupported'); return found[0]; };
      const children = a => { const result = atoms(bytes, a.start, a.end); this.elements += result.length; if (this.elements > LIMIT.elements) fail('limit'); return result; };
      const read = (a, offset) => { if (offset < 0 || a.start + offset + 4 > a.end) fail('malformed'); return u32(bytes, a.start + offset); };
      const top = one(atoms(bytes), 'moov'), moov = children(top), trak = children(one(moov, 'trak'));
      const tkhd = one(trak, 'tkhd'), tkVersion = read(tkhd, 0) >>> 24;
      if (tkVersion > 1 || tkhd.end - tkhd.start < (tkVersion ? 96 : 84)) fail('malformed');
      const id = read(tkhd, tkVersion ? 20 : 12); if (!id) fail('malformed');
      const mdhd = one(children(one(trak, 'mdia')), 'mdhd'), mdVersion = read(mdhd, 0) >>> 24;
      if (mdVersion > 1 || (read(mdhd, 0) & 0xffffff) !== 0 || mdhd.end - mdhd.start !== (mdVersion ? 36 : 24)) fail('malformed');
      const scale = read(mdhd, mdVersion ? 20 : 12); if (!scale) fail('malformed');
      const trex = one(children(one(moov, 'mvex')), 'trex');
      if (trex.end - trex.start !== 24 || read(trex, 0) !== 0 || read(trex, 4) !== id || read(trex, 8) !== 1) fail('malformed');
      this.defaults = read(trex, 12); this.trackId = id; s.audioTimescale = scale; s.completeInit = this.ftyp;
    }
    parseMoof(bytes) {
      const s = this.track.stats; let decode = null, duration = 0, samples = 0, defaultDuration = this.defaults, known = true;
      const traf = atoms(bytes, 8).filter(a => a.kind === 'traf'); if (traf.length !== 1) fail('unsupported');
      const children = atoms(bytes, traf[0].start, traf[0].end);
      const one = kind => { const found = children.filter(a => a.kind === kind); if (found.length !== 1) fail('malformed'); return found[0]; };
      const read = (a, at) => { if (at < a.start || at + 4 > a.end) fail('malformed'); return u32(bytes, at); };
      const tfhd = one('tfhd'), tfdt = one('tfdt'), tfhdWord = read(tfhd, tfhd.start), flags = tfhdWord & 0xffffff;
      if (tfhdWord >>> 24 !== 0 || flags & ~0x03003b || read(tfhd, tfhd.start + 4) !== this.trackId) fail('malformed');
      let at = tfhd.start + 8;
      if (flags & 1) at += 8;
      if (flags & 2) { if (read(tfhd, at) !== 1) fail('unsupported'); at += 4; }
      if (flags & 8) { defaultDuration = read(tfhd, at); at += 4; }
      if (flags & 16) at += 4; if (flags & 32) at += 4;
      if (at !== tfhd.end) fail('malformed');
      const tfdtWord = read(tfdt, tfdt.start), version = tfdtWord >>> 24;
      if (version > 1 || (tfdtWord & 0xffffff) !== 0 || tfdt.end - tfdt.start !== (version ? 12 : 8)) fail('malformed');
      decode = version ? uint(bytes.subarray(tfdt.start + 4, tfdt.end)) : read(tfdt, tfdt.start + 4);
      for (const a of children) {
        if (a.kind === 'trun') {
          const word = read(a, a.start), flags = word & 0xffffff, count = read(a, a.start + 4);
          if (word >>> 24 > 1 || flags & ~0xf05 || (flags & 4 && flags & 0x400)) fail('malformed');
          if (count > 4096) fail('limit'); let at = a.start + 8; if (flags & 1) at += 4; if (flags & 4) at += 4;
          for (let i = 0; i < count; i++) { const d = flags & 0x100 ? read(a, at) : defaultDuration; if (flags & 0x100) at += 4; if (!d) known = false; else duration += d; if (duration > 1e12) fail('limit'); if (flags & 0x200) at += 4; if (flags & 0x400) at += 4; if (flags & 0x800) { if (read(a, at) !== 0) known = false; at += 4; } if (at > a.end) fail('malformed'); } if (at !== a.end) fail('malformed'); samples += count;
        }
      }
      s.audioSamples += samples;
      if (known && samples && decode !== null && s.audioTimescale) { const begin = decode / s.audioTimescale, end = (decode + duration) / s.audioTimescale; s.firstDtsSeconds ??= sourceTime(begin); s.lastEndSeconds = sourceTime(end); clock(this.track, begin, end); }
    }
    feed(bytes) {
      let at = 0, boxes = 0;
      while (at < bytes.length) {
        if (!this.leaf) {
          const n = Math.min(8 - this.header.length, bytes.length - at); for (let i = 0; i < n; i++) this.header.push(bytes[at++]); if (this.header.length < 8) break;
          if (++boxes > LIMIT.perAppend || ++this.elements > LIMIT.elements) fail('limit');
          const header = Uint8Array.from(this.header), size = u32(header, 0), kind = String.fromCharCode(...header.subarray(4)); this.header = [];
          if (size < 8 || size > LIMIT.append) fail('unsupported');
          const retain = kind === 'moov' || kind === 'moof'; if (retain && size > LIMIT.scratch) fail('limit');
          this.leaf = { kind, remaining: size - 8, data: retain ? new Uint8Array(size) : null, written: 8 };
          if (retain) this.leaf.data.set(header);
          this.track.stats.mp4Boxes++;
          if (kind === 'ftyp') this.ftyp = true;
          if (kind === 'mdat') this.track.stats.mdats++;
        }
        const leaf = this.leaf, n = Math.min(leaf.remaining, bytes.length - at);
        if (leaf.data) leaf.data.set(bytes.subarray(at, at + n), leaf.written);
        leaf.written += n; leaf.remaining -= n; at += n;
        if (!leaf.remaining) { if (leaf.kind === 'moov') this.parseMoov(leaf.data); if (leaf.kind === 'moof') { this.track.stats.moofs++; this.parseMoof(leaf.data); } this.leaf = null; }
      }
    }
    snapshot() { this.track.stats.partialHeader = this.header.length > 0; this.track.stats.partialBlock = Boolean(this.leaf && this.leaf.kind !== 'mdat'); }
  }
  const reset = track => { track.stats = stats(); track.clock = []; track.parser = track.kind === 'webm' ? new Webm(track) : track.kind === 'mp4' && track.mime.startsWith('audio/') ? new Mp4(track) : null; track.reason = track.parser ? 'observing' : 'unsupported'; };
  const getSource = mediaSource => {
    let source = sourceMap.get(mediaSource); if (source) return source;
    if (sources.length >= 8) return null;
    source = { source: sources.length + 1, object: mediaSource, tracks: [], closed: false }; sources.push(source); sourceMap.set(mediaSource, source);
    mediaSource.addEventListener('sourceclose', () => { source.closed = true; for (const track of source.tracks) { track.reason = 'closed'; track.parser = null; } for (const [url, value] of urls) if (value === source) urls.delete(url); }); return source;
  };
  const stop = () => { stopped = true; for (const source of sources) for (const track of source.tracks) if (track.reason === 'observing') { track.parser?.snapshot(); track.reason = 'done'; track.parser = null; } };
  const trackAppend = (buffer, value) => {
    if (stopped || !canonical() || performance.now() - start >= 55000) { stop(); return; }
    const track = bufferMap.get(buffer); if (!track || track.detached || track.source.closed || track.reason !== 'observing') return;
    const bytes = bytesOf(value); if (!bytes) { track.reason = 'unsupported'; track.parser = null; return; }
    if (excluded()) { track.reason = 'excluded'; track.parser = null; return; }
    if (track.appends >= LIMIT.appends || bytes.length > LIMIT.append || track.bytes + bytes.length > LIMIT.bytes || totalBytes + bytes.length > LIMIT.total) { track.reason = 'limit'; track.parser = null; return; }
    track.appends++; track.bytes += bytes.length; totalBytes += bytes.length;
    const offset = offsetTime(buffer.timestampOffset); if (track.lastOffset !== undefined && offset !== track.lastOffset) track.offsetChanged = true; track.lastOffset = offset;
    try { track.parser?.feed(bytes); track.parser?.snapshot(); }
    catch (reason) { track.parser?.snapshot(); track.reason = ['limit','malformed','unsupported','source_changed'].includes(reason) ? reason : 'observer_error'; track.parser = null; }
  };
  try {
    const add = MediaSource.prototype.addSourceBuffer, append = SourceBuffer.prototype.appendBuffer;
    MediaSource.prototype.addSourceBuffer = function(...args) { const result = Reflect.apply(add, this, args); try { const source = getSource(this); if (source && source.tracks.length < 2) { const mime = safeMime(args[0]); const track = { source, buffer: result, track: source.tracks.length + 1, generation: 1, mime, kind: mime.startsWith('video/webm;') ? 'webm' : mime.includes('/mp4;') ? 'mp4' : 'other', reason: 'observing', detached: false, appends: 0, bytes: 0, offsetChanged: false }; reset(track); source.tracks.push(track); bufferMap.set(result, track); result.addEventListener('error', () => { track.reason = 'observer_error'; track.parser = null; }); } } catch { observerError = true; } return result; };
    SourceBuffer.prototype.appendBuffer = function(...args) { const result = Reflect.apply(append, this, args); try { trackAppend(this, args[0]); } catch { observerError = true; } return result; };
    if (MediaSource.prototype.removeSourceBuffer) { const remove = MediaSource.prototype.removeSourceBuffer; MediaSource.prototype.removeSourceBuffer = function(...args) { const result = Reflect.apply(remove, this, args); try { const track = bufferMap.get(args[0]); if (track) { track.detached = true; track.reason = 'detached'; track.parser = null; } } catch { observerError = true; } return result; }; }
    if (SourceBuffer.prototype.changeType) { const change = SourceBuffer.prototype.changeType; SourceBuffer.prototype.changeType = function(...args) { const result = Reflect.apply(change, this, args); try { const track = bufferMap.get(this); if (track && !track.detached) { if (track.generation >= 8) { track.reason = 'limit'; track.parser = null; } else { track.generation++; track.mime = safeMime(args[0]); track.kind = track.mime.startsWith('video/webm;') ? 'webm' : track.mime.includes('/mp4;') ? 'mp4' : 'other'; reset(track); } } } catch { observerError = true; } return result; }; }
    const create = URL.createObjectURL, revoke = URL.revokeObjectURL;
    URL.createObjectURL = function(...args) { const result = Reflect.apply(create, this, args); try { if (args[0] instanceof MediaSource && urls.size < 16) { const source = getSource(args[0]); if (source) urls.set(result, source); } } catch { observerError = true; } return result; };
    URL.revokeObjectURL = function(...args) { const result = Reflect.apply(revoke, this, args); try { urls.delete(args[0]); } catch { observerError = true; } return result; };
  } catch { observerError = true; stop(); }
  window.addEventListener('pagehide', stop, { once: true });
  const snapshot = () => {
    if (!canonical() || performance.now() - start >= 55000) stop();
    const video = currentVideo(), selected = currentSource(video);
    let decodedFrames = null; try { decodedFrames = number(video?.getVideoPlaybackQuality?.()?.totalVideoFrames); } catch { /* Optional metric. */ }
    return { version: 1, elapsedMs: Math.max(0, Math.round(performance.now() - start)), stopped, observerError, totalBytes,
      sources: sources.map(source => ({ source: source.source, selected: source === selected, closed: source.closed, tracks: source.tracks.map(track => ({ track: track.track, generation: track.generation, mime: track.mime, kind: track.kind, reason: track.reason, detached: track.detached, appends: track.appends, bytes: track.bytes, mode: ['segments','sequence'].includes(track.buffer.mode) ? track.buffer.mode : 'other', timestampOffset: offsetTime(track.buffer.timestampOffset), offsetChanged: track.offsetChanged, appendWindow: track.buffer.appendWindowStart === 0 && track.buffer.appendWindowEnd === Infinity ? 'default' : 'other', stats: { ...track.stats }, clock: track.clock.map(item => ({ ...item })) })) })),
      video: video ? { currentTime: number(video.currentTime), decodedFrames, readyState: number(video.readyState), paused: Boolean(video.paused), ended: Boolean(video.ended), protected: Boolean(video.mediaKeys), sourceObject: Boolean(video.srcObject), width: number(video.videoWidth), height: number(video.videoHeight), advertisement: Boolean(document.querySelector('#movie_player.ad-showing, #movie_player.ad-interrupting')), selectedSource: selected?.source ?? null } : null };
  };
  Object.defineProperty(window, '__atsumiYouTubeWebmShapeProbe', { value: Object.freeze({ snapshot }), configurable: false });
})();
