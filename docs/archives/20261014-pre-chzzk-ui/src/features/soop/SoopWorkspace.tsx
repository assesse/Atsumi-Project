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

export type SoopWorkspaceProps = {
  active: boolean; runtime: "tauri" | "browser-mock"; privacyMode: boolean;
  view: "explore" | "live" | "favorites" | "screenshots" | "downloads";
  onNotice?: (message: string) => void; api?: SoopApi;
  onViewChange?: (view: "explore" | "live" | "favorites" | "screenshots" | "downloads") => void;
  onWatchingChange?: (watching: boolean) => void;
  autoRecordApi?: SoopAutoRecordApi;
};
type Row = { key: number; input: string };
type RowState = { id: string | null; profile: SoopChannel | null; loading: boolean; error: string };
const statusLabel = (status: SoopChannel["status"] | undefined) => status === "live" ? "방송 중" : status === "offline" ? "방송 종료" : "상태 미확인";
const safeError = (cause: unknown) => typeof cause === "string" ? cause.slice(0, 220) : cause instanceof Error ? cause.message.slice(0, 220) : "채널을 불러오지 못했습니다. 다시 시도해 주세요.";
const publicImage = (value: string | null | undefined) => value && /^data:image\/(?:png|jpeg|webp|gif);base64,[a-zA-Z0-9+/=]+$/.test(value) && value.length < 180_000 ? value : undefined;
function Star({ filled }: { filled: boolean }) {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-2.9-5.6 2.9 1.1-6.2L3 9.6l6.2-.9Z" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" /></svg>;
}
function Avatar({ channel, privacy }: { channel?: Pick<SoopChannel, "image" | "name">; privacy: boolean }) {
  const image = privacy ? undefined : publicImage(channel?.image);
  return <span className={`soop-avatar${!channel ? " is-skeleton" : ""}`} aria-hidden="true">{image ? <img src={image} alt="" onError={event => { event.currentTarget.hidden = true; }} /> : null}{privacy ? "•" : channel?.name.slice(0, 1) || ""}</span>;
}

function ChannelInput({ row, index, api, enabled, privacy, favorite, onInput, onMeta, onFavorite, onFocus }: {
  row: Row; index: number; api: SoopApi; enabled: boolean; privacy: boolean; favorite: boolean;
  onInput: (value: string) => void; onMeta: (key: number, value: RowState) => void;
  onFavorite: (channel: SoopChannel) => void; onFocus: () => void;
}) {
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
    <div className="soop-input-identity"><Avatar channel={profile ?? undefined} privacy={privacy} /><span><strong>{privacy ? "방송" : profile?.name || "채널"}</strong><small className={!privacy && profile?.status === "live" ? "is-live" : ""}>{privacy ? "" : state.loading ? <span className="soop-spinner" aria-label="채널 확인 중" /> : profile ? statusLabel(profile.status) : ""}</small></span></div>
    <div className="soop-input-wrap"><input aria-label={`${index + 1}번 SOOP 채널 링크 또는 ID`} type={privacy ? "password" : "text"} value={row.input} disabled={privacy} spellCheck={false} autoComplete="off" placeholder="채널 링크 또는 ID" onFocus={onFocus} onChange={event => onInput(event.target.value)} onPaste={event => { const parsed = normalizeSoopChannel(event.clipboardData.getData("text")); if (parsed) { event.preventDefault(); onInput(parsed); } }} /><button type="button" className="soop-clear" aria-label={`${index + 1}번 채널 입력 지우기`} disabled={!row.input || privacy} onClick={() => onInput("")}>×</button></div>
    <button type="button" className="soop-star" title={favorite ? "즐겨찾기 해제" : "즐겨찾기 추가"} aria-label={`${index + 1}번 채널 즐겨찾기 ${favorite ? "해제" : "추가"}`} aria-pressed={favorite} disabled={!profile || state.loading || privacy} onClick={() => profile && onFavorite(profile)}><Star filled={favorite} /></button>
    {!privacy && (row.input.trim() && !id || state.error) ? <p className="soop-inline-error" role="alert">{state.error || "채널 링크 또는 ID를 확인해 주세요."}{state.error ? <button type="button" onClick={() => setRetry(value => value + 1)}>다시 시도</button> : null}</p> : null}
  </div>;
}

