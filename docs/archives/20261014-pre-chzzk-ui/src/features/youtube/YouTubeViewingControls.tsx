import { useEffect, useId, useRef, useState } from 'react';
import { targetKey, type YouTubeMetadata } from './model';

export type YouTubeViewingMode = 'all' | 'selected';
type Props = {
  active: boolean; open: boolean; privacyMode: boolean; busy: boolean; noticeError?: string;
  items: YouTubeMetadata[]; playingIds: string[]; favoriteIds: string[];
  mode: YouTubeViewingMode; lead: string;
  onOpenChange: (open: boolean) => void;
  onApply: (ids: string[], mode: YouTubeViewingMode, lead: string) => void;
  onStop: () => void; onExplore: () => void; onCloseItem: (id: string) => void;
  onFavorite: (item: YouTubeMetadata) => void; onDownload: (item: YouTubeMetadata) => void;
};

/** Presentation only: choosing a layout never owns or re-creates a media session. */
export function YouTubeViewingControls(props: Props) {
  const { active, open, privacyMode, items, playingIds, favoriteIds, mode, lead } = props;
  const panelId = useId(), helpId = useId();
  const handle = useRef<HTMLButtonElement>(null), panel = useRef<HTMLDivElement>(null);
  const wasOpen = useRef(false), drag = useRef<{ pointer: number; y: number } | null>(null), suppressClick = useRef(false);
  const [draftIds, setDraftIds] = useState<string[]>([]), [draftMode, setDraftMode] = useState<YouTubeViewingMode>('all'), [draftLead, setDraftLead] = useState(''), [error, setError] = useState('');
  const chosen = items.filter(item => draftIds.includes(targetKey(item)));
  const chosenLead = chosen.some(item => targetKey(item) === draftLead) ? draftLead : chosen[0] ? targetKey(chosen[0]) : '';
  useEffect(() => {
    if (!active || !open) { wasOpen.current = false; return; }
    if (!wasOpen.current) {
      wasOpen.current = true; setDraftIds(playingIds); setDraftMode(mode); setDraftLead(lead); setError('');
      panel.current?.querySelector<HTMLButtonElement>('button')?.focus({ preventScroll: true });
    }
  }, [active, open, playingIds, mode, lead]);
  const close = () => { if (panel.current?.contains(document.activeElement)) handle.current?.focus({ preventScroll: true }); props.onOpenChange(false); };
  const toggle = (id: string) => {
    if (privacyMode) return;
    if (!draftIds.includes(id) && chosen.length >= 4) { setError('동시에 네 개까지 시청할 수 있습니다.'); return; }
    setDraftIds(old => old.includes(id) ? old.filter(value => value !== id) : [...old, id]); setError('');
  };
  const label = (item: YouTubeMetadata) => {
    if (privacyMode) return '영상';
    const author = item.author.trim();
    return !author ? item.title : chosen.filter(value => value.author.trim() === author).length > 1 ? `${author} · ${item.title}` : author;
  };
  return <div className="youtube-viewing-dock" hidden={!active} inert={!active || undefined} data-gallery-shortcuts-suspended>
    <button ref={handle} className="youtube-bottom-handle" type="button" aria-label={`${playingIds.length ? '방송 조작' : '방송 보기'} ${open ? '닫기' : '열기'}`} aria-expanded={open} aria-controls={panelId} title={open ? '누르거나 아래로 끌어 닫기' : '누르거나 위로 끌어 열기'}
      onPointerDown={event => { if (event.button !== 0) return; drag.current = { pointer: event.pointerId, y: event.clientY }; suppressClick.current = false; event.currentTarget.setPointerCapture?.(event.pointerId); }}
      onPointerMove={event => { const gesture = drag.current; if (!gesture || gesture.pointer !== event.pointerId || Math.abs(event.clientY - gesture.y) < 14) return; suppressClick.current = true; if (event.clientY < gesture.y) props.onOpenChange(true); else close(); }}
      onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; suppressClick.current = true; }} onLostPointerCapture={() => { drag.current = null; }}
      onClick={() => { if (suppressClick.current) { suppressClick.current = false; return; } if (open) close(); else props.onOpenChange(true); }}
      onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') suppressClick.current = false; if (event.key === 'Escape' && open && !document.fullscreenElement) { event.preventDefault(); event.stopPropagation(); close(); } }}>
      <svg viewBox="0 0 24 16" aria-hidden="true"><path d={open ? 'm6 5 6 5 6-5' : 'm6 11 6-5 6 5'} /></svg>
    </button>
    {active && open ? <div ref={panel} id={panelId} className="youtube-viewing-panel" role="dialog" aria-modal="false" aria-label={playingIds.length ? '방송 조작' : '방송 보기'} onKeyDown={event => { if (event.key === 'Escape' && !event.defaultPrevented && !document.fullscreenElement) { event.preventDefault(); event.stopPropagation(); close(); } }}>
      <header><h2>{playingIds.length ? '방송 조작' : '방송 보기'}</h2><span className="youtube-viewing-help"><button type="button" aria-label="시청 도움말" aria-describedby={helpId}>?</button><span role="tooltip" id={helpId}>탐색에서 연 영상 중 4개까지 봅니다.<br />영상을 고르고 시청·변경을 누르세요.<br />선택해도 다른 채팅·소리는 유지돼요.<br />X는 창 닫기, 종료는 시청 끝내기입니다.</span></span><button className="youtube-viewing-close" type="button" aria-label="방송 보기 닫기" onClick={close}>×</button></header>
      <div className="youtube-viewing-scroll">
        {items.length ? <div className="youtube-viewing-items" aria-label="열린 영상">{items.map(item => {
          const key = targetKey(item), selected = chosen.some(value => targetKey(value) === key), saved = favoriteIds.includes(key);
          return <div className="youtube-viewing-item" key={key}>
            <button type="button" className="youtube-viewing-choice" aria-pressed={selected} aria-label={`${privacyMode ? '영상' : item.title} 시청 선택`} disabled={privacyMode} onClick={() => toggle(key)}><span className="youtube-viewing-check" aria-hidden="true">{selected ? '✓' : ''}</span><span><strong>{privacyMode ? '프라이버시 모드' : item.title}</strong><small>{privacyMode ? '' : [item.author, item.isLive ? '라이브' : '영상'].filter(Boolean).join(' · ')}</small></span></button>
            <button className="youtube-viewing-star" type="button" aria-label={`${privacyMode ? '영상' : item.title} 즐겨찾기 ${saved ? '해제' : '추가'}`} aria-pressed={saved} disabled={privacyMode} onClick={() => props.onFavorite(item)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3l-5.6 2.9 1.1-6.2L3 9.6l6.2-.9Z" fill={saved ? 'currentColor' : 'none'} /></svg></button>
            <button type="button" className="youtube-viewing-icon" aria-label={`${privacyMode ? '영상' : item.title} 다운로드`} disabled={privacyMode || props.busy || item.kind !== 'video' || item.isLive} onClick={() => props.onDownload(item)}>↓</button>
            <button type="button" className="youtube-viewing-icon" aria-label={`${privacyMode ? '영상' : item.title} 열린 영상 닫기`} onClick={() => props.onCloseItem(key)}>×</button>
          </div>;
        })}</div> : <p className="youtube-viewing-note">탐색에서 보고 싶은 영상을 열어 주세요.</p>}
        <button className="youtube-viewing-explore" type="button" disabled={privacyMode} onClick={() => { close(); props.onExplore(); }}>탐색에서 영상 찾기</button>
        {!privacyMode && (error || props.noticeError) ? <p role="alert" className="youtube-error">{error || props.noticeError}</p> : null}
      </div>
      <footer><div className="youtube-viewing-layout"><div className="youtube-viewing-mode" role="group" aria-label="영상 표시" data-mode={draftMode}><button type="button" aria-pressed={draftMode === 'all'} disabled={privacyMode} onClick={() => setDraftMode('all')}>모두 보기</button><button type="button" aria-pressed={draftMode === 'selected'} disabled={privacyMode} onClick={() => setDraftMode('selected')}>선택 보기</button></div><select aria-label="볼 영상" disabled={privacyMode || draftMode !== 'selected' || !chosen.length} value={chosenLead} onChange={event => setDraftLead(event.target.value)}>{!chosen.length ? <option value="">채널 선택</option> : chosen.map(item => <option key={targetKey(item)} value={targetKey(item)}>{label(item)}</option>)}</select></div>
        <span className="youtube-viewing-actions"><button type="button" className={playingIds.length ? 'youtube-viewing-change' : 'youtube-viewing-start'} disabled={privacyMode || !chosen.length || chosen.length > 4} onClick={() => { props.onApply(chosen.map(targetKey), draftMode, chosenLead); close(); }}>{playingIds.length ? '변경' : '시청'}</button><button type="button" className="youtube-viewing-stop" disabled={!playingIds.length} onClick={() => { props.onStop(); close(); }}>종료</button></span>
      </footer>
    </div> : null}
  </div>;
}
