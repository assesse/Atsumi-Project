import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReplayApi, ReplayMessage, ReplayPage, ReplaySession, ReplaySyncQuality } from "../../api/replay";
import { boundedReplayMessages, formatReplayTime, replayAssetToken, replayMessageTime, replayVirtualRange, safeNicknameColor, type ReplayTimeLabel } from "./RecordingReplayModel";

type Props = { api: ReplayApi; session: ReplaySession; time: number; seekVersion: number; onSeek: (time: number) => void; onIndexReady: (quality: ReplaySyncQuality) => void };
const badgeLabels: Record<string, string> = { subscription: "구독", activity: "활동", profile: "배지", donation: "후원" };
function Decoration({ token, sessionToken, label, api }: { token?: string | null; sessionToken: string; label: string; api: ReplayApi }) {
  const [failed, setFailed] = useState(false);
  const safe = replayAssetToken(token);
  return safe && !failed ? <img className="recording-replay-decoration" src={`${api.mediaUrl(sessionToken)}/asset/${safe}`} alt={label} loading="lazy" referrerPolicy="no-referrer" onError={() => setFailed(true)} /> : <span className="recording-replay-badge" title="오프라인 이미지 없음">{label}</span>;
}
function Message({ item, mode, api, sessionToken, onSeek }: { item: ReplayMessage; mode: ReplayTimeLabel; api: ReplayApi; sessionToken: string; onSeek: Props["onSeek"] }) {
  const stamp = replayMessageTime(item, mode);
  const rich = item.rich;
  // Literal React text nodes preserve emoji syntax and cannot execute saved HTML.
  const parts = rich?.emojis?.length ? item.text.split(/(\{:[a-zA-Z0-9_]+:\})/g) : [item.text];
  const tooltip = `${item.serverTime == null ? "서버 시각 확인되지 않음" : `서버 ${new Date(item.serverTime).toLocaleString("ko-KR")}`} · 수신 ${new Date(item.receivedAt).toLocaleString("ko-KR")} · ${item.syncQuality === "observed_media" ? "관측 영상 기준" : "수신 시각 기준·동기화 근사"}`;
  return <div className="recording-replay-message" data-sequence={item.sequence} title={tooltip}>
    {stamp !== null ? <span className="recording-replay-chat-time">{stamp}</span> : null}
    {rich?.badges?.slice(0, 8).map((badge, index) => <Decoration key={index} token={item.assetIds?.[badge.imageUrl]} sessionToken={sessionToken} label={badge.title || badgeLabels[badge.kind] || "배지"} api={api} />)}
    <strong style={{ color: safeNicknameColor(rich?.nicknameColor) }}>{item.sender}</strong>{" "}
    <span>{parts.map((part, index) => {
      const emoji = rich?.emojis?.find((entry) => part === `{:${entry.id}:}`);
      return emoji && replayAssetToken(item.assetIds?.[emoji.imageUrl]) ? <Decoration key={index} token={item.assetIds?.[emoji.imageUrl]} sessionToken={sessionToken} label={part} api={api} /> : part;
    })}</span>
    <button className="recording-replay-row-seek" type="button" aria-label={`${formatReplayTime(item.mediaTimeSeconds)} 영상 위치로 이동`} onClick={() => onSeek(item.mediaTimeSeconds)}>↗</button>
  </div>;
}

