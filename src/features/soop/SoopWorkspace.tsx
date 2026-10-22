import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createSoopApi, type SoopApi, type SoopChannel } from "../../api/soop";
import { normalizeSoopChannel } from "./soopLinks";
import { soopAutoRecordApi, type SoopAutoRecordApi } from "../../api/soopAutoRecord";
import { autoRecordLabel, useSoopAutoRecord } from "./useSoopAutoRecord";
import { readSoopFavorites, SOOP_FAVORITES_LIMIT, writeSoopFavorites, type SoopFavorite } from "./soopFavorites";
import "./SoopWorkspace.css";
import { SoopDiscovery } from "./SoopDiscovery";
import { PlatformMediaLibrary } from "../platforms/PlatformMediaLibrary";
import { PlatformScreenshotLibrary } from "../platforms/PlatformScreenshotLibrary";
import { PlatformLivePlayer } from "../platforms/PlatformLivePlayer";
import { FluentIcon } from "../../components/FluentIcon";
import { usePanelHandleGesture } from "../streaming/usePanelHandleGesture";
import { SoopConnectionSetup, type SoopLiveArrangement } from "./SoopConnectionSetup";
import { SoopLivePaneGrid } from "./SoopLivePaneGrid";
import "./SoopLiveShell.css";
import { SoopOriginalPlayer } from "./SoopOriginalPlayer";
import { soopOriginalApi, type SoopAccountSnapshot } from "../../api/soopOriginal";

export type SoopWorkspaceProps = {
  active: boolean; runtime: "tauri" | "browser-mock"; privacyMode: boolean;
  view: "explore" | "live" | "favorites" | "screenshots" | "downloads";
  onNotice?: (message: string) => void; api?: SoopApi;
  onViewChange?: (view: "explore" | "live" | "favorites" | "screenshots" | "downloads") => void;
  onWatchingChange?: (watching: boolean) => void;
  autoRecordApi?: SoopAutoRecordApi;
  previewWidth?: number;
};
type Row = { key: number; input: string };
type RowState = { id: string | null; profile: SoopChannel | null; loading: boolean; error: string };
const statusLabel = (status: SoopChannel["status"] | undefined) => status === "live" ? "● 방송 중" : status === "offline" ? "방송 종료" : "방송 확인 중";
const ARRANGEMENT_KEY = "atsumi.soop.live.arrangement.v1";
const safeError = (cause: unknown) => typeof cause === "string" ? cause.slice(0, 220) : cause instanceof Error ? cause.message.slice(0, 220) : "채널을 불러오지 못했습니다. 다시 시도해 주세요.";
const publicImage = (value: string | null | undefined) => value && /^data:image\/(?:png|jpeg|webp|gif);base64,[a-zA-Z0-9+/=]+$/.test(value) && value.length < 180_000 ? value : undefined;
function Star({ filled }: { filled: boolean }) {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-2.9-5.6 2.9 1.1-6.2L3 9.6l6.2-.9Z" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" /></svg>;
}
function Avatar({ channel, privacy }: { channel?: Pick<SoopChannel, "image" | "name">; privacy: boolean }) {
  const image = privacy ? undefined : publicImage(channel?.image);
  return <span className={`soop-avatar${!channel ? " is-skeleton" : ""}`} aria-hidden="true">{image ? <img src={image} alt="" onError={event => { event.currentTarget.hidden = true; }} /> : null}{privacy ? "•" : channel?.name.slice(0, 1) || ""}</span>;
}

