// One logical clock for both a completed file and incrementally merged parts.
// Adapt only this frame's video instance; the pinned SDK and controls stay intact.
const finite = (n, min, max) => typeof n === 'number' && Number.isFinite(n) && n >= min && n <= max;
export function recordingSource(data, allowed) {
  if (!data || !allowed(data.url) || !finite(data.duration, .001, 604800)) return null;
  if (data.parts !== undefined && (!Array.isArray(data.parts) || !data.parts.length || data.parts.length > 10000)) return null;
  const parts = data.parts ?? [{ index: 0, startSeconds: 0, durationSeconds: data.duration }];
  let end = 0;
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i];
    if (!p || p.index !== i || !finite(p.startSeconds, 0, 604800) || !finite(p.durationSeconds, .001, 604800) || Math.abs(p.startSeconds - end) > .01) return null;
    end = p.startSeconds + p.durationSeconds;
  }
  if (Math.abs(end - data.duration) > .01) return null;
  return { url: data.url, duration: data.duration, parts: parts.map(p => ({ ...p, url: data.parts ? `${data.url}/part/${p.index}` : data.url })) };
}
export function attachRecordingSource(video, initial, onTail) {
  const proto = Object.getPrototypeOf(video);
  const descriptor = name => {
    for (let p = proto; p; p = Object.getPrototypeOf(p)) { const d = Object.getOwnPropertyDescriptor(p, name); if (d) return d; }
    throw new Error(`Missing media property: ${name}`);
  };
  const native = Object.fromEntries(['currentTime', 'duration', 'buffered', 'seeking', 'ended', 'paused'].map(name => [name, descriptor(name)]));
  const read = name => native[name].get.call(video);
  const play = video.play.bind(video), pause = video.pause.bind(video);
  let source = initial, index = 0, pending = null, paused = true, disposed = false, tailSent = false;
  const part = () => source.parts[index];
  const currentTime = () => pending ?? Math.min(source.duration, part().startSeconds + Math.min(part().durationSeconds, read('currentTime') || 0));
  const ranges = entries => ({ length: entries.length, start(i) { if (!entries[i]) throw new DOMException('Invalid range', 'IndexSizeError'); return entries[i][0]; }, end(i) { if (!entries[i]) throw new DOMException('Invalid range', 'IndexSizeError'); return entries[i][1]; } });
  const find = time => { let lo = 0, hi = source.parts.length - 1; while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (source.parts[mid].startSeconds <= time) lo = mid; else hi = mid - 1; } return lo; };
  const resume = () => { if (!paused && !disposed) void play().catch(error => { if (error?.name !== 'AbortError' && !disposed) { paused = true; video.dispatchEvent(new Event('pause')); } }); };
  const change = (next, time) => {
    pending = time; index = next; tailSent = false;
    pause(); video.src = part().url; video.load();
  };
  const seek = time => {
    if (disposed || !finite(time, 0, 604800)) return;
    time = Math.min(source.duration, time); const next = find(time); tailSent = false;
    if (next !== index) change(next, time);
    else if (pending !== null || video.readyState < 1) pending = time;
    else native.currentTime.set.call(video, Math.max(0, Math.min(read('duration'), time - part().startSeconds)));
  };
  const loaded = () => {
    if (disposed) return;
    const time = pending; pending = null;
    if (time !== null) native.currentTime.set.call(video, Math.max(0, Math.min(read('duration'), time - part().startSeconds)));
    resume();
  };
  const ended = event => {
    if (disposed) return;
    if (index < source.parts.length - 1) { event.stopImmediatePropagation(); change(index + 1, source.parts[index + 1].startSeconds); }
    else if (!tailSent) { tailSent = true; onTail(); }
  };
  Object.defineProperties(video, {
    currentTime: { configurable: true, get: currentTime, set: seek },
    duration: { configurable: true, get: () => source.duration },
    seekable: { configurable: true, get: () => ranges([[0, source.duration]]) },
    buffered: { configurable: true, get: () => {
      if (pending !== null) return ranges([]);
      const r = read('buffered'), entries = [];
      for (let i = 0; i < r.length; i++) entries.push([part().startSeconds + r.start(i), Math.min(source.duration, part().startSeconds + r.end(i))]);
      return ranges(entries);
    } },
    seeking: { configurable: true, get: () => pending !== null || read('seeking') },
    ended: { configurable: true, get: () => pending === null && index === source.parts.length - 1 && read('ended') },
    paused: { configurable: true, get: () => pending !== null ? paused : read('paused') },
    play: { configurable: true, value: () => { paused = false; return play(); } },
    pause: { configurable: true, value: () => { paused = true; pause(); } },
  });
  video.addEventListener('loadedmetadata', loaded, true);
  video.addEventListener('ended', ended, true);
  return {
    update(next) {
      if (disposed || next.url === source.url) return;
      const time = Math.min(currentTime(), next.duration);
      source = next; change(find(time), time);
    },
    dispose() {
      disposed = true; pause(); video.removeEventListener('loadedmetadata', loaded, true); video.removeEventListener('ended', ended, true);
    },
  };
}
