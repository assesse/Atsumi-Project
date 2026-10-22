import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { parseYouTubeTarget, targetKey, type YouTubeMetadata } from './model';
import type { YouTubeApi } from './api';
import { usePanelHandleGesture } from '../streaming/usePanelHandleGesture';
import { YouTubeConnectionHelp } from './YouTubeConnectionHelp';
import { FluentIcon } from '../../components/FluentIcon';
import './YouTubeConnection.css';
import type { YouTubeOriginalApi, YouTubeAccountSnapshot } from '../../api/youtubeOriginal';
import type { YouTubeAutoRecordApi } from '../../api/youtubeAutoRecord';
import { autoRecordLabel, useYouTubeAutoRecord } from './useYouTubeAutoRecord';
import { UNKNOWN_VIDEO_STATE, verifiedYouTubeMetadata } from './metadataResolution';
import { YouTubePortrait } from './YouTubePortrait';
import type { YouTubeRecordingObservation } from './YouTubeOriginalPlayer';

export type YouTubeViewingMode = 'all' | 'selected';
export type YouTubeArrangement = 'channels' | 'grouped';
type Resolved = { input: string; item?: YouTubeMetadata; video?: YouTubeMetadata; pending?: boolean; error?: string };
type Props = {
  active: boolean; open: boolean; privacyMode: boolean; busy: boolean; noticeError?: string;
  items: YouTubeMetadata[]; playingIds: string[]; favoriteIds: string[]; favorites?: YouTubeMetadata[]; api?: YouTubeApi;
  accountApi?: Pick<YouTubeOriginalApi, 'account' | 'login' | 'logout'>;
  autoApi?: YouTubeAutoRecordApi;
  observations?: readonly YouTubeRecordingObservation[];
  mode: YouTubeViewingMode; lead: string; arrangement?: YouTubeArrangement;
  onArrangement?: (arrangement: YouTubeArrangement) => void;
  onOpenChange: (open: boolean) => void;
  onApply: (ids: string[], mode: YouTubeViewingMode, lead: string, resolved?: YouTubeMetadata[]) => void;
  onStop: () => void; onExplore: (saved?: boolean) => void; onCloseItem: (id: string) => void;
  onFavorite: (item: YouTubeMetadata) => void; onDownload: (item: YouTubeMetadata) => void;
};
const parseInput = (input: string) => parseYouTubeTarget(input.trim().startsWith('@') ? 'https://www.youtube.com/' + input.trim() : input);
const message = (cause: unknown) => cause instanceof Error ? cause.message : '주소를 확인하지 못했습니다.';
function Star({ saved }: { saved: boolean }) { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3l-5.6 2.9 1.1-6.2L3 9.6l6.2-.9Z" fill={saved ? 'currentColor' : 'none'} /></svg>; }

/** CHZZK presentation clone, with YouTube-only drafts and resolution. Never owns a receiver. */
export function YouTubeViewingControls(props: Props) {
  const { active, open, privacyMode, items, playingIds, favoriteIds, mode, lead } = props;
  const panelId = useId(), fieldId = useId();
  const handle = useRef<HTMLButtonElement>(null), panel = useRef<HTMLDivElement>(null);
  const wasOpen = useRef(false), initialized = useRef(false);
  const [inputs, setInputs] = useState<string[]>(['']), [resolved, setResolved] = useState<Resolved[]>([]);
  const [draftMode, setDraftMode] = useState<YouTubeViewingMode>('all'), [draftLead, setDraftLead] = useState(''), [error, setError] = useState('');
  const savedFavorites = useMemo(() => props.favorites ?? items.filter(item => favoriteIds.includes(targetKey(item))), [props.favorites, items, favoriteIds]);
  const automatic = useYouTubeAutoRecord(props.autoApi, active && open && !privacyMode, savedFavorites);
  const favorites = automatic.rows;
  const known = useRef(new Map<string, YouTubeMetadata>());
  // Polling updates these lookup seeds, not the lifetime of an explicit input
  // resolution. Otherwise every reservation snapshot cancels its deadline.
  useEffect(() => { known.current = new Map([...favorites, ...items].map(item => [targetKey(item), item])); }, [favorites, items]);
  const revision = useRef(0);
  const [account,setAccount]=useState<YouTubeAccountSnapshot|null>(null),[accountBusy,setAccountBusy]=useState(false),[accountError,setAccountError]=useState('');
  const accountGeneration=useRef(0);
  useEffect(()=>{const generation=++accountGeneration.current;let pending=false;setAccount(null);setAccountError('');setAccountBusy(false);if(!active||!open||!props.accountApi)return;
    const refresh=()=>{if(pending)return;pending=true;void props.accountApi!.account().then(value=>{if(accountGeneration.current===generation)setAccount(value);}).catch(cause=>{if(accountGeneration.current===generation)setAccountError(typeof cause==='string'?cause:message(cause));}).finally(()=>{pending=false;});};refresh();const timer=setInterval(refresh,2000);return()=>{++accountGeneration.current;clearInterval(timer);};
  },[active,open,props.accountApi]);
  const changeAccount=async(logout:boolean)=>{if(!props.accountApi||accountBusy||(!logout&&account?.authenticated===true))return;const generation=accountGeneration.current;setAccountBusy(true);setAccountError('');try{const value=await (logout?props.accountApi.logout():props.accountApi.login());if(generation===accountGeneration.current)setAccount(value);}catch(cause){if(generation===accountGeneration.current)setAccountError(typeof cause==='string'?cause:message(cause));}finally{if(generation===accountGeneration.current)setAccountBusy(false);}};
  useEffect(() => {
    if (!active || !open) { wasOpen.current = false; return; }
    if (!wasOpen.current) {
      wasOpen.current = true;
      if (playingIds.length || !initialized.current) {
        const initial = items.filter(item => playingIds.length ? playingIds.includes(targetKey(item)) : item.kind === 'video').slice(0, 4);
        setInputs(initial.length ? initial.map(item => item.url) : ['']); initialized.current = true;
      }
      setDraftMode(mode); setDraftLead(lead); setError('');
    }
  }, [active, open, playingIds, items, mode, lead]);
  useEffect(() => {
    const current = ++revision.current;
    if (!active || !open || privacyMode) { setResolved([]); return; }
    const controllers: AbortController[] = [], timers: ReturnType<typeof setTimeout>[] = [];
    const rows: Resolved[] = inputs.map(input => {
      if (!input.trim()) return { input };
      const target = parseInput(input);
      if (!target || target.kind === 'playlist') return { input, error: '영상 또는 채널 주소를 입력하세요.' };
      const item = known.current.get(targetKey(target));
      return item?.kind === 'video' && playingIds.includes(targetKey(item)) && typeof item.isLive === 'boolean' ? { input, item, video: item } : { input, item, pending: true };
    });
    setResolved(rows);
    const apply = (index: number, value: Resolved) => { if (revision.current === current) setResolved(old => old.map((row, n) => n === index ? value : row)); };
    const timer = setTimeout(() => rows.forEach((row, index) => {
      if (!row.pending) return;
      const target = parseInput(row.input)!;
      if (!props.api) { apply(index, { ...row, pending: false, error: '탐색에서 영상을 먼저 열어 주세요.' }); return; }
      const controller = new AbortController(); controllers.push(controller);
      let finished = false;
      const timeout = setTimeout(() => { finished = true; controller.abort(); apply(index, { ...row, pending: false, error: '확인이 지연됩니다. 주소를 다시 입력해 주세요.' }); }, 10000); timers.push(timeout);
      void (async () => {
        const item = target.kind === 'video' ? verifiedYouTubeMetadata(target, await props.api!.metadata(target, controller.signal)) : row.item ?? await props.api!.metadata(target, controller.signal);
        if (finished || revision.current !== current || controller.signal.aborted) throw new Error('요청이 취소됐습니다.');
        if (item.kind === 'video') return { input: row.input, item, video: item };
        if (!props.api!.search) throw new Error('이 채널의 라이브를 확인할 수 없습니다.');
        const result = await props.api!.search({ query: '', channel: target.url, filter: 'live', sort: 'relevance' });
        const live = result.items.filter(value => value.kind === 'video' && value.isLive);
        if (!live.length) throw new Error(result.items.some(value => value.kind === 'video' && typeof value.isLive !== 'boolean') ? UNKNOWN_VIDEO_STATE : '진행 중인 라이브가 없습니다.');
        if (live.length > 1) throw new Error('라이브가 여러 개입니다. 탐색에서 영상을 골라 주세요.');
        const video = verifiedYouTubeMetadata(live[0]!, await props.api!.metadata(live[0]!, controller.signal));
        if (!video.isLive) throw new Error('이 방송은 종료됐습니다. 탐색에서 다시 확인해 주세요.');
        return { input: row.input, item, video };
      })().then(value => { if (!finished) apply(index, value); }).catch(cause => { if (!finished) apply(index, { ...row, pending: false, error: message(cause) }); }).finally(() => clearTimeout(timeout));
    }), 300);
    return () => { revision.current++; clearTimeout(timer); timers.forEach(clearTimeout); controllers.forEach(controller => controller.abort()); };
  }, [active, open, privacyMode, inputs, props.api, playingIds]);
  const rows = inputs.map((input, index) => resolved[index]?.input === input ? resolved[index]! : { input, pending: !!input.trim() });
  const chosen = rows.flatMap(row => row.video ? [row.video] : []).filter((item, index, all) => all.findIndex(value => targetKey(value) === targetKey(item)) === index);
  const chosenLead = chosen.some(item => targetKey(item) === draftLead) ? draftLead : chosen[0] ? targetKey(chosen[0]) : '';
  const pending = rows.some(row => row.pending), invalid = rows.some(row => !!row.input.trim() && !row.video);
  const close = () => { handle.current?.focus({ preventScroll: true }); props.onOpenChange(false); };
  const changeOpen = (next: boolean) => next ? props.onOpenChange(true) : close();
  const gesture = usePanelHandleGesture(open, 'y', changeOpen);
  const updateInput = (index: number, value: string) => { setInputs(old => old.map((input, n) => n === index ? value : input)); setError(''); };
  const chooseFavorite = (item: YouTubeMetadata) => {
    if (privacyMode) return;
    const key = targetKey(item), match = inputs.findIndex(input => { const target = parseInput(input); return target && targetKey(target) === key; });
    if (match >= 0) { setInputs(old => old.length > 1 ? old.filter((_, index) => index !== match) : ['']); return; }
    const empty = inputs.findIndex(input => !input.trim());
    if (empty >= 0) updateInput(empty, item.url);
    else if (inputs.length < 4) setInputs(old => [...old, item.url]);
    else setError('동시에 네 개까지 시청할 수 있습니다.');
  };
  const label = (item: YouTubeMetadata) => privacyMode ? '영상' : item.author || item.title;
  return <div className="youtube-connection-dock" hidden={!active} inert={!active || undefined} data-gallery-shortcuts-suspended>
    <button ref={handle} className="youtube-bottom-handle" data-platform-native-occlusion="true" type="button" aria-label={(playingIds.length ? '방송 조작' : '방송 보기') + (open ? ' 닫기' : ' 열기')} aria-expanded={open} aria-controls={panelId} title={open ? '누르거나 아래로 끌어 닫기' : '누르거나 위로 끌어 열기'} {...gesture}
      onKeyDown={event => { if (event.key === 'Escape' && open && !document.fullscreenElement) { event.preventDefault(); event.stopPropagation(); close(); } }}>
      <svg viewBox="0 0 24 16" aria-hidden="true"><path d={open ? 'm6 5 6 5 6-5' : 'm6 11 6-5 6 5'} /></svg>
    </button>
    {active && open ? <div ref={panel} id={panelId} className="youtube-connection-panel" data-platform-native-occlusion="true" role="dialog" aria-modal="false" aria-label={playingIds.length ? '방송 조작' : '방송 보기'} onKeyDown={event => { if (event.key === 'Escape' && !event.defaultPrevented && !document.fullscreenElement) { event.preventDefault(); event.stopPropagation(); close(); } }}>
      <div className="youtube-connection-surface" data-native-dialog-surface="true"><div className="youtube-connection-setup">
        <div className="youtube-connection-heading"><strong>방송 보기</strong><YouTubeConnectionHelp>
          <p>채널 주소는 현재 라이브, 영상 주소는 해당 영상</p><p>별표는 입력한 항목 저장 · +로 최대 4개</p><p>즐겨찾기 선택 후 시청 시작 · 모두/선택 보기</p><p>라이브 녹화는 플레이어의 녹화 버튼</p><p>X는 창 닫기, 종료는 시청 끝내기</p>
        </YouTubeConnectionHelp><button type="button" className="youtube-connection-close" aria-label="연결 설정 닫기" title="닫기" onClick={close}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button></div>
        <section className="youtube-channel-picker" aria-label="즐겨찾기">
          <div className="youtube-channel-heading"><strong>즐겨찾기</strong><button type="button" title="저장한 영상과 채널은 탐색에서 관리합니다." aria-label="즐겨찾기 관리" onClick={() => { close(); props.onExplore(true); }}>⋯</button></div>
          {!privacyMode && automatic.error ? <p role="alert" className="youtube-connection-error">{automatic.error}</p> : null}
          {favorites.length ? <div className="youtube-channel-list">{favorites.map(item => {
            const key = targetKey(item), selected = inputs.some(input => { const target = parseInput(input); return target && targetKey(target) === key; }), reservation = automatic.rowFor(item), saved = favoriteIds.includes(key);
            const observed = props.observations?.filter(value => {
              if(!playingIds.includes('video:'+value.videoId))return false;
              if(item.kind==='video')return item.id===value.videoId;
              if(item.kind!=='channel')return false;
              const canonical=automatic.idFor(item);
              if(canonical)return canonical===value.canonicalChannelId;
              const metadata=items.find(video=>video.kind==='video'&&video.id===value.videoId&&video.isLive===true);
              const author=metadata?.authorUrl?parseYouTubeTarget(metadata.authorUrl):null;
              return author?.kind==='channel'&&targetKey(author)===key;
            }) ?? [];
            const presence = observed.some(value=>value.canonicalChannelId&&value.state==='playing') ? 'live' : reservation?.liveStatus ?? (item.kind === 'channel' ? 'unknown' : 'saved');
            const presenceLabel = presence === 'live' ? '● 방송 중' : presence === 'offline' ? '방송 종료' : presence === 'unknown' ? '방송 상태 미확인' : item.kind === 'video' && item.isLive ? '라이브 영상' : '영상';
            const observedRecording=observed.some(value=>!!value.recordingJob&&!value.recordingStopping);
            const recording=observedRecording||reservation?.status==='recording';
            const stopping=observed.some(value=>!!value.recordingJob&&value.recordingStopping)||reservation?.status==='stopping';
            // The auto snapshot exposes only one representative job ID. Never
            // add viewer counts to it: that would count viewed auto jobs twice.
            const recordingLabel=observedRecording?'녹화 중':reservation?.status==='recording'?autoRecordLabel(reservation):stopping?'마무리 중':autoRecordLabel(reservation);
            const recordingTone = recording ? ' is-recording' : !stopping&&reservation?.enabled && reservation.status === 'failed' ? ' is-problem' : '';
            return <article className="youtube-channel-card" key={key} data-selected={selected}>
              <button type="button" className="youtube-channel-select" aria-pressed={selected} aria-label={(privacyMode ? '항목' : item.title) + ' 시청 선택'} disabled={privacyMode} onClick={() => chooseFavorite(item)}>
                <YouTubePortrait item={item} api={props.api} active={active && open} privacy={privacyMode} /><span className="youtube-channel-copy"><strong>{privacyMode ? '프라이버시 모드' : item.author || item.title}</strong><small className={'youtube-channel-presence' + (privacyMode ? '' : ' is-' + presence)}>{privacyMode ? '' : presenceLabel}</small><small className={'youtube-channel-record-status' + (privacyMode ? '' : recordingTone)}>{privacyMode ? '' : recordingLabel}</small></span>
              </button>
              <span className="youtube-channel-actions"><button type="button" className="youtube-channel-auto" data-reserved={privacyMode ? undefined : !!reservation?.enabled} disabled={!automatic.canToggle(item)} title={item.kind !== 'channel' ? '채널을 즐겨찾기하면 자동 녹화를 예약할 수 있습니다.' : !props.autoApi ? '자동 녹화는 Atsumi 앱에서 사용할 수 있습니다.' : reservation?.error || autoRecordLabel(reservation)} aria-label={(privacyMode ? '항목' : item.title) + ' 자동 녹화'} aria-pressed={!!reservation?.enabled} aria-busy={automatic.busy === key || undefined} onClick={() => void automatic.toggle(item)}>{automatic.busy === key ? <span className="youtube-spinner" /> : <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="7" /><circle cx="12" cy="12" r="3" fill="currentColor" /></svg>}</button><button type="button" className="youtube-channel-star" disabled={privacyMode} aria-label={(privacyMode ? '항목' : item.title) + ' 즐겨찾기 ' + (saved ? '해제' : '추가')} aria-pressed={saved} onClick={() => props.onFavorite(item)}><Star saved={saved} /></button></span>
            </article>;
          })}</div> : <p className="youtube-channel-empty">즐겨찾기한 채널이나 영상이 없습니다.</p>}
        </section>
        <form onSubmit={event => { event.preventDefault(); if (privacyMode || props.busy || pending || invalid || !chosen.length) return; props.onApply(chosen.map(targetKey), draftMode, chosenLead, chosen); close(); }}>
          <div className="youtube-connection-inputs">{rows.map((row, index) => {
            const item = row.item, saved = !!item && favoriteIds.includes(targetKey(item)), name = privacyMode ? '' : item ? item.author || item.title : '';
            return <div className="youtube-channel-edit-row" key={index}><div className="youtube-channel-field">
              <label className="youtube-channel-number" htmlFor={fieldId + '-' + index}>{index + 1}</label><span className={'youtube-channel-profile' + (!name ? ' is-skeleton' : '')} aria-busy={row.pending || undefined}><YouTubePortrait item={item} api={props.api} active={active && open} privacy={privacyMode} /><span>{name || (row.pending && !privacyMode ? '확인 중' : '')}</span></span>
              <div className="youtube-channel-input-wrap"><input id={fieldId + '-' + index} aria-label={'YouTube 방송 ' + (index + 1)} placeholder="YouTube 채널·영상 주소 또는 @핸들" value={privacyMode ? '' : row.input} disabled={privacyMode || props.busy} onChange={event => updateInput(index, event.target.value)} autoComplete="off" spellCheck={false} />
                <button type="button" className="youtube-channel-clear" aria-label={'방송 ' + (index + 1) + ' 지우기'} disabled={privacyMode || props.busy || (!row.input && inputs.length === 1)} onClick={() => setInputs(old => old.length > 1 ? old.filter((_, n) => n !== index) : [''])}><FluentIcon glyph="\uE711" /></button>
              </div><button type="button" className="youtube-channel-star" aria-label={'방송 ' + (index + 1) + ' 즐겨찾기 ' + (saved ? '해제' : '추가')} aria-pressed={saved} disabled={privacyMode || !item || row.pending} onClick={() => item && props.onFavorite(item)}><Star saved={saved} /></button>
            </div>{row.error && !privacyMode ? <small className="youtube-connection-error" role="alert">{row.error}</small> : null}</div>;
          })}</div>
          <button type="button" className="youtube-channel-add" aria-label="방송 추가" disabled={privacyMode || props.busy || inputs.length >= 4} onClick={() => setInputs(old => [...old, ''])}><FluentIcon glyph="\uE710" /></button>
          <div className="youtube-connection-account"><button type="button" disabled={!props.accountApi||accountBusy||privacyMode||account?.authenticated===true} onClick={()=>void changeAccount(false)}>로그인</button><button type="button" disabled={!props.accountApi||accountBusy||privacyMode||account?.authenticated!==true} onClick={()=>void changeAccount(true)}>로그아웃</button><span role="status">{accountBusy?<span className="youtube-spinner"/>:null}{!props.accountApi?'앱에서 로그인 가능':account?.authenticated===true?'로그인됨':account?.authenticated===false?'로그아웃됨':account?.open?'로그인 창에서 확인':'계정 확인 전'}</span><button type="button" className="youtube-connection-explore" disabled={privacyMode} onClick={() => { close(); props.onExplore(); }}>탐색에서 영상 찾기</button></div>
          {accountError?<p role="alert" className="youtube-connection-error">{accountError}</p>:null}
          {!privacyMode && (error || props.noticeError) ? <p role="alert" className="youtube-connection-error">{error || props.noticeError}</p> : null}
          <div className="youtube-connection-actions"><div className="youtube-connection-options">
            <div className="youtube-viewing-mode" role="group" aria-label="영상 표시" data-mode={draftMode}><button type="button" aria-pressed={draftMode === 'all'} disabled={privacyMode} onClick={() => setDraftMode('all')}>모두 보기</button><button type="button" aria-pressed={draftMode === 'selected'} disabled={privacyMode} onClick={() => setDraftMode('selected')}>선택 보기</button></div>
            <label className="youtube-broadcast-select"><span>{draftMode === 'selected' ? '영상 방송' : '화면 배치'}</span><span className="youtube-broadcast-select-control">{draftMode === 'selected' ? <select aria-label="볼 영상" disabled={privacyMode || props.busy || !chosen.length} value={chosenLead} onChange={event => setDraftLead(event.target.value)}>{!chosen.length ? <option value="">채널 선택</option> : chosen.map(item => <option key={targetKey(item)} value={targetKey(item)}>{label(item)}</option>)}</select> : <select aria-label="방송 배치" disabled={privacyMode || props.busy} value={props.arrangement ?? 'channels'} onChange={event => props.onArrangement?.(event.target.value as YouTubeArrangement)}><option value="channels">방송 · 채팅 / 방송 · 채팅</option><option value="grouped">방송 모아 보기 / 채팅 모아 보기</option></select>}<FluentIcon glyph="\uE70D" /></span></label>
          </div><div className="youtube-session-actions"><button type="submit" className={playingIds.length ? 'youtube-viewing-change' : 'youtube-viewing-start'} aria-busy={pending || undefined} disabled={privacyMode || props.busy || pending || invalid || !chosen.length}>{pending ? <span className="youtube-spinner" aria-hidden="true" /> : null}{playingIds.length ? '변경' : '시청 시작'}</button>{playingIds.length ? <button type="button" className="youtube-viewing-stop" onClick={() => { props.onStop(); close(); }}>종료</button> : null}</div></div>
        </form>
      </div></div>
    </div> : null}
  </div>;
}
