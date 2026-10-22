import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import Hls from 'hls.js';
import { platformLiveApi, type MediaLiveSnapshot, type MediaRef, type PlatformLiveApi } from './platformMedia';
import { PlatformLiveChat } from './PlatformLiveChat';
import { clampLiveChat, liveSplit, liveSplitKey, LIVE_SPLITTER_SIZE } from './liveSplit';
import { platformScreenshotsApi, type PlatformScreenshotsApi } from './platformScreenshots';
import { screenshotShortcut, useLiveScreenshot } from './useLiveScreenshot';
import { ScreenshotPreview } from './ScreenshotPreview';
import './PlatformLivePlayer.css';

const errorText = (error: unknown) => typeof error === 'string' ? error : error instanceof Error ? error.message : '라이브를 연결하지 못했습니다.';
type LivePortrait = { identity: string; token: string; api: PlatformLiveApi; source: string | null };
const localPortrait = (value: string | null): string | null => value && value.length <= 174_790 && /^data:image\/(?:png|gif);base64,[A-Za-z0-9+/]+={0,2}$/.test(value) ? value : null;
export function PlatformLivePlayer({ reference, title, privacyMode, active = true, videoVisible = true, flattened = false, sideBySide = false, api = platformLiveApi, screenshotsApi = platformScreenshotsApi }: { reference: MediaRef; title: string; privacyMode: boolean; active?: boolean; videoVisible?: boolean; flattened?: boolean; sideBySide?: boolean; api?: PlatformLiveApi; screenshotsApi?: PlatformScreenshotsApi }) {
  const [session, setSession] = useState<MediaLiveSnapshot | null>(null), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const [pollError, setPollError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const [chatOpen, setChatOpen] = useState(true), [chatWidth, setChatWidth] = useState(240);
  const [compactFraction, setCompactFraction] = useState(.44);
  const [pairSize, setPairSize] = useState({ width: 0, height: 0 });
  const [ownFullscreen, setOwnFullscreen] = useState(false);
  const split = liveSplit(pairSize.width, pairSize.height, chatWidth, compactFraction, sideBySide && !ownFullscreen);
  const flattenedLayout = flattened && !ownFullscreen;
  const compact = !flattenedLayout && split.compact;
  const pair = useRef<HTMLDivElement>(null), generation = useRef(0), mutation = useRef(0);
  useEffect(() => {
    const update = () => setOwnFullscreen(document.fullscreenElement === pair.current && pair.current !== null);
    document.addEventListener('fullscreenchange', update);
    return () => document.removeEventListener('fullscreenchange', update);
  }, []);
  const [paused, setPaused] = useState(false), [controls, setControls] = useState(true), [volume, setVolume] = useState(1), [trayMuted, setTrayMuted] = useState(false);
  const surface = useRef<HTMLDivElement>(null), video = useRef<HTMLVideoElement>(null), lease = useRef<string | null>(null), timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const identity = `${reference.provider}:${reference.externalId}`;
  const currentSession = session?.reference.provider === reference.provider && session.reference.externalId === reference.externalId && session.reference.kind === reference.kind ? session : null;
  const identityVisible = active && videoVisible && !privacyMode;
  const portraitToken = currentSession?.state === 'playing' ? currentSession.token : null;
  const [portrait, setPortrait] = useState<LivePortrait | null>(null), portraitCache = useRef<LivePortrait | null>(null);
  const portraitSource = identityVisible && portrait?.identity === identity && portrait.token === portraitToken && portrait.api === api ? portrait.source : null;
  const captureEnabled = identityVisible && currentSession?.state === 'playing';
  const screenshot = useLiveScreenshot({ token: currentSession?.token ?? null, identity, enabled: captureEnabled, api: screenshotsApi }, video);
  useEffect(() => {
    const element = pair.current;
    if (!element) return;
    const measure = () => {
      const bounds = element.getBoundingClientRect();
      const width = Math.round(bounds.width), height = Math.round(bounds.height);
      if (width <= 0 || height <= 0) return;
      setPairSize(previous => previous.width === width && previous.height === height ? previous : { width, height });
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure); observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const current = ++generation.current;
    let gone = false, running = false; setSession(null); setError(''); setPollError(''); setBusy(false); const requestId = crypto.randomUUID();
    void api.open(reference, requestId).then(value => { if (gone) { void api.close(value.token).catch(() => undefined); return; } lease.current = value.token; setSession(value); }).catch(cause => { if (!gone) setError(errorText(cause)); });
    const poll = setInterval(() => { const token = lease.current, revision = mutation.current; if (!token || running || gone) return; running = true; void api.snapshot(token).then(value => { if (!gone && current === generation.current && token === lease.current && revision === mutation.current) { setSession(value); setPollError(''); } }).catch(cause => { if (!gone && current === generation.current && token === lease.current && revision === mutation.current) setPollError(errorText(cause)); }).finally(() => { running = false; }); }, 1000);
    return () => { gone = true; ++generation.current; clearInterval(poll); const token = lease.current; lease.current = null; if (token) void api.close(token).catch(() => undefined); if (timer.current) clearTimeout(timer.current); };
  }, [identity, api, attempt]);
  useEffect(() => {
    if (!identityVisible || !portraitToken || !api.liveProfile) { setPortrait(null); return; }
    const token = portraitToken, current = generation.current;
    const cached = portraitCache.current;
    if (cached?.identity === identity && cached.token === token && cached.api === api) { setPortrait(cached); return; }
    let gone = false;
    const finish = (value: string | null) => {
      if (gone || generation.current !== current || lease.current !== token) return;
      const next = { identity, token, api, source: localPortrait(value) };
      portraitCache.current = next; setPortrait(next);
    };
    void api.liveProfile(token).then(finish).catch(() => finish(null));
    return () => { gone = true; };
  }, [api, identity, identityVisible, portraitToken]);
  useEffect(() => {
    if (!session || session.state !== 'playing' || !video.current) return;
    const element = video.current; let hls: Hls | null = null;
    if (Hls.isSupported()) {
      hls = new Hls({ maxBufferLength: 24, backBufferLength: 12, maxMaxBufferLength: 48, liveSyncDurationCount: 2, liveMaxLatencyDurationCount: 6, enableWorker: true });
      hls.on(Hls.Events.MANIFEST_PARSED, () => { void element.play().catch(() => setPaused(true)); });
      hls.on(Hls.Events.ERROR, (_, data) => { if (data.fatal) setError('라이브 재생이 중단됐습니다. 녹화 상태는 보관함에서 확인할 수 있습니다.'); });
      hls.loadSource(session.url); hls.attachMedia(element);
    } else if (element.canPlayType('application/vnd.apple.mpegurl')) { element.src = session.url; void element.play().catch(() => setPaused(true)); }
    else setError('현재 재생 환경에서 라이브를 열 수 없습니다.');
    return () => { hls?.destroy(); element.pause(); element.removeAttribute('src'); element.load(); };
  }, [session?.token, session?.state === 'playing']);
  useEffect(() => { let gone = false; let unlisten: (() => void) | undefined; void listen<boolean>('platform-media:tray-muted', event => setTrayMuted(event.payload)).then(fn => { if (gone) fn(); else unlisten = fn; }).catch(() => undefined); return () => { gone = true; unlisten?.(); }; }, []);
  useEffect(() => { if (video.current) { video.current.volume = volume; video.current.muted = privacyMode || trayMuted || volume === 0; } }, [privacyMode, trayMuted, volume, session?.token]);
  const move = () => { if (!active) return; setControls(true); if (timer.current) clearTimeout(timer.current); timer.current = setTimeout(() => { if (!video.current?.paused) setControls(false); }, 2200); };
  const record = async () => { if (!session || busy) return; const current = generation.current, token = session.token; ++mutation.current; setBusy(true); try { const next = await api.record(token, !session.recordingJob); if (current === generation.current && token === lease.current) { setSession(next); setError(''); } } catch (cause) { if (current === generation.current) setError(errorText(cause)); } finally { if (current === generation.current) { ++mutation.current; setBusy(false); } } };
  const fullscreen = () => { const action = document.fullscreenElement ? document.exitFullscreen?.() : pair.current?.requestFullscreen?.(); void action?.catch(() => setError('전체화면을 열지 못했습니다.')); };
  const playPause = () => { if (video.current?.paused) void video.current.play().catch(cause => setError(errorText(cause))); else video.current?.pause(); };
  const showChat = chatOpen || !videoVisible;
  const resizeChat = (pixels: number) => {
    const next = clampLiveChat(pixels, split.min, split.max);
    if (compact) setCompactFraction(split.available ? next / split.available : .44);
    else setChatWidth(next);
  };
  return <div ref={pair} className={`platform-live-pair${showChat ? ' has-chat' : ''}${!videoVisible ? ' chat-only' : ''}${compact ? ' is-compact' : ''}${flattenedLayout ? ' is-flattened' : ''}`} style={{ '--platform-live-chat-size': `${pairSize.width ? split.pixels : chatWidth}px` } as CSSProperties} inert={!active || undefined}>
    <div ref={surface} className={`platform-live-player${controls || paused ? ' show-controls' : ''}${privacyMode ? ' is-private' : ''}`} aria-hidden={!videoVisible || undefined} onPointerMove={move} onPointerLeave={() => { if (!paused) setControls(false); }} tabIndex={active && videoVisible ? 0 : -1} onKeyDown={event => {
      if (captureEnabled && screenshotShortcut(event.nativeEvent, event.currentTarget)) { event.preventDefault(); event.stopPropagation(); void screenshot.save(); return; }
      if (!active || privacyMode || event.altKey || event.ctrlKey || event.metaKey || event.target instanceof HTMLInputElement || event.target instanceof HTMLButtonElement) return;
      if (event.key === ' ' || event.key.toLowerCase() === 'k') { event.preventDefault(); playPause(); }
      if (event.key.toLowerCase() === 'm') { event.preventDefault(); setVolume(value => value ? 0 : 1); }
      if (event.key.toLowerCase() === 'f') { event.preventDefault(); fullscreen(); }
    }}>
    <video ref={video} playsInline crossOrigin="anonymous" onPlay={() => { setPaused(false); move(); }} onPause={() => setPaused(true)} />
    {privacyMode ? <div className="platform-live-privacy">프라이버시 모드</div> : null}
    <header className="platform-live-overlay">{identityVisible ? <div className="platform-live-identity"><span className="platform-live-avatar" aria-hidden="true">{portraitSource ? <img src={portraitSource} alt="" onError={() => { const next = { identity, token: portraitToken!, api, source: null }; portraitCache.current = next; setPortrait(next); }} /> : <svg viewBox="0 0 24 24" fill="none" focusable="false"><circle cx="12" cy="8" r="3" /><path d="M5 20v-2a7 7 0 0 1 14 0v2" /></svg>}</span><div className="platform-live-identity-copy"><strong className="platform-live-channel">{currentSession?.channelName || ''}</strong><span className="platform-live-title">{currentSession?.title || title}</span></div></div> : <strong>{privacyMode ? '프라이버시 모드' : ''}</strong>}<span className="platform-live-badge">{currentSession?.recordingJob ? currentSession.recordingStopping ? '녹화 마무리 중' : '녹화 중' : currentSession?.state === 'playing' ? 'LIVE' : ''}</span></header>
    {!error && !pollError && (!session || session.state === 'queued' || session.state === 'resolving') ? <div role="status" className="platform-live-status"><span className="platform-live-spinner" />{session?.state === 'queued' ? '연결 대기 중' : '라이브 연결 중'}</div> : null}
    {session?.state === 'ended' ? <div className="platform-live-status">방송이 종료됐습니다.</div> : null}
    {error || pollError || session?.error || session?.state === 'failed' ? <div role="alert" className="platform-live-error"><span>{error || pollError || session?.error || '라이브 수신이 중단됐습니다.'}</span><button onClick={() => setAttempt(value => value + 1)}>다시 시도</button></div> : null}
    <footer className="platform-live-overlay"><button aria-label={paused ? '재생' : '일시정지'} disabled={session?.state !== 'playing'} onClick={playPause}>{paused ? '▶' : 'Ⅱ'}</button><input type="range" aria-label="음량" min="0" max="1" step="0.05" value={volume} onChange={event => setVolume(Number(event.target.value))} /><button className="platform-live-record" aria-pressed={!!session?.recordingJob} disabled={busy || session?.state !== 'playing' || session.recordingStopping} onClick={() => void record()}>{busy || session?.recordingStopping ? <span className="platform-live-spinner" /> : <span aria-hidden="true">●</span>}{session?.recordingJob ? '녹화 중지' : '녹화'}</button><button className="platform-live-screenshot" aria-label="스크린샷 저장 (S)" title="스크린샷 저장 (S)" disabled={!captureEnabled || screenshot.busy || screenshot.preview?.undoing} onClick={() => void screenshot.save()}>{screenshot.busy ? <span className="platform-live-spinner" /> : <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 7h4l2-3h4l2 3h4v13H4z" /><circle cx="12" cy="13" r="4" /></svg>}</button><button aria-label="채팅 표시" aria-pressed={chatOpen} onClick={() => setChatOpen(value => !value)}>채팅</button><button className="platform-live-fullscreen" aria-label="전체화면" onClick={fullscreen}>⛶</button></footer>
    {screenshot.busy ? <div className="platform-live-capture-notice" role="status">스크린샷 저장 중…</div> : null}
    {screenshot.error ? <div className="platform-live-capture-notice is-error" role="alert"><span>{screenshot.error}</span><button type="button" aria-label="스크린샷 오류 닫기" onClick={screenshot.clearError}>×</button></div> : null}
    {screenshot.preview ? <ScreenshotPreview preview={screenshot.preview} onDismiss={screenshot.dismiss} onUndo={()=>void screenshot.undo()} /> : null}
    </div>
    {showChat && videoVisible ? <div className="platform-live-resize" role="separator" aria-label={compact ? '채팅 높이' : '채팅 너비'} aria-orientation={compact ? 'horizontal' : 'vertical'} aria-valuemin={Math.round(split.min)} aria-valuemax={Math.round(split.max)} aria-valuenow={Math.round(split.pixels)} tabIndex={active && !flattenedLayout ? 0 : -1}
      onPointerDown={event => { if (!active || flattenedLayout || event.button !== 0) return; event.preventDefault(); event.currentTarget.focus(); event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (active && !flattenedLayout && event.currentTarget.hasPointerCapture(event.pointerId)) { const bounds = pair.current?.getBoundingClientRect(); if (bounds) resizeChat((compact ? bounds.bottom - event.clientY : bounds.right - event.clientX) - LIVE_SPLITTER_SIZE / 2); } }}
      onPointerUp={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }}
      onPointerCancel={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }}
      onKeyDown={event => { if (!active || flattenedLayout || event.altKey || event.ctrlKey || event.metaKey) return; const next = liveSplitKey(event.key, split.pixels, split.min, split.max, compact); if (next !== null) { event.preventDefault(); resizeChat(next); } }} /> : null}
    {showChat ? <PlatformLiveChat token={session?.token ?? null} api={api} active={active} privacyMode={privacyMode} title={session?.channelName || title} /> : null}
  </div>;
}
