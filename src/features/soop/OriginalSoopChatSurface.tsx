import { useLayoutEffect, useState, type ReactNode, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import originalChat from './generated/official-chat.css?raw';
import originalBadges from './generated/official-badges.css?raw';
import adapter from './OriginalSoopChatSurface.css?raw';
import './SoopChatFont.css';

/** Original local SOOP styles, isolated exactly as the CHZZK replay surface is.
 * No website bootstrap, account session, remote asset, or second chat receiver. */
export function OriginalSoopChatSurface({ hostRef, active, children, theme = 'dark' }: {
  hostRef: RefObject<HTMLElement | null>; active: boolean; children: ReactNode; theme?: 'light' | 'dark';
}) {
  const [surface, setSurface] = useState<ShadowRoot | null>(null);
  useLayoutEffect(() => {
    const host = hostRef.current!;
    setSurface(host.shadowRoot ?? host.attachShadow({ mode: 'open' }));
  }, [hostRef]);
  return <aside ref={hostRef} className="platform-replay-chat" data-provider="soop" lang="ko-KR" aria-label="녹화 채팅" hidden={!active}>
    {surface && createPortal(<><style>{originalChat}</style><style>{originalBadges}</style><style>{adapter}</style><div className={`soop-original-chat-root${theme === 'dark' ? ' thema_dark' : ''}`} {...{dark:String(theme === 'dark')}}>{children}</div></>, surface)}
  </aside>;
}
