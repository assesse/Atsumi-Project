import { useEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject, type VideoHTMLAttributes } from 'react';
import type { MediaProvider } from './platformMedia';
import './PlatformReplayPlayer.css';

type Icon = 'play' | 'pause' | 'replay' | 'volume' | 'muted' | 'settings' | 'fullscreen' | 'exit' | 'close' | 'check' | 'more';
/** Local vectors only. No remote player, icon font, account or media source. */
export function ReplayIcon({ name, provider = 'youtube' }: { name: Icon; provider?: MediaProvider }) {
  if (provider === 'soop' && name === 'play') return <svg viewBox="0 0 32 32" aria-hidden="true"><path fill="currentColor" d="M24.528 15.149a1 1 0 0 1 0 1.702l-13 8.01a1 1 0 0 1-1.524-.852V7.991a1 1 0 0 1 1.524-.852l13 8.01Z" /></svg>;
  if (provider === 'soop' && name === 'fullscreen') return <svg viewBox="0 0 32 32" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M8 13V8.5a.5.5 0 0 1 .5-.5H13M13 24H8.5a.5.5 0 0 1-.5-.5V19M24 19v4.5a.5.5 0 0 1-.5.5H19M19 8h4.5a.5.5 0 0 1 .5.5V13" /></svg>;
  return <svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">{
    name === 'play' ? <path d="M8 5v14l11-7z" /> :
    name === 'pause' ? <path d="M6 5h4v14H6zm8 0h4v14h-4z" /> :
    name === 'replay' ? <path d="M12 5V1L7 6l5 5V7a7 7 0 1 1-7 7H3a9 9 0 1 0 9-9Z" /> :
    name === 'volume' || name === 'muted' ? <><path d="M3 9v6h4l5 4V5L7 9Z" />{name === 'muted' ? <path d="m16 8 5 8m0-8-5 8" fill="none" stroke="currentColor" strokeWidth="2" /> : <path d="M15 8a5 5 0 0 1 0 8m3-11a9 9 0 0 1 0 14" fill="none" stroke="currentColor" strokeWidth="1.8" />}</> :
    name === 'settings' ? <path d="m19.4 13 .1-1-.1-1 2-1.5-2-3.4-2.3.9a8 8 0 0 0-1.7-1L15 3h-4l-.4 3a8 8 0 0 0-1.7 1l-2.3-.9-2 3.4 2 1.5-.1 1 .1 1-2 1.5 2 3.4 2.3-.9a8 8 0 0 0 1.7 1l.4 3h4l.4-3a8 8 0 0 0 1.7-1l2.3.9 2-3.4-2-1.5ZM13 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7Z" /> :
    name === 'fullscreen' ? <path d="M3 3h7v2H5v5H3zm11 0h7v7h-2V5h-5zM3 14h2v5h5v2H3zm16 0h2v7h-7v-2h5z" /> :
    name === 'exit' ? <path d="M8 3h2v7H3V8h5zm6 0h2v5h5v2h-7zM3 14h7v7H8v-5H3zm11 0h7v2h-5v5h-2z" /> :
    name === 'more' ? <><circle cx="12" cy="5" r="1.8" /><circle cx="12" cy="12" r="1.8" /><circle cx="12" cy="19" r="1.8" /></> :
    name === 'check' ? <path d="m5 12 4 4L19 6" fill="none" stroke="currentColor" strokeWidth="2" /> :
    <path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="1.8" />
  }</svg>;
}

export const replayTime = (value: number) => {
  const seconds = Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;
  const hours = Math.floor(seconds / 3600), minutes = Math.floor(seconds / 60) % 60;
  return `${hours ? `${hours}:${String(minutes).padStart(2, '0')}` : minutes}:${String(seconds % 60).padStart(2, '0')}`;
};
type MediaEvents = Pick<VideoHTMLAttributes<HTMLVideoElement>, 'onPlay' | 'onPause' | 'onEnded' | 'onLoadedMetadata' | 'onTimeUpdate' | 'onError'>;
export interface PlatformReplayPlayerProps {
  provider: MediaProvider; url: string; videoRef: RefObject<HTMLVideoElement | null>; mediaEvents: MediaEvents;
  active: boolean; privacyMode: boolean; title: ReactNode; paused: boolean; ended: boolean;
  position: number; duration: number; volume: number; muted: boolean; busy: boolean; error: string;
  onShowControls(): void; onLeaveControls(): void; onToggle(): void; onReplay(): void;
  onSeek(seconds: number): void; onVolume(value: number): void; onMute(): void;
  onFullscreen(): void; onRetry(): void; onClose?: () => void;
}