export const RecordingReplayChat = memo(function RecordingReplayChat({ api, session, time, seekVersion, onSeek, onIndexReady }: Props) {
  const [page, setPage] = useState<ReplayPage | null>(null);
  const [following, setFollowing] = useState(true);
  const [mode, setMode] = useState<ReplayTimeLabel>("recording");
  const [fontSize, setFontSize] = useState(13);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const scroll = useRef<HTMLDivElement>(null);
  const clock = useRef(time); clock.current = time;
  const request = useRef<(() => void) | null>(null);
  const generation = useRef(0);
  const readyCallback = useRef(onIndexReady); readyCallback.current = onIndexReady;
  const pageRequest = useRef(false);
  const mounted = useRef(true);
  const [viewport, setViewport] = useState({ top: 0, height: 600 });
  const [heights, setHeights] = useState<Map<number, number>>(new Map());
  const [logMode, setLogMode] = useState(false);
  const hasChat = session.chatStatus !== "disabled";
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++generation.current; }; }, []);
  useEffect(() => { setFollowing(true); }, [seekVersion]);
  const items = page?.items;
  const virtual = useMemo(() => replayVirtualRange((items ?? []).map((item) => heights.get(item.sequence) ?? 44), viewport.top, viewport.height, following), [items, heights, viewport, following]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const update = () => setViewport((previous) => ({ top: previous.top, height: element.clientHeight || 600 }));
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update); observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { setHeights(new Map()); }, [fontSize, mode, viewport.height]);
  useLayoutEffect(() => {
    if (!items || !scroll.current) return;
    const allowed = new Set(items.map((item) => item.sequence));
    const measure = (entries: { target: Element }[]) => {
      setHeights((previous) => {
        const next = new Map([...previous].filter(([sequence]) => allowed.has(sequence)));
        let changed = next.size !== previous.size;
        for (const entry of entries) {
          const row = entry.target as HTMLElement;
          const sequence = Number(row.dataset.sequence), height = row.getBoundingClientRect().height;
          if (height > 0 && next.get(sequence) !== height) { next.set(sequence, height); changed = true; }
        }
        return changed ? next : previous;
      });
    };
    const rows = [...scroll.current.querySelectorAll<HTMLElement>("[data-sequence]")];
    measure(rows.map((target) => ({ target })));
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure); rows.forEach((row) => observer.observe(row));
    return () => observer.disconnect();
  }, [items, virtual.start, virtual.end]);

  useEffect(() => {
    if (!following || !hasChat) { request.current = null; return; }
    const version = ++generation.current;
    let disposed = false, inFlight = false, lastTime = -1;
    let retry: ReturnType<typeof setTimeout> | undefined;
    setError(null); setPage(null); setLogMode(false);
    const query = () => {
      if (disposed || inFlight) return;
      const target = clock.current;
      if (Math.abs(target - lastTime) < 0.2) return;
      lastTime = target; inFlight = true; setLoading(true);
      void api.chatAt(session.token, target, version).then((result) => {
        if (disposed || generation.current !== version) return;
        if (!result.ok) { setError(result.error.message); return; }
        if (result.data.generation !== version) return;
        setError(null); setPage({ ...result.data, items: boundedReplayMessages(result.data.items) });
        if (result.data.indexState === "ready") readyCallback.current(result.data.syncQuality);
        if (result.data.indexState === "building") { clearTimeout(retry); retry = setTimeout(() => { lastTime = -1; query(); }, 1000); }
      }).catch(() => { if (!disposed) setError("채팅을 읽지 못했습니다. 다시 시도해 주세요."); }).finally(() => {
        inFlight = false;
        if (!disposed && generation.current === version) { setLoading(false); if (Math.abs(clock.current - target) >= 0.2) query(); }
      });
    };
    request.current = query; query();
    return () => { disposed = true; clearTimeout(retry); request.current = null; };
  }, [api, session.token, session.manualOffsetSeconds, following, seekVersion, refresh, hasChat]);
  useEffect(() => { request.current?.(); }, [time]);
  useLayoutEffect(() => { if (following && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; }, [page, following, virtual.total]);
  const resume = () => { setFollowing(true); setLogMode(false); setRefresh((value) => value + 1); };
  const browse = useCallback(async (cursor: string | null) => {
    if (pageRequest.current) return;
    pageRequest.current = true; setFollowing(false); setLogMode(true); setLoading(true); setError(null);
    // Invalidate the time-window request synchronously before it can overwrite history.
    ++generation.current;
    const version = ++generation.current;
    try {
      const result = await api.chatPage(session.token, cursor, version);
      if (!mounted.current || generation.current !== version) return;
      if (result.ok && result.data.generation === version) {
        setPage({ ...result.data, items: boundedReplayMessages(result.data.items) });
        if (scroll.current) scroll.current.scrollTop = 0;
      } else if (!result.ok) setError(result.error.message);
    } catch { if (generation.current === version) setError("전체 로그를 읽지 못했습니다."); }
    finally { pageRequest.current = false; if (mounted.current) setLoading(false); }
  }, [api, session.token]);
  const status = session.chatStatus === "disabled" ? "채팅 저장 안 함" : session.chatStatus === "unknown" ? "채팅 저장 상태 확인되지 않음" : ["partial", "storage_failed", "connection_gap", "disconnected", "failed", "queue_overflow"].includes(session.chatStatus) ? "채팅 일부 누락 가능" : "저장된 채팅";
  return <aside className="recording-replay-chat" aria-label="저장 채팅 다시보기">
    <header><strong>채팅 다시보기</strong><span className="recording-replay-offline" title="저장된 메시지·배지만 표시합니다. 채팅 전송·후원은 제공하지 않습니다. 오프라인 이미지가 없으면 글자로 표시합니다.">읽기 전용</span></header>
    <div className="recording-replay-chat-options">
      <label><span className="recording-replay-sr-only">채팅 시각 표시</span><select aria-label="채팅 시각 표시" value={mode} onChange={(event) => setMode(event.target.value as ReplayTimeLabel)}><option value="recording">녹화 경과</option><option value="broadcast">방송 업타임</option><option value="hidden">시각 숨김</option></select></label>
      <label><span className="recording-replay-sr-only">채팅 글자 크기</span><select aria-label="채팅 글자 크기" value={fontSize} onChange={(event) => setFontSize(Number(event.target.value))}><option value={12}>작게</option><option value={13}>보통</option><option value={15}>크게</option><option value={18}>아주 크게</option></select></label>
      <button type="button" disabled={!hasChat || loading} onClick={() => void browse(null)}>전체 로그</button>
    </div>
    <p className="recording-replay-chat-status">{status}{page?.indexState === "building" ? " · 채팅 기록 준비 중" : ""}</p>
    {error ? <p className="recording-replay-error" role="alert">{error} <button type="button" onClick={resume}>다시 시도</button></p> : null}
    {page?.warnings.length ? <p className="recording-replay-warning" role="status">{page.warnings.join(" · ")}</p> : null}
    {logMode ? <nav className="recording-replay-log-nav" aria-label="전체 채팅 로그 페이지"><button type="button" disabled={!page?.previousCursor || loading} onClick={() => void browse(page!.previousCursor!)}>이전 기록</button><span>한 번에 최대 200개</span><button type="button" disabled={!page?.nextCursor || loading} onClick={() => void browse(page!.nextCursor!)}>다음 기록</button></nav> : null}
    <div ref={scroll} className="recording-replay-chat-scroll" style={{ fontSize }} tabIndex={0} role="region" aria-label="채팅 기록" aria-busy={loading} onScroll={() => {
      const element = scroll.current;
      if (element) setViewport((previous) => ({ ...previous, top: element.scrollTop }));
      if (following && element && element.scrollHeight - element.scrollTop - element.clientHeight > 48) { ++generation.current; setFollowing(false); setLoading(false); }
    }}>
      {hasChat && page?.previousCursor && !logMode ? <button className="recording-replay-older" type="button" onClick={() => void browse(page.previousCursor!)}>이전 채팅 더 보기</button> : null}
      <div aria-hidden="true" style={{ height: virtual.before }} />
      {page?.items.slice(virtual.start, virtual.end).map((item) => <Message key={item.sequence} item={item} mode={mode} api={api} sessionToken={session.token} onSeek={(target) => { resume(); onSeek(target); }} />)}
      <div aria-hidden="true" style={{ height: virtual.after }} />
      {!page?.items.length ? <p className="recording-replay-empty">{!hasChat ? "이 녹화에는 채팅을 저장하지 않았습니다." : loading ? "채팅을 준비하고 있습니다…" : "이 위치에 표시할 채팅이 없습니다."}</p> : null}
    </div>
    {!following ? <button className="recording-replay-follow" type="button" onClick={resume}>↓ 현재 재생 위치로</button> : null}
  </aside>;
});
