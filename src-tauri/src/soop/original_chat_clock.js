// SOOP-local optional public event/source-clock observer.
// Keeps only a public packet UUID/service and the already-admitted source clock.
// No packet body, profile, socket URL, credential, DOM text or request is read.
(() => {
  'use strict';
  const PUBLIC = new Set([5, 18, 33, 91, 93, 105, 109, 121]);
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const NIL = '00000000-0000-0000-0000-000000000000';
  const uuid = value => typeof value === 'string' && UUID.test(value) && value !== NIL;
  const number = (v, min, max) => typeof v === 'number' && Number.isFinite(v) && v >= min && v <= max;
  const ownData = (object, key) => {
    if (!object || typeof object !== 'object') return undefined;
    const descriptor = Object.getOwnPropertyDescriptor(object, key);
    return descriptor && 'value' in descriptor ? descriptor.value : undefined;
  };
  const publicIdentity = packet => {
    const service = ownData(packet, 'serviceCode');
    if (!PUBLIC.has(service) || ownData(packet, 'retCode') !== 0) return null;
    const messageId = ownData(packet, 'chatUniqueId');
    return uuid(messageId) ? { service, messageId } : null;
  };
  const clock = (value, sourceId) => {
    if (!value || value.sourceId !== sourceId || !value.sample) return null;
    const s = value.sample;
    if (!number(s.sourceTimeSeconds, 0, 1e9) || !number(s.currentTimeSeconds, 0, 1e9)
      || !number(s.timestampOffsetSeconds, -1e9, 1e9) || !number(s.performanceNowMs, 0, 1e12)
      || !number(s.playbackRate, .0625, 16) || !Number.isInteger(s.readyState) || s.readyState < 0 || s.readyState > 4
      || typeof s.paused !== 'boolean' || typeof s.seeking !== 'boolean'
      || Math.abs(s.currentTimeSeconds - s.timestampOffsetSeconds - s.sourceTimeSeconds) > 1e-6) return null;
    return { sourceTimeSeconds: s.sourceTimeSeconds, currentTimeSeconds: s.currentTimeSeconds,
      timestampOffsetSeconds: s.timestampOffsetSeconds, performanceNowMs: s.performanceNowMs,
      playbackRate: s.playbackRate, readyState: s.readyState, paused: s.paused, seeking: s.seeking };
  };
  const create = ({ current, readClock, readVisibility, emit }) => {
    let session = null, binding = null;
    let attempted = 0, emitted = 0, dropped = 0;
    const detach = () => {
      const previous = session; session = null;
      if (!previous) return;
      try {
        const descriptor = Object.getOwnPropertyDescriptor(previous.socket, 'received');
        if (descriptor?.value === previous.wrapper) {
          if (previous.own) Object.defineProperty(previous.socket, 'received', previous.own);
          else delete previous.socket.received;
        }
      } catch { /* A site-locked old wrapper still delegates, but session is revoked. */ }
    };
    const stop = () => { detach(); binding = null; };
    const arm = ({ socket, runId, sourceId, epoch }) => {
      if (!uuid(runId) || !uuid(sourceId) || !Number.isSafeInteger(epoch) || epoch < 1) { detach(); return false; }
      if (binding && (binding.runId !== runId || binding.sourceId !== sourceId || binding.epoch !== epoch)) {
        detach(); binding.revoked = true; return false;
      }
      if (!binding) binding = { runId, sourceId, epoch, sequence: 0, windowAt: null, inWindow: 0, revoked: false };
      if (binding.revoked) return false;
      if (!socket) { detach(); return false; }
      let own, original;
      try { own = Object.getOwnPropertyDescriptor(socket, 'received'); original = socket.received; }
      catch { detach(); return false; }
      if (session?.socket === socket && own?.value === session.wrapper) return true;
      detach();
      if (own && (!('value' in own) || !own.configurable || !own.writable)) return false;
      if (typeof original !== 'function') return false;
      const bound = binding;
      const pinned = { socket, runId, sourceId, epoch, own, original, wrapper: null };
      const stillCurrent = () => binding === bound && !bound.revoked && session === pinned && socket.received === pinned.wrapper
        && current({ socket, runId, sourceId, epoch }) === true;
      pinned.wrapper = function (...args) {
        let observed = null;
        try {
          if (this === socket && stillCurrent()) {
            const identity = publicIdentity(args[0]);
            if (identity) {
              const sample = clock(readClock(runId), sourceId);
              if (sample) { const visible = readVisibility?.(); observed = { identity, sample, liveVisible: typeof visible === 'boolean' ? visible : null }; }
            }
          }
        } catch { /* Observation cannot change official dispatch. */ }
        // Preserve this, arguments, synchronous return and original exception.
        // A handler that throws is not labeled successfully dispatched.
        const result = Reflect.apply(original, this, args);
        try {
          if (observed && stillCurrent()) {
            attempted++;
            const sequence = bound.sequence++;
            if (!Number.isSafeInteger(sequence)) { dropped++; stop(); return result; }
            const now = observed.sample.performanceNowMs;
            if (bound.windowAt === null || now < bound.windowAt || now - bound.windowAt >= 1000) {
              bound.windowAt = now; bound.inWindow = 0;
            }
            // This lane is optional. Floods drop clock evidence, never chat.
            if (++bound.inWindow > 120) { dropped++; return result; }
            const accepted = emit({ version: 1, runId, sourceId, documentEpoch: epoch, sequence,
              service: observed.identity.service, messageId: observed.identity.messageId,
              sample: observed.sample, liveVisible: observed.liveVisible, phase: 'public-receiver-dispatch', presentationUncertainty: 'unknown' });
            if (accepted === true) emitted++; else dropped++;
          }
        } catch { dropped++; }
        return result;
      };
      try {
        Object.defineProperty(socket, 'received', { configurable: true, writable: true,
          enumerable: own?.enumerable ?? false, value: pinned.wrapper });
        session = pinned;
        return true;
      } catch { return false; }
    };
    return Object.freeze({ arm, stop, facts: () => ({ active: !!session, attempted, emitted, dropped }) });
  };
  Object.defineProperty(globalThis, '__atsumiSoopChatClockObserver', { value: Object.freeze({ create }) });
})();
