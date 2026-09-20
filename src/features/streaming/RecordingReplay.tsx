import { useCallback, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { createReplayApi, type ReplayApi, type ReplaySession, type ReplayTimeline } from "../../api/replay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import { OriginalChzzkPlayer, type OriginalPlayerHandle, type OriginalPlayerState } from "./OriginalChzzkPlayer";
import { replayFocusableControls, replayShortcutAllowed, visibleReplayWarnings } from "./RecordingReplayModel";
import { useReplayLayout } from "./RecordingReplayLayout";
import "./RecordingReplay.css";
import { openRecordingChannel } from "../../api/recordingProfile";

export type RecordingReplayProps = { recordingId: string; runtime: "tauri" | "browser-mock"; privacyMode: boolean; liveRecording: boolean; onClose: () => void; api?: ReplayApi };
export function RecordingReplay({ recordingId, runtime, privacyMode, liveRecording, onClose, api: suppliedApi }: RecordingReplayProps) {
  const api = useMemo(() => suppliedApi ?? createReplayApi(runtime), [suppliedApi, runtime]);
  const [session, setSession] = useState<ReplaySession | null>(null), [error, setError] = useState<string | null>(null);
  const [time, setTime] = useState(0), [mediaAspect, setMediaAspect] = useState(16 / 9);
  const [mediaDuration, setMediaDuration] = useState(0), [fullscreen, setFullscreen] = useState(false);
  const [seekVersion, setSeekVersion] = useState(0), [timeline, setTimeline] = useState<ReplayTimeline | null>(null);
  const [wide, setWide] = useState(false);
  const [refreshing, setRefreshing] = useState(false), [refreshNotice, setRefreshNotice] = useState("");
  const lease = useRef<{ release(token: string): void; add(token: string): void } | null>(null);
  const refreshInFlight = useRef<number | null>(null);
  const retiredTokens = useRef(new Set<string>());
  const currentTime = useRef(0); currentTime.current = time;
  const layout = useReplayLayout(mediaAspect, wide || fullscreen);
  const player = useRef<OriginalPlayerHandle>(null), dialog = useRef<HTMLDivElement>(null), closeButton = useRef<HTMLButtonElement>(null);
  const previousState = useRef<OriginalPlayerState | null>(null), titleId = useId();
  const timelineRequested = useRef(false), timelineGeneration = useRef(0), sessionGeneration = useRef(0), offsetSaving = useRef(false);
  const duration = mediaDuration || session?.durationSeconds || 0;
  useEffect(() => {
    let disposed = false;
    const owned = new Set<string>();
    const owner = {
      release(token: string) { if (owned.delete(token)) void api.close(token).catch(() => {}); },
      add(token: string) { if (disposed) void api.close(token).catch(() => {}); else owned.add(token); },
    };
    lease.current = owner;
    ++sessionGeneration.current; ++timelineGeneration.current; timelineRequested.current = false; previousState.current = null;
    setSession(null); setTimeline(null); setError(null); setTime(0); setMediaAspect(16 / 9); setMediaDuration(0); offsetSaving.current = false;
    refreshInFlight.current = null; retiredTokens.current.clear(); setRefreshing(false); setRefreshNotice("");
    void api.open(recordingId).then(result => {
      if (result.ok) { owner.add(result.data.token); if (!disposed) setSession(result.data); }
      else if (!disposed) setError(result.error.message);
    }).catch(() => { if (!disposed) setError("저장 영상을 열지 못했습니다."); });
    return () => { disposed = true; ++sessionGeneration.current; ++timelineGeneration.current; for (const token of owned) owner.release(token); if (lease.current === owner) lease.current = null; };
  }, [api, recordingId]);
  useEffect(() => { const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null; closeButton.current?.focus(); return () => { if (previous?.isConnected) previous.focus(); }; }, []);
  useEffect(() => { closeButton.current?.focus(); }, [Boolean(session), privacyMode]);
  useEffect(() => { const change = () => setFullscreen(document.fullscreenElement === dialog.current); document.addEventListener("fullscreenchange", change); return () => document.removeEventListener("fullscreenchange", change); }, []);
  const stateChanged = useCallback((state: OriginalPlayerState) => {
    const previous = previousState.current;
    if (state.seeking && !previous?.seeking || previous && state.time < previous.time) setSeekVersion(value => value + 1);
    previousState.current = state;
    if (!previous || Math.floor(previous.time * 10) !== Math.floor(state.time * 10)) setTime(Math.floor(state.time * 10) / 10);
    if (!previous || previous.aspect !== state.aspect) setMediaAspect(state.aspect);
    if (state.duration > 0 && previous?.duration !== state.duration) setMediaDuration(state.duration);
  }, []);
  const seek = useCallback((target: number) => {
    if (!Number.isFinite(target)) return;
    const next = Math.max(0, Math.min(duration, target));
    player.current?.seek(next); setTime(next); setSeekVersion(value => value + 1);
  }, [duration]);
  const toggleFullscreen = useCallback(async () => {
    try { if (document.fullscreenElement === dialog.current) await document.exitFullscreen(); else if (dialog.current?.requestFullscreen) await dialog.current.requestFullscreen(); }
    catch { setError("전체화면을 전환하지 못했습니다."); }
  }, []);
  const close = () => { player.current?.pause(); if (document.fullscreenElement === dialog.current) void document.exitFullscreen().catch(() => {}); onClose(); };
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.nativeEvent.composedPath()[0] ?? event.target;
    if (event.key === "Tab") {
      const controls = replayFocusableControls(event.currentTarget);
      const first = controls[0], last = controls[controls.length - 1];
      if (event.shiftKey && target === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && target === last) { event.preventDefault(); first?.focus(); }
      return;
    }
    if (event.key === "Escape" && !document.fullscreenElement) { event.preventDefault(); close(); return; }
    if (event.altKey || event.ctrlKey || event.metaKey || !replayShortcutAllowed(target)) return;
    switch (event.key.toLowerCase()) {
      case " ": case "k": event.preventDefault(); if (!privacyMode) player.current?.toggle(); break;
      case "arrowleft": event.preventDefault(); seek(time - 5); break;
      case "arrowright": event.preventDefault(); seek(time + 5); break;
      case "m": event.preventDefault(); player.current?.mute(); break;
      case "f": event.preventDefault(); void toggleFullscreen(); break;
      case "t": event.preventDefault(); setWide(value => !value); break;
    }
  };
  const loadTimeline = useCallback(() => {
    if (!session || timelineRequested.current) return;
    timelineRequested.current = true; const generation = ++timelineGeneration.current;
    void api.timeline(session.token, Math.max(2, Math.ceil(session.durationSeconds / 600))).then(result => {
      if (generation !== timelineGeneration.current) return;
      if (result.ok) { setTimeline(result.data); if (result.data.indexState !== "ready") timelineRequested.current = false; } else timelineRequested.current = false;
    }).catch(() => { if (generation === timelineGeneration.current) timelineRequested.current = false; });
  }, [api, session]);
  useEffect(() => { if (session?.indexState === "ready") loadTimeline(); }, [session, loadTimeline]);
  const indexReady = useCallback((quality: ReplaySession["syncQuality"]) => {
    setSession(previous => previous && (previous.syncQuality !== quality || previous.indexState !== "ready") ? { ...previous, syncQuality: quality, indexState: "ready" } : previous); loadTimeline();
  }, [loadTimeline]);
  const saveOffset = useCallback(async (value: number): Promise<string | null> => {
    if (!session || offsetSaving.current) return "채팅 보정을 저장 중입니다.";
    if (!Number.isFinite(value) || Math.abs(value) > 3600) return "채팅 보정은 -3600~3600초로 입력해 주세요.";
    offsetSaving.current = true; const generation = sessionGeneration.current;
    try {
      const result = await api.setOffset(session.token, value); if (generation !== sessionGeneration.current) return "다시보기가 종료됐습니다.";
      if (!result.ok) return result.error.message;
      ++timelineGeneration.current; setSession(previous => previous && { ...previous, manualOffsetSeconds: result.data }); setSeekVersion(previous => previous + 1); timelineRequested.current = false; setTimeline(null);
      return null;
    } catch { return "채팅 보정을 저장하지 못했습니다."; }
    finally { if (generation === sessionGeneration.current) offsetSaving.current = false; }
  }, [api, session]);
  const refresh = async () => {
    if (!session || refreshInFlight.current !== null || !lease.current || retiredTokens.current.size) return;
    const generation = sessionGeneration.current, old = session, owner = lease.current;
    refreshInFlight.current = generation; setRefreshing(true); setRefreshNotice("");
    try {
      const result = await api.open(recordingId);
      if (result.ok) owner.add(result.data.token);
      if (generation !== sessionGeneration.current) { if (result.ok) owner.release(result.data.token); return; }
      if (!result.ok) { setRefreshNotice(result.error.message); return; }
      if (result.data.durationSeconds <= old.durationSeconds + .01 && (result.data.parts?.length ?? 0) > 0) {
        owner.release(result.data.token); setSession(previous => previous && { ...previous, recordingActive: result.data.recordingActive });
        setRefreshNotice(result.data.recordingActive ? "아직 새로 확정된 구간이 없습니다. 녹화는 계속됩니다." : "현재 저장된 마지막 구간입니다."); return;
      }
      ++timelineGeneration.current; timelineRequested.current = false; setTimeline(null);
      retiredTokens.current.add(old.token);
      setMediaDuration(0); setSession(result.data); setSeekVersion(value => value + 1);
    } catch { if (generation === sessionGeneration.current) setRefreshNotice("새 저장 구간을 확인하지 못했습니다."); }
    finally { if (refreshInFlight.current === generation) refreshInFlight.current = null; if (generation === sessionGeneration.current) setRefreshing(false); }
  };
  const latestRefresh = useRef(refresh); latestRefresh.current = refresh;
  useEffect(() => {
    if (!session?.parts?.length || !session.recordingActive || privacyMode) return;
    // Poll only at the saved tail, not throughout playback. The capture worker is untouched.
    const timer = setInterval(() => { if (currentTime.current >= session.durationSeconds - .5) void latestRefresh.current(); }, 15000);
    return () => clearInterval(timer);
  }, [session?.token, session?.durationSeconds, session?.recordingActive, privacyMode]);
  const sourceReady = () => { for (const token of retiredTokens.current) lease.current?.release(token); retiredTokens.current.clear(); };
  const savedDuration = (seconds: number) => [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, Math.floor(seconds) % 60].map(value => String(value).padStart(2, "0")).join(":");
  const dismiss = <button ref={closeButton} type="button" className="recording-replay-dismiss" aria-label="다시보기 닫기" title="닫기 (Esc)" onClick={close}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" /></svg></button>;
  const geometry = { "--replay-width": `${layout.width}px`, "--replay-height": `${layout.height}px`, "--replay-chat-size": `${layout.chatSize}px`,
    width: layout.width + layout.border * 2, height: layout.height + layout.border * 2 } as CSSProperties;
  return createPortal(<div className="recording-replay-backdrop" data-native-overlay="true" style={{ padding: layout.padding }}>
    <div ref={dialog} className={"recording-replay" + (wide ? " is-wide" : "")} style={geometry} role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={keys} tabIndex={-1}>
      <h2 id={titleId} className="recording-replay-sr-only">{privacyMode ? "프라이버시 모드" : session?.title || "저장 영상 다시보기"}</h2>
      {!session || privacyMode ? dismiss : null}
      <div className="recording-replay-notices">
      {liveRecording ? <p className="recording-replay-warning" role="status">다른 녹화가 진행 중입니다. 저장본 재생은 추가 CPU·GPU·디스크 자원을 사용합니다.</p> : null}
      {error ? <p className="recording-replay-error" role="alert">{error}</p> : null}
      {visibleReplayWarnings(session?.warnings).length ? <p className="recording-replay-warning" role="status">{visibleReplayWarnings(session?.warnings).join(" · ")}</p> : null}
      </div>
      {!session ? <div className="recording-replay-loading">{error ? "녹화 목록에서 외부 플레이어로 열기를 사용할 수 있습니다." : "저장 영상을 준비하고 있습니다…"}</div> : <>
        {privacyMode ? <div className="recording-replay-loading">프라이버시 모드에서 영상과 채팅을 가렸습니다.</div> : null}
        <div className="recording-replay-body" data-layout={layout.stacked ? "stacked" : "side"} hidden={privacyMode}>
          <section className="recording-replay-player" aria-label="저장 영상 플레이어"><div className="recording-replay-stage"><div className="recording-replay-visual" style={{ "--video-aspect": mediaAspect } as CSSProperties}>
            <OriginalChzzkPlayer key={recordingId} ref={player} runtime={runtime} mediaUrl={new URL(api.mediaUrl(session.token), location.href).href} duration={session.durationSeconds} parts={session.parts} privacy={privacyMode} timeline={timeline} wide={wide} fullscreen={fullscreen}
              recording={{ title: session.title, channelName: session.channelName, recordedAt: session.recordedAt, profileImage: session.channelProfileImage }}
              onState={stateChanged} onError={setError} onFullscreen={() => void toggleFullscreen()} onWide={() => setWide(value => !value)} onClose={close}
              onTail={() => { if (session.parts?.length) void refresh(); }} onSourceReady={sourceReady}
              onChannel={() => { void openRecordingChannel(recordingId).catch(() => setError("채널 페이지를 열지 못했습니다.")); }} />
            {session.parts?.length ? <div className="recording-replay-saved-range" role="status">
              <span>현재 {savedDuration(session.durationSeconds)}까지 재생 가능{session.recordingActive ? " · 녹화 중" : ""}</span>
              <button type="button" disabled={refreshing} onClick={() => void refresh()} aria-label="새 저장 구간 불러오기" title="새 저장 구간 불러오기">{refreshing ? "확인 중…" : "새로고침"}</button>
              {refreshNotice ? <small>{refreshNotice}</small> : null}
            </div> : null}
            {!privacyMode ? dismiss : null}
          </div></div></section>
          <RecordingReplayChat api={api} session={session} time={Math.floor(time * 5) / 5} seekVersion={seekVersion} onSeek={seek} onIndexReady={indexReady} onSetOffset={saveOffset} />
        </div>
      </>}
    </div>
  </div>, document.body);
}
