import type { YouTubeApi } from './api';
import { parseYouTubeTarget, targetKey, type YouTubeMetadata, type YouTubeTarget } from './model';

export const UNKNOWN_VIDEO_STATE = '현재 라이브인지 확인하지 못했습니다. 다시 확인해 주세요.';
export function verifiedYouTubeMetadata(target: YouTubeTarget, value: YouTubeMetadata): YouTubeMetadata {
  const requested = parseYouTubeTarget(target.url), received = value && parseYouTubeTarget(value.url);
  if (!requested || targetKey(requested) !== targetKey(target) || !received
    || targetKey(received) !== targetKey(requested) || targetKey(value) !== targetKey(requested)) {
    throw new Error('요청한 영상과 다른 정보가 반환됐습니다. 다시 확인해 주세요.');
  }
  if (requested.kind === 'video' && typeof value.isLive !== 'boolean') throw new Error(UNKNOWN_VIDEO_STATE);
  return { ...value, ...requested };
}

/** Component-local queue: only explicit detail/input/download actions enter it. */
export function createYouTubeMetadataResolver(api: YouTubeApi): YouTubeApi['metadata'] {
  let running = 0;
  const waiters: Array<() => void> = [];
  return async (target, signal) => {
    if (signal?.aborted) throw new DOMException('요청이 취소됐습니다.', 'AbortError');
    if (running >= 2) await new Promise<void>((resolve, reject) => {
      // The saved-list selection cap is100. Queued work has its own deadline;
      // timing it out never releases a still-running native request's slot.
      if (waiters.length >= 100) { reject(new Error('영상 확인 요청이 많습니다. 잠시 후 다시 확인해 주세요.')); return; }
      const cleanup = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); };
      const remove = (reason: unknown) => { const index = waiters.indexOf(start); if (index >= 0) waiters.splice(index, 1); cleanup(); reject(reason); };
      const abort = () => remove(new DOMException('요청이 취소됐습니다.', 'AbortError'));
      const start = () => { cleanup(); resolve(); };
      const timer = setTimeout(() => remove(new Error('영상 확인이 지연됩니다. 다시 확인해 주세요.')), 12_000);
      signal?.addEventListener('abort', abort, { once: true });
      waiters.push(start);
    });
    else running++;
    try {
      if (signal?.aborted) throw new DOMException('요청이 취소됐습니다.', 'AbortError');
      const value = await api.metadata(target, signal);
      if (signal?.aborted) throw new DOMException('요청이 취소됐습니다.', 'AbortError');
      return verifiedYouTubeMetadata(target, value);
    } finally {
      const next = waiters.shift();
      if (next) next(); else running--;
    }
  };
}
