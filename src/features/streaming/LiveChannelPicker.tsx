import { useEffect, useMemo, useRef, useState } from "react";
import { createLiveChannelsApi, mergeLiveChannels, type LiveChannelsApi, type LiveFavorite, type LiveChannelProfile } from "../../api/liveChannels";
import { createAutoRecordingApi, type AutoRecordingApi, type AutoRecordingEntry } from "../../api/autoRecording";
import { normalizeMultiviewChannel } from "../../api/multiview";
import "./LiveChannelPicker.css";

export function LiveChannelPicker({ runtime, inputs, multiple, privacy, disabled, onInputs, api: suppliedApi, autoApi: suppliedAutoApi }: {
  runtime: "tauri" | "browser-mock"; inputs: string[]; multiple: boolean; privacy: boolean; disabled: boolean;
  onInputs(inputs: string[]): void; api?: LiveChannelsApi; autoApi?: AutoRecordingApi;
}) {
  const api = useMemo(() => suppliedApi ?? createLiveChannelsApi(runtime), [runtime, suppliedApi]);
  const autoApi = useMemo(() => suppliedAutoApi ?? createAutoRecordingApi(runtime), [runtime, suppliedAutoApi]);
  const [favorites, setFavorites] = useState<LiveFavorite[]>([]), [scheduled, setScheduled] = useState<AutoRecordingEntry[]>([]);
  const [error, setError] = useState<string | null>(null), [pending, setPending] = useState(false);
  const [filter, setFilter] = useState("");
  const [profiles, setProfiles] = useState<Record<string, LiveChannelProfile>>({});
  const version = useRef(0), alive = useRef(false), busy = useRef(false);
  useEffect(() => {
    alive.current = true; let cancelled = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const current = version.current;
      try {
        const [saved, auto] = await Promise.all([api.snapshot(), autoApi.snapshot()]);
        if (cancelled || version.current !== current || busy.current) return;
        if (saved.ok && Array.isArray(saved.data)) setFavorites(saved.data);
        if (auto.ok && Array.isArray(auto.data.channels)) setScheduled(auto.data.channels);
        setError(!saved.ok ? saved.error.message : !auto.ok ? auto.error.message : null);
      } catch { if (!cancelled) setError("내 채널을 불러오지 못했습니다. 주소로 연결할 수 있습니다."); }
      finally { if (!cancelled) timer = setTimeout(poll, 3000); }
    };
    if (runtime === "tauri") void poll();
    return () => { cancelled = true; alive.current = false; version.current++; clearTimeout(timer); };
  }, [api, autoApi, runtime]);
  const selected = inputs.slice(0, multiple ? 4 : 1).map(normalizeMultiviewChannel);
  const choose = (id: string) => {
    setError(null);
    if (!multiple) { onInputs([id, ...inputs.slice(1)]); return; }
    const existing = selected.indexOf(id);
    if (existing >= 0) { onInputs(inputs.map((value, index) => index === existing ? "" : value)); return; }
    const empty = inputs.findIndex(value => !value.trim());
    if (empty < 0) { setError("마도는 최대 네 방송까지 선택할 수 있습니다."); return; }
    onInputs(inputs.map((value, index) => index === empty ? id : value));
  };
  const toggle = async (input: string, favorite: boolean) => {
    if (busy.current) return;
    busy.current = true; setPending(true); setError(null); const request = ++version.current;
    try {
      const result = await api.set(input, favorite);
      if (!alive.current || request !== version.current) return;
      if (result.ok) setFavorites(result.data); else setError(result.error.message);
    } catch { if (alive.current) setError("즐겨찾기를 변경하지 못했습니다."); }
    finally { busy.current = false; if (alive.current) setPending(false); }
  };
  const channels = mergeLiveChannels(favorites, scheduled);
  const profileKey = privacy ? "" : channels.map(row => row.channelId).sort().join(",");
  useEffect(() => {
    let cancelled = false;
    setProfiles({});
    const queue = profileKey ? profileKey.split(",") : [];
    // Portraits must never delay selecting a channel or refreshing its status.
    const load = async () => {
      while (!cancelled && queue.length) {
        const id = queue.shift()!;
        try {
          const result = await api.profile(id);
          if (!cancelled && result.ok) setProfiles(previous => ({ ...previous, [id]: result.data }));
        } catch { /* Keep the channel initial when offline or an image is missing. */ }
      }
    };
    if (runtime === "tauri") { void load(); void load(); }
    return () => { cancelled = true; };
  }, [api, profileKey, runtime]);
  const needle = filter.trim().toLocaleLowerCase();
  const draft = [...new Set(selected.filter((id): id is string => !!id))].filter(id => !favorites.some(f => f.channelId === id));
  return <section className="live-channel-picker" aria-label="내 채널">
    <div className="live-channel-picker-title"><strong>내 채널</strong><input aria-label="채널 검색" placeholder="검색" value={filter} onChange={e => setFilter(e.target.value)} /></div>
    <div className="live-channel-list">
      {channels.filter(row => !needle || (!privacy && (profiles[row.channelId]?.channelName || row.channelName).toLocaleLowerCase().includes(needle))).map((row, index) => {
        const profile = privacy ? undefined : profiles[row.channelId];
        const name = privacy ? `채널 ${index + 1}` : profile?.channelName || row.channelName || row.channelId;
        const status = row.scheduled?.status;
        const badge = status === "recording" ? "녹화 중" : status === "starting" ? "녹화 준비" : status === "stopping" ? "마무리 중" : row.scheduled ? row.scheduled.enabled ? "녹화 예약" : "예약 꺼짐" : "즐겨찾기";
        return <div className={`live-channel-row${row.favorite ? " is-favorite" : ""}`} key={row.channelId}>
          <button type="button" className="live-channel-choice" disabled={disabled} aria-pressed={selected.includes(row.channelId)} aria-label={`${name} 선택`} onClick={() => choose(row.channelId)}>
            <span className="live-channel-avatar" aria-hidden="true">{profile?.image && /^data:image\/(?:png|jpeg|gif|webp);base64,/.test(profile.image) ? <img src={profile.image} alt="" decoding="async" onError={event => { event.currentTarget.hidden = true; }} /> : null}<span>{privacy ? "·" : Array.from(name)[0]}</span></span>
            <span className="live-channel-identity"><span className="live-channel-name" title={name}>{name}</span><small className={status === "recording" ? "is-recording" : ""}>{badge}</small></span>
          </button>
          <button type="button" className="live-channel-star" disabled={pending || runtime !== "tauri"} aria-pressed={row.favorite} aria-label={`${name} 즐겨찾기 ${row.favorite ? "해제" : "추가"}`} title="즐겨찾기만 변경합니다. 녹화 예약은 유지됩니다." onClick={() => void toggle(row.channelId, !row.favorite)}>{row.favorite ? "★" : "☆"}</button>
        </div>;
      })}
      {!channels.length ? <p>즐겨찾기와 녹화 예약 채널이 여기에 표시됩니다.</p> : null}
    </div>
    {draft.length ? <div className="live-channel-save">{draft.map((id, index) => <button type="button" key={id} disabled={pending || disabled} onClick={() => void toggle(id, true)}>{pending ? "저장 중…" : draft.length === 1 ? "☆ 입력한 채널 즐겨찾기" : `☆ ${index + 1}번 채널 즐겨찾기`}</button>)}</div> : null}
    {error ? <small role="alert">{error}</small> : null}
  </section>;
}
