import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { soopVodApi, type SoopVodApi, type SoopVodSnapshot } from '../../api/soopVod';
import { measureNativePlayerGeometry, observeNativePlayerGeometry } from '../platforms/nativePlayerGeometry';
import { clampLiveChat, liveSplit, liveSplitKey, LIVE_SPLITTER_SIZE } from '../platforms/liveSplit';
import './SoopVodPlayer.css';

const errorText = (cause: unknown) => typeof cause === 'string' ? cause : cause instanceof Error ? cause.message : '영상을 연결하지 못했습니다.';
const validIdentity = (value: SoopVodSnapshot, videoId: string, lease?: string) => value.videoId === videoId && !!value.token && (!lease || value.token === lease);

/** Slots for one official VOD document, not a second media element or chat receiver. */
export function SoopVodPlayer({ videoId, title, active, privacyMode, api = soopVodApi }: {
  videoId: string; title: string; active: boolean; privacyMode: boolean; api?: SoopVodApi;
}) {
  const [snapshot, setSnapshot] = useState<SoopVodSnapshot | null>(null);
  const [error, setError] = useState(''), [viewportError, setViewportError] = useState(''), [attempt, setAttempt] = useState(0);
  const [trayMuted, setTrayMuted] = useState(false), [ownFullscreen, setOwnFullscreen] = useState(false);
  const [chatWidth, setChatWidth] = useState(240), [fraction, setFraction] = useState(.44);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const pair = useRef<HTMLDivElement>(null), player = useRef<HTMLDivElement>(null), chat = useRef<HTMLDivElement>(null);
  const token = useRef<string | null>(null), generation = useRef(0), viewportSequence = useRef(0);
  const refreshSnapshot = useRef<() => void>(() => {}), retryViewport = useRef<() => void>(() => {});
  const hasChat = snapshot?.chatState === 'available';
  const split = liveSplit(size.width, size.height, chatWidth, fraction);

  useEffect(() => {
    const element = pair.current;
    if (!element) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) setSize(previous => previous.width === rect.width && previous.height === rect.height ? previous : { width: rect.width, height: rect.height });
    };
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(element);
    window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, []);

  useEffect(() => {
    let gone = false, polling = false;
    const current = ++generation.current;
    setSnapshot(null); setError(''); setViewportError(''); setOwnFullscreen(false);
    void api.open(videoId, crypto.randomUUID()).then(value => {
      if (gone || current !== generation.current) { void api.close(value.token).catch(() => undefined); return; }
      if (!validIdentity(value, videoId)) {
        if (value.token) void api.close(value.token).catch(() => undefined);
        throw new Error('요청한 영상과 재생 응답이 일치하지 않습니다.');
      }
      token.current = value.token; setSnapshot(value);
    }).catch(cause => { if (!gone && current === generation.current) setError(errorText(cause)); });
    const refresh = () => {
      const lease = token.current;
      if (!lease || gone || polling) return;
      polling = true;
      void api.snapshot(lease).then(value => {
        if (gone || current !== generation.current || token.current !== lease) return;
        if (!validIdentity(value, videoId, lease)) throw new Error('재생 상태 응답이 현재 영상과 일치하지 않습니다.');
        setSnapshot(value); setError('');
      }).catch(cause => {
        if (!gone && current === generation.current && token.current === lease) setError(errorText(cause));
      }).finally(() => { polling = false; });
    };
    refreshSnapshot.current = refresh;
    const timer = window.setInterval(refresh, 1500);
    return () => {
      gone = true; ++generation.current; window.clearInterval(timer);
      refreshSnapshot.current = () => {};
      const lease = token.current; token.current = null;
      if (lease) void api.close(lease).catch(() => undefined);
    };
  }, [api, videoId, attempt]);

  useEffect(() => {
    let gone = false;
    const stops: (() => void)[] = [];
    void listen<boolean>('platform-media:tray-muted', event => { if (!gone) setTrayMuted(event.payload === true); })
      .then(stop => { if (gone) stop(); else stops.push(stop); }).catch(() => undefined);
    void listen<{ videoId: string; enabled: boolean }>('soop-vod:fullscreen', event => {
      // Events name a shared official document, not one viewer lease. Read the
      // current exact-token state instead of applying a delayed old alias event.
      if (!gone && token.current && event.payload.videoId === videoId) refreshSnapshot.current();
    }).then(stop => { if (gone) stop(); else stops.push(stop); }).catch(() => undefined);
    return () => { gone = true; stops.forEach(stop => stop()); };
  }, [videoId]);
  useEffect(() => { if (snapshot) setOwnFullscreen(snapshot.fullscreen); }, [snapshot?.token, snapshot?.fullscreen]);
  useEffect(() => {
    if (!ownFullscreen || active && !privacyMode) return;
    const lease = token.current;
    if (lease) void api.fullscreen(lease, false).catch(() => undefined);
    setOwnFullscreen(false);
  }, [api, active, privacyMode, ownFullscreen]);
  useEffect(() => {
    if (!ownFullscreen || !active || privacyMode) return;
    const keydown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      const lease = token.current, current = generation.current;
      if (!lease) return;
      event.preventDefault(); event.stopImmediatePropagation();
      void api.fullscreen(lease, false).then(() => {
        if (token.current === lease && generation.current === current) setOwnFullscreen(false);
      }).catch(cause => { if (token.current === lease && generation.current === current) setError(errorText(cause)); });
    };
    window.addEventListener('keydown', keydown, true);
    return () => window.removeEventListener('keydown', keydown, true);
  }, [api, active, privacyMode, ownFullscreen]);

  useEffect(() => {
    const lease = snapshot?.token;
    if (!lease) return;
    let previous = '', disposed = false, sending = false, retries = 0;
    let retryTimer: number | undefined;
    let queued: ReturnType<typeof measureNativePlayerGeometry> | null = null;
    let latest: ReturnType<typeof measureNativePlayerGeometry> | null = null;
    const send = async () => {
      if (sending || disposed) return;
      sending = true;
      let failed: ReturnType<typeof measureNativePlayerGeometry> | null = null;
      try {
        while (queued && !disposed) {
          const next = queued; queued = null; failed = next;
          await api.viewport(lease, next);
          failed = null; retries = 0;
          if (!disposed && token.current === lease) setViewportError('');
        }
      } catch (cause) {
        if (!disposed && token.current === lease) {
          setViewportError(errorText(cause));
          if (retries < 2) {
            // Native consumes a sequence before paint. A failed paint retry
            // therefore needs a fresh sequence, not an acknowledged no-op.
            if (!queued && failed) queued = { ...failed, sequence: ++viewportSequence.current };
            const delay = retries++ === 0 ? 250 : 750;
            retryTimer = window.setTimeout(() => { retryTimer = undefined; void send(); }, delay);
          } else queued = null;
        }
      }
      finally { sending = false; }
    };
    const measure = () => {
      const viewport = measureNativePlayerGeometry(player.current, hasChat ? chat.current : null, {
        sequence: ++viewportSequence.current, visible: active && !privacyMode, muted: privacyMode || trayMuted,
      });
      const key = JSON.stringify({ ...viewport, sequence: 0 });
      if (key === previous) return;
      previous = key; latest = viewport; queued = viewport; retries = 0;
      if (retryTimer !== undefined) { window.clearTimeout(retryTimer); retryTimer = undefined; }
      void send();
    };
    retryViewport.current = () => {
      if (disposed || !latest) return;
      if (retryTimer !== undefined) { window.clearTimeout(retryTimer); retryTimer = undefined; }
      retries = 0; queued = { ...latest, sequence: ++viewportSequence.current }; void send();
    };
    const stop = observeNativePlayerGeometry([player.current, hasChat ? chat.current : null], measure);
    measure();
    return () => { disposed = true; if (retryTimer !== undefined) window.clearTimeout(retryTimer); retryViewport.current = () => {}; stop(); };
  }, [api, snapshot?.token, active, privacyMode, trayMuted, hasChat, ownFullscreen]);

  const resize = (pixels: number) => {
    const next = clampLiveChat(pixels, split.min, split.max);
    if (split.compact) setFraction(split.available ? next / split.available : .44);
    else setChatWidth(next);
  };
  const message = error || viewportError || snapshot?.error;
  return <div ref={pair} className={`soop-vod-pair${hasChat ? ' has-chat' : ''}${hasChat && split.compact ? ' is-compact' : ''}${ownFullscreen ? ' is-native-fullscreen' : ''}`}
    data-platform-native-fullscreen={ownFullscreen ? 'true' : undefined} data-state={snapshot?.state ?? 'loading'}
    style={{ '--soop-vod-chat-size': `${size.width ? split.pixels : chatWidth}px` } as CSSProperties} inert={!active || privacyMode || undefined}>
    <div ref={player} className="soop-vod-slot" tabIndex={active && !privacyMode ? 0 : -1} aria-label={privacyMode ? '영상' : title}>
      {privacyMode ? <div className="soop-vod-privacy">프라이버시 모드</div> : null}
      {!privacyMode && !message && (!snapshot || snapshot.state === 'loading' || snapshot.state === 'preparing') ? <div className="soop-vod-status" role="status"><span className="soop-spinner" />영상 연결 중</div> : null}
      {!privacyMode && message ? <div className="soop-vod-status is-error" role="alert" data-platform-native-occlusion="true"><span>{message}</span><button type="button" onClick={() => { if (viewportError && !error && !snapshot?.error) retryViewport.current(); else setAttempt(value => value + 1); }}>다시 시도</button></div> : null}
    </div>
    {hasChat ? <><div className="soop-vod-resize" role="separator" aria-label={split.compact ? '채팅 높이' : '채팅 너비'} aria-orientation={split.compact ? 'horizontal' : 'vertical'}
      aria-valuemin={Math.round(split.min)} aria-valuemax={Math.round(split.max)} aria-valuenow={Math.round(split.pixels)} tabIndex={active && !privacyMode ? 0 : -1}
      onPointerDown={event => { if (!active || privacyMode || event.button !== 0) return; event.preventDefault(); event.currentTarget.focus(); event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (active && !privacyMode && event.currentTarget.hasPointerCapture(event.pointerId)) { const rect = pair.current?.getBoundingClientRect(); if (rect) resize((split.compact ? rect.bottom - event.clientY : rect.right - event.clientX) - LIVE_SPLITTER_SIZE / 2); } }}
      onPointerUp={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }}
      onPointerCancel={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }}
      onKeyDown={event => { if (!active || privacyMode || event.altKey || event.ctrlKey || event.metaKey) return; const next = liveSplitKey(event.key, split.pixels, split.min, split.max, split.compact); if (next !== null) { event.preventDefault(); resize(next); } }} />
      <div ref={chat} className="soop-vod-chat" aria-label="SOOP 원본 다시보기 채팅" />
    </> : null}
  </div>;
}
