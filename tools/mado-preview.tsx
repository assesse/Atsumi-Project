// Local UI fixture only. No native invoke, external URLs, media or persistence.
import { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ApiResult } from "../src/api/contracts";
import type { OfficialBrowserViewport } from "../src/api/officialBrowser";
import { emptyMultiview, type MultiviewApi, type MultiviewEntry, type MultiviewSnapshot } from "../src/api/multiview";
import { MadoWorkspace } from "../src/features/streaming/MadoWorkspace";
import "../src/styles.css";
import "../src/features/streaming/StreamingWorkspace.css";

const channels = ["a", "b", "c", "d"].map((letter) => letter.repeat(32));
const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const make = (entries: MultiviewEntry[], epoch: number): MultiviewSnapshot => ({ active: true, epoch, audioOwner: null, panes: entries.flatMap((entry) => [
  ...(entry.video ? [{ paneId: `fixture-${epoch}-${entry.channelId}-video`, channelId: entry.channelId, channelName: `도서관 ${channels.indexOf(entry.channelId) + 1}`, kind: "video" as const, status: "ready", ready: true, audioEnabled: false }] : []),
  ...(entry.chat ? [{ paneId: `fixture-${epoch}-${entry.channelId}-chat`, channelId: entry.channelId, channelName: `도서관 ${channels.indexOf(entry.channelId) + 1}`, kind: "chat" as const, status: "ready" }] : []),
]) });
function Fixture() {
  const [scenario, setScenario] = useState("paired");
  const [privacy, setPrivacy] = useState(false);
  const [message, setMessage] = useState("");
  const [recordingIds, setRecordingIds] = useState<string[]>([]);
  const [viewports, setViewports] = useState<Record<string, OfficialBrowserViewport>>({});
  const api = useMemo<MultiviewApi>(() => {
    let state = scenario === "initial" ? emptyMultiview() : make(channels.slice(0, scenario === "one" ? 1 : 4).map((channelId, index) => ({ channelId, video: scenario !== "chats" || index === 1, chat: true })), 50);
    return {
      runtime: "tauri", snapshot: async () => ok(state),
      configure: async (entries) => {
        if (scenario === "error") return { ok: false, error: { code: "FIXTURE_DENIED", message: "로컬 테스트 오류: 녹화를 먼저 중지해 주세요.", retryable: true } };
        state = make(entries, state.epoch + 1); return ok(state);
      },
      close: async (epoch) => {
        if (epoch !== state.epoch) return { ok: false, error: { code: "MULTIVIEW_STALE", message: "이전 테스트 배치입니다.", retryable: false } };
        state = { ...emptyMultiview(), epoch: state.epoch + 1 }; return ok(state);
      },
      setAudio: async (channelId, epoch) => {
        if (epoch !== state.epoch) return { ok: false, error: { code: "MULTIVIEW_STALE", message: "이전 테스트 배치입니다.", retryable: false } };
        state = { ...state, audioOwner: channelId }; return ok(state);
      },
      setPaneAudio: async (paneId, enabled) => { state = { ...state, panes: state.panes.map((pane) => pane.paneId === paneId ? { ...pane, audioEnabled: enabled } : pane) }; return ok(state); },
      requestControl: async (paneId, action) => {
        const pane = state.panes.find((item) => item.paneId === paneId)!;
        state = { ...state, pendingControl: { paneId, id: `preview-${Date.now()}`, action, channelId: pane.channelId, expiresAt: Date.now() + 20000 } }; return ok(state);
      },
      confirmControl: async ({ paneId, requestId, approve, rightsAcknowledged }) => {
        const intent = state.pendingControl;
        if (!intent || intent.id !== requestId || intent.paneId !== paneId || intent.expiresAt <= Date.now() || (approve && intent.action !== "record_stop" && !rightsAcknowledged)) return { ok: false, error: { code: "FIXTURE_DENIED", message: "테스트 요청과 권한을 확인해 주세요.", retryable: false } };
        state = { ...state, pendingControl: null };
        if (!approve) return ok(state);
        state = { ...state, panes: state.panes.map((pane) => pane.paneId !== paneId ? pane : intent.action === "screenshot"
          ? { ...pane, lastScreenshot: { id: "fixture-shot", channelId: pane.channelId, fileName: "local-preview-only.png", createdAt: Date.now() } }
          : { ...pane, recordingId: intent.action === "record_start" ? `fixture-recording-${paneId}` : null, recordingStatus: intent.action === "record_start" ? "recording" : "stopped" }) };
        setRecordingIds(state.panes.filter((pane) => !!pane.recordingId).map((pane) => pane.paneId)); return ok(state);
      },
      ackUiAction: async () => { state = { ...state, pendingUiAction: null }; return ok(state); },
      setViewport: async (paneId, viewport) => { setViewports((current) => ({ ...current, [paneId]: viewport })); return ok(undefined); },
    };
  }, [scenario]);
  return <div style={{ display: "flex", flexDirection: "column", height: "100dvh" }}>
    <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 12, padding: "8px 12px", color: "#fff", background: "#263547", fontSize: 12 }}>
      <strong>로컬 마도 UI · 실제 방송·계정 없음</strong>
      <label>배치 <select aria-label="테스트 배치" value={scenario} onChange={(event) => { setViewports({}); setMessage(""); setRecordingIds([]); setScenario(event.target.value); }}><option value="initial">처음 연결</option><option value="paired">4방송 · 4채팅</option><option value="chats">1방송 · 4채팅</option><option value="one">1방송 · 1채팅</option><option value="error">적용 오류</option></select></label>
      <label><input type="checkbox" checked={privacy} onChange={(event) => setPrivacy(event.target.checked)} /> 프라이버시</label>
      <span role="status">{message}</span>
    </div>
    <div className="app-shell streaming-shell" style={{ flex: 1, minHeight: 0, gridTemplateColumns: "minmax(0, 1fr)" }}><main className="streaming-workspace is-official-view">
      <MadoWorkspace key={scenario} runtime="tauri" api={api} privacy={privacy} onLeave={() => setMessage("한 방송 보기로 나가기 요청 · 테스트에서만 표시")} />
    </main></div>
    {Object.entries(viewports).filter(([, viewport]) => viewport.visible).map(([paneId, viewport]) => {
      const chat = paneId.endsWith("-chat"), clip = viewport.clip;
      const channel = channels.findIndex((id) => paneId.includes(id)) + 1;
      return <div key={paneId} aria-label={`로컬 방송 ${channel} ${chat ? "채팅" : "영상"}`} style={{ position: "fixed", zIndex: 2, pointerEvents: "none", left: viewport.x, top: viewport.y, width: viewport.width, height: viewport.height, clipPath: clip ? `inset(${clip.y}px ${viewport.width - clip.x - clip.width}px ${viewport.height - clip.y - clip.height}px ${clip.x}px)` : undefined, background: chat ? "#131b24" : `linear-gradient(130deg,hsl(${150 + channel * 25} 35% 24%),#102025)`, color: "#d6efe5", fontSize: 12, display: "flex", flexDirection: "column", justifyContent: chat ? "space-between" : "center", alignItems: chat ? "stretch" : "center", padding: 12, boxSizing: "border-box" }}>
        <strong>방송 {channel} · {chat ? "채팅" : "영상"} 자리</strong>
        {chat ? <div style={{ padding: 8, border: "1px solid #4c6576", borderRadius: 6 }}>채팅 입력 위치</div> : <><small>실제 미디어 수신 없음</small><div style={{ display: "flex", gap: 6, marginTop: 14, pointerEvents: "auto" }}><button onClick={() => void api.requestControl(paneId, recordingIds.includes(paneId) ? "record_stop" : "record_start", viewport.epoch!)}>{recordingIds.includes(paneId) ? "녹화 중지" : "녹화"}</button><button onClick={() => void api.requestControl(paneId, "screenshot", viewport.epoch!)}>화면 저장</button></div></>}
      </div>;
    })}
  </div>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