/** Presentation for the existing owned-file video. The parent retains its token and media clock. */
export function PlatformReplayPlayer(props: PlatformReplayPlayerProps) {
  const { provider, url, videoRef, mediaEvents, active, privacyMode, paused, ended, position, duration, volume, muted } = props;
  const [settings, setSettings] = useState(false), [speed, setSpeed] = useState(1), [full, setFull] = useState(false);
  const stage = useRef<HTMLDivElement>(null), gear = useRef<HTMLButtonElement>(null);
  const disabled = privacyMode || !active;
  useEffect(() => { setSettings(false); setSpeed(videoRef.current?.playbackRate ?? 1); }, [url, active, privacyMode, videoRef]);
  useEffect(() => {
    const changed = () => setFull(document.fullscreenElement === stage.current?.closest('.platform-media-player'));
    document.addEventListener('fullscreenchange', changed); changed();
    return () => document.removeEventListener('fullscreenchange', changed);
  }, []);
  useEffect(() => {
    if (!settings) return;
    const pointer = (event: PointerEvent) => {
      if (!(event.target instanceof Element) || !event.target.closest('.platform-replay-settings, .platform-replay-settings-toggle')) setSettings(false);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) return;
      const own = stage.current?.closest('.platform-media-player');
      if (document.documentElement.dataset.tutorialOpen === 'true' || [...document.querySelectorAll<HTMLElement>('dialog[open], [aria-modal="true"], [data-card-context-menu]')].some(node => node !== own && !own?.contains(node) && !node.closest('[hidden],[inert]') && getComputedStyle(node).display !== 'none')) return;
      if (event.target instanceof Element && event.target.closest('.platform-replay-chat')) return;
      event.preventDefault(); event.stopPropagation(); setSettings(false); gear.current?.focus({ preventScroll: true });
    };
    document.addEventListener('pointerdown', pointer, true); document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('pointerdown', pointer, true); document.removeEventListener('keydown', key, true); };
  }, [settings]);
  const limit = Number.isFinite(duration) && duration > 0 ? duration : 0;
  const at = Number.isFinite(position) ? Math.min(limit, Math.max(0, position)) : 0;
  const progress = { '--replay-progress': `${limit ? at / limit * 100 : 0}%` } as CSSProperties;
  const control = (name: Icon, label: string, action: () => void, extra?: string) => <button type="button" className={`platform-replay-control ${extra ?? ''}`} title={label} aria-label={label} disabled={disabled} onClick={() => { if (!disabled) { props.onShowControls(); action(); } }}><ReplayIcon name={name} provider={provider} /></button>;
  return <div ref={stage} className={`platform-media-player-stage platform-replay-player is-${provider}${settings ? ' settings-open' : ''}`} onPointerMove={props.onShowControls} onPointerLeave={props.onLeaveControls}>
    <video ref={videoRef} src={url} autoPlay={!privacyMode} playsInline {...mediaEvents} onRateChange={event => setSpeed(event.currentTarget.playbackRate)} />
    <header className="platform-media-overlay"><strong>{props.title}</strong>{props.onClose ? <button type="button" className="platform-replay-control platform-replay-close" title="재생 닫기" aria-label="재생 닫기" disabled={!active} onClick={props.onClose}><ReplayIcon name="close" /></button> : null}</header>
    {props.error ? <div className="platform-media-playback-error" role="alert"><p>{props.error}</p><button disabled={props.busy || disabled} onClick={props.onRetry}>다시 시도</button></div> : null}
    {ended ? <button type="button" className="platform-media-replay" disabled={disabled} onClick={props.onReplay}><ReplayIcon name="replay" provider={provider} /><span>다시보기</span></button> : null}
    <footer className="platform-media-overlay platform-replay-controls">
      <input className="platform-replay-progress" style={progress} type="range" aria-label="재생 위치" aria-valuetext={`${replayTime(at)} / ${replayTime(limit)}`} disabled={disabled || !limit} min="0" max={limit || 1} step="0.1" value={at} onChange={event => props.onSeek(Number(event.target.value))} />
      <div className="platform-replay-control-row">
        {control(paused || ended ? 'play' : 'pause', paused || ended ? '재생' : '일시정지', ended ? props.onReplay : props.onToggle, 'platform-replay-play')}
        <div className="platform-replay-volume"><button type="button" className="platform-replay-control" aria-label={muted ? '음소거 해제' : '음소거'} title={muted ? '음소거 해제' : '음소거'} aria-pressed={muted} disabled={disabled} onClick={props.onMute}><ReplayIcon name={muted || volume === 0 ? 'muted' : 'volume'} provider={provider} /></button><input type="range" aria-label="음량" disabled={disabled} min="0" max="1" step="0.05" value={volume} onChange={event => props.onVolume(Number(event.target.value))} /></div>
        <span className="platform-replay-time">{replayTime(at)} <span>/</span> {replayTime(limit)}</span>
        <span className="platform-replay-control-spacer" />
        <button ref={gear} type="button" className="platform-replay-control platform-replay-settings-toggle" aria-label="재생 설정" title="재생 설정" aria-expanded={settings} disabled={disabled} onClick={() => { props.onShowControls(); setSettings(value => !value); }}><ReplayIcon name="settings" provider={provider} /></button>
        {control(full ? 'exit' : 'fullscreen', full ? '전체화면 종료' : '전체화면', props.onFullscreen, 'platform-media-fullscreen')}
      </div>
    </footer>
    {settings && !disabled ? <div className="platform-replay-settings" role="menu" aria-label="재생 속도"><div>재생 속도</div>{[.25, .5, .75, 1, 1.25, 1.5, 1.75, 2].map(value => <button type="button" key={value} role="menuitemradio" aria-checked={speed === value} onClick={() => {
      if (disabled || !videoRef.current) return;
      videoRef.current.playbackRate = value; setSpeed(value); setSettings(false); props.onShowControls(); gear.current?.focus({ preventScroll: true });
    }}><span>{speed === value ? <ReplayIcon name="check" /> : null}</span>{value === 1 ? '일반' : `${value}배`}</button>)}</div> : null}
  </div>;
}
