import { useCallback, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { createReplayApi, type ReplayApi, type ReplaySession, type ReplayTimeline } from "../../api/replay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import { OriginalChzzkPlayer, type OriginalPlayerHandle, type OriginalPlayerState } from "./OriginalChzzkPlayer";
import { replayFocusableControls, replayShortcutAllowed, visibleReplayWarnings } from "./RecordingReplayModel";
import { useReplayLayout } from "./RecordingReplayLayout";
import "./RecordingReplay.css";

export type RecordingReplayProps = { recordingId: string; runtime: "tauri" | "browser-mock"; privacyMode: boolean; liveRecording: boolean; onClose: () => void; api?: ReplayApi };
export function RecordingReplay({ recordingId, runtime, privacyMode, liveRecording, onClose, api: suppliedApi }: RecordingReplayProps) {
  const api = useMemo(() => suppliedApi ?? createReplayApi(runtime), [suppliedApi, runtime]);
  const [session, setSession] = useState<ReplaySession | null>(null), [error, setError] = useState<string | null>(null);
  const [time, setTime] = useState(0), [mediaAspect, setMediaAspect] = useState(16 / 9);
  const [mediaDuration, setMediaDuration] = useState(0), [fullscreen, setFullscreen] = useState(false);
  const [seekVersion, setSeekVersion] = useState(0), [timeline, setTimeline] = useState<ReplayTimeline | null>(null);
  const [wide, setWide] = useState(false);
  const layout = useReplayLayout(mediaAspect, wide || fullscreen);
  const player = useRef<OriginalPlayerHandle>(null), dialog = useRef<HTMLDivElement>(null), closeButton = useRef<HTMLButtonElement>(null);
  const previousState = useRef<OriginalPlayerState | null>(null), titleId = useId();
  const timelineRequested = useRef(false), timelineGeneration = useRef(0), sessionGeneration = useRef(0), offsetSaving = useRef(false);
  const duration = mediaDuration || session?.durationSeconds || 0;
  useEffect(() => {
    let disposed = false, opened: string | undefined;
    ++sessionGeneration.current; ++timelineGeneration.current; timelineRequested.current = false; previousState.current = null;
    setSession(null); setTimeline(null); setError(null); setTime(0); setMediaAspect(16 / 9); setMediaDuration(0); offsetSaving.current = false;
    void api.open(recordingId).then(result => {
      if (result.ok) { opened = result.data.token; if (disposed) { void api.close(opened); return; } setSession(result.data); }
      else if (!disposed) setError(result.error.message);
    }).catch(() => { if (!disposed) setError("저장 영상을 열지 못했습니다."); });
    return () => { disposed = true; ++sessionGeneration.current; ++timelineGeneration.current; if (opened) void api.close(opened); };
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
            <OriginalChzzkPlayer key={session.token} ref={player} runtime={runtime} mediaUrl={new URL(api.mediaUrl(session.token), location.href).href} duration={duration} privacy={privacyMode} timeline={timeline} wide={wide} fullscreen={fullscreen}
              recording={{ title: session.title, channelName: session.channelName, recordedAt: session.recordedAt, profileImage: session.channelProfileImage }}
              onState={stateChanged} onError={setError} onFullscreen={() => void toggleFullscreen()} onWide={() => setWide(value => !value)} onClose={close} />
            {!privacyMode ? dismiss : null}
          </div></div></section>
          <RecordingReplayChat api={api} session={session} time={Math.floor(time * 5) / 5} seekVersion={seekVersion} onSeek={seek} onIndexReady={indexReady} onSetOffset={saveOffset} />
        </div>
      </>}
    </div>
  </div>, document.body);
}
