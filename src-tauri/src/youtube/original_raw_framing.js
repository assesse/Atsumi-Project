// YouTube-local structural framing only. Native parses codec/clock/media facts.
// No hooks, network, source storage, or playback/recording authority lives here.
(() => {
  "use strict";
  if (window.__atsumiYouTubeRawFraming) return;
  const INIT = 64 * 1024, APPEND = 16 * 1024 * 1024, UNIT = 32 * 1024 * 1024;
  const bad = () => { throw new Error("raw_framing_unsupported"); };
  const mime = value => {
    if (typeof value !== "string" || value.length > 120) return null;
    const text = value.toLowerCase().replace(/[ \t\r\n\f\v"]/g, "");
    if (/^video\/webm;codecs=(?:vp9|vp09(?:\.\d{2}){3,8})$/.test(text)) return "video";
    return text === "audio/mp4;codecs=mp4a.40.2" ? "audio" : null;
  };
  const vint = (bytes, at, id = false) => {
    if (at >= bytes.length) return null;
    let width = 1, bit = 128;
    while (bit && !(bytes[at] & bit)) { width++; bit >>= 1; }
    if (!bit || width > (id ? 4 : 8)) bad();
    if (at + width > bytes.length) return null;
    let value = BigInt(id ? bytes[at] : bytes[at] & (bit - 1));
    for (let n = 1; n < width; n++) value = value * 256n + BigInt(bytes[at + n]);
    const unknown = !id && value === (1n << BigInt(width * 7)) - 1n;
    if (!unknown && value > BigInt(Number.MAX_SAFE_INTEGER)) bad();
    return { width, value: unknown ? 0 : Number(value), unknown };
  };
  class Framer {
    constructor(role) {
      if (role !== "video" && role !== "audio") bad();
      this.role = role; this.offset = 0; this.header = []; this.leaf = null;
      this.phase = "empty"; this.cluster = null; this.init = null;
      this.scratch = new Uint8Array(INIT); this.initLength = 0; this.collecting = false;
      this.unitStart = null; this.media = false; this.info = false; this.tracks = false;
      this.completed = 0; this.lastCompleteAt = null; this.armedAt = null;
      this.captureStart = null; this.unitIndex = -1; this.capturing = false; this.emit = null;
      this.failed = false; this.pendingMdat = false; this.initialized = false;
    }
    arm() { if (this.failed) bad(); this.armedAt = this.offset; this.captureStart = null; this.unitIndex = -1; this.capturing = false; }
    disarm() { this.armedAt = null; this.captureStart = null; this.capturing = false; }
    boundary(begin) {
      if (this.unitStart !== null) {
        if (!this.initialized || !this.media || this.pendingMdat) bad();
        this.completed = Math.min(1000000, this.completed + 1); this.lastCompleteAt = Date.now();
      }
      this.unitStart = begin; this.media = false; this.info = false; this.tracks = false;
      this.initialized = false; this.collecting = true; this.initLength = 0;
      this.capturing = this.armedAt !== null && begin >= this.armedAt;
      if (this.capturing) {
        this.captureStart ??= begin; this.unitIndex++;
        if (!Number.isSafeInteger(this.unitIndex) || this.unitIndex > 1100000) bad();
      }
    }
    deliver(bytes) {
      if (!bytes.length) return;
      if (this.unitStart !== null && this.offset - this.unitStart > UNIT) bad();
      if (this.collecting) {
        if (this.initLength + bytes.length > INIT) bad();
        this.scratch.set(bytes, this.initLength); this.initLength += bytes.length;
      } else if (this.capturing) this.emit({ kind: "data", bytes });
    }
    initialization() {
      if (!this.collecting || !this.initLength) bad();
      this.init = this.scratch.slice(0, this.initLength); this.collecting = false; this.initialized = true;
      if (this.capturing) {
        const startOffset = this.unitStart - this.captureStart;
        this.emit({ kind: "init", unitIndex: this.unitIndex, startOffset, init: this.init });
        this.emit({ kind: "data", bytes: this.init,
          ...(this.role === "video" && this.unitIndex === 1 ? { rawFirstUnitEnd: startOffset } : {}) });
      }
    }
    webmHeader(header, begin, id, size) {
      const EBML = 0x1a45dfa3, SEGMENT = 0x18538067, CLUSTER = 0x1f43b675;
      if (this.cluster !== null) {
        if (this.cluster !== Infinity && begin >= this.cluster) bad();
        // Unknown-sized Cluster ends only on a recognized level0/1 sibling.
        if (this.cluster === Infinity && [EBML, SEGMENT, CLUSTER, 0x1549a966, 0x1654ae6b].includes(id)) this.cluster = null;
      }
      if (id === EBML) {
        if (this.cluster !== null || size === null || size > INIT || this.phase !== "empty" && this.phase !== "segment") bad();
        this.boundary(begin); this.phase = "ebml";
      } else if (id === SEGMENT) {
        if (this.cluster !== null || this.phase !== "after-ebml" || size !== null) bad();
        this.phase = "segment";
      } else if (this.phase !== "segment") bad();
      else if (this.cluster === null) {
        if (id === 0x1549a966) { if (this.info || this.tracks || size === null) bad(); this.info = true; }
        else if (id === 0x1654ae6b) { if (!this.info || this.tracks || size === null) bad(); this.tracks = true; }
        else if (id === CLUSTER) {
          if (!this.initialized) bad(); this.cluster = size === null ? Infinity : this.offset + size;
        } else if (id !== 0xec || size === null) bad();
      } else if (![0xe7, 0xa3, 0xa0, 0xa7, 0xab, 0xbf, 0xec].includes(id) || size === null) bad();
      if (size !== null && (size > UNIT || this.cluster !== null && this.cluster !== Infinity && this.offset + size > this.cluster)) bad();
      this.deliver(header);
      if (id === SEGMENT || id === CLUSTER) return;
      this.leaf = { kind: id, remaining: size };
      if (!size) this.endLeaf();
    }
    mp4Header(header, begin) {
      const size = new DataView(header.buffer, header.byteOffset, header.length).getUint32(0);
      const kind = String.fromCharCode(...header.subarray(4));
      if (size < 8 || size > UNIT || !["ftyp", "moov", "moof", "mdat", "skip", "free", "styp", "sidx", "emsg", "prft"].includes(kind)) bad();
      if (kind === "ftyp") { this.boundary(begin); this.phase = "ftyp"; }
      else if (kind === "moov") { if (this.phase !== "ftyp" || this.initialized || size > INIT) bad(); }
      else if (!this.initialized && !["free", "skip"].includes(kind)) bad();
      if (kind === "moof") { if (this.pendingMdat) bad(); this.pendingMdat = true; }
      if (kind === "mdat" && !this.pendingMdat) bad();
      this.deliver(header); this.leaf = { kind, remaining: size - 8 };
      if (!this.leaf.remaining) this.endLeaf();
    }
    endLeaf() {
      const kind = this.leaf.kind; this.leaf = null;
      if (this.role === "video") {
        if (kind === 0x1a45dfa3) this.phase = "after-ebml";
        else if (kind === 0x1654ae6b) this.initialization();
        else if (kind === 0xa3 || kind === 0xa0) this.media = true;
      } else {
        if (kind === "moov") { this.phase = "media"; this.initialization(); }
        if (kind === "mdat") { this.pendingMdat = false; this.media = true; }
      }
    }
    feed(bytes, emit = () => {}) {
      if (this.failed || !(bytes instanceof Uint8Array) || bytes.length > APPEND) bad();
      this.emit = emit;
      let at = 0, headers = 0;
      try {
        while (at < bytes.length) {
          if (this.leaf) {
            const count = Math.min(this.leaf.remaining, bytes.length - at);
            const part = bytes.subarray(at, at + count); at += count; this.offset += count;
            this.leaf.remaining -= count; this.deliver(part); if (!this.leaf.remaining) this.endLeaf();
            continue;
          }
          if (!this.header.length && this.cluster !== null && this.cluster !== Infinity && this.offset === this.cluster) this.cluster = null;
          this.header.push(bytes[at++]); this.offset++;
          if (this.header.length > 12 || !Number.isSafeInteger(this.offset)) bad();
          let size, id;
          if (this.role === "video") {
            id = vint(this.header, 0, true); if (!id) continue;
            size = vint(this.header, id.width); if (!size) continue;
          } else if (this.header.length < 8) continue;
          if (++headers > 4096) bad();
          const header = Uint8Array.from(this.header), begin = this.offset - header.length; this.header = [];
          if (this.role === "video") this.webmHeader(header, begin, id.value, size.unknown ? null : size.value);
          else this.mp4Header(header, begin);
        }
        // A finite Cluster boundary may coincide with the end of an append.
        if (!this.leaf && !this.header.length && this.cluster !== Infinity && this.offset === this.cluster) this.cluster = null;
      } catch (error) { this.failed = true; this.disarm(); throw error; }
      finally { this.emit = null; }
    }
  }
  Object.defineProperty(window, "__atsumiYouTubeRawFraming", { value: Object.freeze({ role: mime, create: role => new Framer(role) }) });
})();
