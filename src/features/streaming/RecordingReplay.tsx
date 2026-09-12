import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { createReplayApi, type ReplayApi, type ReplaySession, type ReplayTimeline } from "../../api/replay";
import { RecordingReplayChat } from "./RecordingReplayChat";
import { formatReplayTime, replayShortcutAllowed } from "./RecordingReplayModel";
import "./RecordingReplay.css";

export type RecordingReplayProps = {
  recordingId: string; runtime: "tauri" | "browser-mock"; privacyMode: boolean;
  liveRecording: boolean; onClose: () => void; api?: ReplayApi;
};

export function RecordingReplay({ recordingId, runtime, privacyMode, liveRecording, onClose, api: suppliedApi }: RecordingReplayProps) {
  const api = useMemo(() => suppliedApi ?? createReplayApi(runtime), [suppliedApi, runtime]);
  const [session, setSession] = useState<ReplaySession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [time, setTime] = useState(0);
  const displayTime = useRef(time); displayTime.current = time;
  const [mediaDuration, setMediaDuration] = useState(0);
  const [paused, setPaused] = useState(true);
  const [buffering, setBuffering] = useState(false);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);
  const [seekVersion, setSeekVersion] = useState(0);
  const [offset, setOffset] = useState("0");
  const [savingOffset, setSavingOffset] = useState(false);
  const [timeline, setTimeline] = useState<ReplayTimeline | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [settings, setSettings] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const timelineRequested = useRef(false);
  const offsetSaving = useRef(false);
  const duration = mediaDuration || session?.durationSeconds || 0;
  useEffect(() => {
    let disposed = false, opened: string | undefined;
    setSession(null); setError(null); timelineRequested.current = false;
    void api.open(recordingId).then((result) => {
      if (result.ok) {
        opened = result.data.token;
        if (disposed) { void api.close(opened); return; }
        setSession(result.data); setOffset(String(result.data.manualOffsetSeconds));
      } else if (!disposed) setError(result.error.message);
    }).catch(() => { if (!disposed) setError("저장 영상을 열지 못했습니다."); });
    return () => { disposed = true; if (opened) void api.close(opened); };
  }, [api, recordingId]);
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButton.current?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, []);
  useEffect(() => {
    const change = () => setFullscreen(document.fullscreenElement === dialog.current);
    document.addEventListener("fullscreenchange", change);
    return () => document.removeEventListener("fullscreenchange", change);
  }, []);
  useEffect(() => {
    // Privacy hides local playback as well as native live surfaces.
    if (privacyMode) { video.current?.pause(); setPaused(true); }
  }, [privacyMode]);
  const readTime = useCallback(() => {
    const value = video.current?.currentTime;
    if (value === undefined || !Number.isFinite(value)) return;
    const next = Math.floor(value * 10) / 10;
    if (next !== displayTime.current) { displayTime.current = next; setTime(next); }
  }, []);
  useEffect(() => {
    if (paused || buffering || privacyMode) return;
    let frame = 0;
    const update = () => { if (!document.hidden) readTime(); frame = requestAnimationFrame(update); };
    frame = requestAnimationFrame(update);
    return () => cancelAnimationFrame(frame);
  }, [paused, buffering, privacyMode, readTime]);
  const seek = useCallback((target: number) => {
    if (!video.current || !Number.isFinite(target)) return;
    const next = Math.max(0, Math.min(duration, target));
    video.current.currentTime = next; setTime(next); setSeekVersion((value) => value + 1);
  }, [duration]);
  const togglePlay = () => {
    const element = video.current;
    if (!element || privacyMode) return;
    if (!element.paused) element.pause();
    else { setError(null); void element.play().catch(() => setError("이 환경에서 영상 재생을 시작하지 못했습니다. 외부 플레이어로 열기를 이용할 수 있습니다.")); }
  };
  const changeVolume = (next: number) => {
    if (!video.current) return;
    video.current.volume = next; video.current.muted = false; setVolume(next); setMuted(false);
  };
  const toggleMute = () => { if (video.current) { video.current.muted = !video.current.muted; setMuted(video.current.muted); } };
  const toggleFullscreen = async () => {
    try {
      if (document.fullscreenElement === dialog.current) await document.exitFullscreen();
      else if (dialog.current?.requestFullscreen) await dialog.current.requestFullscreen();
      else setError("이 환경에서는 전체화면을 사용할 수 없습니다.");
    } catch { setError("전체화면을 전환하지 못했습니다."); }
  };
  const close = () => { video.current?.pause(); if (document.fullscreenElement === dialog.current) void document.exitFullscreen().catch(() => {}); onClose(); };
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Tab") {
      const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]')].filter((item) => !item.closest("[hidden]"));
      const first = controls[0], last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      return;
    }
    if (event.key === "Escape" && !document.fullscreenElement) { event.preventDefault(); if (settings) setSettings(false); else close(); return; }
    if (event.altKey || event.ctrlKey || event.metaKey || !replayShortcutAllowed(event.target)) return;
    switch (event.key.toLowerCase()) {
      case " ": case "k": event.preventDefault(); togglePlay(); break;
      case "arrowleft": event.preventDefault(); seek(time - 5); break;
      case "arrowright": event.preventDefault(); seek(time + 5); break;
      case "m": event.preventDefault(); toggleMute(); break;
      case "f": event.preventDefault(); void toggleFullscreen(); break;
    }
  };
  const loadTimeline = useCallback(() => {
    if (!session || timelineRequested.current) return;
    timelineRequested.current = true;
    void api.timeline(session.token, Math.max(10, Math.ceil(session.durationSeconds / 240))).then((result) => {
      if (result.ok) { setTimeline(result.data); if (result.data.indexState !== "ready") timelineRequested.current = false; }
      else timelineRequested.current = false;
    }).catch(() => { timelineRequested.current = false; });
  }, [api, session]);
  useEffect(() => { if (session?.indexState === "ready") loadTimeline(); }, [session, loadTimeline]);
  const saveOffset = async () => {
    if (!session || offsetSaving.current) return;
    const value = Number(offset);
    if (!offset.trim() || !Number.isFinite(value) || Math.abs(value) > 3600) { setError("채팅 보정은 -3600~3600초로 입력해 주세요."); return; }
    offsetSaving.current = true; setSavingOffset(true); setError(null);
    try {
      const result = await api.setOffset(session.token, value);
      if (result.ok) { setSession((previous) => previous && { ...previous, manualOffsetSeconds: result.data }); setOffset(String(result.data)); setSeekVersion((previous) => previous + 1); timelineRequested.current = false; setTimeline(null); }
      else setError(result.error.message);
    } catch { setError("채팅 보정을 저장하지 못했습니다."); }
    finally { offsetSaving.current = false; setSavingOffset(false); }
  };
  const indexReady = useCallback((quality: ReplaySession["syncQuality"]) => {
    setSession((previous) => previous && (previous.syncQuality !== quality || previous.indexState !== "ready") ? { ...previous, syncQuality: quality, indexState: "ready" } : previous);
    loadTimeline();
  }, [loadTimeline]);
  const peak = Math.max(1, ...(timeline?.buckets.map((bucket) => bucket.chatCount) ?? []));
  return createPortal(<div className="recording-replay-backdrop" data-native-overlay="true">
    <div ref={dialog} className="recording-replay" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={keys} tabIndex={-1}>
      <header className="recording-replay-heading"><div><span className="recording-replay-mark">ATSUMI <span>다시보기</span></span><h2 id={titleId}>{privacyMode ? "프라이버시 모드" : session?.title || "저장 영상 다시보기"}</h2></div><div><span className="recording-replay-offline" title="녹화된 영상과 채팅을 표시합니다. 시청자 수: 기록 없음">로컬 저장본</span><button ref={closeButton} type="button" aria-label="다시보기 닫기" onClick={close}>✕</button></div></header>
      {liveRecording ? <p className="recording-replay-warning" role="status">다른 녹화가 진행 중입니다. 저장본 재생은 추가 CPU·GPU·디스크 자원을 사용합니다.</p> : null}
      {error ? <p className="recording-replay-error" role="alert">{error}</p> : null}
      {session?.warnings.length ? <p className="recording-replay-warning" role="status">{session.warnings.join(" · ")}</p> : null}
      {!session ? <div className="recording-replay-loading">{error ? "녹화 목록에서 외부 플레이어로 열기를 사용할 수 있습니다." : "저장 영상을 준비하고 있습니다…"}</div> : <>
      {privacyMode ? <div className="recording-replay-loading">프라이버시 모드에서 영상과 채팅을 가렸습니다.</div> : null}
      <div className="recording-replay-body" hidden={privacyMode}>
        <section className="recording-replay-player" aria-label="저장 영상 플레이어">
          <div className="recording-replay-stage" tabIndex={0} aria-label="영상, Space 재생 또는 일시정지, 방향키 5초 이동">
            <video ref={video} src={api.mediaUrl(session.token) || undefined} preload="metadata" playsInline controls={false} onClick={togglePlay} onDoubleClick={() => void toggleFullscreen()}
              onTimeUpdate={readTime} onSeeking={() => { readTime(); setSeekVersion((value) => value + 1); }} onSeeked={() => { setBuffering(false); readTime(); }}
              onLoadedMetadata={() => { const value = video.current?.duration; if (value && Number.isFinite(value)) setMediaDuration(value); }}
              onDurationChange={() => { const value = video.current?.duration; if (value && Number.isFinite(value)) setMediaDuration(value); }}
              onPlay={() => setPaused(false)} onPause={() => { setPaused(true); readTime(); }} onPlaying={() => setBuffering(false)} onWaiting={() => setBuffering(true)} onEnded={() => { setPaused(true); readTime(); }}
              onError={() => setError("저장 영상의 형식이나 파일을 읽을 수 없습니다. 외부 플레이어로 열기를 이용해 주세요.")} />
            {paused ? <button type="button" className="recording-replay-center-play" aria-label="영상 재생" onClick={togglePlay}>▶</button> : null}
            {buffering ? <span className="recording-replay-buffering" role="status">영상 읽는 중…</span> : null}
          </div>
          <div className="recording-replay-controls">
            {timeline?.buckets.length ? <div className="recording-replay-heatstrip" aria-label="시간대별 저장 채팅 수">{timeline.buckets.slice(0, 720).map((bucket) => <button key={bucket.startSeconds} type="button" title={`${formatReplayTime(bucket.startSeconds)} · 채팅 ${bucket.chatCount.toLocaleString("ko-KR")}개 · 고유 참여자 ${bucket.uniqueSenderCount == null ? "확인되지 않음" : `${bucket.uniqueSenderCount}명`}`} aria-label={`${formatReplayTime(bucket.startSeconds)} 채팅 ${bucket.chatCount}개 위치로 이동`} onClick={() => seek(bucket.startSeconds)}><span style={{ height: `${Math.max(5, bucket.chatCount / peak * 100)}%` }} /></button>)}</div> : null}
            <input className="recording-replay-seek" type="range" aria-label="영상 재생 위치" min={0} max={duration || 1} step={0.1} value={Math.min(time, duration || 1)} disabled={!duration} onChange={(event) => seek(Number(event.target.value))} aria-valuetext={`${formatReplayTime(time)} / ${formatReplayTime(duration)}`} />
            <div className="recording-replay-control-row">
              <button type="button" aria-label={paused ? "재생" : "일시정지"} onClick={togglePlay}>{paused ? "▶" : "Ⅱ"}</button>
              <button type="button" aria-label={muted ? "음소거 해제" : "음소거"} onClick={toggleMute}>{muted || volume === 0 ? "♪̸" : "♪"}</button>
              <input className="recording-replay-volume" type="range" aria-label="음량" min={0} max={1} step={0.01} value={muted ? 0 : volume} onChange={(event) => changeVolume(Number(event.target.value))} />
              <span className="recording-replay-clock">{formatReplayTime(time)} <span>/ {formatReplayTime(duration)}</span></span>
              <span className="recording-replay-control-spacer" />
              <span className="recording-replay-quality" title={`${session.syncQuality === "observed_media" ? "관측 영상 시각 기준" : "수신 시각 기준·동기화 근사"} · 채팅 보정 ${session.manualOffsetSeconds > 0 ? "+" : ""}${session.manualOffsetSeconds}초`}>{session.syncQuality === "observed_media" ? "영상 시각 기준" : "동기화 근사"}</span>
              <select aria-label="재생 속도" value={rate} onChange={(event) => { const value = Number(event.target.value); if (video.current) video.current.playbackRate = value; setRate(value); }}>{[0.5, 0.75, 1, 1.25, 1.5, 2].map((value) => <option key={value} value={value}>{value}×</option>)}</select>
              <button type="button" aria-label="채팅 동기화 설정" aria-expanded={settings} onClick={() => setSettings(!settings)}>⚙</button>
              <button type="button" aria-label={fullscreen ? "전체화면 종료" : "영상과 채팅 전체화면"} onClick={() => void toggleFullscreen()}>⛶</button>
            </div>
          </div>
          {settings ? <form className="recording-replay-sync" onSubmit={(event) => { event.preventDefault(); void saveOffset(); }}><label htmlFor={`${titleId}-offset`}>채팅 시간 보정 (초)</label><div><input id={`${titleId}-offset`} type="number" min={-3600} max={3600} step={0.1} value={offset} onChange={(event) => setOffset(event.target.value)} /><button type="submit" disabled={savingOffset}>{savingOffset ? "저장 중…" : "보정 저장"}</button></div><p>양수는 채팅을 더 늦게 표시합니다. 이 녹화의 재생 설정에 저장됩니다. 이전 기록은 버퍼·회전 공백 때문에 구간별 차이가 남을 수 있습니다.</p></form> : null}
        </section>
        <RecordingReplayChat api={api} session={session} time={Math.floor(time * 5) / 5} seekVersion={seekVersion} onSeek={seek} onIndexReady={indexReady} />
      </div></>}
    </div>
  </div>, document.body);
}
