import { useEffect, useRef, useState } from 'react';
import { platformMediaApi, type MediaChatAsset, type MediaProvider, type MediaReplayChatItem, type MediaReplayChatPage } from './platformMedia';
import { PlatformChatParts, useChatAssets } from './PlatformChatParts';
import { ReplayIcon } from './PlatformReplayPlayer';
import './PlatformReplayChat.css';

export type ReplayChatItem = MediaReplayChatItem;
export type ReplayChatPage = MediaReplayChatPage;
export interface ReplayChatApi { page(libraryId: string, positionMs: number, cursor: string | null, limit: number): Promise<ReplayChatPage>; asset?(libraryId: string, assetId: string): Promise<MediaChatAsset> }
const nativeApi: ReplayChatApi = { page: (libraryId, positionMs, cursor, limit) => platformMediaApi.replayChat!(libraryId, positionMs, cursor ?? undefined, limit), asset: platformMediaApi.replayChatAsset };
export interface PlatformReplayChatProps { provider?: MediaProvider; libraryId: string; currentTime: number; active: boolean; privacyMode: boolean; onClose?: () => void; api?: ReplayChatApi }
const time = (ms: number) => { const seconds = Math.max(0, Math.floor(ms / 1000)); const minutes = Math.floor(seconds / 60); return `${minutes}:${String(seconds % 60).padStart(2, '0')}`; };

