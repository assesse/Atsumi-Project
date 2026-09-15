import * as React from "react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { ReplayApi, ReplayMessage, ReplayPage, ReplaySession, ReplaySyncQuality } from "../../api/replay";
import { boundedReplayMessages, formatReplayTime, replayAssetToken, replayMessageTime, replayVirtualRange, safeNicknameColor, safeReplayProfile, visibleReplayWarnings, type ReplayTimeLabel } from "./RecordingReplayModel";
import { OriginalChzzkChatSurface } from "./OriginalChzzkChatSurface";
import { createOriginalChatPresentation, type OriginalChatMessage } from "./generated/originalChatPresentation.js";
import { createSurfaces } from "./generated/accepted/original-surfaces.js";

type Props = { api: ReplayApi; session: ReplaySession; time: number; seekVersion: number; onSeek: (time: number) => void; onIndexReady: (quality: ReplaySyncQuality) => void; onSetOffset?: (value: number) => Promise<string | null> };
const badgeLabels: Record<string, string> = { subscription: "구독", activity: "활동", profile: "배지", donation: "후원" };
type Presentation = ReturnType<typeof createOriginalChatPresentation>;
function Message({ item, mode, api, sessionToken, onSeek, presentation }: { item: ReplayMessage; mode: ReplayTimeLabel; api: ReplayApi; sessionToken: string; onSeek: Props["onSeek"]; presentation: Presentation }) {
  const stamp = replayMessageTime(item, mode);
  const rich = item.rich;
  const row = useRef<HTMLDivElement>(null);
  const [failedAssets, setFailedAssets] = useState<Set<string>>(new Set());
  const [profileError, setProfileError] = useState<string | null>(null);
  const [openingProfile, setOpeningProfile] = useState(false);
  const hasProfile = !!api.openProfile && safeReplayProfile(rich?.profileUrl);
  const openProfile = async () => {
    if (!api.openProfile || openingProfile || !safeReplayProfile(rich?.profileUrl)) return;
    setOpeningProfile(true); setProfileError(null);
    try { const result = await api.openProfile(sessionToken, item.sequence); if (!result.ok) setProfileError(result.error.message); }
    catch { setProfileError("프로필을 열지 못했습니다."); }
    finally { setOpeningProfile(false); }
  };
  const normalized = useMemo(() => {
    const localAsset = (url: string) => {
      const token = replayAssetToken(item.assetIds?.[url]);
      const src = token ? `${api.mediaUrl(sessionToken)}/asset/${token}` : null;
      return src && !failedAssets.has(src) ? src : null;
    };
    const badges = (rich?.badges ?? []).slice(0, 8).map((badge) => ({ src: localAsset(badge.imageUrl), label: badge.title || badgeLabels[badge.kind] || "배지" }));
    // Only literal React text and token-local images enter the original renderer.
    // Its original replay branch supplies the 24px disabled emoji button.
    const content = rich?.emojis?.length ? item.text.split(/(\{:[a-zA-Z0-9_]+:\})/g).map((part, index) => {
      const emoji = rich.emojis.find((entry) => part === `{:${entry.id}:}`);
      const src = emoji && localAsset(emoji.imageUrl);
      return src ? <img key={index} src={src} alt={part} title={emoji.id} /> : part;
    }) : item.text;
    const nicknameColor = safeNicknameColor(rich?.nicknameColor) ?? null;
    const profile = { nickname: item.sender, userIdHash: item.senderKey || String(item.sequence),
      viewerBadges: badges.flatMap((badge, order) => badge.src ? [{ order, activatedV2: true, badge: { scope: "CHANNEL", imageUrl: badge.src, title: badge.label, description: "" } }] : []) };
    const message: OriginalChatMessage = { key: String(item.sequence), user: profile.userIdHash, time: item.receivedAt, type: 1, status: "NORMAL", content, profile,
      displayBadgeList: badges.flatMap((badge) => badge.src ? [{ type: "ACTIVITY", imageSource: badge.src, title: badge.label, description: "" }] : []),
      displayNicknameColor: { light: nicknameColor, dark: nicknameColor } };
    return { message, missing: badges.filter(badge => !badge.src) };
  }, [item, rich, api, sessionToken, failedAssets]);
  useLayoutEffect(() => {
    row.current?.querySelectorAll("img").forEach(image => { image.referrerPolicy = "no-referrer"; });
    const button = row.current?.querySelector<HTMLButtonElement>('button[class*="_nickname_"]');
    if (!button) return;
    // The original online profile popup is replaced by the token-scoped local action.
    button.removeAttribute("aria-haspopup"); button.removeAttribute("aria-expanded");
    button.disabled = !hasProfile || openingProfile;
    button.setAttribute("aria-label", hasProfile ? `${item.sender} 프로필 열기` : item.sender);
    button.title = hasProfile ? `${item.sender} 프로필 열기` : "저장된 프로필 링크 없음";
  }, [hasProfile, openingProfile, item.sender, normalized]);
  const tooltip = `${item.serverTime == null ? "서버 시각 확인되지 않음" : `서버 ${new Date(item.serverTime).toLocaleString("ko-KR")}`} · 수신 ${new Date(item.receivedAt).toLocaleString("ko-KR")} · ${item.syncQuality === "observed_media" ? "관측 영상 기준" : "수신 시각 기준·동기화 근사"}`;
  const { ChatRow } = presentation;
  return <div ref={row} className="recording-replay-message" data-sequence={item.sequence} title={tooltip} style={{ "--replay-text-color": safeNicknameColor(rich?.textColor) } as CSSProperties} onErrorCapture={(event) => {
    if (event.target instanceof HTMLImageElement) { const src = event.target.src; setFailedAssets(previous => new Set([...previous, src])); }
  }}>
    {stamp !== null ? <span className="recording-replay-chat-time">{stamp}</span> : null}
    {normalized.missing.map((badge, index) => <span key={index} className="recording-replay-missing-badge" title="오프라인 이미지 없음">{badge.label}</span>)}
    <ChatRow chatMessage={normalized.message} onNicknameClick={() => void openProfile()} isCleanBotWorking={false} />
    {profileError ? <small className="recording-replay-profile-error" role="status">{profileError}</small> : null}
    <button className="recording-replay-row-seek" type="button" aria-label={`${formatReplayTime(item.mediaTimeSeconds)} 영상 위치로 이동`} onClick={() => onSeek(item.mediaTimeSeconds)}>↗</button>
  </div>;
}

