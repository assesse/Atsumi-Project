// SPDX-License-Identifier: MIT
// YouTube-local copy of the bounded observer algorithm; no CHZZK runtime state.
// Observe already-appended clear fMP4 in this Window only. Never fetch, clone a
// response, read credentials, create a player/decoder, or wait in appendBuffer.
(() => {
  "use strict";
  const channel = () => window.location.origin === "https://www.youtube.com" && window.location.pathname === "/watch" ?
    /^[A-Za-z0-9_-]{11}$/.exec(new URLSearchParams(window.location.search).get("v") ?? "")?.[0] ?? null : null;
  if (!channel() || window.top !== window || window.__atsumiYouTubeEncodedCapture) return;
  const MAX_INIT = 64 * 1024;
  const MAX_APPEND = 16 * 1024 * 1024;
  const MAX_QUEUE = 64 * 1024 * 1024;
  const MAX_CONTINUITY = 32 * 1024 * 1024;
  const MAX_CONTINUITY_FRAGMENTS = 128;
  const MAX_CONTINUITY_PIECES = 4096;
  const CONTINUITY_LIFETIME = 15_000;
  const MAX_MOOF_DIAGNOSTIC = 64 * 1024;
  const CHUNK = 128 * 1024;
  const RAW_FORMAT = "youtubeVp9AacRawV1";
  const rawFraming = window.__atsumiYouTubeRawFraming;
  let rawCaptureEnabled = false;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const recordingID = (id) => typeof id === "string" && (UUID.test(id) || /^[a-f0-9]{32}$/i.test(id));
  const NativeMediaSource = window.MediaSource;
  const NativeSourceBuffer = window.SourceBuffer;
  const NativeURL = window.URL;
  const sources = new WeakMap();
  const buffers = new WeakMap();
  const urls = new Map();
  const observed = new Set();
  const hooks = [];
  let installed = false;
  let active = null;
  let recovering = null;
  let continuity = null;
  const cancelledRequests = new Set();
  let lastDetail = "encoded_unavailable";
  let appendCount = 0;
  let appendBytes = 0;
  let lastAppendAt = null;
  let lastStop = null;
  let lastTransportFault = null;
  let lastStart = null;
  let latestRecord = null;
  let maxAckMs = 0;
  const increment = (value) => Math.min(0xffff_ffff, value + 1);

  const initTiming = (bytes) => {
    try {
      if (!bytes || bytes.length > MAX_INIT) throw "size_limit";
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength); let nodes = 0;
      const children = (start, end) => {
        const result = [];
        for (let at = start; at < end;) {
          if (++nodes > 64) throw "node_limit";
          if (at + 8 > end) throw "box_header_invalid";
          const size = view.getUint32(at);
          if (size < 8 || size > end - at) throw "box_size_invalid";
          result.push({ type: String.fromCharCode(...bytes.subarray(at + 4, at + 8)), start: at + 8, end: at + size }); at += size;
        }
        return result;
      };
      const one = (items, type) => { const found = items.filter(item => item.type === type); if (found.length !== 1) throw "init_timing_unsupported"; return found[0]; };
      const number = (item, offset) => { if (item.start + offset + 4 > item.end) throw "init_timing_truncated"; return view.getUint32(item.start + offset); };
      const moov = one(children(0, bytes.length), "moov"), movie = children(moov.start, moov.end);
      const mvex = one(movie, "mvex"), defaults = children(mvex.start, mvex.end).filter(item => item.type === "trex");
      const traks = movie.filter(item => item.type === "trak");
      if (!traks.length || traks.length > 2 || defaults.length > 2) throw "init_timing_unsupported";
      const tracks = traks.map(trak => {
        const boxes = children(trak.start, trak.end), tkhd = one(boxes, "tkhd"), mdia = one(boxes, "mdia");
        const tkhdVersion = number(tkhd, 0) >>> 24;
        if (tkhdVersion > 1) throw "init_timing_unsupported";
        const trackId = number(tkhd, tkhdVersion ? 20 : 12), media = children(mdia.start, mdia.end);
        const mdhd = one(media, "mdhd"), mdhdVersion = number(mdhd, 0) >>> 24, hdlr = one(media, "hdlr");
        if (mdhdVersion > 1) throw "init_timing_unsupported";
        const timescale = number(mdhd, mdhdVersion ? 20 : 12), handler = number(hdlr, 8);
        const kind = handler === 0x76696465 ? "video" : handler === 0x736f756e ? "audio" : null;
        const matching = defaults.filter(item => number(item, 4) === trackId);
        if (!kind || !timescale || matching.length !== 1 || number(matching[0], 0) !== 0) throw "init_timing_unsupported";
        const trex = matching[0];
        return { trackId, kind, timescale, defaultDuration: number(trex, 12), defaultSize: number(trex, 16), defaultFlags: number(trex, 20) };
      });
      return { tracks };
    } catch (reason) {
      return { dropped: ["size_limit", "node_limit", "box_header_invalid", "box_size_invalid", "init_timing_unsupported", "init_timing_truncated"].includes(reason) ? reason : "invalid" };
    }
  };

  const aacShape = (bytes, start, end) => {
    const descriptor = (at, stop) => {
      if (at >= stop) return null;
      const tag = bytes[at++]; let length = 0, done = false;
      for (let n = 0; n < 4 && at < stop; n++) { const value = bytes[at++]; length = length * 128 + (value & 127); if (!(value & 128)) { done = true; break; } }
      if (!done || length > 4096 || at + length > stop) return null;
      return { tag, start: at, end: at + length, length };
    };
    if (start + 4 > end) return { invalid: true };
    const es = descriptor(start + 4, end); if (!es || es.length < 3) return { invalid: true };
    const decoder = descriptor(es.start + 3, es.end); if (!decoder || decoder.length < 13) return { esTag: es.tag, invalid: true };
    const asc = descriptor(decoder.start + 13, decoder.end);
    return { esTag: es.tag, esBytes: es.length, esFlags: bytes[es.start + 2], decoderTag: decoder.tag,
      decoderBytes: decoder.length, objectType: bytes[decoder.start], streamType: bytes[decoder.start + 1] >> 2,
      ascTag: asc?.tag ?? null, ascBytes: asc?.length ?? null,
      // Explicitly approved bounded codec configuration, never media samples.
      ascConfigHex: asc ? Array.from(bytes.subarray(asc.start, Math.min(asc.end, asc.start + 32)), value => value.toString(16).padStart(2, "0")).join("") : null,
      audioObjectType: asc && asc.length >= 2 ? bytes[asc.start] >> 3 : null,
      frequencyIndex: asc && asc.length >= 2 ? ((bytes[asc.start] & 7) << 1) | (bytes[asc.start + 1] >> 7) : null,
      channelConfig: asc && asc.length >= 2 ? (bytes[asc.start + 1] >> 3) & 15 : null };
  };

  // Structure only, never sample/config payload. Deliberately bounded even for
  // malformed input; this is diagnostic evidence, not relaxed mux admission.
  const initTree = (bytes) => {
    if (!bytes || bytes.byteLength > MAX_INIT) return [];
    let nodes = 0;
    const walk = (start, end, depth) => {
      const result = [];
      for (let at = start; at + 8 <= end && nodes < 96 && depth < 8;) {
        const size = new DataView(bytes.buffer, bytes.byteOffset + at, 4).getUint32(0);
        const raw = String.fromCharCode(...bytes.subarray(at + 4, at + 8));
        const type = /^[A-Za-z0-9 ]{4}$/.test(raw) ? raw : "non_ascii";
        const item = { type, size }; result.push(item); nodes++;
        if (size < 8 || at + size > end) { item.invalid = true; break; }
        if (type === "esds") item.aac = aacShape(bytes, at + 8, at + size);
        let child = null;
        if (["moov", "trak", "mdia", "minf", "stbl", "mvex"].includes(type)) child = at + 8;
        else if (type === "stsd") child = at + 16;
        else if (["avc1", "avc3"].includes(type)) child = at + 86;
        else if (type === "mp4a") child = at + 36;
        if (child !== null && child < at + size) item.children = walk(child, at + size, depth + 1);
        at += size;
      }
      return result;
    };
    return walk(0, bytes.byteLength, 0);
  };

  // Diagnostic-only paired projection. Raw ranges stay local and are used only
  // for equality; no config bytes, hashes or arbitrary box text leave here.
  const initChangeShape = (bytes) => {
    if (!bytes || bytes.byteLength > MAX_INIT) throw "size_limit";
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const tracks = [], ranges = new Map(), structure = []; let nodes = 0;
    const number = (at, size, stop) => {
      if (at < 0 || at + size > stop) throw "invalid";
      return size === 2 ? view.getUint16(at) : view.getUint32(at);
    };
    const walk = (start, end, depth, path, track) => {
      if (depth > 8) throw "structure_limit";
      let index = 0;
      for (let at = start; at < end; index++) {
        if (++nodes > 96) throw "structure_limit";
        const size = number(at, 4, end);
        if (size < 8 || at + size > end) throw "invalid";
        const type = String.fromCharCode(...bytes.subarray(at + 4, at + 8));
        const key = `${path}/${index}:${type}`, stop = at + size, payload = at + 8;
        structure.push([key, size]);
        let child = null, current = track, mask = 32;
        if (["moov", "trak", "mdia", "minf", "stbl", "mvex"].includes(type)) {
          child = payload;
          if (type === "trak") {
            if (track || tracks.length >= 2) throw "unsupported";
            current = { kind: 0, codecFamily: 0, width: 0, height: 0, timescale: 0,
              audioChannels: 0, audioSampleRate: 0, codecProfile: 0, codecCompatibility: 0, codecLevel: 0 };
            tracks.push(current);
          }
        } else if (type === "stsd") {
          if (number(payload, 4, stop) !== 0 || number(payload + 4, 4, stop) !== 1) throw "unsupported";
          child = payload + 8;
        } else if (["avc1", "avc3", "mp4a"].includes(type)) {
          if (!current || current.codecFamily) throw "unsupported";
          mask = 1; current.codecFamily = type === "avc1" ? 1 : type === "avc3" ? 2 : 3;
          if (type === "mp4a") {
            current.audioChannels = number(at + 24, 2, stop);
            current.audioSampleRate = number(at + 32, 4, stop) >>> 16;
            child = at + 36;
          } else {
            current.width = number(at + 32, 2, stop); current.height = number(at + 34, 2, stop);
            child = at + 86;
          }
          if (child > stop) throw "invalid";
        } else if (type === "hdlr") {
          if (!current || current.kind) throw "unsupported";
          const kind = number(payload + 8, 4, stop);
          current.kind = kind === 0x76696465 ? 1 : kind === 0x736f756e ? 2 : 0;
        } else if (type === "mdhd") {
          if (!current || current.timescale) throw "unsupported";
          const version = number(payload, 4, stop) >>> 24;
          if (version > 1) throw "unsupported";
          current.timescale = number(payload + (version ? 20 : 12), 4, stop); mask = 8;
        } else if (type === "avcC") {
          if (!current || ![1, 2].includes(current.codecFamily) || number(payload, 4, stop) >>> 24 !== 1) throw "unsupported";
          current.codecProfile = bytes[payload + 1]; current.codecCompatibility = bytes[payload + 2]; current.codecLevel = bytes[payload + 3]; mask = 2;
        } else if (type === "esds") mask = 4;
        else if (["mvhd", "tkhd", "trex"].includes(type)) mask = 8;
        else if (type === "btrt") mask = 16;
        else if (type === "ftyp") mask = 128;
        if (child === null) ranges.set(key, { bytes: bytes.subarray(payload, stop), mask });
        else {
          if (child > payload) ranges.set(`${key}/header`, { bytes: bytes.subarray(payload, child), mask });
          walk(child, stop, depth + 1, key, current);
        }
        at = stop;
      }
    };
    walk(0, bytes.byteLength, 0, "", null);
    if (!tracks.length || tracks.some(track => !track.timescale || track.timescale > 1_000_000_000 ||
      (track.kind === 1 ? ![1, 2].includes(track.codecFamily) || !track.width || !track.height || !track.codecProfile :
        track.kind === 2 ? track.codecFamily !== 3 || !track.audioChannels || track.audioChannels > 32 || !track.audioSampleRate : true))) throw "unsupported";
    return { shape: { initBytes: bytes.byteLength, tracks }, ranges, structure };
  };
  const initChange = (previous, next, trackIndex) => {
    try {
      const before = initChangeShape(previous), after = initChangeShape(next);
      let changeMask = JSON.stringify(before.structure) === JSON.stringify(after.structure) ? 0 : 64;
      for (const key of new Set([...before.ranges.keys(), ...after.ranges.keys()])) {
        const old = before.ranges.get(key), replacement = after.ranges.get(key);
        if (!old || !replacement || old.bytes.length !== replacement.bytes.length || old.bytes.some((value, index) => value !== replacement.bytes[index])) {
          changeMask |= (old?.mask ?? 0) | (replacement?.mask ?? 0);
        }
      }
      return { version: 1, trackIndex, changeMask, previous: before.shape, next: after.shape };
    } catch (reason) {
      return { version: 1, trackIndex, dropped: ["size_limit", "structure_limit", "unsupported"].includes(reason) ? reason : "invalid" };
    }
  };

  // Numeric fixed headers only. Never traverse sample arrays or inspect mdat.
  // This evidence does not admit a fragment rejected by the native validator.
  const moofShape = (bytes) => {
    const dropped = (reason) => ({ moofBytes: bytes.length, dropped: reason });
    if (bytes.length > MAX_MOOF_DIAGNOSTIC) return dropped("size_limit");
    try {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      const typeAt = (at) => String.fromCharCode(...bytes.subarray(at + 4, at + 8));
      if (bytes.length < 8 || view.getUint32(0) !== bytes.length || typeAt(0) !== "moof") return dropped("box_header_invalid");
      let nodes = 1;
      const children = (start, end, visit) => {
        for (let at = start; at < end;) {
          if (++nodes > 64) throw "node_limit";
          if (at + 8 > end) throw "box_header_invalid";
          const size = view.getUint32(at);
          if (size < 8 || size > end - at) throw "box_size_invalid";
          visit(typeAt(at), at + 8, at + size); at += size;
        }
      };
      const fullBox = (at) => ({ version: bytes[at], flags: (bytes[at + 1] << 16) | (bytes[at + 2] << 8) | bytes[at + 3] });
      const trafs = [];
      children(8, bytes.length, (type, start, end) => {
        if (type !== "traf") return;
        if (trafs.length >= 2) throw "traf_limit";
        const traf = { tfhd: null, tfdt: null, truns: [] }; trafs.push(traf);
        children(start, end, (kind, at, stop) => {
          if (kind === "tfhd") {
            if (traf.tfhd) throw "duplicate_tfhd";
            if (stop - at < 8) throw "tfhd_truncated";
            const fields = fullBox(at), flags = fields.flags;
            const descriptionAt = at + 8 + (flags & 1 ? 8 : 0);
            const required = 8 + (flags & 1 ? 8 : 0) + (flags & 2 ? 4 : 0) + (flags & 8 ? 4 : 0) + (flags & 16 ? 4 : 0) + (flags & 32 ? 4 : 0);
            if (stop - at < required) throw "tfhd_truncated";
            traf.tfhd = { ...fields, trackId: view.getUint32(at + 4), baseOffsetPresent: Boolean(flags & 1), defaultBase: Boolean(flags & 0x020000),
              ...(flags & 2 ? { descriptionIndex: view.getUint32(descriptionAt) } : {}) };
            let next = descriptionAt + (flags & 2 ? 4 : 0);
            for (const [bit, name] of [[8, "defaultDuration"], [16, "defaultSize"], [32, "defaultFlags"]]) {
              if (flags & bit) { traf.tfhd[name] = view.getUint32(next); next += 4; }
            }
          } else if (kind === "tfdt") {
            if (traf.tfdt) throw "duplicate_tfdt";
            if (stop - at < 4) throw "tfdt_truncated";
            const fields = fullBox(at);
            if (fields.version > 1 || fields.flags !== 0) throw "tfdt_unsupported";
            if (stop - at !== (fields.version ? 12 : 8)) throw "tfdt_truncated";
            const time = fields.version ? view.getUint32(at + 4) * 0x1_0000_0000 + view.getUint32(at + 8) : view.getUint32(at + 4);
            if (!Number.isSafeInteger(time)) throw "tfdt_unsafe_integer";
            traf.tfdt = { ...fields, baseDecodeTime: time };
          } else if (kind === "trun") {
            if (stop - at < 8) throw "trun_truncated";
            const fields = fullBox(at), required = 8 + (fields.flags & 1 ? 4 : 0) + (fields.flags & 4 ? 4 : 0);
            if (stop - at < required) throw "trun_truncated";
            const run = { ...fields, sampleCount: view.getUint32(at + 4),
              ...(fields.flags & 1 ? { dataOffset: view.getInt32(at + 8) } : {}) };
            if (fields.flags & 4) run.firstSampleFlags = view.getUint32(at + 8 + (fields.flags & 1 ? 4 : 0));
            // Only the first fixed-width sample record; never loop sampleCount.
            const first = {}, sampleStart = at + required;
            if (fields.version > 1 || fields.flags & ~0xf05 || !run.sampleCount) first.dropped = "first_sample_unsupported";
            else {
              let next = sampleStart;
              for (const [bit, name] of [[0x100, "duration"], [0x200, "size"], [0x400, "flags"], [0x800, "compositionOffset"]]) {
                if (!(fields.flags & bit)) continue;
                if (next + 4 > stop) { first.dropped = "first_sample_truncated"; break; }
                first[name] = bit === 0x800 && fields.version === 1 ? view.getInt32(next) : view.getUint32(next); next += 4;
              }
            }
            run.firstSample = first; traf.truns.push(run);
          }
        });
      });
      return { moofBytes: bytes.length, trafCount: trafs.length, trafs };
    } catch (reason) {
      return dropped(["node_limit", "box_header_invalid", "box_size_invalid", "traf_limit", "duplicate_tfhd", "tfhd_truncated", "trun_truncated", "duplicate_tfdt", "tfdt_truncated", "tfdt_unsupported", "tfdt_unsafe_integer"].includes(reason) ? reason : "invalid");
    }
  };

  const recordFragmentSummary = (summary, timing) => {
    if (summary.dropped) return { ...summary };
    return { ...summary, trafs: summary.trafs.map(traf => {
      const defaults = timing.tracks?.find(item => item.trackId === traf.tfhd?.trackId);
      return { ...traf, timescale: defaults?.timescale ?? null, kind: defaults?.kind ?? null, truns: traf.truns.map(run => {
        const first = run.firstSample;
        if (first?.dropped) return run;
        const flags = first?.flags ?? run.firstSampleFlags ?? traf.tfhd?.defaultFlags ?? defaults?.defaultFlags ?? null;
        return { ...run, firstSample: { ...first,
          effectiveDuration: first?.duration ?? traf.tfhd?.defaultDuration ?? defaults?.defaultDuration ?? null,
          effectiveSize: first?.size ?? traf.tfhd?.defaultSize ?? defaults?.defaultSize ?? null,
          effectiveFlags: flags, randomAccess: flags === null ? null : (flags & 0x10000) === 0 && ((flags >>> 24) & 3) === 2 } };
      }) };
    }) };
  };

  const safeNotify = (current, detail) => {
    lastDetail = detail;
    try { current?.options.onStatus?.(detail); } catch { /* UI callbacks never affect playback. */ }
  };
  const supportedMime = (type) => {
    if (rawFraming?.role(type) === "video") return true;
    if (typeof type !== "string" || type.length > 120) return false;
    const normalized = type.toLowerCase().replace(/[\s"]/g, "");
    const match = /^(video|audio)\/mp4;codecs=(.+)$/.exec(normalized);
    if (!match) return false;
    const codecs = match[2].split(",");
    return codecs.length >= 1 && codecs.length <= 2 && new Set(codecs).size === codecs.length &&
      codecs.every((codec) => /^(avc[13]\.[a-f0-9]{6}|mp4a\.40\.2)$/.test(codec)) &&
      (match[1] !== "audio" || (codecs.length === 1 && codecs[0] === "mp4a.40.2"));
  };
  const configurationOK = (state) => {
    try { return state.buffer.mode === "segments" && Number.isFinite(state.buffer.timestampOffset) && Math.abs(state.buffer.timestampOffset) <= 1e9 &&
      state.buffer.appendWindowStart === 0 && state.buffer.appendWindowEnd === Infinity; }
    catch { return false; }
  };
  const rawConfigurationOK = state => configurationOK(state) && state.buffer.timestampOffset === 0;
  const integrity = () => installed && hooks.every(({ target, name, wrapped }) => target[name] === wrapped);
  const sourceForVideo = (video) => {
    try {
      // Decoder readiness can drop during catch-up/rebuffering while the same
      // compressed stream is still arriving. It is NOT source identity.
      if (!video || video.isConnected === false || video.mediaKeys || video.srcObject) return null;
      const source = urls.get(video.currentSrc || video.src);
      return source && source.channelId === channel() ? source : null;
    } catch { return null; }
  };
  const candidate = (video) => {
    if (!video || video.readyState < 2 || video.ended || document.querySelector("#movie_player.ad-showing, #movie_player.ad-interrupting")) return null;
    const source = sourceForVideo(video);
    if (!integrity() || !source || source.blocked || source.buffers.length < 1 || source.buffers.length > 2) return null;
    try { if (source.mediaSource.sourceBuffers.length !== source.buffers.length) return null; } catch { return null; }
    if (source.buffers.some(state => state.rawRole === "video")) {
      return rawCaptureEnabled && source.buffers.length === 2 &&
        source.buffers.filter(state => state.rawRole === "video").length === 1 &&
        source.buffers.filter(state => state.rawRole === "audio").length === 1 &&
        source.buffers.every(state => !state.blocked && state.rawFramer?.init && !state.rawFramer.failed && rawConfigurationOK(state)) ? source : null;
    }
    if (source.buffers.some((state) => state.blocked || !state.init || state.awaitingInit || state.initParts.length || !configurationOK(state))) return null;
    // MIME is only an early eligibility check. Native init/sample validation,
    // including real A/V tracks and encryption rejection, remains authoritative.
    const declared = source.buffers.map((state) => state.mimeType.toLowerCase()).join(",");
    return /avc[13]\./.test(declared) && /mp4a\.40\.2/.test(declared) ? source : null;
  };
  const selectedVideo = () => {
    const video = document.querySelector("#movie_player video");
    return video?.tagName === "VIDEO" && video.isConnected !== false && video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0 && !video.ended ? video : null;
  };
  // Automatic starts have no human watching the initial adaptive-quality ramp.
  // Wait on this same observed receiver, without changing manual eligibility.
  const readyForAutomatic = (video = selectedVideo()) => {
    const source = candidate(video);
    if (!source || video.paused !== false || video.seeking || !Number.isFinite(video.currentTime)) {
      if (source) source.automaticPlayback = null;
      return false;
    }
    const now = Date.now();
    let playback = source.automaticPlayback;
    if (!playback || playback.video !== video || video.currentTime < playback.last) {
      playback = { video, initial: video.currentTime, last: video.currentTime };
      source.automaticPlayback = playback;
    }
    playback.last = video.currentTime;
    if (source.buffers.some(state => state.rawRole === "video")) return video.currentTime >= playback.initial + 1 && source.buffers.every(state =>
      state.rawFramer.completed >= 2 && state.rawFramer.lastCompleteAt !== null && now - state.rawFramer.lastCompleteAt <= 10000);
    return video.currentTime >= playback.initial + 1 && source.buffers.every(state =>
      state.initChangedAt !== null && now - state.initChangedAt >= 5000 &&
      state.completeFragments >= 2 && state.lastFragmentAt !== null && now - state.lastFragmentAt <= 10000);
  };
  // Only bounded capability facts. Never expose blob/media URLs, credentials,
  // page text or raw media in diagnostics.
  const diagnostics = () => {
    const video = active?.video ?? selectedVideo();
    const source = video ? sourceForVideo(video) : null;
    const sourcesList = [...observed].slice(0, 8);
    const rawSource = source?.buffers.some(state => state.rawRole === "video");
    const reason = !installed ? "mse_unavailable" : !integrity() ? "observer_changed" : !video ? "waiting_video" :
      video.mediaKeys ? "encrypted" : video.srcObject ? "source_object_unsupported" : !source ? "source_not_observed" :
      video.readyState < 2 ? "buffering" :
      source.blocked ? source.reason ?? "source_blocked" : !source.buffers.length ? "waiting_tracks" :
      source.buffers.some(state => state.blocked) ? source.buffers.find(state => state.blocked)?.reason ?? "buffer_blocked" :
      source.buffers.some(state => !configurationOK(state) || rawSource && !rawConfigurationOK(state)) ? "timeline_unsupported" :
      source.buffers.some(state => rawSource ? !state.rawFramer?.init : !state.init || state.awaitingInit || state.initParts.length) ? "waiting_init" : candidate(video) ? "ready" : "tracks_unsupported";
    return { version: 1, reason, installed, sourceCount: sourcesList.length, appendCount, appendBytes, lastStop, lastTransportFault, lastStart, latestRecord,
      width: video?.videoWidth ?? 0, height: video?.videoHeight ?? 0,
      queuedBytes: active?.queuedBytes ?? 0,
      maxAckMs,
      lastAppendAgoMs: lastAppendAt === null ? null : Math.max(0, Date.now() - lastAppendAt),
      sources: sourcesList.map(item => ({ selected: item === source, blocked: item.blocked, reason: item.reason ?? null,
        tracks: item.buffers.slice(0, 2).map(state => ({ mimeType: state.mimeType.slice(0, 120), blocked: state.blocked,
          reason: state.reason ?? null, initBytes: item.buffers.some(track => track.rawRole === "video") ? state.rawFramer?.init?.byteLength ?? 0 : state.init?.byteLength ?? 0,
          initTree: item.buffers.some(track => track.rawRole === "video") ? [] : initTree(state.init), unsupportedBox: state.unsupportedBox ?? null,
          mode: state.buffer.mode === "segments" ? "segments" : "sequence", timestampOffset: Number.isFinite(state.buffer.timestampOffset) ? state.buffer.timestampOffset : null })) })) };
  };
  const status = () => {
    const video = active?.video ?? selectedVideo();
    const ready = Boolean(candidate(video));
    return { active: Boolean(active || recovering), recording: Boolean(active || recovering), starting: Boolean(recovering || active && (!active.recordingId || active.raw && !active.rawReady)),
      stopping: Boolean(active?.stopping), recordingId: active?.recordingId ?? null, channelId: active?.channelId ?? channel() ?? null,
      detail: recovering ? "encoded_starting" : active ? active.stopping ? "encoded_saving" : active.raw && !active.rawReady ? "encoded_starting" : active.recordingId ? active.source.lastAppendAt !== null && Date.now() - active.source.lastAppendAt > 15_000 ? "encoded_waiting" : "encoded_recording" : "encoded_starting" : ready ? "encoded_ready" : lastDetail,
      ready, captureChat: active?.captureChat === true, captureMode: "encoded", rateControlAllowed: canChangePlaybackRate(video) };
  };
  const canChangePlaybackRate = (video) => Boolean(active && !active.raw && active.video === video && active.nativeApproved && active.accepting && !active.transportFailed && !active.stopping &&
    active.recordingId && candidate(video) === active.source);
  const getReplayClock = (video) => {
    if (!canChangePlaybackRate(video) || video.seeking) return null;
    const videoBuffer = active.source.buffers.find(state => /avc[13]\./i.test(state.mimeType));
    if (!videoBuffer || !configurationOK(videoBuffer)) return null;
    const sourceTimeSeconds = video.currentTime - videoBuffer.buffer.timestampOffset;
    if (!Number.isFinite(sourceTimeSeconds) || sourceTimeSeconds < 0 || sourceTimeSeconds > 1e9) return null;
    // MSE adds timestampOffset to coded PTS. Undo that display-only offset;
    // never modify the recorded samples. Native muxing subtracts DTS and keeps
    // CTS: output PTS = source PTS - origin (not PTS == DTS).
    return { clock: "mse_presentation_v1", sourceId: active.source.sourceId, sourceTimeSeconds };
  };
  const bytesOf = (value) => {
    try {
      if (ArrayBuffer.isView(value)) {
        if (typeof SharedArrayBuffer !== "undefined" && value.buffer instanceof SharedArrayBuffer) return null;
        return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      }
      return value instanceof ArrayBuffer ? new Uint8Array(value) : null;
    } catch { return null; }
  };
  const base64 = (bytes) => {
    let value = "";
    for (let at = 0; at < bytes.length; at += 8192) value += String.fromCharCode(...bytes.subarray(at, at + 8192));
    return btoa(value);
  };
  const failure = (current, reason) => {
    if (!current) return;
    if (!current.raw && ["init_changed", "codec_changed"].includes(reason)) beginContinuity(current);
    else if (continuity?.owner === current) discardContinuity(true);
    if (!current.interrupted) lastStop = { reason, readyState:current.video.readyState, paused:Boolean(current.video.paused),
      ended:Boolean(current.video.ended), sourceAttached:sourceForVideo(current.video) === current.source, sourceClosed:current.source.blocked };
    current.interrupted = true;
    if (!current.reason || current.reason === "user_stop") current.reason = reason;
    if (current.raw) { current.rawDraining = true; flushRawBucket(current); }
    current.accepting = false;
    if (current.raw) for (const state of current.source.buffers) state.rawFramer?.disarm();
    // A failure raised by a queue operation must not await that same queue.
    void Promise.resolve().then(() => finish(current)).catch(() => undefined);
  };
  const block = (state, reason) => {
    state.blocked = true;
    state.reason = reason;
    if (active?.source === state.source) failure(active, reason);
  };
  const makeSource = (mediaSource) => {
    let source = sources.get(mediaSource);
    if (source) return source;
    if (observed.size >= 8) return null;
    source = { mediaSource, sourceId: crypto.randomUUID(), channelId: channel(), buffers: [], blocked: false, lastAppendAt: null, automaticPlayback: null };
    sources.set(mediaSource, source); observed.add(source);
    mediaSource.addEventListener("sourceclose", () => {
      source.blocked = true; observed.delete(source);
      for (const [url, bound] of urls) if (bound === source) urls.delete(url);
      if (active?.source === source) failure(active, "source_changed");
    });
    return source;
  };
  const makeBuffer = (source, buffer, mimeType) => {
    if (source.buffers.length >= 2 || !supportedMime(mimeType)) { source.blocked = true; source.reason = "codec_unsupported"; return; }
    const state = { source, buffer, trackIndex: source.buffers.length, mimeType, blocked: false, header: new Uint8Array(8), headerBytes: 0,
      kind: null, remaining: 0, initParts: [], initSize: 0, ftyp: null, init: null, awaitingInit: false, forwarding: false, fragmentPending: false, fragmentDiagnostic: null,
      initChangedAt: null, completeFragments: 0, lastFragmentAt: null,
      rawRole: rawFraming?.role(mimeType) ?? null, rawFramer: null };
    if (state.rawRole) state.rawFramer = rawFraming.create(state.rawRole);
    source.buffers.push(state); buffers.set(buffer, state);
    buffer.addEventListener("error", () => block(state, "source_buffer_error"));
    if (active?.source === source) failure(active, "track_changed");
  };
  // Native receipts deduplicate the immediately previous, byte-identical
  // chunk. Keep its indexes/payload unchanged and do not advance the serial
  // queue before ACK. Never retry a parser, ownership or disk error.
  const appendWithRetry = async (current, fields, fragments) => {
    const retryStartedAt = Date.now();
    for (let attempt = 0; ; attempt++) {
      const requestedAt = Date.now();
      try { const result = await current.options.request("encoded_append", fields); maxAckMs = Math.max(maxAckMs, Date.now() - requestedAt); return result; }
      catch (error) {
        lastTransportFault = { code: error?.code ?? "BRIDGE_POST_FAILED", attempt, at: Date.now(),
          message: typeof error?.diagnosticMessage === "string" && error.diagnosticMessage.length <= 160 ? error.diagnosticMessage : null,
          appendIndex: fields.appendIndex, chunkIndex: fields.chunkIndex, queuedBytes: current.queuedBytes, fragments };
        if (attempt >= 32 || Date.now() - retryStartedAt >= 60_000 ||
            !["BRIDGE_BUSY", "BROWSER_ACK_TIMEOUT"].includes(error?.code)) throw error;
        await new Promise(resolve => setTimeout(resolve, Math.min(100 * 2 ** Math.min(attempt, 5), 2000)));
      }
    }
  };
  const enqueue = (current, state, parts, byteLength, fragmentDiagnostics) => {
    if (!current.accepting || byteLength === 0) return;
    if (byteLength > MAX_APPEND || current.queuedBytes + byteLength > MAX_QUEUE) { failure(current, "queue_overflow"); return; }
    current.queuedBytes += byteLength;
    let bytes;
    try { bytes = new Uint8Array(byteLength); let at = 0; for (const part of parts) { bytes.set(part, at); at += part.byteLength; } }
    catch { current.queuedBytes -= byteLength; failure(current, "queue_overflow"); return; }
    const trackIndex = state.trackIndex;
    const appendIndex = current.appendIndexes[trackIndex]++;
    // Snapshot now, not after ACK: later MSE appends can finish another moof.
    const fragments = fragmentDiagnostics.map(item => JSON.parse(JSON.stringify(item.summary)));
    current.queue = current.queue.then(async () => {
      await current.begin;
      if (current.transportFailed) return;
      let chunkIndex = 0;
      for (let at = 0; at < bytes.length; at += CHUNK) {
        const end = Math.min(bytes.length, at + CHUNK);
        await appendWithRetry(current, { recordingId: current.recordingId, trackIndex,
          appendIndex, chunkIndex: chunkIndex++, finalChunk: end === bytes.length, data: base64(bytes.subarray(at, end)) }, fragments);
      }
      const diagnostic = current.diagnostics.tracks[trackIndex];
      diagnostic.acknowledgedAppends = increment(diagnostic.acknowledgedAppends);
    }).catch((error) => { current.transportFailed = true;
      failure(current, ({ BROWSER_ACK_TIMEOUT:"bridge_ack_timeout", BRIDGE_BUSY:"bridge_busy", BRIDGE_POST_FAILED:"bridge_post_failed" })[error?.code] ?? "native_rejected"); })
      .finally(() => { current.queuedBytes -= byteLength; });
  };
  const rawQueue = (current, work, release = () => {}, reserved = false) => {
    if (current.transportFailed || !reserved && (!current.accepting || current.rawDraining)) return false;
    if (!reserved) {
      if (current.rawPending >= 512) { failure(current, "queue_overflow"); return false; }
      current.rawPending++;
    }
    current.queue = current.queue.then(async () => {
      await current.begin;
      if (!current.transportFailed) await work();
    }).catch(() => {
      current.transportFailed = true;
      failure(current, current.rawInitChanged ? "init_changed" : "native_rejected");
    }).finally(() => { current.rawPending--; release(); });
    return true;
  };
  const enqueueRawInit = (current, state, event) => {
    flushRawBucket(current);
    if (!current.accepting || current.rawDraining || current.transportFailed) return;
    const size = event.init.length;
    if (event.unitIndex === 0) current.rawInitialLengths[state.trackIndex] = size;
    if (current.queuedBytes + size > MAX_QUEUE) { failure(current, "queue_overflow"); return; }
    const initialization = event.init, unitIndex = event.unitIndex, startOffset = event.startOffset;
    current.queuedBytes += size;
    if (!rawQueue(current, async () => {
      const response = await current.options.request("encoded_raw_init", { recordingId: current.recordingId,
        trackIndex: state.trackIndex, unitIndex, startOffset, data: base64(initialization) });
      if (response?.accepted === false && response.changed === true) current.rawInitChanged = true;
      if (response?.accepted !== true || response.changed !== false) throw new Error("raw_init_rejected");
    }, () => { current.queuedBytes -= size; })) current.queuedBytes -= size;
  };
  // Sealed bytes already own their byte reservation. A staged bucket also owns
  // one pending slot, which is transferred even when a local stop closes input.
  const queueRawOwned = (current, state, owned, hint, reserved = false) => {
    const size = owned.length;
    const appendIndex = current.appendIndexes[state.trackIndex]++;
    if (!rawQueue(current, async () => {
      for (let offset = 0, chunkIndex = 0; offset < owned.length; offset += CHUNK, chunkIndex++) {
        const part = owned.subarray(offset, offset + CHUNK);
        await appendWithRetry(current, { recordingId: current.recordingId, trackIndex: state.trackIndex,
          appendIndex, chunkIndex, finalChunk: offset + part.length === owned.length, data: base64(part),
          ...(chunkIndex === 0 && hint !== undefined ? { rawFirstUnitEnd: hint } : {}) }, []);
        current.rawBytes[state.trackIndex] += part.length;
        if (current.rawBytes[state.trackIndex] > current.rawInitialLengths[state.trackIndex]) current.rawTracks.add(state.trackIndex);
        if (chunkIndex === 0 && hint !== undefined) current.rawBoundary = true;
        if (!current.rawReady && current.rawTracks.size === 2 && current.rawBoundary) {
          current.rawReady = true; current.starting = false;
          if (!current.stopping) safeNotify(current, "encoded_recording");
        }
      }
    }, () => { current.queuedBytes -= size; }, reserved)) {
      current.queuedBytes -= size;
      if (reserved) current.rawPending--;
    }
  };
  const discardRawBucket = (current) => {
    const bucket = current.rawBucket;
    if (!bucket) return;
    current.rawBucket = null; clearTimeout(bucket.timer);
    current.queuedBytes -= bucket.size; current.rawPending--;
  };
  const flushRawBucket = (current) => {
    const bucket = current.rawBucket;
    if (!bucket) return;
    if (current.transportFailed) { discardRawBucket(current); return; }
    // Detach before queue installation; failure/stop cannot seal it twice.
    current.rawBucket = null; clearTimeout(bucket.timer);
    queueRawOwned(current, bucket.state, bucket.bytes.subarray(0, bucket.size), undefined, true);
  };
  const enqueueRaw = (current, state, parts, size, hint, unitIndex) => {
    if (!size || !current.accepting || current.rawDraining || current.transportFailed) return;
    const previous = current.rawBucket;
    if (previous && (previous.state !== state || previous.unitIndex !== unitIndex ||
        Date.now() >= previous.expires || previous.size + size > CHUNK || hint !== undefined)) flushRawBucket(current);
    if (size > MAX_APPEND + MAX_INIT || current.queuedBytes + size > MAX_QUEUE) { failure(current, "queue_overflow"); return; }
    if (hint !== undefined || size >= CHUNK) {
      flushRawBucket(current);
      let owned;
      try { owned = new Uint8Array(size); let at = 0; for (const part of parts) { owned.set(part, at); at += part.length; } }
      catch { failure(current, "queue_overflow"); return; }
      current.queuedBytes += size; queueRawOwned(current, state, owned, hint);
      return;
    }
    let bucket = current.rawBucket;
    if (!bucket) {
      if (current.rawPending >= 512) { failure(current, "queue_overflow"); return; }
      let bytes;
      try { bytes = new Uint8Array(size); } catch { failure(current, "queue_overflow"); return; }
      bucket = { state, unitIndex, bytes, size: 0, expires: Date.now() + 250, timer: null };
      current.rawBucket = bucket; current.rawPending++;
      const exact = bucket;
      bucket.timer = setTimeout(() => {
        if (active !== current || current.rawBucket !== exact || !current.nativeApproved || current.rawDraining || !current.accepting) return;
        const reason = safetyReason(current);
        if (reason) failure(current, reason); else flushRawBucket(current);
      }, 250);
    } else {
      // Keep queued backing allocations equal to their charged byte lengths,
      // rather than retaining a full128KiB slab for every tiny sealed bucket.
      let bytes;
      try { bytes = new Uint8Array(bucket.size + size); bytes.set(bucket.bytes); }
      catch { failure(current, "queue_overflow"); return; }
      bucket.bytes = bytes;
    }
    // The only retained copy is ours, made before the original caller returns.
    let at = bucket.size;
    for (const part of parts) { bucket.bytes.set(part, at); at += part.length; }
    bucket.size += size; current.queuedBytes += size;
    if (bucket.size === CHUNK) flushRawBucket(current);
  };
  const observeRaw = (state, bytes, current) => {
    let parts = [], size = 0, hint, unitIndex;
    const flush = () => {
      if (current && size) enqueueRaw(current, state, parts, size, hint, unitIndex);
      parts = []; size = 0; hint = undefined; unitIndex = undefined;
    };
    try {
      state.rawFramer.feed(bytes, event => {
        if (!current?.nativeApproved || !current.accepting || current.rawDraining) return;
        if (event.kind === "init") { flush(); enqueueRawInit(current, state, event); }
        else { parts.push(event.bytes); size += event.bytes.length; hint ??= event.rawFirstUnitEnd; unitIndex ??= state.rawFramer.unitIndex; }
      });
      flush();
    } catch {
      flush(); block(state, "container_unsupported");
    }
  };
  // This scratch exists only after native approval, on the exact original
  // receiver. Keep one completed fragment and the unfinished prefix per track
  // while recording; only a format boundary expands it into a bounded pocket.
  // It never fetches another stream or makes unapproved media durable.
  const discardContinuity = (cancel = false) => {
    const pocket = continuity;
    if (!pocket) return;
    continuity = null; clearInterval(pocket.timer);
    if (cancel) { pocket.owner.cancelContinuation = true; if (pocket.pendingApproval) pocket.owner.transportFailed = true; }
    pocket.entries.length = 0; pocket.tracks.clear(); pocket.bytes = 0; pocket.pieces = 0;
  };
  const continuityFault = (reason) => {
    const pocket = continuity;
    if (!pocket) return;
    const owner = pocket.owner;
    // A successor whose begin ACK is still pending may not flush its pocket
    // after the transition deadline or a revoked source/stop condition.
    if (pocket.pendingApproval) owner.transportFailed = true;
    owner.continuityFailure = reason; discardContinuity(true);
    if (recovering?.current === owner) { clearInterval(recovering.timer); recovering = null; }
    safeNotify(owner, "encoded_interrupted");
    if (active === owner && !owner.stopping) failure(owner, reason);
  };
  const makeContinuity = (current) => {
    discardContinuity();
    continuity = { owner: current, source: current.source, video: current.video, channelId: current.channelId,
      tracks: new Map(current.source.buffers.map(state => [state, { latest: null, fragment: null }])),
      entries: [], bytes: 0, pieces: 0, sequence: 0, transition: false, pendingApproval: false, expires: null, timer: null };
  };
  const checkContinuity = (pocket) => {
    if (continuity !== pocket || pocket.owner.cancelContinuation || pocket.owner.transportFailed ||
        sourceForVideo(pocket.video) !== pocket.source || channel() !== pocket.channelId ||
        pocket.video.ended || pocket.video.mediaKeys || !integrity() || pocket.source.blocked ||
        document.querySelector("#movie_player.ad-showing, #movie_player.ad-interrupting") ||
        pocket.source.buffers.length !== pocket.tracks.size ||
        pocket.source.buffers.some(state => !pocket.tracks.has(state) || state.blocked || !configurationOK(state))) return false;
    return pocket.expires === null || Date.now() < pocket.expires;
  };
  const beginContinuity = (current) => {
    const pocket = continuity;
    if (!current.nativeApproved || !pocket || pocket.owner !== current || !checkContinuity(pocket)) {
      current.cancelContinuation = true; return false;
    }
    if (!pocket.transition) {
      pocket.transition = true; pocket.expires = Date.now() + CONTINUITY_LIFETIME;
      pocket.entries = [...pocket.tracks.values()].flatMap(track => track.latest ? [track.latest] : []).sort((a, b) => a.sequence - b.sequence);
      pocket.timer = setInterval(() => { if (!checkContinuity(pocket)) continuityFault("timeline_changed"); }, 250);
    }
    return true;
  };
  const clearContinuityTrack = (state) => {
    const pocket = continuity, track = pocket?.tracks.get(state);
    if (!track) return;
    const removed = new Set(pocket.entries.filter(entry => entry.state === state));
    if (track.latest) removed.add(track.latest);
    if (track.fragment) removed.add(track.fragment);
    if (pocket.transition && [...removed].some(entry => entry.afterBoundary)) { continuityFault("timeline_changed"); return; }
    for (const entry of removed) { pocket.bytes -= entry.bytes; pocket.pieces -= entry.parts.length; }
    pocket.entries = pocket.entries.filter(entry => entry.state !== state);
    track.latest = null; track.fragment = null;
  };
  const queueContinuity = (current, fragment) => {
    // Full init equality is represented by immutable buffer identity inside
    // this document; native still performs its complete init/sample checks.
    if (fragment.init !== current.initSnapshot.get(fragment.state) || fragment.mimeType !== fragment.state.mimeType) {
      continuityFault("timeline_changed"); return false;
    }
    const summary = fragment.summary;
    const track = current.diagnostics.tracks[fragment.state.trackIndex];
    track.completeMoofs = increment(track.completeMoofs); track.completeMdats = increment(track.completeMdats);
    const timing = recordFragmentSummary(summary, track.init); track.first ??= timing; track.last = timing;
    enqueue(current, fragment.state, fragment.parts, fragment.bytes, [{ summary }]);
    return !current.interrupted;
  };
  const collectContinuity = (state, part, header = false) => {
    const pocket = continuity, track = pocket?.tracks.get(state);
    if (!pocket || !track) return;
    if (!checkContinuity(pocket)) { continuityFault("timeline_changed"); return; }
    if (header && state.kind === "moof") {
      if (track.fragment) { continuityFault("container_unsupported"); return; }
      track.fragment = { state, init: state.init, mimeType: state.mimeType, parts: [], bytes: 0,
        summary: { dropped: "incomplete" }, moofBytes: 0, sequence: pocket.sequence++, handoff: false, afterBoundary: false };
    }
    const fragment = track.fragment;
    if (!fragment || ["ftyp", "moov", "skip"].includes(state.kind)) return;
    if (fragment.init !== state.init || fragment.mimeType !== state.mimeType ||
        fragment.bytes + part.length > MAX_APPEND || pocket.bytes + part.length > MAX_CONTINUITY || pocket.pieces >= MAX_CONTINUITY_PIECES) {
      continuityFault("queue_overflow"); return;
    }
    try { fragment.parts.push(part.slice()); }
    catch { continuityFault("queue_overflow"); return; }
    fragment.bytes += part.length; pocket.bytes += part.length; pocket.pieces++;
    fragment.afterBoundary ||= pocket.transition;
    if (state.kind === "moof") fragment.moofBytes += part.length;
  };
  const completeContinuity = (state) => {
    const pocket = continuity, track = pocket?.tracks.get(state), fragment = track?.fragment;
    if (!fragment) return;
    if (fragment.moofBytes <= MAX_MOOF_DIAGNOSTIC) {
      try {
        const moof = new Uint8Array(fragment.moofBytes); let at = 0;
        for (const part of fragment.parts) { const length = Math.min(part.length, fragment.moofBytes - at); if (length <= 0) break; moof.set(part.subarray(0, length), at); at += length; }
        fragment.summary = moofShape(moof);
      } catch { continuityFault("queue_overflow"); return; }
    } else fragment.summary = { dropped: "size_limit" };
    track.fragment = null;
    if (pocket.transition) {
      if (pocket.entries.length >= MAX_CONTINUITY_FRAGMENTS) { continuityFault("queue_overflow"); return; }
      pocket.entries.push(fragment);
    } else if (track.latest) { pocket.bytes -= track.latest.bytes; pocket.pieces -= track.latest.parts.length; }
    track.latest = fragment;
    if (fragment.handoff && !pocket.transition && active === pocket.owner && pocket.owner.accepting) queueContinuity(pocket.owner, fragment);
  };
  const handoffContinuity = (pocket, current) => {
    pocket.owner = current; pocket.transition = false; pocket.pendingApproval = true;
    const entries = pocket.entries; pocket.entries = [];
    for (const track of pocket.tracks.values()) if (track.fragment) track.fragment.handoff = true;
    for (const entry of entries) {
      if (!queueContinuity(current, entry)) break;
      if (pocket.tracks.get(entry.state)?.latest !== entry) { pocket.bytes -= entry.bytes; pocket.pieces -= entry.parts.length; }
    }
    for (const track of pocket.tracks.values()) {
      if (track.latest) track.latest.afterBoundary = false;
      if (track.fragment) track.fragment.afterBoundary = false;
    }
  };
  const observeAppend = (state, value) => {
    appendCount += 1;
    appendBytes += value?.byteLength ?? 0;
    lastAppendAt = Date.now();
    state.source.lastAppendAt = lastAppendAt;
    if (state.blocked) return;
    if (!configurationOK(state)) { block(state, "timeline_changed"); return; }
    const bytes = bytesOf(value);
    if (!bytes || bytes.byteLength > MAX_APPEND) { block(state, "append_unsupported"); return; }
    const current = active?.source === state.source && active.accepting ? active : null;
    const rawSource = state.source.buffers.some(item => item.rawRole === "video");
    if (rawSource) {
      if (!state.rawFramer || !rawConfigurationOK(state)) { block(state, "timeline_changed"); return; }
      observeRaw(state, bytes, current?.raw ? current : null); return;
    }
    // AAC metadata may arrive before YouTube creates its VP9 SourceBuffer.
    // This observation has no media storage and never alters legacy AVC.
    if (state.rawFramer && !state.rawFramer.failed) { try { state.rawFramer.feed(bytes); } catch { /* Raw capability only is unavailable. */ } }
    const parts = []; let forwarded = 0; let at = 0;
    const recordedTrack = current?.diagnostics.tracks[state.trackIndex];
    const completeMoof = (entry) => {
      if (!entry || !recordedTrack || !state.forwarding || entry.complete) return;
      entry.complete = true; recordedTrack.completeMoofs = increment(recordedTrack.completeMoofs);
      if (entry.bytes) { entry.summary = moofShape(entry.bytes); entry.bytes = null; }
    };
    const fragmentDiagnostics = [];
    const rememberFragment = (entry) => {
      if (!entry || !current || fragmentDiagnostics.includes(entry)) return;
      if (fragmentDiagnostics.length < 4) fragmentDiagnostics.push(entry);
      else if (fragmentDiagnostics.length === 4) fragmentDiagnostics.push({ summary: { dropped: "append_fragment_limit" } });
    };
    if (state.fragmentPending && state.forwarding) rememberFragment(state.fragmentDiagnostic);
    while (at < bytes.length) {
      if (state.remaining === 0) {
        const count = Math.min(8 - state.headerBytes, bytes.length - at);
        state.header.set(bytes.subarray(at, at + count), state.headerBytes); state.headerBytes += count; at += count;
        if (state.headerBytes < 8) break;
        const size = new DataView(state.header.buffer).getUint32(0);
        const kind = String.fromCharCode(...state.header.subarray(4));
        if (size < 8 || size > MAX_APPEND || !["ftyp", "moov", "moof", "mdat", "styp", "sidx", "emsg", "prft", "free", "skip"].includes(kind) || kind === "skip" && state.fragmentPending) {
          // FourCC/length only: no media sample, URL, account or chat content.
          state.unsupportedBox = { type: /^[A-Za-z0-9 ]{4}$/.test(kind) ? kind : "non_ascii", size, appendBytes: bytes.length, offset: at - 8 };
          block(state, "container_unsupported"); return;
        }
        state.kind = kind; state.remaining = size - 8; state.headerBytes = 0;
        if (kind === "ftyp" || kind === "moov") {
          state.awaitingInit = true;
          if (size > MAX_INIT || (kind === "moov" && (!state.ftyp || state.ftyp.length + size > MAX_INIT))) { block(state, "init_unsupported"); return; }
          state.initParts = [state.header.slice()]; state.initSize = 8;
        } else {
          if (state.awaitingInit) { block(state, "init_unsupported"); return; }
          if (kind === "moof") {
            state.forwarding = Boolean(current?.accepting); state.fragmentPending = true;
            state.fragmentDiagnostic = null;
            if (current) {
              const entry = { summary: { moofBytes: size, dropped: size > MAX_MOOF_DIAGNOSTIC ? "size_limit" : "incomplete" }, bytes: null, offset: 8, complete: false };
              if (size <= MAX_MOOF_DIAGNOSTIC) {
                try { entry.bytes = new Uint8Array(size); entry.bytes.set(state.header); }
                catch { entry.summary = { moofBytes: size, dropped: "allocation_failed" }; }
              }
              state.fragmentDiagnostic = entry; rememberFragment(entry);
              if (state.remaining === 0) completeMoof(entry);
            }
          }
          // YouTube inserts ISO BMFF padding after its init. Omit only this
          // explicit non-media box outside a fragment. Never remove padding
          // between moof and mdat: trun offsets could include those bytes.
          if (kind !== "skip" && current && state.forwarding) { parts.push(state.header.slice()); forwarded += 8; }
          collectContinuity(state, state.header, true);
        }
        if (state.remaining === 0) {
          if (kind === "ftyp" || kind === "moov") { block(state, "init_unsupported"); return; }
          continue;
        }
      }
      const count = Math.min(state.remaining, bytes.length - at);
      const part = bytes.subarray(at, at + count);
      if (state.kind === "moof" && state.fragmentDiagnostic?.bytes) {
        const entry = state.fragmentDiagnostic; entry.bytes.set(part, entry.offset); entry.offset += count;
      }
      if (state.kind === "ftyp" || state.kind === "moov") {
        state.initParts.push(part.slice()); state.initSize += count;
      } else if (state.kind !== "skip" && current && state.forwarding) { parts.push(part); forwarded += count; }
      if (state.kind !== "ftyp" && state.kind !== "moov") collectContinuity(state, part);
      at += count; state.remaining -= count;
      if (state.remaining === 0 && state.kind === "moof") completeMoof(state.fragmentDiagnostic);
      if (state.remaining === 0 && state.kind === "mdat" && state.fragmentPending && state.forwarding && recordedTrack) {
        recordedTrack.completeMdats = increment(recordedTrack.completeMdats);
        const summary = recordFragmentSummary(state.fragmentDiagnostic?.summary ?? { dropped: "moof_unobserved" }, recordedTrack.init);
        recordedTrack.first ??= summary; recordedTrack.last = summary;
      }
      if (state.remaining === 0 && state.kind === "mdat") {
        completeContinuity(state);
        if (state.fragmentPending) {
          state.completeFragments = increment(state.completeFragments);
          state.lastFragmentAt = Date.now();
        }
        state.fragmentPending = false;
      }
      if (state.remaining === 0 && (state.kind === "ftyp" || state.kind === "moov")) {
        const complete = new Uint8Array(state.initSize); let offset = 0;
        for (const part of state.initParts) { complete.set(part, offset); offset += part.length; }
        state.initParts = []; state.initSize = 0;
        if (state.kind === "ftyp") { state.ftyp = complete; }
        else {
          const next = new Uint8Array(state.ftyp.length + complete.length); next.set(state.ftyp); next.set(complete, state.ftyp.length);
          const changed = !state.init || state.init.length !== next.length || state.init.some((value, index) => value !== next[index]);
          if (current && state.init && changed) {
            current.initChange ??= initChange(state.init, next, state.trackIndex);
            // This append may contain complete old media before the new init.
            // Queue that old prefix before revoking acceptance. New init/media
            // never enter the old archive, including the rest of this append.
            enqueue(current, state, parts, forwarded, fragmentDiagnostics);
            parts.length = 0; forwarded = 0; fragmentDiagnostics.length = 0;
            failure(current, "init_changed");
            state.forwarding = false;
          }
          if (changed) {
            clearContinuityTrack(state);
            state.initChangedAt = Date.now(); state.completeFragments = 0; state.lastFragmentAt = null;
            state.source.automaticPlayback = null;
          }
          // Preserve immutable identity for byte-identical repeated init.
          if (changed) state.init = next;
          state.awaitingInit = false;
        }
      }
    }
    if (current && forwarded) enqueue(current, state, parts, forwarded, fragmentDiagnostics);
  };
  const hook = (target, name, create) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    if (!descriptor || typeof descriptor.value !== "function") throw new Error("hook_unavailable");
    const wrapped = create(descriptor.value);
    Object.defineProperty(target, name, { ...descriptor, value: wrapped }); hooks.push({ target, name, wrapped, descriptor });
  };
  try {
    if (!NativeMediaSource || !NativeSourceBuffer || !NativeURL) throw new Error("mse_unavailable");
    hook(NativeURL, "createObjectURL", (original) => function createObjectURL(value) {
      const url = Reflect.apply(original, this, arguments);
      try { if (value instanceof NativeMediaSource && urls.size < 16) { const source = makeSource(value); if (source) urls.set(url, source); } } catch { /* Original result is preserved. */ }
      return url;
    });
    hook(NativeMediaSource.prototype, "addSourceBuffer", (original) => function addSourceBuffer(type) {
      const buffer = Reflect.apply(original, this, arguments);
      try { const source = makeSource(this); if (source) makeBuffer(source, buffer, type); } catch { /* Original result is preserved. */ }
      return buffer;
    });
    hook(NativeSourceBuffer.prototype, "appendBuffer", (original) => function appendBuffer(value) {
      const result = Reflect.apply(original, this, arguments);
      // Read/copy after the browser accepted the append, before returning to
      // its caller. Do not detach/reuse/mutate the caller's ArrayBuffer.
      try { const state = buffers.get(this); if (state) observeAppend(state, value); } catch { const state = buffers.get(this); if (state) block(state, "observer_failed"); }
      return result;
    });
    for (const name of ["changeType"]) if (typeof NativeSourceBuffer.prototype[name] === "function") hook(NativeSourceBuffer.prototype, name, (original) => function (mimeType) {
      const result = Reflect.apply(original, this, arguments);
      const state = buffers.get(this);
      if (state && String(mimeType).toLowerCase().replace(/[\s"]/g, "") !== state.mimeType.toLowerCase().replace(/[\s"]/g, "")) {
        if (state.source.buffers.some(item => item.rawRole === "video")) {
          block(state, "codec_changed"); return result;
        }
        if (!supportedMime(mimeType)) block(state, "codec_unsupported");
        else {
          if (active?.source === state.source) failure(active, "codec_changed");
          clearContinuityTrack(state);
          state.mimeType = mimeType; state.init = null; state.ftyp = null; state.awaitingInit = true;
          state.initParts = []; state.initSize = 0; state.headerBytes = 0; state.remaining = 0; state.forwarding = false; state.fragmentPending = false;
          state.fragmentDiagnostic = null;
          state.initChangedAt = null; state.completeFragments = 0; state.lastFragmentAt = null;
          state.source.automaticPlayback = null;
        }
      }
      return result;
    });
    if (typeof NativeMediaSource.prototype.removeSourceBuffer === "function") hook(NativeMediaSource.prototype, "removeSourceBuffer", (original) => function removeSourceBuffer(buffer) {
      const result = Reflect.apply(original, this, arguments); const state = buffers.get(buffer);
      if (state) {
        if (active?.source === state.source) failure(active, "track_changed");
        state.blocked = true; buffers.delete(buffer);
        state.source.buffers = state.source.buffers.filter(item => item !== state);
        state.source.buffers.forEach((item, index) => { item.trackIndex = index; });
      }
      return result;
    });
    installed = true;
  } catch {
    for (const { target, name, descriptor } of hooks.reverse()) { try { Object.defineProperty(target, name, descriptor); } catch { /* Capability remains disabled. */ } }
  }

  const safetyReason = (current) => {
    if (channel() !== current.channelId) return "channel_changed";
    if (document.querySelector("#movie_player.ad-showing, #movie_player.ad-interrupting")) return "advertisement_started";
    if (!integrity()) return "observer_changed";
    if (sourceForVideo(current.video) !== current.source || current.source.blocked) return "source_changed";
    if (current.source.buffers.some((state) => state.blocked || !configurationOK(state) || current.raw && !rawConfigurationOK(state))) return "timeline_changed";
    return null;
  };
  const cleanUp = (current) => {
    clearInterval(current.timer);
    if (current.raw) discardRawBucket(current);
    for (const remove of current.listeners) remove();
    for (const state of current.source.buffers) state.forwarding = false;
    if (current.raw) for (const state of current.source.buffers) state.rawFramer?.disarm();
    if (active === current) active = null;
  };
  const continueRecording = (current, command) => {
    const pocket = continuity;
    if (!command || !UUID.test(command.requestId ?? "") || cancelledRequests.has(command.requestId) || command.channelId !== current.channelId || current.cancelContinuation ||
        !pocket || pocket.owner !== current || !pocket.transition || !checkContinuity(pocket)) return false;
    const recovery = { current, command, expires: pocket.expires, timer: null };
    recovering = recovery;
    safeNotify(current, "encoded_starting");
    recovery.timer = setInterval(() => {
      if (recovering !== recovery) { clearInterval(recovery.timer); return; }
      if (Date.now() >= recovery.expires || channel() !== current.channelId || current.video.ended || current.video.mediaKeys || !integrity()) {
        clearInterval(recovery.timer); recovering = null; discardContinuity(true); safeNotify(current, "encoded_interrupted"); return;
      }
      // Stay on the approved document/player. No reconnect, alternate URL or
      // legacy decoder fallback. A fresh native arm validates the new init.
      if (candidate(current.video) !== current.source || command.automatic === true && !readyForAutomatic(current.video)) return;
      clearInterval(recovery.timer); recovering = null;
      void start({ ...command, video: current.video }, current.options, pocket).catch(() => { discardContinuity(true); safeNotify(current, "encoded_interrupted"); });
    }, 250);
    return true;
  };
  const finish = (current) => {
    if (current.finishPromise) return current.finishPromise;
    if (current.raw) {
      current.rawDraining = true; flushRawBucket(current);
      if (current.source.buffers.some(state => state.rawFramer?.capturing && (state.rawFramer.collecting || state.rawFramer.header.length))) {
        current.interrupted = true; current.reason ??= "init_unsupported";
      }
      for (const state of current.source.buffers) state.rawFramer?.disarm();
    }
    current.accepting = false; current.stopping = true; safeNotify(current, "encoded_saving");
    current.finishPromise = (async () => {
      try {
        await current.begin;
        await current.queue;
        await current.options.beforeFinish?.(current.recordingId);
        const response = await current.options.request("encoded_finish", { recordingId: current.recordingId,
          interrupted: current.interrupted || current.transportFailed,
          reason: current.continuityFailure ?? (current.transportFailed && !current.rawInitChanged && ["track_changed", "codec_changed", "init_changed"].includes(current.reason) ? "native_rejected" : current.reason ?? "user_stop"),
          ...(!current.transportFailed && current.reason === "init_changed" && current.initChange ? { initChange: current.initChange } : {}) });
        if (!response || response.stopped !== true) throw new Error("native_rejected");
        cleanUp(current);
        if (current.transportFailed || !continueRecording(current, response.continuation)) {
          if (continuity?.owner === current) discardContinuity(true);
          // An ended video is provisional until native checks the broadcast.
          // Do not turn that pending classification into a persistent IPC error.
          const ending = current.transportFailed ? null : response.endingReason;
          safeNotify(current, ending === "checking" ? "encoded_end_checking" :
            ["broadcast_ended", "broadcast_changed"].includes(ending) ? "encoded_saved" :
            response.interrupted || current.interrupted || current.transportFailed ? "encoded_interrupted" : "encoded_saved");
        }
        return response;
      } catch (cause) { cleanUp(current); if (continuity?.owner === current) discardContinuity(true); safeNotify(current, "native_rejected"); throw cause; }
    })();
    return current.finishPromise;
  };
  const start = (command, options, pocket = null) => {
    if (active || recovering) return Promise.reject(new Error("recording_active"));
    const video = command?.video ?? selectedVideo(); const source = candidate(video);
    if (!source || command?.channelId !== channel() || command.rightsAcknowledged !== true || !UUID.test(command.requestId ?? "") || cancelledRequests.has(command.requestId) || typeof options?.request !== "function" || command.automatic === true && !readyForAutomatic(video)) {
      return Promise.reject(new Error("encoded_unavailable"));
    }
    if (pocket && (pocket !== continuity || !checkContinuity(pocket) || pocket.source !== source || !pocket.transition ||
        [...pocket.entries, ...[...pocket.tracks.values()].flatMap(track => track.fragment ? [track.fragment] : [])]
          .some(entry => entry.init !== entry.state.init || entry.mimeType !== entry.state.mimeType))) return Promise.reject(new Error("encoded_unavailable"));
    const raw = source.buffers.some(state => state.rawRole === "video");
    const initSnapshot = new Map(source.buffers.map(state => [state, raw ? state.rawFramer.init : state.init]));
    const initTracks = source.buffers.map(state => ({ trackIndex: state.trackIndex, mimeType: state.mimeType, init: base64(initSnapshot.get(state)) }));
    const current = { source, video, channelId: channel(), options, recordingId: null, captureChat: false, nativeApproved: false,
      raw, requestId: command.requestId, rawPending: 0, rawBucket: null, rawDraining: false, rawTracks: new Set(), rawBytes: [0, 0], rawInitialLengths: [Infinity, Infinity], rawBoundary: false, rawReady: false, rawStartedAt: null,
      accepting: true, starting: true, stopping: false, interrupted: false, transportFailed: false, reason: null,
      appendIndexes: source.buffers.map(() => 0), queuedBytes: 0, queue: Promise.resolve(), begin: null, finishPromise: null, listeners: [], timer: null, initSnapshot };
    current.diagnostics = { tracks: source.buffers.map(state => ({ trackIndex: state.trackIndex, init: initTiming(state.init),
      completeMoofs: 0, completeMdats: 0, acknowledgedAppends: 0, first: null, last: null })) };
    latestRecord = current.diagnostics;
    active = current; lastStart = null;
    for (const state of source.buffers) { state.forwarding = false; state.fragmentDiagnostic = null; }
    const listen = (name, reason) => { const callback = () => failure(current, reason); video.addEventListener(name, callback); current.listeners.push(() => video.removeEventListener(name, callback)); };
    // emptied can describe resetting just the decoder. The periodic identity
    // check still stops an actually replaced URL/MediaSource.
    listen("ended", "video_ended"); listen("encrypted", "encrypted");
    current.timer = setInterval(() => {
      const reason = safetyReason(current);
      if (reason && !current.stopping) failure(current, reason);
      else if (current.raw && !current.rawReady && current.rawStartedAt !== null && Date.now() - current.rawStartedAt >= 15000 && !current.stopping) failure(current, "first_append_timeout");
    }, 500);
    safeNotify(current, "encoded_starting");
    current.begin = Promise.resolve().then(() => options.request("encoded_begin", { requestId: command.requestId, channelId: current.channelId,
      title: String(document.title ?? "YouTube").slice(0, 200), sourceId: source.sourceId,
      tracks: initTracks, ...(raw ? { format: RAW_FORMAT, rawMode: "segments", rawVideoTimestampOffset: 0, rawAudioTimestampOffset: 0, rawAppendWindow: "default" } : {}) }))
      .then((response) => {
        if (!response || !recordingID(response.id) || response.mode !== "encoded" || response.nativeApproved !== true || raw && response.format !== RAW_FORMAT) throw new Error("encoded_not_approved");
        current.recordingId = response.id; current.captureChat = response.captureChat === true; current.nativeApproved = true; current.starting = false;
        if (raw) {
          current.starting = true; current.rawStartedAt = Date.now();
          if (!current.stopping && !cancelledRequests.has(command.requestId)) {
            try { for (const state of source.buffers) state.rawFramer.arm(); }
            catch { failure(current, "container_unsupported"); }
          }
        }
        if (pocket) {
          if (!checkContinuity(pocket) || pocket.owner !== current) { current.transportFailed = true; failure(current, "timeline_changed"); }
          else { pocket.pendingApproval = false; pocket.expires = null; clearInterval(pocket.timer); pocket.timer = null; }
        }
        const reason = safetyReason(current); if (reason && !current.stopping) failure(current, reason);
        if (!raw && !pocket && !current.stopping) makeContinuity(current);
        safeNotify(current, current.stopping ? "encoded_saving" : raw ? "encoded_starting" : "encoded_recording");
        if (!current.stopping) options.onStarted?.(response, video);
        return response;
      }).catch((cause) => {
        const code = ["NATIVE_REJECTED", "BROWSER_ACK_TIMEOUT", "BRIDGE_BUSY", "BRIDGE_POST_FAILED", "ENCODED_UNSUPPORTED"].includes(cause?.code) ? cause.code : "START_FAILED";
        // Native supplies diagnosticMessage only from an exact static allowlist.
        lastStart = { code, message: typeof cause?.diagnosticMessage === "string" && cause.diagnosticMessage.length <= 160 ? cause.diagnosticMessage : null };
        current.transportFailed = true; current.accepting = false; cleanUp(current); if (continuity?.owner === current) discardContinuity(true); safeNotify(current, "native_rejected");
        const error = new Error(cause?.message ?? "native_rejected"); error.nativeAttempted = true; error.code = cause?.code;
        // This exact code is issued by native init validation BEFORE consuming
        // the arm or opening a recording. No lost ACK/append/finish failure may
        // fall back: those may already own files or an active native session.
        if (cause?.code === "ENCODED_UNSUPPORTED") { source.blocked = true; source.reason = "native_init_unsupported"; if (!raw) error.allowLegacyFallback = true; }
        throw error;
      });
    if (pocket) handoffContinuity(pocket, current);
    return current.begin;
  };
  window.addEventListener("pagehide", () => { discardContinuity(true); if (recovering) { clearInterval(recovering.timer); recovering = null; } if (active) { active.cancelContinuation = true; failure(active, "page_hidden"); } });
  Object.defineProperty(window, "__atsumiYouTubeEncodedCapture", { value: Object.freeze({
    configure(config) { rawCaptureEnabled = config?.rawCaptureEnabled === true; },
    canStart: (video) => Boolean(candidate(video)), supports: (video) => Boolean(candidate(video)), readyForAutomatic, getDiagnostics: diagnostics,
    canChangePlaybackRate, getReplayClock, getStatus: status, start,
    cancelPending(requestId) {
      if (!UUID.test(requestId ?? "")) return false;
      if (cancelledRequests.size >= 32) cancelledRequests.delete(cancelledRequests.values().next().value);
      cancelledRequests.add(requestId);
      if (active?.raw && active.requestId === requestId && !active.recordingId) {
        active.cancelContinuation = true; failure(active, "user_stop"); return true;
      }
      if (recovering?.command.requestId === requestId) {
        clearInterval(recovering.timer); const current = recovering.current; recovering = null;
        discardContinuity(true);
        current.cancelContinuation = true; safeNotify(current, "encoded_ready"); return true;
      }
      return false;
    },
    stop(reason = "user_stop", interrupted = false) {
      discardContinuity(true);
      if (recovering) { clearInterval(recovering.timer); recovering = null; }
      if (!active) return Promise.resolve({ stopped: true, interrupted: false });
      active.cancelContinuation = true;
      if (!active.interrupted) active.reason = reason;
      active.interrupted ||= interrupted; return finish(active);
    },
  }) });
})();