function ChannelInput({ row, index, api, enabled, privacy, favorite, onInput, onMeta, onFavorite, onRemove }: {
  row: Row; index: number; api: SoopApi; enabled: boolean; privacy: boolean; favorite: boolean;
  onInput: (value: string) => void; onMeta: (key: number, value: RowState) => void;
  onFavorite: (channel: SoopChannel) => void; onRemove?: () => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const id = normalizeSoopChannel(row.input);
  const [state, setState] = useState<RowState>({ id: null, profile: null, loading: false, error: "" });
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let cancelled = false;
    const publish = (value: RowState) => { if (!cancelled) { setState(value); onMeta(row.key, value); } };
    publish({ id, profile: null, loading: !!id && enabled && !privacy, error: "" });
    if (!id || !enabled || privacy) return;
    const timer = window.setTimeout(() => { void api.inspect(id).then(profile => {
      if (normalizeSoopChannel(profile.channelId) !== id) throw new Error("요청한 채널과 응답이 일치하지 않습니다.");
      publish({ id, profile, loading: false, error: "" });
    }).catch(cause => publish({ id, profile: null, loading: false, error: safeError(cause) })); }, 350);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [id, enabled, privacy, api, row.key, onMeta, retry]);
  const profile = state.id === id ? state.profile : null;
  return <div className="soop-channel-row">
    <span className="soop-channel-number">{index + 1}</span>
    <div className={`soop-input-identity${state.loading ? " is-loading" : ""}`} aria-busy={state.loading || undefined} title={!privacy && profile ? statusLabel(profile.status) : undefined}><Avatar channel={profile ?? undefined} privacy={privacy} /><span><strong className={!profile && !privacy ? "soop-profile-skeleton" : undefined}>{privacy ? "방송" : profile?.name || ""}</strong>{state.loading ? <span className="soop-profile-loading" aria-label="채널 확인 중" /> : null}</span></div>
    <div className="soop-input-wrap"><input ref={inputRef} aria-label={`${index + 1}번 SOOP 채널 링크 또는 ID`} type={privacy ? "password" : "text"} value={row.input} disabled={privacy} spellCheck={false} autoComplete="off" placeholder="채널 링크 또는 ID" onChange={event => onInput(event.target.value)} onPaste={event => { const parsed = normalizeSoopChannel(event.clipboardData.getData("text")); if (parsed) { event.preventDefault(); onInput(parsed); } }} /><button type="button" className="soop-clear" aria-label={`${index + 1}번 채널 ${onRemove ? "제거" : "입력 지우기"}`} hidden={!row.input && !onRemove} disabled={privacy} onClick={() => { if (onRemove) onRemove(); else { onInput(""); inputRef.current?.focus(); } }}><FluentIcon glyph={'\uE711'} /></button></div>
    <button type="button" className="soop-star" title={favorite ? "즐겨찾기 해제" : "즐겨찾기 추가"} aria-label={`${index + 1}번 채널 즐겨찾기 ${favorite ? "해제" : "추가"}`} aria-pressed={favorite} disabled={!profile || state.loading || privacy} onClick={() => profile && onFavorite(profile)}><Star filled={favorite} /></button>
    {!privacy && (row.input.trim() && !id || state.error) ? <p className="soop-inline-error" role="alert">{state.error || "채널 링크 또는 ID를 확인해 주세요."}{state.error ? <button type="button" onClick={() => setRetry(value => value + 1)}>다시 시도</button> : null}</p> : null}
  </div>;
}