export const RecordingReplayChat = memo(function RecordingReplayChat(props: Props) {
  return <OriginalChzzkChatSurface>{surface => <ReplayChatContent key={props.session.token} {...props} surface={surface} />}</OriginalChzzkChatSurface>;
});
function ReplayChatContent({ api, session, time, seekVersion, onSeek, onIndexReady, onSetOffset, surface }: Props & { surface: ShadowRoot }) {
  const presentation = useMemo(() => {
    // Original badge images can appear in a child effect after the row commits.
    // Apply transport metadata at element creation, not in a one-time DOM pass.
    const createElement = ((...args: Parameters<typeof React.createElement>) => {
      const [type, props, ...children] = args;
      if (type === "img") return React.createElement("img", { ...props, referrerPolicy: "no-referrer" }, ...children);
      return React.createElement(type, props, ...children);
    }) as typeof React.createElement;
    return createOriginalChatPresentation({ ...React, createElement }, { createPortal: children => createPortal(children, surface) });
  }, [surface]);
  const { MenuIcon } = presentation;
  const { ReplayShell: ChatShell, ReplaySearch } = useMemo(() => createSurfaces(React,
    { createPortal: (children: React.ReactNode) => createPortal(children, surface) }, null,
    { title: "", channelName: "", profileImage: "", viewers: 0, uptime: "", messages: [] }, () => {}), [surface]);
  const [page, setPage] = useState<ReplayPage | null>(null);
  const [following, setFollowing] = useState(true);
  const [mode, setMode] = useState<ReplayTimeLabel>("hidden");
  const [menu, setMenu] = useState(false);
  const [sync, setSync] = useState(false), [offset, setOffset] = useState(String(session.manualOffsetSeconds));
  const [savingOffset, setSavingOffset] = useState(false), [offsetError, setOffsetError] = useState<string | null>(null);
  const offsetInput = useRef<HTMLInputElement>(null);
  const menuAnchor = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const scroll = useRef<HTMLDivElement>(null);
  const clock = useRef(time); clock.current = time;
  const request = useRef<(() => void) | null>(null);
  const generation = useRef(0);
  const readyCallback = useRef(onIndexReady); readyCallback.current = onIndexReady;
  const pageRequest = useRef(false);
  const scrollToStart = useRef(false);
  const mounted = useRef(true);
  const [viewport, setViewport] = useState({ top: 0, height: 600, width: 340 });
  const [heights, setHeights] = useState<Map<number, number>>(new Map());
  const [logMode, setLogMode] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const searching = searchQuery.trim().length > 0;
  const hasChat = session.chatStatus !== "disabled";
  const setScroll = useCallback((element: HTMLDivElement | null) => {
    scroll.current = element;
    if (element) { element.classList.add("recording-replay-chat-scroll"); element.tabIndex = 0; element.setAttribute("aria-label", "채팅 기록"); }
  }, []);
  useEffect(() => { scroll.current?.setAttribute("aria-busy", String(loading)); }, [loading]);
  useEffect(() => {
    if (!menu) return;
    menuAnchor.current?.querySelector<HTMLButtonElement>('[role="radio"][aria-checked="true"]')?.focus();
    const dismiss = (event: PointerEvent) => { if (menuAnchor.current && !event.composedPath().includes(menuAnchor.current)) setMenu(false); };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, [menu]);
  useEffect(() => { if (menu && sync) offsetInput.current?.focus(); }, [menu, sync]);
  const closeMenu = () => { setMenu(false); menuAnchor.current?.querySelector<HTMLButtonElement>("button")?.focus(); };
  const applyOffset = async () => {
    if (!onSetOffset || savingOffset) return;
    const value = Number(offset);
    if (!offset.trim() || !Number.isFinite(value) || Math.abs(value) > 3600) { setOffsetError("-3600~3600초로 입력해 주세요."); return; }
    setSavingOffset(true); setOffsetError(null);
    try {
      const message = await onSetOffset(value);
      if (!mounted.current) return;
      if (message) setOffsetError(message); else { setSync(false); closeMenu(); }
    } catch { if (mounted.current) setOffsetError("채팅 보정을 저장하지 못했습니다."); }
    finally { if (mounted.current) setSavingOffset(false); }
  };
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++generation.current; }; }, []);
  useEffect(() => { if (!searching) setFollowing(true); }, [seekVersion]);
  const items = page?.items;
  const virtual = useMemo(() => replayVirtualRange((items ?? []).map((item) => heights.get(item.sequence) ?? 44), viewport.top, viewport.height, following), [items, heights, viewport, following]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const update = () => setViewport((previous) => {
      const height = element.clientHeight || 600, width = element.clientWidth || 340;
      return previous.height === height && previous.width === width ? previous : { ...previous, height, width };
    });
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update); observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { setHeights(new Map()); }, [mode, viewport.height, viewport.width]);
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
    if (!following || !hasChat || searching) { request.current = null; return; }
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
  }, [api, session.token, session.manualOffsetSeconds, following, seekVersion, refresh, hasChat, searching]);
  useEffect(() => {
    if (!searching || !hasChat) return;
    const version = ++generation.current;
    let disposed = false;
    let retry: ReturnType<typeof setTimeout>;
    setFollowing(false); setLogMode(true); setLoading(true); setPage(null); setError(null);
    const load = async () => {
      if (disposed) return;
      try {
        if (!api.chatSearch) { setError("이 환경에서는 저장 채팅 검색을 사용할 수 없습니다."); return; }
        const result = await api.chatSearch(session.token, searchQuery, "all", null, version);
        if (disposed || generation.current !== version) return;
        if (!result.ok) { setError(result.error.message); return; }
        if (result.data.generation !== version) return;
        scrollToStart.current = true;
        setPage({ ...result.data, items: boundedReplayMessages(result.data.items) });
        if (result.data.indexState === "ready") readyCallback.current(result.data.syncQuality);
        else if (result.data.indexState === "building") retry = setTimeout(() => void load(), 1000);
      } catch { if (!disposed && generation.current === version) setError("검색 결과를 읽지 못했습니다."); }
      finally { if (!disposed && generation.current === version) setLoading(false); }
    };
    retry = setTimeout(() => void load(), 200);
    return () => { disposed = true; clearTimeout(retry); };
  }, [api, session.token, session.manualOffsetSeconds, searching, searchQuery, hasChat, refresh]);
  useEffect(() => { request.current?.(); }, [time]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    if (following) element.scrollTop = 0;
    else if (scrollToStart.current) {
      element.scrollTop = -element.scrollHeight; scrollToStart.current = false;
      setViewport(previous => ({ ...previous, top: 0 }));
    }
  }, [page, following, virtual.total]);
  const resume = () => { ++generation.current; setSearchQuery(""); setFollowing(true); setLogMode(false); setRefresh((value) => value + 1); };
  const changeQuery = (value: string) => { if (value === searchQuery) return; ++generation.current; setSearchQuery(value.slice(0, 256)); setFollowing(!value.trim()); if (!value.trim()) setRefresh(current => current + 1); };
  const browse = useCallback(async (cursor: string | null, query = searchQuery) => {
    if (pageRequest.current) return;
    pageRequest.current = true; setFollowing(false); setLogMode(true); setLoading(true); setError(null);
    // Invalidate the time-window request synchronously before it can overwrite history.
    ++generation.current;
    const version = ++generation.current;
    try {
      const result = query.trim() && api.chatSearch
        ? await api.chatSearch(session.token, query, "all", cursor, version)
        : await api.chatPage(session.token, cursor, version);
      if (!mounted.current || generation.current !== version) return;
      if (result.ok && result.data.generation === version) {
        scrollToStart.current = true;
        setPage({ ...result.data, items: boundedReplayMessages(result.data.items) });
      } else if (!result.ok) setError(result.error.message);
    } catch { if (generation.current === version) setError("전체 로그를 읽지 못했습니다."); }
    finally { pageRequest.current = false; if (mounted.current && generation.current === version) setLoading(false); }
  }, [api, session.token, searchQuery]);
  const status = session.chatStatus === "disabled" ? "채팅 저장 안 함" : session.chatStatus === "unknown" ? "채팅 저장 상태 확인되지 않음" : ["partial", "storage_failed", "connection_gap", "disconnected", "failed", "queue_overflow"].includes(session.chatStatus) ? "채팅 일부 누락 가능" : "저장된 채팅";
  return <ChatShell title="채팅" listRef={setScroll} onScroll={() => {
    const element = scroll.current;
    if (element) setViewport(previous => ({ ...previous, top: Math.max(0, element.scrollHeight - element.clientHeight + element.scrollTop) }));
    // Original CHZZK uses column-reverse: zero is the newest message, upward scroll is negative.
    if (following && element && element.scrollTop < -48) { ++generation.current; setFollowing(false); setLoading(false); }
  }} headerControls={<div ref={menuAnchor} className="recording-replay-menu-anchor" onKeyDown={event => {
    if (event.key === "Escape" && menu) { event.preventDefault(); event.stopPropagation(); closeMenu(); }
    else if (menu && event.key !== "Tab") event.stopPropagation();
  }}>
    <button type="button" className="recording-replay-menu-toggle" aria-label="채팅 메뉴" title="채팅 메뉴" aria-expanded={menu} onClick={() => { setMenu(value => !value); setSync(false); setOffsetError(null); }}><MenuIcon aria-hidden /></button>
    {menu && <div className="recording-replay-chat-options">
      <p className="recording-replay-menu-label">시간 표시</p>
      <div className="recording-replay-time-options" role="radiogroup" aria-label="채팅 시각 표시" onKeyDown={event => {
        if (!["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key)) return;
        event.preventDefault(); event.stopPropagation();
        const options = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
        const index = options.indexOf(event.target as HTMLButtonElement), direction = ["ArrowUp", "ArrowLeft"].includes(event.key) ? -1 : 1;
        const next = options[(index + direction + options.length) % options.length]; next?.click(); next?.focus();
      }}>
        {([['hidden', '시각 숨김'], ['recording', '녹화 경과'], ['broadcast', '방송 업타임']] as const).map(([value, label]) => <button key={value} type="button" role="radio" aria-checked={mode === value} tabIndex={mode === value ? 0 : -1} onClick={() => setMode(value)}>{label}<span aria-hidden="true">{mode === value ? '✓' : ''}</span></button>)}
      </div>
      {onSetOffset ? <button type="button" aria-label="채팅 시간 조절" aria-expanded={sync} onClick={() => { setSync(value => !value); setOffset(String(session.manualOffsetSeconds)); setOffsetError(null); }}>채팅 시간 조절<span aria-hidden="true">{sync ? '−' : '+'}</span></button> : null}
      {sync ? <form className="recording-replay-sync" onSubmit={event => { event.preventDefault(); void applyOffset(); }}><label>채팅 시간 (초)<input ref={offsetInput} type="number" min={-3600} max={3600} step={.1} value={offset} disabled={savingOffset} onChange={event => setOffset(event.target.value)} /></label>{offsetError ? <p className="recording-replay-error" role="alert">{offsetError}</p> : null}<button type="submit" disabled={savingOffset}>{savingOffset ? "저장 중…" : "적용"}</button></form> : null}
      <button type="button" disabled={!hasChat || loading} onClick={() => { setMenu(false); setSearchQuery(""); void browse(null, ""); }}>전체 로그</button>
      <p className="recording-replay-chat-status">{status}</p>
    </div>}
  </div>} floatingContent={!following ? <button className="recording-replay-follow" type="button" onClick={resume}>↓ 현재 재생 위치로</button> : null} footer={<><div className="recording-replay-chat-notices">
    {page?.indexState === "building" ? <p className="recording-replay-chat-status" role="status">채팅 기록 준비 중</p> : null}
    {status !== "저장된 채팅" && hasChat ? <p className="recording-replay-warning" role="status">{status}</p> : null}
    {error ? <p className="recording-replay-error" role="alert">{error} <button type="button" onClick={() => searching ? setRefresh(value => value + 1) : resume()}>다시 시도</button></p> : null}
    {visibleReplayWarnings(page?.warnings).length ? <p className="recording-replay-warning" role="status">{visibleReplayWarnings(page?.warnings).join(" · ")}</p> : null}
    {logMode ? <nav className="recording-replay-log-nav" aria-label="전체 채팅 로그 페이지"><button type="button" disabled={!page?.previousCursor || loading} onClick={() => void browse(page!.previousCursor!)}>이전 기록</button><button type="button" disabled={!page?.nextCursor || loading} onClick={() => void browse(page!.nextCursor!)}>다음 기록</button></nav> : null}
  </div><ReplaySearch query={searchQuery} count={page?.items.length ?? 0}
    countText={loading ? "불러오는 중" : `${page?.items.length ?? 0}개 표시`}
    onQuery={changeQuery} /></>}>
    <div aria-hidden="true" style={{ height: virtual.after }} />
    {page?.items.slice(virtual.start, virtual.end).reverse().map(item => <Message key={item.sequence} item={item} mode={mode} api={api} sessionToken={session.token} presentation={presentation} onSeek={target => { resume(); onSeek(target); }} />)}
    <div aria-hidden="true" style={{ height: virtual.before }} />
    {hasChat && page?.previousCursor && !logMode ? <button className="recording-replay-older" type="button" onClick={() => void browse(page.previousCursor!)}>이전 채팅 더 보기</button> : null}
    {!page?.items.length ? <p className="recording-replay-empty">{!hasChat ? "이 녹화에는 채팅을 저장하지 않았습니다." : loading ? "채팅을 준비하고 있습니다…" : searching ? "검색 결과가 없습니다." : "이 위치에 표시할 채팅이 없습니다."}</p> : null}
  </ChatShell>;
}
