import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createLiveChannelsApi, mergeLiveChannels, type LiveChannelsApi, type LiveFavorite, type LiveChannelProfile } from "../../api/liveChannels";
import { createAutoRecordingApi, type AutoRecordingApi, type AutoRecordingEntry } from "../../api/autoRecording";
import { normalizeMultiviewChannel } from "../../api/multiview";
import type { ApiError } from "../../api/contracts";

type Directory = {
  channels: ReturnType<typeof mergeLiveChannels>;
  favorites: LiveFavorite[];
  profiles: Record<string, LiveChannelProfile>;
  profileErrors: ReadonlySet<string>;
  pending: boolean; error: string | null; pollError: ApiError | null;
  hasSnapshot: boolean; hasFavorites: boolean;
  toggle(input: string, favorite: boolean): Promise<void>;
};
const Context = createContext<Directory | null>(null);
export const useLiveChannelDirectory = () => useContext(Context);
export const safeChannelPortrait = (image: string | null | undefined) => !!image && /^data:image\/(?:png|jpeg|gif|webp);base64,/.test(image);

/** One directory for the saved picker and input previews, including serialized
 * favorite writes. It never starts/stops playback or changes recording schedules. */
export function LiveChannelDirectory({ runtime, privacy, inputs, api: suppliedApi, autoApi: suppliedAutoApi, children }: {
  runtime: "tauri" | "browser-mock"; privacy: boolean; inputs: string[];
  api?: LiveChannelsApi; autoApi?: AutoRecordingApi; children: ReactNode;
}) {
  const api = useMemo(() => suppliedApi ?? createLiveChannelsApi(runtime), [runtime, suppliedApi]);
  const autoApi = useMemo(() => suppliedAutoApi ?? createAutoRecordingApi(runtime), [runtime, suppliedAutoApi]);
  const [favorites, setFavorites] = useState<LiveFavorite[]>([]), [scheduled, setScheduled] = useState<AutoRecordingEntry[]>([]);
  const [pending, setPending] = useState(false), [error, setError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<ApiError | null>(null);
  const [hasSnapshot, setHasSnapshot] = useState(runtime !== "tauri"), [hasFavorites, setHasFavorites] = useState(false);
  const [profiles, setProfiles] = useState<Record<string, LiveChannelProfile>>({});
  const [profileErrors, setProfileErrors] = useState<ReadonlySet<string>>(new Set());
  const version = useRef(0), alive = useRef(false), busy = useRef(false);
  useEffect(() => {
    alive.current = true; let cancelled = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const current = version.current;
      try {
        const [saved, auto] = await Promise.all([api.snapshot(), autoApi.snapshot()]);
        if (cancelled || version.current !== current || busy.current) return;
        if (saved.ok && Array.isArray(saved.data)) { setFavorites(saved.data); setHasFavorites(true); }
        if (auto.ok && Array.isArray(auto.data.channels)) setScheduled(auto.data.channels);
        setPollError(!saved.ok ? saved.error : !auto.ok ? auto.error : null);
        if (saved.ok && auto.ok) setHasSnapshot(true);
      } catch { if (!cancelled && version.current === current && !busy.current) setPollError({ code: "CHANNELS_TRANSPORT", message: "저장한 채널을 불러오지 못했습니다. 주소로 연결할 수 있습니다.", retryable: true }); }
      finally { if (!cancelled) timer = setTimeout(poll, 3000); }
    };
    if (runtime === "tauri") void poll();
    return () => { cancelled = true; alive.current = false; version.current++; clearTimeout(timer); };
  }, [api, autoApi, runtime]);
  const toggle = async (input: string, favorite: boolean) => {
    if (busy.current || runtime !== "tauri") return;
    busy.current = true; setPending(true); setError(null); const request = ++version.current;
    try {
      const result = await api.set(input, favorite);
      if (!alive.current || request !== version.current) return;
      if (result.ok) { setFavorites(result.data); setHasFavorites(true); }
      else if (result.error.code === "BROWSER_INITIALIZING") setPollError(result.error);
      else setError(result.error.message);
    } catch { if (alive.current && request === version.current) setError("즐겨찾기를 변경하지 못했습니다."); }
    finally { busy.current = false; if (alive.current) setPending(false); }
  };
  const channels = mergeLiveChannels(favorites, scheduled);
  const draftKey = inputs.map(normalizeMultiviewChannel).filter(Boolean).sort().join(",");
  const [settledDraft, setSettledDraft] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSettledDraft(draftKey), 250);
    return () => clearTimeout(timer);
  }, [draftKey]);
  const profileKey = privacy ? "" : [...new Set([...channels.map(row => row.channelId), ...settledDraft.split(",").filter(Boolean)])].sort().join(",");
  const requests = useMemo(() => new Map<string, ReturnType<LiveChannelsApi["profile"]>>(), [api]);
  useEffect(() => {
    let cancelled = false;
    const queue = profileKey ? profileKey.split(",") : [];
    const load = async () => {
      while (!cancelled && queue.length) {
        const id = queue.shift()!;
        try {
          let request = requests.get(id);
          if (!request) {
            request = api.profile(id); requests.set(id, request);
            // Bound the cache to this open form. Never retain an unlimited history.
            if (requests.size > 128) requests.delete(requests.keys().next().value!);
          }
          const result = await request;
          if (cancelled) return;
          if (result.ok) {
            setProfiles(previous => ({ ...Object.fromEntries(Object.entries(previous).slice(-127)), [id]: result.data }));
            setProfileErrors(previous => new Set([...previous].filter(key => key !== id)));
          } else { requests.delete(id); setProfileErrors(previous => new Set([...previous].slice(-127).concat(id))); }
        } catch { requests.delete(id); if (!cancelled) setProfileErrors(previous => new Set([...previous].slice(-127).concat(id))); }
      }
    };
    if (runtime === "tauri") { void load(); void load(); }
    return () => { cancelled = true; };
  }, [api, profileKey, requests, runtime]);
  return <Context.Provider value={{ channels, favorites, profiles, profileErrors, pending, error, pollError, hasSnapshot, hasFavorites, toggle }}>{children}</Context.Provider>;
}