export function SoopWorkspace({ active, runtime, privacyMode, view: requestedView, onNotice, api: suppliedApi, onViewChange, onWatchingChange, autoRecordApi, previewWidth = 320 }: SoopWorkspaceProps) {
  // Accept an old/shared navigation value without restoring a removed sidebar page.
  const view = requestedView === "favorites" ? "explore" : requestedView;
  useEffect(() => { if (active && requestedView === "favorites") onViewChange?.("explore"); }, [active, requestedView, onViewChange]);
  const api = useMemo(() => suppliedApi ?? createSoopApi(runtime), [runtime, suppliedApi]);
  const reservations = useSoopAutoRecord(autoRecordApi ?? (runtime === "tauri" ? soopAutoRecordApi : null), active, onNotice);
  const [rows, setRows] = useState<Row[]>([{ key: 0, input: "" }]);
  const [rowStates, setRowStates] = useState<Record<number, RowState>>({});
  const [favorites, setFavorites] = useState<SoopFavorite[]>(readSoopFavorites);
  const [favoriteOrder, setFavoriteOrder] = useState(() => favorites.map(channel => channel.channelId));
  const [profiles, setProfiles] = useState<Record<string, SoopChannel>>({});
  const [pendingFavorites, setPendingFavorites] = useState<string[]>([]);
  const [watching, setWatching] = useState<SoopChannel[]>([]);
  const [resumeRequests,setResumeRequests]=useState<Record<string,number>>({});
  const isWatching = watching.length > 0;
  useEffect(() => { onWatchingChange?.(isWatching); }, [isWatching, onWatchingChange]);
  useEffect(() => () => { onWatchingChange?.(false); }, [onWatchingChange]);
  const [mode, setMode] = useState<"all" | "selected">("all"), [appliedMode, setAppliedMode] = useState<"all" | "selected">("all");
  const [lead, setLead] = useState(""), [appliedLead, setAppliedLead] = useState("");
  const [drawerOpen, setDrawerOpen] = useState(false), [error, setError] = useState("");
  const [account, setAccount] = useState<SoopAccountSnapshot | null>(null), [accountBusy, setAccountBusy] = useState(false);
  const accountAction = useRef(0), accountActionPending = useRef(false);
  useEffect(() => { if (runtime !== "tauri" || !active || !drawerOpen) return; let gone=false,running=false; const refresh=()=>{if(running||accountActionPending.current)return;running=true;const action=accountAction.current;void soopOriginalApi.account().then(value=>{if(!gone&&action===accountAction.current&&!accountActionPending.current)setAccount(value);}).catch(()=>undefined).finally(()=>{running=false;});};refresh();const timer=setInterval(refresh,2000);return()=>{gone=true;clearInterval(timer);}; }, [runtime,active,drawerOpen]);
  const changeAccount=(logout:boolean)=>{
    if(runtime!=="tauri"||!active||privacyMode||accountActionPending.current||(logout?account?.authenticated!==true:account?.authenticated===true))return;
    const action=++accountAction.current;accountActionPending.current=true;setAccountBusy(true);
    void (logout?soopOriginalApi.logout():soopOriginalApi.login()).then(value=>{if(action===accountAction.current)setAccount(value);}).catch(cause=>{if(action===accountAction.current)setError(safeError(cause));}).finally(()=>{if(action===accountAction.current){accountActionPending.current=false;setAccountBusy(false);}});
  };
  const login=()=>changeAccount(false), logout=()=>changeAccount(true);
  const [arrangement, setArrangement] = useState<SoopLiveArrangement>(() => { try { return localStorage.getItem(ARRANGEMENT_KEY) === "grouped" ? "grouped" : "channels"; } catch { return "channels"; } });
  const [appliedArrangement, setAppliedArrangement] = useState<SoopLiveArrangement>(arrangement);
  const [selectedChat, setSelectedChat] = useState("");
  const [compactPanes, setCompactPanes] = useState(false);
  const playerArea = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const area = playerArea.current;
    if (!area) return;
    const measure = () => { const width = area.getBoundingClientRect().width; if (width > 0) setCompactPanes(width < 800 || window.innerHeight < 700); };
    measure(); const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure); observer?.observe(area);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, []);
  const chatId = watching.some(channel => channel.channelId === selectedChat) ? selectedChat : watching[0]?.channelId ?? "";
  const nextRow = useRef(1);
  const handle = useRef<HTMLButtonElement>(null), drawer = useRef<HTMLDivElement>(null);
  const handleGesture = usePanelHandleGesture(drawerOpen, "y", open => { if (open) setDrawerOpen(true); else closeDrawer(); });
  const visibleFavorites = useMemo(() => {
    const all = new Map(favorites.map(channel => [channel.channelId, channel]));
    for (const channel of reservations.snapshot.channels) {
      if ((channel.enabled || channel.jobId || ["starting", "recording", "stopping"].includes(channel.status)) && !all.has(channel.channelId)) all.set(channel.channelId, { channelId: channel.channelId, name: channel.name });
    }
    const ordered = favoriteOrder.filter(id => all.has(id));
    const retained = new Set(ordered);
    return [...ordered, ...[...all.keys()].filter(id => !retained.has(id))].map(id => all.get(id)!);
  }, [favorites, reservations.snapshot.channels, favoriteOrder]);
  const favoritesKey = visibleFavorites.map(entry => entry.channelId).join(",");
  useEffect(() => {
    const next = favoritesKey ? favoritesKey.split(",") : [];
    setFavoriteOrder(previous => previous.join(",") === favoritesKey ? previous : next);
  }, [favoritesKey]);
  const rowMeta = useCallback((key: number, value: RowState) => {
    setRowStates(saved => ({ ...saved, [key]: value }));
    if (value.profile) setProfiles(saved => ({ ...saved, [value.profile!.channelId]: value.profile! }));
  }, []);
  useEffect(() => {
    const rowKeys = new Set(rows.map(row => String(row.key)));
    const ids = new Set([...rows.map(row => normalizeSoopChannel(row.input)), ...visibleFavorites.map(item => item.channelId), ...watching.map(item => item.channelId)]);
    // Avatars are bounded data URLs: retain only the four drafts/players and up to 100 favorites.
    setRowStates(previous => Object.keys(previous).every(key => rowKeys.has(key)) ? previous : Object.fromEntries(Object.entries(previous).filter(([key]) => rowKeys.has(key))));
    setProfiles(previous => Object.keys(previous).every(id => ids.has(id)) ? previous : Object.fromEntries(Object.entries(previous).filter(([id]) => ids.has(id))));
  }, [rows, visibleFavorites, watching]);
  useEffect(() => {
    if (!active || privacyMode || !favoritesKey || (runtime === "browser-mock" && !suppliedApi)) return;
    let cancelled = false, timer: number | undefined;
    const ids = favoritesKey.split(",");
    const refresh = async () => {
      setPendingFavorites(ids); let cursor = 0;
      const worker = async () => {
        while (!cancelled && cursor < ids.length) {
          const id = ids[cursor++]; if (!id) continue;
          try { const value = await api.inspect(id); if (!cancelled && normalizeSoopChannel(value.channelId) === id) setProfiles(saved => ({ ...saved, [id]: value })); }
          catch { if (!cancelled) setProfiles(saved => saved[id] ? { ...saved, [id]: { ...saved[id]!, status: "unknown", viewers: null } } : saved); }
          finally { if (!cancelled) setPendingFavorites(pending => pending.filter(entry => entry !== id)); }
        }
      };
      await Promise.all([worker(), worker()]);
      if (!cancelled) timer = window.setTimeout(() => { void refresh(); }, 60_000);
    };
    void refresh(); return () => { cancelled = true; if (timer !== undefined) window.clearTimeout(timer); };
  }, [active, privacyMode, favoritesKey, api, runtime, suppliedApi]);
  function toggleFavorite(channel: SoopFavorite) {
    const exists = favorites.some(entry => entry.channelId === channel.channelId);
    if (!exists && favorites.length >= SOOP_FAVORITES_LIMIT) { setError("즐겨찾기는 100개까지 저장할 수 있습니다."); return; }
    const next = exists ? favorites.filter(entry => entry.channelId !== channel.channelId) : [...favorites, { channelId: channel.channelId, name: channel.name }];
    if (!writeSoopFavorites(next)) { setError("즐겨찾기를 저장하지 못했습니다. 저장 공간을 확인해 주세요."); return; }
    setFavorites(next);
  }
  function closeDrawer() {
    if (drawer.current?.contains(document.activeElement)) handle.current?.focus({ preventScroll: true });
    setDrawerOpen(false);
  }
  function chooseFavorite(id: string) {
    if (rows.some(row => normalizeSoopChannel(row.input) === id)) { setRows(current => current.map(row => normalizeSoopChannel(row.input) === id ? { ...row, input: "" } : row)); setDrawerOpen(true); onViewChange?.("live"); return; }
    if (rows.length >= 4 && rows.every(row => row.input.trim())) { setError("방송은 최대 4개까지 선택할 수 있습니다."); return; }
    setRows(current => {
      const empty = current.find(row => !row.input.trim());
      if (empty) return current.map(row => row.key === empty.key ? { ...row, input: id } : row);
      if (current.length < 4) return [...current, { key: nextRow.current++, input: id }];
      return current;
    });
    setDrawerOpen(true); onViewChange?.("live");
  }
  const entered = rows.filter(row => row.input.trim());
  const ready = entered.length > 0 && entered.every(row => {
    const id = normalizeSoopChannel(row.input), state = rowStates[row.key];
    return !!id && state?.id === id && !state.loading && !!state.profile && state.profile.status !== "unknown";
  });
  const liveRows = rows.flatMap(row => { const id = normalizeSoopChannel(row.input), profile = rowStates[row.key]?.profile; return profile?.channelId === id && profile.status === "live" ? [profile] : []; });
  const selectedLead = liveRows.some(channel => channel.channelId === lead) ? lead : liveRows[0]?.channelId ?? "";
  function start() {
    if (!ready || !liveRows.length || privacyMode || view !== "live") return;
    const unique = [...new Map(liveRows.map(channel => [channel.channelId, channel])).values()];
    // Unchanged channel keys preserve their native receiver. Input edits remain drafts until apply.
    setWatching(current => unique.map(channel => current.find(old => old.channelId === channel.channelId) ?? channel));
    setAppliedMode(mode); setAppliedLead(selectedLead); setAppliedArrangement(arrangement); setError(""); closeDrawer();
    try { localStorage.setItem(ARRANGEMENT_KEY, arrangement); } catch { /* Layout preference is optional. */ }
    if (liveRows.length < entered.length) onNotice?.("종료된 방송을 제외하고 시청합니다.");
  }
  const favoritePanel = <section className="soop-channel-picker"><div className="soop-channel-picker-title"><strong>즐겨찾기</strong><span>자동 녹화 ◉</span><button type="button" className="soop-channel-manage" disabled title="SOOP은 채널 옆 녹화 아이콘에서 예약을 설정합니다." aria-label="자동 녹화 관리">⋯</button></div><div className="soop-favorites" aria-label="즐겨찾기 채널">
    {visibleFavorites.length === 0 ? <p className="soop-favorites-empty">별표를 눌러 채널을 저장하세요.</p> : visibleFavorites.map(favorite => {
      const channel = profiles[favorite.channelId], label = privacyMode ? "즐겨찾기 채널" : channel?.name || favorite.name;
      const selected = rows.some(row => normalizeSoopChannel(row.input) === favorite.channelId);
      const reservation = reservations.snapshot.channels.find(entry => entry.channelId === favorite.channelId);
      const starred = favorites.some(entry => entry.channelId === favorite.channelId);
      const liveStatus = reservation?.checkedAt && reservation.liveStatus !== "unknown" ? reservation.liveStatus : channel?.status;
      const saving = reservations.pending === favorite.channelId;
      const reservationText = reservations.snapshot.ready ? autoRecordLabel(reservation) : reservations.error ? "예약 확인 불가" : "예약 확인 중";
      return <div className={`soop-favorite${selected ? " is-selected" : ""}${starred ? " is-favorite" : ""}`} key={favorite.channelId}>
        <button type="button" className="soop-favorite-select" disabled={privacyMode} aria-label={`${label} 선택`} aria-pressed={selected} onClick={() => chooseFavorite(favorite.channelId)}><Avatar channel={channel ?? { name: favorite.name, image: null }} privacy={privacyMode} /><span className="soop-favorite-copy"><strong>{label}</strong><span className={`soop-channel-presence${!privacyMode && liveStatus === "live" ? " is-live" : ""}`}>{privacyMode ? "" : statusLabel(liveStatus)}{pendingFavorites.includes(favorite.channelId) && !privacyMode ? <span className="soop-spinner" aria-label="채널 확인 중" /> : null}</span><small className={reservation?.status === "failed" ? "soop-reservation-failed" : ""} title={privacyMode ? undefined : reservation?.error ?? reservationText}>{privacyMode ? "" : reservationText}</small></span></button>
        <div className="soop-channel-actions">
        <button type="button" className="soop-record-toggle" disabled={privacyMode || !reservations.canToggle(favorite.channelId)} aria-label={`${label} 녹화 예약 ${reservation?.enabled ? "끄기" : "켜기"}`} aria-pressed={!!reservation?.enabled} aria-busy={saving} title={reservation?.enabled ? "녹화 예약 끄기" : "녹화 예약 켜기"} onClick={() => void reservations.toggle(favorite.channelId, channel?.name || favorite.name)}>{saving ? <span className="soop-spinner" aria-label="예약 저장 중" /> : <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8.5" /><circle cx="12" cy="12" r="4" /></svg>}</button>
        <button type="button" className="soop-star" disabled={privacyMode} aria-label={`${label} 즐겨찾기 ${starred ? "해제" : "추가"}`} aria-pressed={starred} title={starred ? "즐겨찾기 해제" : "즐겨찾기 추가"} onClick={() => toggleFavorite(favorite)}><Star filled={starred} /></button>
        </div>
      </div>;
    })}
  </div>{reservations.error && !privacyMode ? <p className="soop-inline-error" role="alert">{reservations.error}</p> : null}</section>;
  return <section className={`soop-workspace${view === "live" ? " is-live-view" : ""}`} data-tour="soop-content" aria-label="SOOP" hidden={!active} inert={!active}>
    <div hidden={view !== "explore"}>
      <SoopDiscovery api={api} active={active && view === "explore"} legacyFavoritesView={requestedView === "favorites"} privacyMode={privacyMode} onNotice={onNotice} previewWidth={previewWidth}
        isChannelFavorite={id => favorites.some(item => item.channelId === id)} onFavoriteChannel={item => toggleFavorite({ channelId: item.channelId, name: item.channelName })}
        onLive={item => { const channel: SoopChannel = { channelId: item.channelId, name: item.channelName, title: item.title, image: null, status: "live", viewers: item.views }; setRows([{ key: nextRow.current++, input: item.channelId }]); if(watching.some(old=>old.channelId===item.channelId))setResumeRequests(old=>({...old,[item.channelId]:(old[item.channelId]??0)+1}));setWatching(current => [current.find(old => old.channelId === item.channelId) ?? channel]); setAppliedMode("all"); closeDrawer(); onViewChange?.("live"); }} />
    </div>
    <div hidden={view !== "downloads"}><PlatformMediaLibrary provider="soop" active={active && view === "downloads"} privacyMode={privacyMode} onNotice={onNotice} /></div>
    <PlatformScreenshotLibrary provider="soop" active={active && view === "screenshots"} privacyMode={privacyMode} onNotice={onNotice} />
    <div ref={playerArea} className="soop-player-area" hidden={view !== "live"} data-count={watching.length} data-mode={appliedMode} data-arrangement={appliedArrangement} data-tabbed={compactPanes && watching.length > 1}>
      <SoopLivePaneGrid kind="pair" className="soop-channel-pairs" storageId="channel-pairs" grouped={appliedMode === "all" && appliedArrangement === "grouped"} tabbed={appliedMode === "all" && compactPanes && watching.length > 1} onResizing={() => undefined} accessory={<div className="soop-chat-tabs" role="tablist" aria-label="방송 채팅">{watching.map((channel, index) => <button key={channel.channelId} type="button" role="tab" aria-selected={chatId === channel.channelId} onClick={() => setSelectedChat(channel.channelId)}>{privacyMode ? `방송 ${index + 1}` : channel.name}</button>)}</div>}>
        {watching.map((channel, index) => <div className="soop-player-cell" key={channel.channelId} data-channel-index={index} data-chat-selected={channel.channelId === chatId} data-video-visible={appliedMode === "all" || channel.channelId === appliedLead}>
          {runtime === "tauri" ? <SoopOriginalPlayer onClose={()=>setWatching(current=>current.filter(item=>item.channelId!==channel.channelId))} resumeRequest={resumeRequests[channel.channelId]??0} channelId={channel.channelId} title={channel.title || channel.name} privacyMode={privacyMode} active={active && view === "live"} videoVisible={appliedMode === "all" || channel.channelId === appliedLead} flattened={appliedMode === "all" && watching.length > 1 && (appliedArrangement === "grouped" || compactPanes)} sideBySide={appliedMode === "all" && appliedArrangement === "channels" && !compactPanes && watching.length > 1} /> : <PlatformLivePlayer reference={{ provider: "soop", kind: "live", externalId: channel.channelId }} title={channel.title || channel.name} privacyMode={privacyMode} active={active && view === "live"} videoVisible={appliedMode === "all" || channel.channelId === appliedLead} flattened={appliedMode === "all" && watching.length > 1 && (appliedArrangement === "grouped" || compactPanes)} sideBySide={appliedMode === "all" && appliedArrangement === "channels" && !compactPanes && watching.length > 1} />}
        </div>)}
      </SoopLivePaneGrid>
      {!watching.length && !drawerOpen ? <div className="soop-player-empty"><div className="soop-empty-copy"><svg className="soop-empty-icon" viewBox="0 0 32 32" aria-hidden="true"><rect x="3" y="5" width="26" height="19" rx="3" /><path d="m13 10 8 4.5-8 4.5zM11 28h10" /></svg><h2>방송을 시청해 보세요</h2><p>{privacyMode ? <>프라이버시 모드를 끈 뒤<br />아래 손잡이를 눌러주세요.</> : "아래 손잡이를 누르거나 끌어올려 방송을 선택하세요."}</p></div><svg className="soop-empty-direction" viewBox="0 0 24 64" aria-hidden="true"><path d="M12 2v56m-7-7 7 7 7-7" /></svg></div> : null}
    </div>
    <div className="soop-connection-dock" hidden={view !== "live"}>
      <button ref={handle} className="soop-bottom-handle" type="button" data-tour="soop-input" data-platform-native-occlusion="true" aria-label={`${watching.length ? "방송 조작" : "방송 보기"} ${drawerOpen ? "닫기" : "열기"}`} aria-expanded={drawerOpen} aria-controls="soop-connection-panel" title={drawerOpen ? "누르거나 아래로 끌어 닫기" : "누르거나 위로 끌어 열기"}
        {...handleGesture} onKeyDown={event => { if (event.key === "Escape" && drawerOpen) { event.preventDefault(); closeDrawer(); } }}>
        <svg viewBox="0 0 24 16" aria-hidden="true"><path d={drawerOpen ? "m6 5 6 5 6-5" : "m6 11 6-5 6 5"} /></svg>
      </button>
      {drawerOpen ? <div ref={drawer} id="soop-connection-panel" className="soop-connection-panel" role="dialog" aria-modal="false" aria-label="방송 보기" data-platform-native-occlusion="true" data-gallery-shortcuts-suspended onKeyDown={event => { if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); closeDrawer(); } }}>
        <div className="soop-connection-surface" data-native-dialog-surface="true"><SoopConnectionSetup active={active && view === "live"} favorites={favoritePanel} watching={isWatching} disabled={privacyMode}
          ready={ready && !!liveRows.length} pending={entered.some(row => rowStates[row.key]?.loading)} rowCount={rows.length}
          mode={mode} onMode={setMode} channels={[...new Map(liveRows.map(channel => [channel.channelId, channel])).values()]}
          lead={selectedLead} onLead={setLead} arrangement={arrangement} onArrangement={setArrangement} error={error}
          onAdd={() => setRows(current => current.length < 4 ? [...current, { key: nextRow.current++, input: "" }] : current)}
          onStart={start} onStop={() => { setWatching([]); setAppliedLead(""); }} onClose={closeDrawer}
          onLogin={runtime === "tauri" ? login : undefined} onLogout={runtime === "tauri" ? logout : undefined} accountBusy={accountBusy} authenticated={account?.authenticated === true} accountLabel={account?.authenticated === true ? "로그인됨" : account?.authenticated === false ? "로그아웃됨" : account?.open ? "SOOP 창에서 로그인" : "계정 확인 전"}>
          {rows.map((row, index) => <div className="soop-connection-channel-edit-row" key={row.key}><ChannelInput row={row} index={index} api={api} enabled={active && view === "live"} privacy={privacyMode} favorite={favorites.some(item => item.channelId === normalizeSoopChannel(row.input))} onInput={input => setRows(current => current.map(item => item.key === row.key ? { ...item, input } : item))} onRemove={rows.length > 1 ? () => setRows(current => current.filter(item => item.key !== row.key)) : undefined} onMeta={rowMeta} onFavorite={toggleFavorite} /></div>)}
        </SoopConnectionSetup></div>
      </div> : null}
    </div>
  </section>;
}