/** Video position is a prop, never a second media clock or network receiver. */
export function PlatformReplayChat({ provider, libraryId, currentTime, active, privacyMode, onClose, api = nativeApi }: PlatformReplayChatProps) {
  const [page, setPage] = useState<ReplayChatPage | null>(null), [error, setError] = useState(false), [loading, setLoading] = useState(false);
  const images = useChatAssets(libraryId, page?.items ?? [], active && !privacyMode, api.asset);
  const [following, setFollowing] = useState(true), [cursor, setCursor] = useState<string | null>(null), [retry, setRetry] = useState(0);
  const [settings, setSettings] = useState(false), [showTimes, setShowTimes] = useState(false), [profile, setProfile] = useState<ReplayChatItem | null>(null);
  const root = useRef<HTMLElement>(null), scroll = useRef<HTMLDivElement>(null), generation = useRef(0), previousTime = useRef(currentTime);
  const inFlight = useRef(false), wanted = useRef<{ token: number; id: string; position: number; cursor: string | null; api: ReplayChatApi; retry: number } | null>(null);
  const failed = useRef<{ id: string; api: ReplayChatApi; retry: number } | null>(null);
  const second = Math.max(0, Math.floor(Number.isFinite(currentTime) ? currentTime : 0));
  const preparingSecond = useRef(second), latestSecond = useRef(second);
  latestSecond.current = second;
  // Preparation advances on its own bounded timer, not every video tick. Each
  // actual request still uses the newest video position after a seek or pause.
  const querySecond = page?.state === 'indexing' ? preparingSecond.current : page && page.state !== 'ready' ? 0 : second;
  useEffect(() => () => { generation.current++; wanted.current = null; }, []);
  useEffect(() => { generation.current++; failed.current = null; setPage(null); setCursor(null); setFollowing(true); setProfile(null); setSettings(false); setError(false); }, [libraryId]);
  useEffect(() => {
    if (currentTime < previousTime.current - .5) { setFollowing(true); setCursor(null); setProfile(null); }
    previousTime.current = currentTime;
  }, [currentTime]);
  useEffect(() => {
    if (!active || privacyMode) { generation.current++; wanted.current = null; setProfile(null); setSettings(false); setLoading(false); return; }
    if (!following && cursor === null) { generation.current++; wanted.current = null; return; }
    if (failed.current?.id === libraryId && failed.current.api === api && failed.current.retry === retry) return;
    const token = ++generation.current;
    wanted.current = { token, id: libraryId, position: (page?.state === 'indexing' ? latestSecond.current : querySecond) * 1000, cursor, api, retry };
    const drain = async () => {
      if (inFlight.current || !wanted.current) return;
      const request = wanted.current; inFlight.current = true; setLoading(true); setError(false);
      try {
        const value = await request.api.page(request.id, request.position, request.cursor, 120);
        if (request.token === generation.current) {
          if (value.state === 'indexing') preparingSecond.current = querySecond;
          setPage(value);
        }
      }
      catch { if (request.token === generation.current) { failed.current = { id: request.id, api: request.api, retry: request.retry }; setError(true); } }
      finally {
        inFlight.current = false;
        if (request.token === generation.current) setLoading(false);
        else if (wanted.current) void drain();
      }
    };
    void drain();
  }, [active, privacyMode, libraryId, api, following, cursor, retry, querySecond]);
  useEffect(() => {
    if (!active || privacyMode || error || loading || (!following && cursor === null) || page?.state !== 'indexing') return;
    const timer = window.setTimeout(() => setRetry(value => value + 1), 100);
    return () => window.clearTimeout(timer);
  }, [active, privacyMode, error, loading, page, libraryId, following, cursor]);
  useEffect(() => { if (following && page?.state === 'ready' && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; }, [page, following]);
  useEffect(() => {
    if (!settings && !profile) return;
    const outside = (event: PointerEvent) => { if (!(event.target instanceof Node) || !root.current?.contains(event.target)) { setSettings(false); setProfile(null); } };
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setSettings(false); setProfile(null); } };
    document.addEventListener('pointerdown', outside, true); document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('pointerdown', outside, true); document.removeEventListener('keydown', key, true); };
  }, [settings, profile]);
  const navigate = (value: string | null) => { setCursor(value); setFollowing(true); setProfile(null); if (scroll.current) scroll.current.scrollTop = 0; };
  const prepared = page?.preparation;
  const preparationPercent = prepared && Number.isFinite(prepared.scannedBytes) && Number.isFinite(prepared.totalBytes) && prepared.totalBytes > 0
    ? Math.max(0, Math.min(99, Math.floor(prepared.scannedBytes / prepared.totalBytes * 100))) : null;
  return <aside ref={root} className="platform-replay-chat" data-provider={provider} aria-label="녹화 채팅" hidden={!active}>
    <div className="platform-replay-chat-header"><strong>채팅</strong><div>
      <button type="button" aria-label="채팅 표시 설정" title="채팅 표시 설정" aria-expanded={settings} disabled={privacyMode} onClick={() => { setSettings(value => !value); setProfile(null); }}><ReplayIcon name="more" provider={provider} /></button>
      {onClose ? <button type="button" aria-label="재생 닫기" title="재생 닫기" onClick={onClose}><ReplayIcon name="close" provider={provider} /></button> : null}
    </div></div>
    {privacyMode ? <div className="platform-replay-chat-empty">프라이버시 모드</div> : <>
      {settings ? <div className="platform-replay-chat-settings"><label><input type="checkbox" checked={showTimes} onChange={event => setShowTimes(event.target.checked)} />시각 표시</label></div> : null}
      {page?.timelineQuality === 'receive-time-approximate' ? <p className="platform-replay-chat-note" title="영상 조각과 채팅을 받은 시각으로 맞췄습니다. 실제 장면과 차이가 날 수 있습니다.">수신 시각 기준</p> : null}
      {page?.state === 'unmapped' ? <p className="platform-replay-chat-note">시간 정보 없음 · 기록 순서</p> : null}
      {page && (page.malformedLines > 0 || page.partialFile) ? <p className="platform-replay-chat-note" title={page.malformedLines > 0 ? '읽을 수 없는 기록은 건너뛰었습니다. 원본 기록은 변경하지 않습니다.' : '채팅 보존 상태가 완전히 확인되지 않았습니다. 원본 기록은 변경하지 않습니다.'}>일부 기록만 표시될 수 있습니다.</p> : null}
      <div ref={scroll} className="platform-replay-chat-messages" role="log" aria-live="off" onScroll={event => {
        if (page?.state !== 'ready') return;
        const node = event.currentTarget; setFollowing(node.scrollHeight - node.clientHeight - node.scrollTop < 48);
      }}>
        {!page && loading ? <p role="status"><span className="platform-replay-chat-spinner" />불러오는 중</p> : null}
        {page?.state === 'indexing' ? !error && <p role="status" className="platform-replay-chat-preparing"><span className="platform-replay-chat-spinner" />채팅 준비 중{preparationPercent !== null ? <span className="platform-replay-chat-percent">{preparationPercent}%</span> : null}</p> : page?.state === 'missing' ? <p className="platform-replay-chat-empty">저장된 채팅이 없습니다.</p> : page?.state === 'empty' ? <p className="platform-replay-chat-empty">표시할 채팅이 없습니다.</p> : page?.items.map(row => <div key={row.id} className={`platform-replay-chat-row is-${['donation', 'membership', 'gap', 'connected', 'ended'].includes(row.kind) ? row.kind : 'chat'}`}>
          {showTimes && row.timeMs !== null ? <time>{time(row.timeMs)}</time> : null}
          {row.author ? <button type="button" className="platform-replay-chat-author" style={row.color && /^#[a-f\d]{6}$/i.test(row.color) ? { color: row.color } : undefined} onClick={() => { setProfile(row); setSettings(false); }}>{row.author}</button> : null}
          {row.amount ? <strong className="platform-replay-chat-amount">{row.amount}</strong> : null}
          {row.kind === 'membership' ? <span className="platform-replay-chat-kind">멤버십</span> : null}
          <span className="platform-replay-chat-text"><PlatformChatParts text={row.text} parts={row.parts} images={images} /></span>
        </div>)}
      </div>
      {!following && page?.state === 'ready' ? <button type="button" className="platform-replay-chat-follow" onClick={() => { setCursor(null); setFollowing(true); }}>현재 시각으로 ↓</button> : null}
      {page?.state === 'unmapped' ? <div className="platform-replay-chat-pagination"><button type="button" disabled={!page.previousCursor || loading} onClick={() => navigate(page.previousCursor)}>이전</button><button type="button" disabled={!page.cursor || loading} onClick={() => navigate(page.cursor)}>다음</button></div> : null}
      {error ? <p role="alert" className="platform-replay-chat-error">채팅을 불러오지 못했습니다.<button type="button" onClick={() => setRetry(value => value + 1)}>다시 시도</button></p> : null}
      {profile ? <div role="dialog" aria-label="채팅 사용자 정보" className="platform-replay-chat-profile"><button type="button" className="platform-replay-chat-profile-x" aria-label="사용자 정보 닫기" onClick={() => setProfile(null)}><ReplayIcon name="close" provider={provider} /></button><strong>{profile.author}</strong><small>저장 당시 정보</small>{profile.badges.length ? <ul>{profile.badges.slice(0, 8).map((badge, index) => <li key={index}>{badge}</li>)}</ul> : <p>추가 정보 없음</p>}</div> : null}
    </>}
  </aside>;
}