export function SoopWorkspace({ active, runtime, privacyMode, view: requestedView, onNotice, api: suppliedApi, onViewChange, onWatchingChange, autoRecordApi }: SoopWorkspaceProps) {
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
  const isWatching = watching.length > 0;
  useEffect(() => { onWatchingChange?.(isWatching); }, [isWatching, onWatchingChange]);
  useEffect(() => () => { onWatchingChange?.(false); }, [onWatchingChange]);
  const [mode, setMode] = useState<"all" | "selected">("all"), [appliedMode, setAppliedMode] = useState<"all" | "selected">("all");
  const [lead, setLead] = useState(""), [appliedLead, setAppliedLead] = useState("");
  const [drawerOpen, setDrawerOpen] = useState(false), [error, setError] = useState("");
  const nextRow = useRef(1), focusedRow = useRef<number | null>(null);
  const handle = useRef<HTMLButtonElement>(null), drawer = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; y: number } | null>(null), suppressClick = useRef(false);
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
    if (rows.some(row => normalizeSoopChannel(row.input) === id)) { setLead(id); setDrawerOpen(true); onViewChange?.("live"); return; }
    setRows(current => {
      const empty = current.find(row => !row.input.trim());
      if (empty) return current.map(row => row.key === empty.key ? { ...row, input: id } : row);
      if (current.length < 4) return [...current, { key: nextRow.current++, input: id }];
      const replace = current.some(row => row.key === focusedRow.current) ? focusedRow.current : current[0]!.key;
      return current.map(row => row.key === replace ? { ...row, input: id } : row);
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
    setAppliedMode(mode); setAppliedLead(selectedLead); setError(""); closeDrawer();
    if (liveRows.length < entered.length) onNotice?.("종료된 방송을 제외하고 시청합니다.");
  }
  const favoritePanel = <><div className="soop-favorites" aria-label="즐겨찾기 채널">
    {visibleFavorites.length === 0 ? <p className="soop-favorites-empty">별표를 눌러 채널을 저장하세요.</p> : visibleFavorites.map(favorite => {
      const channel = profiles[favorite.channelId], label = privacyMode ? "즐겨찾기 채널" : channel?.name || favorite.name;
      const selected = rows.some(row => normalizeSoopChannel(row.input) === favorite.channelId);
      const reservation = reservations.snapshot.channels.find(entry => entry.channelId === favorite.channelId);
      const starred = favorites.some(entry => entry.channelId === favorite.channelId);
      const liveStatus = reservation?.checkedAt && reservation.liveStatus !== "unknown" ? reservation.liveStatus : channel?.status;
      const saving = reservations.pending === favorite.channelId;
      const reservationText = reservations.snapshot.ready ? autoRecordLabel(reservation) : reservations.error ? "예약 확인 불가" : "예약 확인 중";
      return <div className={`soop-favorite${selected ? " is-selected" : ""}`} key={favorite.channelId}>
        <button type="button" className="soop-favorite-select" disabled={privacyMode} aria-label={`${label} 선택`} aria-pressed={selected} onClick={() => chooseFavorite(favorite.channelId)}><Avatar channel={channel ?? { name: favorite.name, image: null }} privacy={privacyMode} /><span className="soop-favorite-copy"><strong>{label}</strong><small className={reservation?.status === "failed" ? "soop-reservation-failed" : !privacyMode && liveStatus === "live" ? "is-live" : ""} title={privacyMode ? undefined : reservation?.error ?? `${statusLabel(liveStatus)} · ${reservationText}`}><span>{privacyMode ? "" : `${statusLabel(liveStatus)} · ${reservationText}`}</span>{pendingFavorites.includes(favorite.channelId) && !privacyMode ? <span className="soop-spinner" aria-label="채널 확인 중" /> : null}</small></span></button>
        <button type="button" className="soop-record-toggle" disabled={privacyMode || !reservations.canToggle(favorite.channelId)} aria-label={`${label} 녹화 예약 ${reservation?.enabled ? "끄기" : "켜기"}`} aria-pressed={!!reservation?.enabled} aria-busy={saving} title={reservation?.enabled ? "녹화 예약 끄기" : "녹화 예약 켜기"} onClick={() => void reservations.toggle(favorite.channelId, channel?.name || favorite.name)}>{saving ? <span className="soop-spinner" aria-label="예약 저장 중" /> : <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8.5" /><circle cx="12" cy="12" r="4" /></svg>}</button>
        <button type="button" className="soop-star" disabled={privacyMode} aria-label={`${label} 즐겨찾기 ${starred ? "해제" : "추가"}`} aria-pressed={starred} title={starred ? "즐겨찾기 해제" : "즐겨찾기 추가"} onClick={() => toggleFavorite(favorite)}><Star filled={starred} /></button>
      </div>;
    })}
  </div>{reservations.error && !privacyMode ? <p className="soop-inline-error" role="alert">{reservations.error}</p> : null}</>;
  return <section className={`soop-workspace${view === "live" ? " is-live-view" : ""}`} data-tour="soop-content" aria-label="SOOP" hidden={!active} inert={!active}>
    <div hidden={view !== "explore"}>
      <SoopDiscovery api={api} active={active && view === "explore"} legacyFavoritesView={requestedView === "favorites"} privacyMode={privacyMode} onNotice={onNotice}
        isChannelFavorite={id => favorites.some(item => item.channelId === id)} onFavoriteChannel={item => toggleFavorite({ channelId: item.channelId, name: item.channelName })}
        onLive={item => { const channel: SoopChannel = { channelId: item.channelId, name: item.channelName, title: item.title, image: null, status: "live", viewers: item.views }; setRows([{ key: nextRow.current++, input: item.channelId }]); setWatching(current => [current.find(old => old.channelId === item.channelId) ?? channel]); setAppliedMode("all"); closeDrawer(); onViewChange?.("live"); }} />
    </div>
    <div hidden={view !== "downloads"}><PlatformMediaLibrary provider="soop" active={active && view === "downloads"} privacyMode={privacyMode} onNotice={onNotice} /></div>
    <PlatformScreenshotLibrary provider="soop" active={active && view === "screenshots"} privacyMode={privacyMode} onNotice={onNotice} />
    <div className="soop-player-area" hidden={view !== "live"} data-count={watching.length} data-mode={appliedMode}>
      {watching.map(channel => <div className="soop-player-cell" key={channel.channelId} data-video-visible={appliedMode === "all" || channel.channelId === appliedLead}>
        <PlatformLivePlayer reference={{ provider: "soop", kind: "live", externalId: channel.channelId }} title={channel.title || channel.name} privacyMode={privacyMode} active={active && view === "live"} videoVisible={appliedMode === "all" || channel.channelId === appliedLead} />
      </div>)}
      {!watching.length ? <div className="soop-player-empty"><span aria-hidden="true">▷</span><h3>{privacyMode ? "프라이버시 모드" : "방송을 시청해 보세요"}</h3><p>{privacyMode ? "프라이버시 모드를 끈 뒤 아래 손잡이를 눌러주세요." : "아래 손잡이를 누르거나 끌어올려 방송을 선택하세요."}</p><svg className="soop-empty-direction" viewBox="0 0 24 64" aria-hidden="true"><path d="M12 2v56m-7-7 7 7 7-7" /></svg></div> : null}
    </div>
    <div className="soop-connection-dock" hidden={view !== "live"}>
      <button ref={handle} className="soop-bottom-handle" type="button" data-tour="soop-input" aria-label={`${watching.length ? "방송 조작" : "방송 보기"} ${drawerOpen ? "닫기" : "열기"}`} aria-expanded={drawerOpen} aria-controls="soop-connection-panel" title={drawerOpen ? "누르거나 아래로 끌어 닫기" : "누르거나 위로 끌어 열기"}
        onPointerDown={event => { if (event.button !== 0) return; drag.current = { pointer: event.pointerId, y: event.clientY }; suppressClick.current = false; event.currentTarget.setPointerCapture?.(event.pointerId); }}
        onPointerMove={event => { const gesture = drag.current; if (!gesture || gesture.pointer !== event.pointerId) return; if (Math.abs(event.clientY - gesture.y) >= 14) { suppressClick.current = true; if (event.clientY < gesture.y) setDrawerOpen(true); else closeDrawer(); } }}
        onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; suppressClick.current = true; }} onLostPointerCapture={() => { drag.current = null; }}
        onClick={() => { if (suppressClick.current) { suppressClick.current = false; return; } if (drawerOpen) closeDrawer(); else setDrawerOpen(true); }} onKeyDown={event => { if (event.key === "Escape" && drawerOpen) { event.preventDefault(); closeDrawer(); } }}>
        <svg viewBox="0 0 24 16" aria-hidden="true"><path d={drawerOpen ? "m6 5 6 5 6-5" : "m6 11 6-5 6 5"} /></svg>
      </button>
      {drawerOpen ? <div ref={drawer} id="soop-connection-panel" className="soop-connection-panel" role="dialog" aria-modal="false" aria-label="방송 보기" data-gallery-shortcuts-suspended onKeyDown={event => { if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); closeDrawer(); } }}>
        <header><h2>{watching.length ? "방송 조작" : "방송 보기"}</h2><span className="soop-help"><button type="button" aria-label="SOOP 이용 안내">?</button><span role="tooltip">채널을 선택하거나 링크를 입력하세요.<br />+로 최대 4개 방송을 추가합니다.<br />별표는 즐겨찾기, 모두·선택은 영상 배치입니다.<br />공개 방송만 지원합니다. 로그인이 필요한 방송은 지원하지 않습니다.</span></span><button type="button" className="soop-drawer-close" aria-label="방송 보기 닫기" onClick={closeDrawer}>×</button></header>
        <div className="soop-connection-scroll">{favoritePanel}<form className="soop-input-area" onSubmit={event => { event.preventDefault(); start(); }}>
          {rows.map((row, index) => <ChannelInput key={row.key} row={row} index={index} api={api} enabled={active && view === "live"} privacy={privacyMode} favorite={favorites.some(item => item.channelId === normalizeSoopChannel(row.input))} onInput={input => setRows(current => current.map(item => item.key === row.key ? { ...item, input } : item))} onFocus={() => { focusedRow.current = row.key; }} onMeta={rowMeta} onFavorite={toggleFavorite} />)}
          <div className="soop-row-actions"><button type="button" aria-label="방송 추가" disabled={rows.length >= 4 || privacyMode} onClick={() => setRows(current => current.length < 4 ? [...current, { key: nextRow.current++, input: "" }] : current)}>+</button></div>
        </form>{error && !privacyMode ? <p className="soop-inline-error" role="alert">{error}</p> : null}</div>
        <footer className="soop-watch-actions"><div className="soop-view-choice"><div className="soop-mode" role="group" aria-label="영상 표시" data-mode={mode}><button type="button" aria-pressed={mode === "all"} onClick={() => setMode("all")}>모두 보기</button><button type="button" aria-pressed={mode === "selected"} onClick={() => setMode("selected")}>선택 보기</button></div><select aria-label="볼 영상" value={selectedLead} disabled={mode !== "selected" || !liveRows.length || privacyMode} onChange={event => setLead(event.target.value)}><option value="" disabled>채널 선택</option>{[...new Map(liveRows.map(channel => [channel.channelId, channel])).values()].map(channel => <option key={channel.channelId} value={channel.channelId}>{privacyMode ? "채널" : channel.name}</option>)}</select></div><span><button type="button" className={watching.length ? "soop-change" : "soop-start"} disabled={!ready || !liveRows.length || privacyMode} onClick={start}>{watching.length ? "변경" : "시청"}</button><button type="button" className="soop-stop" disabled={!watching.length} onClick={() => { setWatching([]); setAppliedLead(""); }}>종료</button></span></footer>
      </div> : null}
    </div>
  </section>;
}
