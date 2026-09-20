import { useEffect, useMemo, useRef, useState } from "react";
import { createAutoRecordingApi, type AutoRecordingApi, type AutoRecordingSnapshot } from "../../api/autoRecording";
import type { ApiResult } from "../../api/contracts";
import { createAutoWatchApi, type AutoWatchApi, type AutoWatchTarget } from "../../api/autoWatch";
import "./AutoRecordingPanel.css";

const statuses: Record<string, string> = {
  waiting: "방송 대기", disabled: "꺼짐", starting: "녹화 준비", recording: "녹화 중",
  stopping: "마무리 중", queued: "자리 대기", retry: "연결 확인 · 재시도 대기", skipped: "이번 방송 건너뜀",
  ending: "방송 종료 확인 중", attention: "반복 실패 · 확인 필요",
};
export function AutoRecordingPanel({ runtime, privacy = false, api: suppliedApi, watchApi: suppliedWatchApi, onWatch }: {
  runtime: "tauri" | "browser-mock"; privacy?: boolean; api?: AutoRecordingApi;
  watchApi?: AutoWatchApi; onWatch?(target: AutoWatchTarget): void;
}) {
  const api = useMemo(() => suppliedApi ?? createAutoRecordingApi(runtime), [runtime, suppliedApi]);
  const watchApi = useMemo(() => suppliedWatchApi ?? createAutoWatchApi(runtime), [runtime, suppliedWatchApi]);
  const [snapshot, setSnapshot] = useState<AutoRecordingSnapshot>({ channels: [], captureChat: true, error: null });
  const [input, setInput] = useState("");
  const [pending, setPending] = useState(false);
  const [opening, setOpening] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const version = useRef(0), mounted = useRef(false), busy = useRef(false);
  useEffect(() => {
    mounted.current = true;
    let disposed = false, timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const request = version.current;
      if (!busy.current) {
        try {
          const result = await api.snapshot();
          if (!disposed && request === version.current && !busy.current) {
            if (result.ok) { setSnapshot(result.data); setPollError(null); } else setPollError(result.error.message);
          }
        } catch { if (!disposed) setPollError("자동 녹화 상태를 확인하지 못했습니다."); }
      }
      if (!disposed) timer = setTimeout(poll, 2000);
    };
    void poll();
    return () => { disposed = true; mounted.current = false; version.current++; clearTimeout(timer); };
  }, [api]);
  const mutate = async (action: () => Promise<ApiResult<AutoRecordingSnapshot>>, clearInput = false) => {
    if (busy.current) return;
    busy.current = true; setPending(true); setError(null);
    const request = ++version.current;
    try {
      const result = await action();
      if (mounted.current && request === version.current) {
        if (result.ok) { setSnapshot(result.data); if (clearInput) setInput(""); }
        else setError(result.error.message);
      }
    } catch { if (mounted.current) setError("요청을 처리하지 못했습니다. 다시 시도해 주세요."); }
    finally { busy.current = false; if (mounted.current) setPending(false); }
  };
  const watch = async (channelId: string, recordingId: string) => {
    if (busy.current || !onWatch) return;
    busy.current = true; setPending(true); setOpening(channelId); setError(null);
    const request = ++version.current;
    try {
      const result = await watchApi.open(channelId, recordingId);
      if (!mounted.current || request !== version.current) {
        if (result.ok && result.data.watchId) void watchApi.close(result.data.watchId).catch(() => {});
        return;
      }
      if (result.ok) onWatch(result.data); else setError(result.error.message);
    } catch { if (mounted.current) setError("실시간 보기를 열지 못했습니다. 다시 시도해 주세요."); }
    finally { busy.current = false; if (mounted.current) { setPending(false); setOpening(null); } }
  };
  const disabled = pending || runtime !== "tauri";
  return <section className="auto-record-panel" aria-label="자동 녹화">
    <header><div><h2>자동 녹화</h2><p>방송이 시작되면 녹화합니다.</p></div>
      <span className="auto-record-help" tabIndex={0} aria-label="자동 녹화 도움말" title="앱 실행 중(트레이 포함) 약 30초마다 확인합니다. 등록 시 이미 방송 중이면 녹화를 시작합니다. 최대 4개 동시 녹화, 초과 채널은 대기합니다. 직접 중지한 회차는 다시 녹화하지 않습니다. 로그인·그리드는 라이브에서 먼저 연결하세요.">?</span></header>
    <form onSubmit={(event) => { event.preventDefault(); if (input.trim()) void mutate(() => api.add(input.trim()), true); }}>
      <input aria-label="자동 녹화 채널 주소" placeholder="채널 주소 또는 ID" maxLength={300} value={input} disabled={disabled} onChange={(event) => setInput(event.target.value)} />
      <button type="submit" disabled={disabled || !input.trim() || snapshot.channels.length >= 32}>등록</button>
    </form>
    {error || pollError || snapshot.error ? <p className="auto-record-error" role="alert">{error || pollError || snapshot.error}</p> : null}
    <div className="auto-record-channels">
      {snapshot.channels.map((channel, index) => {
        const active = ["recording", "starting", "stopping"].includes(channel.status);
        const name = privacy ? `채널 ${index + 1}` : channel.channelName || channel.channelId;
        return <article key={channel.channelId} className={`auto-record-channel is-${channel.status}`} aria-label={name}>
          <div className="auto-record-identity"><strong>{name}</strong>
            <span className="auto-record-status" title={channel.checkedAt ? `최근 확인 ${new Date(channel.checkedAt).toLocaleTimeString("ko-KR")}` : "방송 상태 확인 중"}>{statuses[channel.status] ?? "확인 중"}</span>
            {channel.status === "starting" ? <span className="auto-record-not-started">아직 저장이 시작되지 않았습니다</span> : null}
            {channel.message ? <span className="auto-record-problem" title={channel.message}>ⓘ <span>{channel.message}</span></span> : null}
            {channel.lastFailure ? <div className="auto-record-last-failure" aria-label="최근 저장 미시작 기록">
              <strong>최근 저장 미시작 · {new Date(channel.lastFailure.occurredAt).toLocaleString("ko-KR")}</strong>
              <p>{channel.lastFailure.message}</p>
            </div> : null}
          </div>
          <div className="auto-record-actions">
            {channel.status === "recording" && channel.recordingId && onWatch ? <button type="button" className="auto-record-watch" disabled={disabled} aria-label={`${name} 실시간 보기`} title="녹화 중인 수신 화면을 그대로 봅니다. 녹화와 채팅 저장은 계속됩니다." onClick={() => void watch(channel.channelId, channel.recordingId!)}>{opening === channel.channelId ? "여는 중…" : "실시간 보기"}</button> : null}
            {active || channel.status === "retry" ? <button type="button" className="auto-record-stop" disabled={disabled || channel.status === "stopping"} onClick={() => void mutate(() => api.update(channel.channelId, { action: "stop" }))}>{channel.status === "starting" || channel.status === "retry" ? "시도 중지" : "녹화 중지"}</button> : null}
            <button type="button" aria-label={`${name} 자동 녹화 ${channel.enabled ? "끄기" : "켜기"}`} aria-pressed={channel.enabled} disabled={disabled} title="끄면 자동으로 시작한 녹화도 마무리합니다. 다시 켜면 현재 방송부터 확인합니다." onClick={() => void mutate(() => api.update(channel.channelId, { action: "enabled", enabled: !channel.enabled }))}>{channel.enabled ? "켜짐" : "꺼짐"}</button>
            <button type="button" aria-label={`${name} 등록 해제`} disabled={disabled} title="자동 녹화 등록만 해제합니다. 저장된 녹화본은 남습니다." onClick={() => void mutate(() => api.update(channel.channelId, { action: "remove" }))}>해제</button>
          </div>
        </article>;
      })}
      {!snapshot.channels.length ? <div className="auto-record-empty">자동으로 녹화할 채널을 등록해 주세요.</div> : null}
    </div>
  </section>;
}
