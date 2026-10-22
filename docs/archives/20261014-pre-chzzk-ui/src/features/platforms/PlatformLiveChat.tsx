import { useEffect, useRef, useState } from 'react';
import type { MediaLiveChatRow, PlatformLiveApi } from './platformMedia';
import { PlatformChatParts, useChatAssets } from './PlatformChatParts';

export function PlatformLiveChat({ token, api, active, privacyMode, width, title }: { token: string | null; api: PlatformLiveApi; active: boolean; privacyMode: boolean; width?: number; title: string }) {
  const [rows, setRows] = useState<MediaLiveChatRow[]>([]), [status, setStatus] = useState(''), [gap, setGap] = useState(false);
  const cursor = useRef(0), pinned = useRef(true), list = useRef<HTMLDivElement>(null), identity = useRef(token);
  const images = useChatAssets(token ?? '', rows, active && !privacyMode, api.chatAsset, true);
  useEffect(() => {
    if (identity.current !== token) { identity.current = token; cursor.current = 0; setRows([]); setStatus(''); setGap(false); }
    if (!token || !active || privacyMode || !api.chat) return;
    let gone = false, running = false;
    const refresh = async () => {
      if (gone || running) return; running = true;
      try { const page = await api.chat!(token, cursor.current, 120); if (gone) return; cursor.current = page.cursor; setStatus(page.status); setGap(value => value || page.gap); if (page.items.length) setRows(previous => [...previous, ...page.items].slice(-500)); }
      catch (cause) { if (!gone) setStatus(typeof cause === 'string' ? cause : '채팅을 불러오지 못했습니다.'); }
      finally { running = false; }
    };
    void refresh(); const timer = setInterval(() => void refresh(), 1000);
    return () => { gone = true; clearInterval(timer); };
  }, [token, api, active, privacyMode]);
  useEffect(() => { if (pinned.current && list.current) list.current.scrollTop = list.current.scrollHeight; }, [rows]);
  return <aside className="platform-live-chat" style={{ width }} aria-label="실시간 채팅"><header><strong>{privacyMode ? '채팅' : title}</strong><span title={privacyMode ? undefined : status}>{privacyMode ? '' : status}</span></header>{privacyMode ? <div className="platform-live-chat-private">프라이버시 모드</div> : <><div ref={list} className="platform-live-chat-messages" onScroll={() => { const element = list.current; if (element) pinned.current = element.scrollHeight - element.scrollTop - element.clientHeight < 50; }}>{gap ? <p className="platform-live-chat-note">최근 메시지만 표시합니다.</p> : null}{rows.length ? rows.map(row => <p key={row.sequence} className={`platform-live-chat-row${row.kind === 'chat' ? '' : ' is-event'}`}><strong style={row.color && /^#[0-9a-f]{6}$/i.test(row.color) ? { color: row.color } : undefined}>{row.author}</strong>{row.amount ? <b>{row.amount}</b> : null}<span><PlatformChatParts text={row.text || (row.kind === 'gap' ? '채팅 수신 공백' : row.kind === 'connected' ? '채팅 연결됨' : '')} parts={row.parts} images={images} /></span></p>) : <p className="platform-live-chat-note">아직 받은 채팅이 없습니다.</p>}</div><small>읽기 전용</small></>}</aside>;
}
