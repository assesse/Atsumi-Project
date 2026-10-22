import { useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { playerErrorMessage, youTubeEmbedUrl, type YouTubeMetadata } from './model';

export function YouTubePlayer({ item, privacyMode, onExternal }: { item: YouTubeMetadata; privacyMode: boolean; onExternal: () => void }) {
  const frame = useRef<HTMLIFrameElement>(null), [error, setError] = useState(''), [ready, setReady] = useState(false), [version, setVersion] = useState(0);
  const [trayMuted, setTrayMuted] = useState(false);
  const forcedMute = useRef(false), previousMute = useRef(false), reportedMute = useRef(false);
  const embed = useMemo(() => youTubeEmbedUrl(item, window.location.origin), [item]);
  const command = (func: string) => frame.current?.contentWindow?.postMessage(JSON.stringify({ event: 'command', func, args: [] }), 'https://www.youtube.com');
  useEffect(() => { let gone = false; let unlisten: (() => void) | undefined; void listen<boolean>('platform-media:tray-muted', event => setTrayMuted(event.payload)).then(fn => { if (gone) fn(); else unlisten = fn; }).catch(() => undefined); return () => { gone = true; unlisten?.(); }; }, []);
  useEffect(() => { const force = privacyMode || trayMuted; if (force) { if (!forcedMute.current) previousMute.current = reportedMute.current; command('mute'); } else if (forcedMute.current) command(previousMute.current ? 'mute' : 'unMute'); forcedMute.current = force; }, [privacyMode, trayMuted, ready]);
  useEffect(() => {
    setError(''); setReady(false);
    let failed = false;
    const subscription = setInterval(() => {
      frame.current?.contentWindow?.postMessage(JSON.stringify({ event: 'listening', id: 'atsumi-youtube' }), 'https://www.youtube.com');
      for (const event of ['onReady', 'onError', 'onStateChange']) frame.current?.contentWindow?.postMessage(JSON.stringify({ event: 'command', func: 'addEventListener', args: [event] }), 'https://www.youtube.com');
    }, 600);
    const timer = setTimeout(() => { failed = true; clearInterval(subscription); setError('재생 응답이 없습니다. 다시 시도해 주세요.'); }, 20000);
    const receive = (event: MessageEvent) => {
      if (event.origin !== 'https://www.youtube.com' || event.source !== frame.current?.contentWindow) return;
      let payload = event.data; if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch { return; } }
      if (!payload || typeof payload !== 'object') return;
      if (payload.event === 'infoDelivery' && typeof payload.info?.muted === 'boolean' && !forcedMute.current) reportedMute.current = payload.info.muted;
      if (['onReady', 'initialDelivery'].includes(payload.event) && !failed) { clearTimeout(timer); clearInterval(subscription); setReady(true); setError(''); }
      if (payload.event === 'onError') { failed = true; clearTimeout(timer); clearInterval(subscription); setError(playerErrorMessage(Number(payload.info))); }
    };
    window.addEventListener('message', receive); return () => { clearTimeout(timer); clearInterval(subscription); window.removeEventListener('message', receive); };
  }, [embed, version]);
  return <div className={`youtube-player${privacyMode ? ' is-private' : ''}`}>
    {embed ? <iframe key={version} ref={frame} src={embed} title="YouTube 플레이어" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowFullScreen referrerPolicy="strict-origin-when-cross-origin" sandbox="allow-scripts allow-same-origin allow-presentation" /> : null}
    {privacyMode ? <div className="youtube-privacy-mask">프라이버시 모드</div> : null}
    {!ready && !error ? <div role="status" className="youtube-player-status"><span className="youtube-spinner" />플레이어 연결 중</div> : null}
    {error ? <div className="youtube-player-error" role="alert"><span>{error}</span><button onClick={() => setVersion(v => v + 1)}>다시 시도</button><button onClick={onExternal}>YouTube에서 열기</button></div> : null}
  </div>;
}
