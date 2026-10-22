// Isolated QA-only accepted-buffer capture. Never imported by production.
// Owns at most 32 MiB for a <=15-second collection, with no extra media request.
(() => {
  'use strict';
  const videoId = 'T4XZmMPQ9Kw', captureId = window.__atsumiYouTubeRawPairQaCaptureId;
  const uuid = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
  const canonical = () => window.top === window && location.origin === 'https://www.youtube.com' && location.pathname === '/watch' &&
    new URLSearchParams(location.search).getAll('v').length === 1 && new URLSearchParams(location.search).get('v') === videoId;
  if (!uuid(captureId) || !canonical() || window.__atsumiYouTubeRawPairQa) return;
  let documentId; try { documentId = crypto.randomUUID(); } catch { return; } if (!uuid(documentId)) return;
  const LIMIT = { bytes: 16777216, total: 33554432, append: 16777216, scratch: 65536, elements: 16384, perAppend: 4096, chunk: 131072 };
  const numeric = 'timestampScale trackNumber defaultDurationNs width height trackScale codecDelayNs flagLacing vp9Profile vp9Width vp9Height vp9BitDepth vp9ColorSpace vp9FullRange firstPtsSeconds lastPtsSeconds minCadenceSeconds maxCadenceSeconds audioTimescale firstDtsSeconds lastEndSeconds'.split(' ');
  const counts = 'ebmlHeaders segments segmentUnknown infoCount tracksCount trackEntries videoTracks codecVp9 codecOther codecPrivate contentEncodings compression encryption clusters unknownClusters closedClusters simpleBlocks blockGroups blocks lacingNone lacingXiph lacingFixed lacingEbml lacedFrames referenceBlocks blockDurations containerKeys otherTrackBlocks vp9Keys vp9Inter vp9Invisible vp9ShowExisting vp9MarkerErrors vp9HeaderUnknown superframes superframeFrames superframeInvalid mp4Boxes moofs mdats audioSamples'.split(' ');
  const stats = () => Object.assign(Object.fromEntries(numeric.map(k => [k, null])), Object.fromEntries(counts.map(k => [k, 0])), { completeInit: false, partialHeader: false, openUnknownCluster: false, partialBlock: false });
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
      if (node.id === 0x1654ae6b) { s.completeInit = this.sawSegment && this.sawInfo && s.videoTracks === 1 && s.codecVp9 === 1 && s.trackEntries === 1; this.track.initEnd = this.offset; }
      if (node.id === 0x1f43b675) s.closedClusters++;
      if (node.id === 0xa0 && node.block) this.record(node.block, node.duration, node.references === 0);
    }
    record(block, durationTicks, key) {
      const s = this.track.stats;
      if (s.trackScale !== 1 || s.codecDelayNs !== 0) fail('unsupported');
      if (block.lace || block.track !== s.trackNumber || !block.header || block.header.invalid || block.superframe?.valid === false || s.contentEncodings || s.compression || s.encryption) fail('unsupported');
      if (s.vp9Profile !== null && block.header.profile !== s.vp9Profile || block.header.key &&
        (s.vp9BitDepth !== null && block.header.bitDepth !== s.vp9BitDepth ||
         s.vp9ColorSpace !== null && block.header.colorSpace !== s.vp9ColorSpace ||
         s.vp9FullRange !== null && block.header.fullRange !== s.vp9FullRange)) fail('config_changed');
      if (block.header.key && (block.header.width !== s.width || block.header.height !== s.height)) fail('config_changed');
      if (s.blocks === 0) this.track.rap = Boolean(key && block.header.key && block.header.show);
      s.blocks++; s[['lacingNone','lacingXiph','lacingFixed','lacingEbml'][block.lace]]++; s.lacedFrames += block.lace ? block.frames : 0;
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
      if (duration === null || duration <= 0 || this.track.lastEnd !== null && pts < this.track.lastStart) this.track.intervalKnown = false;
      this.track.lastStart = pts; this.track.lastEnd = duration === null ? null : pts + duration;
      clock(this.track, pts, this.track.lastEnd);
    }
    scalar(id, bytes) {
      const s = this.track.stats, entry = this.entry(), group = this.group(), cluster = this.cluster();
      if (id === 0x4282) { if (String.fromCharCode(...bytes) !== 'webm') fail('unsupported'); return; }
      if (id === 0x86) { if (entry) entry.fields.codec = String.fromCharCode(...bytes) === 'V_VP9' ? 'vp9' : 'other'; return; }
      if (id === 0xfb) { s.referenceBlocks++; if (group) group.references++; return; }
      if (id === 0x23314f) { if (entry && [4,8].includes(bytes.length)) { const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length); entry.fields.trackScale = number(bytes.length === 4 ? view.getFloat32(0) : view.getFloat64(0)); } else fail('malformed'); return; }
      const value = uint(bytes);
      if (id === 0x2ad7b1) { if (!value) fail('malformed'); if (s.blocks && s.timestampScale !== value) fail('config_changed'); s.timestampScale = value; }
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
            this.track.boundary(begin);
            if (s.ebmlHeaders) {
              if (this.stack.length) fail('malformed');
              if (this.track.generation >= 16) fail('limit');
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
    constructor(track) { this.track = track; this.offset = 0; this.header = []; this.leaf = null; this.defaults = null; this.trackId = null; this.ftyp = false; this.elements = 0; this.pendingMdat = false; this.completeMdats = 0; }
    parseMoov(bytes) {
      const s = this.track.stats;
      if (s.completeInit || !this.ftyp) fail('malformed');
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
      this.defaults = read(trex, 12); this.trackId = id; s.audioTimescale = scale; s.completeInit = this.ftyp; this.track.initEnd = this.offset;
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
      if (this.pendingMdat || !s.completeInit) fail('malformed'); this.pendingMdat = true;
      if (!known || !samples || decode === null || !s.audioTimescale) this.track.intervalKnown = false;
      s.audioSamples += samples;
      if (known && samples && decode !== null && s.audioTimescale) { const begin = decode / s.audioTimescale, end = (decode + duration) / s.audioTimescale; if (this.track.lastEnd !== null && Math.abs(begin - this.track.lastEnd) > 1 / s.audioTimescale) this.track.intervalKnown = false; this.track.lastStart = begin; this.track.lastEnd = end; s.firstDtsSeconds ??= sourceTime(begin); s.lastEndSeconds = sourceTime(end); clock(this.track, begin, end); }
    }
    feed(bytes) {
      let at = 0, boxes = 0;
      while (at < bytes.length) {
        if (!this.leaf) {
          const n = Math.min(8 - this.header.length, bytes.length - at); for (let i = 0; i < n; i++) this.header.push(bytes[at++]); this.offset += n; if (this.header.length < 8) break;
          if (++boxes > LIMIT.perAppend || ++this.elements > LIMIT.elements) fail('limit');
          const header = Uint8Array.from(this.header), size = u32(header, 0), kind = String.fromCharCode(...header.subarray(4)); this.header = [];
          if (size < 8 || size > LIMIT.append) fail('unsupported');
          if (kind === 'ftyp') { this.track.boundary(this.offset - 8); if (this.ftyp) { if (this.track.generation >= 16) fail('limit'); this.track.generation++; Object.assign(this.track.stats, stats()); this.track.clock = []; this.defaults = null; this.trackId = null; } this.pendingMdat = false; this.completeMdats = 0; }
          else if (!this.ftyp) fail('malformed');
          if (kind === 'mdat' && !this.pendingMdat) fail('malformed');
          const retain = kind === 'moov' || kind === 'moof'; if (retain && size > LIMIT.scratch) fail('limit');
          this.leaf = { kind, remaining: size - 8, data: retain ? new Uint8Array(size) : null, written: 8 };
          if (retain) this.leaf.data.set(header);
          this.track.stats.mp4Boxes++;
          if (kind === 'ftyp') this.ftyp = true;
          if (kind === 'mdat') this.track.stats.mdats++;
        }
        const leaf = this.leaf, n = Math.min(leaf.remaining, bytes.length - at);
        if (leaf.data) leaf.data.set(bytes.subarray(at, at + n), leaf.written);
        leaf.written += n; leaf.remaining -= n; at += n; this.offset += n;
        if (!leaf.remaining) { if (leaf.kind === 'mdat') { this.pendingMdat = false; this.completeMdats++; } if (leaf.kind === 'moov') this.parseMoov(leaf.data); if (leaf.kind === 'moof') { this.track.stats.moofs++; this.parseMoof(leaf.data); } this.leaf = null; }
      }
    }
    snapshot() { this.track.stats.partialHeader = this.header.length > 0; this.track.stats.partialBlock = Boolean(this.leaf && this.leaf.kind !== 'mdat'); }
  }

  const integer = (value, min, max) => Number.isInteger(value) && value >= min && value <= max;
  const configuration = track => track.kind === 'webm' ? {
    kind: 'vp9_webm', trackNumber: track.stats.trackNumber, timestampScaleNs: track.stats.timestampScale,
    defaultDurationNs: track.stats.defaultDurationNs, width: track.stats.width, height: track.stats.height,
    profile: track.stats.vp9Profile, bitDepth: track.stats.vp9BitDepth
  } : { kind: 'aac_mp4', trackId: track.parser.trackId, timescale: track.stats.audioTimescale };
  const validConfiguration = track => {
    const c = configuration(track);
    return c.kind === 'vp9_webm' ? integer(c.trackNumber, 1, 2147483647) && integer(c.timestampScaleNs, 1, 1e9) &&
      (c.defaultDurationNs === null || integer(c.defaultDurationNs, 1, 1e9)) && integer(c.width, 1, 16384) && integer(c.height, 1, 16384) &&
      integer(c.profile, 0, 3) && [8,10,12].includes(c.bitDepth) : integer(c.trackId, 1, 2147483647) && integer(c.timescale, 1, 768000);
  };
  const projectUnit = (u, index) => Object.freeze({ unit: index + 1, track: u.track.kind === 'webm' ? 'video' : 'audio', generation: u.generation,
    startOffset: u.start, endOffset: u.end, byteLength: u.end - u.start, initLength: u.initEnd - u.start,
    startSeconds: u.startSeconds, endSeconds: u.endSeconds, configuration: Object.freeze({ ...u.configuration }) });
  const projectDiagnostics = kind => {
    const units = boundSource?.tracks.find(track => track.kind === kind)?.units ?? [];
    const result = { units: units.length, adjacentPairs: 0, adjacentTriples: 0, initDifferences: 0,
      configurationDifferences: 0, byteGenerationBreaks: 0, clockOrderBreaks: 0, gaps: 0 };
    for (let i = 1; i < units.length; i++) {
      const unit = units[i];
      if (unit.adjacent) { result.adjacentPairs++; if (i > 1 && units[i - 1].adjacent) result.adjacentTriples++; }
      for (const key of ['initDifferences','configurationDifferences','byteGenerationBreaks','clockOrderBreaks','gaps']) {
        if (unit.pairReasons[key]) result[key]++;
      }
    }
    return Object.freeze(result);
  };
  const projectSnapshot = () => {
    const trackBytes = { video: 0, audio: 0 }; let partial = 0;
    for (const track of boundSource?.tracks ?? []) {
      if (track.kind === 'webm') trackBytes.video += track.bytes; else if (track.kind === 'mp4') trackBytes.audio += track.bytes;
      if (track.unitStart !== null && track.bytes > track.unitStart) partial++;
    }
    return Object.freeze({ version: 3, captureId, documentId, state: !stopped ? 'collecting' : selectedUnits.length ? 'ready' : 'stopped',
      reason: terminalReason, sourceOrdinal: boundSource?.source ?? null,
      elapsedMs: collectionStart === null ? 0 : Math.min(15000, Math.max(0, Math.floor(performance.now() - collectionStart))),
      totalBytes, trackBytes: Object.freeze(trackBytes), excludedPartialUnits: partial, requiresAudioJoin: selectedUnits.length > 0,
      diagnostics: Object.freeze({ video: projectDiagnostics('webm'), audio: projectDiagnostics('mp4') }),
      units: Object.freeze(selectedUnits.map(projectUnit)) });
  };

  // This store owns copies of accepted appends; it never keeps caller-owned views.
  const sources = [], sourceMap = new WeakMap(), bufferMap = new WeakMap(), urls = new Map();
  let boundSource = null, collectionStart = null, totalBytes = 0, appendCalls = 0;
  let stopped = false, terminalReason = 'observing', selectedUnits = [], frozenSnapshot = null;
  const currentVideo = () => document.querySelector('#movie_player video');
  const currentSource = () => { const video = currentVideo(); return video && !video.mediaKeys && !video.srcObject ? urls.get(video.currentSrc || video.src) ?? null : null; };
  const excluded = () => Boolean(document.querySelector('#movie_player.ad-showing, #movie_player.ad-interrupting') || currentVideo()?.mediaKeys || currentVideo()?.srcObject);
  const clock = (track, begin, end) => {
    const startSeconds = sourceTime(begin), endSeconds = sourceTime(end);
    if (startSeconds === null || endSeconds === null) track.intervalKnown = false;
    track.clock.push({ startSeconds, endSeconds }); if (track.clock.length > 8) track.clock.shift();
  };
  const sliceOwned = (track, start, end) => {
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end <= start || end > track.bytes || end - start > LIMIT.chunk) fail('invalid_transfer');
    const out = new Uint8Array(end - start); let copied = 0;
    for (const part of track.parts) {
      const begin = Math.max(start, part.start), finish = Math.min(end, part.start + part.bytes.length);
      if (finish > begin) { out.set(part.bytes.subarray(begin - part.start, finish - part.start), begin - start); copied += finish - begin; }
    }
    if (copied !== out.length) fail('invalid_transfer'); return out;
  };
  const equalInit = (a, b) => {
    const length = a.initEnd - a.start; if (length <= 0 || length > LIMIT.scratch || length !== b.initEnd - b.start) return false;
    let ai = 0, bi = 0;
    for (let i = 0; i < length; i++) {
      while (a.track.parts[ai] && a.track.parts[ai].start + a.track.parts[ai].bytes.length <= a.start + i) ai++;
      while (b.track.parts[bi] && b.track.parts[bi].start + b.track.parts[bi].bytes.length <= b.start + i) bi++;
      const left = a.track.parts[ai], right = b.track.parts[bi];
      if (!left || !right || left.bytes[a.start + i - left.start] !== right.bytes[b.start + i - right.start]) return false;
    } return true;
  };
  const equalConfiguration = (a, b) => {
    if (a.kind !== b.kind) return false;
    const keys = a.kind === 'vp9_webm' ? ['trackNumber','timestampScaleNs','defaultDurationNs','width','height','profile','bitDepth'] : ['trackId','timescale'];
    return keys.every(key => a[key] === b[key]);
  };
  const boundary = (track, end) => {
    const s = track.stats, parser = track.parser;
    if (track.unitStart !== null) {
      const startTime = track.kind === 'webm' ? s.firstPtsSeconds : s.firstDtsSeconds;
      const complete = s.completeInit && validConfiguration(track) && track.intervalKnown && track.initEnd !== null && track.initEnd > track.unitStart && track.initEnd <= end && track.initEnd - track.unitStart <= LIMIT.scratch &&
        sourceTime(startTime) !== null && sourceTime(track.lastEnd) !== null && track.lastEnd > startTime &&
        (track.kind === 'webm' ? s.blocks > 0 && track.rap && !s.contentEncodings : parser.completeMdats > 0 && !parser.pendingMdat);
      if (complete) {
        if (track.units.length >= 16) fail('limit');
        const unit = { track, generation: track.generation, start: track.unitStart, end, initEnd: track.initEnd,
          startSeconds: startTime, endSeconds: track.lastEnd, stats: { ...s }, configuration: configuration(track), rap: track.kind === 'webm', adjacent: false, pairReasons: null };
        const previous = track.units.at(-1);
        // Compare the bounded original init once per completed unit, never on
        // every append/window search. Gaps from rejected units remain gaps.
        if (previous) {
          const reasons = { initDifferences: !equalInit(previous, unit),
            configurationDifferences: !equalConfiguration(previous.configuration, unit.configuration),
            byteGenerationBreaks: previous.end !== unit.start || previous.generation + 1 !== unit.generation,
            clockOrderBreaks: !(previous.startSeconds < unit.startSeconds && previous.endSeconds < unit.endSeconds),
            gaps: unit.startSeconds > previous.endSeconds };
          unit.pairReasons = Object.freeze(reasons);
          // A reported video gap is not a new admission rule. The original
          // selection still decides audio coverage separately.
          unit.adjacent = !reasons.initDifferences && !reasons.configurationDifferences && !reasons.byteGenerationBreaks && !reasons.clockOrderBreaks;
        }
        track.units.push(unit);
      }
    }
    track.unitStart = end; track.initEnd = null; track.lastStart = null; track.lastEnd = null; track.rap = false; track.intervalKnown = true;
  };
  const candidates = (allowTwoVideo = false) => {
    const videos = boundSource?.tracks.filter(t => t.kind === 'webm') ?? [], audios = boundSource?.tracks.filter(t => t.kind === 'mp4') ?? [];
    if (videos.length !== 1 || audios.length !== 1) return null;
    const video = videos[0].units, audio = audios[0].units;
    // V3 exports independent original units for initialization review, not a
    // joined stream. Keep exact-init adjacency diagnostics separate and strict.
    const rawAdjacent = unit => unit.pairReasons && !unit.pairReasons.configurationDifferences &&
      !unit.pairReasons.byteGenerationBreaks && !unit.pairReasons.clockOrderBreaks;
    const run = (units, at, count) => units.slice(at, at + count).every((unit, index) => index === 0 || rawAdjacent(unit));
    for (const videoCount of allowTwoVideo ? [3,2] : [3]) for (let vi = 0; vi + videoCount <= video.length; vi++) {
      if (!run(video, vi, videoCount)) continue;
      const begin = video[vi].startSeconds, end = video[vi + videoCount - 1].endSeconds;
      for (const audioCount of [2,3]) for (let ai = 0; ai + audioCount <= audio.length; ai++) {
        if (!run(audio, ai, audioCount) || audio[ai].startSeconds > begin || audio[ai + audioCount - 1].endSeconds < end) continue;
        const selectedAudio = audio.slice(ai, ai + audioCount);
        if (!selectedAudio.every((unit, index) => unit.startSeconds < end && unit.endSeconds > begin &&
          (index === 0 || unit.startSeconds <= selectedAudio[index - 1].endSeconds))) continue;
        // These floats select raw evidence only. No epsilon, rescaling or
        // packet/codec/relocation equivalence is inferred here.
        return { units: [...video.slice(vi, vi + videoCount), ...selectedAudio] };
      }
    }
    return null;
  };
  const stop = (reason, allowSelection = false, allowTwoVideo = false) => {
    if (stopped) return; stopped = true;
    let candidate = null;
    try { candidate = allowSelection ? candidates(allowTwoVideo) : null; } catch { reason = 'observer_error'; }
    if (candidate) { selectedUnits = candidate.units; terminalReason = 'complete'; }
    else terminalReason = reason;
    frozenSnapshot = projectSnapshot();
  };
  const check = () => {
    if (!canonical()) { stop('source_changed'); return false; }
    if (stopped) return false;
    if (excluded()) { if (collectionStart !== null) stop('excluded'); return false; }
    const selected = currentSource();
    if (boundSource && selected !== boundSource) { stop('source_changed'); return false; }
    if (boundSource && boundSource.tracks.some(t => t.buffer.mode !== 'segments' || t.buffer.timestampOffset !== 0 || t.buffer.appendWindowStart !== 0 || t.buffer.appendWindowEnd !== Infinity)) { stop('configuration_changed'); return false; }
    if (collectionStart !== null && performance.now() - collectionStart >= 15000) { stop('no_candidate', true, true); return false; }
    return true;
  };
  const getSource = object => {
    let source = sourceMap.get(object); if (source) return source;
    if (sources.length >= 8) return null;
    source = { source: sources.length + 1, object, tracks: [], closed: false, invalid: false }; sources.push(source); sourceMap.set(object, source);
    object.addEventListener('sourceclose', () => { source.closed = true; if (source === boundSource) stop('source_changed'); });
    return source;
  };
  const appendOwned = (buffer, input) => {
    if (!check()) return;
    const track = bufferMap.get(buffer), source = currentSource(); if (!track || !source || track.source !== source || source.closed) return;
    if (!boundSource) boundSource = source;
    if (track.kind === 'other' || boundSource.invalid || new Set(boundSource.tracks.map(t => t.kind)).size !== boundSource.tracks.length) { stop('unsupported'); return; }
    if (buffer.mode !== 'segments' || buffer.timestampOffset !== 0 || buffer.appendWindowStart !== 0 || buffer.appendWindowEnd !== Infinity) { stop('configuration_changed'); return; }
    const view = bytesOf(input); if (!view) { stop('unsupported'); return; }
    if (appendCalls >= 4096 || view.length > LIMIT.append || track.bytes + view.length > LIMIT.bytes || totalBytes + view.length > LIMIT.total) { stop('limit'); return; }
    if (collectionStart === null) collectionStart = performance.now();
    terminalReason = 'observing'; appendCalls++;
    const owned = view.slice(); track.parts.push({ start: track.bytes, bytes: owned }); track.bytes += owned.length; totalBytes += owned.length;
    try { track.parser.feed(owned); } catch (reason) { stop(reason === 'config_changed' ? 'configuration_changed' : ['limit','malformed','unsupported'].includes(reason) ? reason : 'observer_error'); return; }
    if (candidates()) stop('no_candidate', true);
  };
  const addTrack = (source, buffer, mime) => {
    if (source.tracks.length >= 2) { source.invalid = true; if (source === boundSource) stop('unsupported'); return; }
    const kind = mime.startsWith('video/webm;') ? 'webm' : mime === 'audio/mp4;codecs=mp4a.40.2' ? 'mp4' : 'other';
    const track = { source, buffer, track: source.tracks.length + 1, mime, kind, generation: 1, parts: [], bytes: 0, units: [],
      stats: stats(), clock: [], unitStart: null, initEnd: null, lastStart: null, lastEnd: null, intervalKnown: true, rap: false, parser: null };
    track.boundary = end => boundary(track, end);
    track.parser = kind === 'webm' ? new Webm(track) : kind === 'mp4' ? new Mp4(track) : null;
    source.tracks.push(track); bufferMap.set(buffer, track);
    buffer.addEventListener('error', () => { if (source === boundSource) stop('observer_error'); });
  };
  try {
    const add = MediaSource.prototype.addSourceBuffer, append = SourceBuffer.prototype.appendBuffer;
    MediaSource.prototype.addSourceBuffer = function(...args) { const result = Reflect.apply(add, this, args); try { const source = getSource(this); if (source) addTrack(source, result, safeMime(args[0])); } catch { stop('observer_error'); } return result; };
    SourceBuffer.prototype.appendBuffer = function(...args) { const result = Reflect.apply(append, this, args); try { appendOwned(this, args[0]); } catch { stop('observer_error'); } return result; };
    if (MediaSource.prototype.removeSourceBuffer) { const remove = MediaSource.prototype.removeSourceBuffer; MediaSource.prototype.removeSourceBuffer = function(...args) { const result = Reflect.apply(remove, this, args); try { if (bufferMap.get(args[0])?.source === boundSource) stop('source_changed'); } catch { stop('observer_error'); } return result; }; }
    if (SourceBuffer.prototype.changeType) { const change = SourceBuffer.prototype.changeType; SourceBuffer.prototype.changeType = function(...args) { const result = Reflect.apply(change, this, args); try { if (bufferMap.get(this)?.source === boundSource) stop('configuration_changed'); } catch { stop('observer_error'); } return result; }; }
    const create = URL.createObjectURL, revoke = URL.revokeObjectURL;
    URL.createObjectURL = function(...args) { const result = Reflect.apply(create, this, args); try { if (args[0] instanceof MediaSource && urls.size < 16) { const source = getSource(args[0]); if (source) urls.set(result, source); } } catch { stop('observer_error'); } return result; };
    URL.revokeObjectURL = function(...args) { const result = Reflect.apply(revoke, this, args); try { const source = urls.get(args[0]); urls.delete(args[0]); if (source && source === boundSource && !stopped) stop('source_changed'); } catch { stop('observer_error'); } return result; };
  } catch { stop('observer_error'); }
  window.addEventListener('pagehide', () => stop('closed'), { once: true });

  const snapshot = () => {
    try { check(); } catch { stop('observer_error'); }
    return frozenSnapshot ?? projectSnapshot();
  };
  const readChunk = (requestCapture, requestDocument, unit, offset) => {
    if (!canonical() || requestCapture !== captureId || requestDocument !== documentId || !stopped || !selectedUnits.length ||
      !integer(unit, 1, selectedUnits.length) || !integer(offset, 0, LIMIT.bytes - 1)) return null;
    const selected = selectedUnits[unit - 1], size = selected.end - selected.start;
    if (offset >= size) return null;
    try {
      const byteLength = Math.min(LIMIT.chunk, size - offset), bytes = sliceOwned(selected.track, selected.start + offset, selected.start + offset + byteLength);
      let binary = ''; for (let at = 0; at < bytes.length; at += 8192) binary += String.fromCharCode(...bytes.subarray(at, at + 8192));
      return { version: 3, captureId, documentId, unit, generation: selected.generation, offset, byteLength,
        final: offset + byteLength === size, base64: btoa(binary) };
    } catch { return null; }
  };
  Object.defineProperty(window, '__atsumiYouTubeRawPairQa', { value: Object.freeze({ snapshot, readChunk }), configurable: false });
})();

