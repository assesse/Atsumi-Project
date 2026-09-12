// Development-only visual fixture. No invoke, account, media, fetch or storage.
import { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ApiResult } from "../src/api/contracts";
import { emptyOfficialBrowserSnapshot, type BrowserRecording, type OfficialBrowserApi, type OfficialBrowserSnapshot, type OfficialBrowserViewport } from "../src/api/officialBrowser";
import { OfficialBrowserPanel } from "../src/features/streaming/OfficialBrowserPanel";
import "../src/styles.css";
import "../src/features/streaming/StreamingWorkspace.css";

type Scenario = "initial" | "ready" | "recording" | "connecting" | "error" | "request-start" | "request-stop" | "request-screenshot";
const ok = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const recording = (): BrowserRecording => ({ id: "fixture-recording", channelId: "a".repeat(32), title: "로컬 테스트 방송", startedAt: 1800000000000, updatedAt: 1800000060000, status: "recording", mimeType: "video/webm", outputDir: "", segmentCount: 4, bytesWritten: 24 * 1024 ** 2, durationSeconds: 60, lastError: null, segments: [] });
function Fixture() {
  const [scenario, setScenario] = useState<Scenario>("ready");
  const [privacy, setPrivacy] = useState(false);
  const [viewport, setViewport] = useState<OfficialBrowserViewport | null>(null);
  const [view, setView] = useState<"live" | "recordings">("live");
  const api = useMemo<OfficialBrowserApi>(() => {
    let state: OfficialBrowserSnapshot = { ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: scenario !== "initial", ready: scenario === "ready" || scenario === "recording", channelId: "a".repeat(32), status: scenario === "initial" ? "closed" : scenario === "recording" ? "recording" : scenario === "error" ? "error" : "ready", extensionStatus: scenario === "connecting" ? "연결 중" : "확장 로드 완료 · 공식 페이지 감지 확인 중", loginStatus: "로그인 여부는 공식 화면에서 확인", videoWidth: 1920, videoHeight: 1080, chatStatus: "receiving", chatCount: 123, error: scenario === "error" ? "테스트 오류: 영상 재생 상태를 확인해 주세요." : null, recordings: scenario === "recording" ? [recording()] : [], recordingId: scenario === "recording" ? "fixture-recording" : null };
    const update = (patch: Partial<OfficialBrowserSnapshot>) => { state = { ...state, ...patch }; return Promise.resolve(ok(state)); };
    if (scenario.startsWith("request-")) {
      state.ready = true;
      state.pendingControl = { id: "fixture-intent", action: scenario === "request-start" ? "record_start" : scenario === "request-stop" ? "record_stop" : "screenshot", channelId: "a".repeat(32), expiresAt: Date.now() + 20000 };
      if (scenario === "request-stop") { state.recordingId = "fixture-recording"; state.recordings = [recording()]; state.status = "recording"; }
    }
    return {
      runtime: "tauri", snapshot: async () => ok(state),
      open: async () => update({ windowOpen: true, ready: true, status: "ready", error: null }),
      start: async ({ rightsAcknowledged }) => rightsAcknowledged ? update({ recordingId: "fixture-recording", status: "recording", recordings: [recording()] }) : { ok: false, error: { code: "FIXTURE_RIGHTS", message: "저장 권한을 확인해 주세요.", retryable: false } },
      stop: async () => update({ recordingId: null, status: "ready", recordings: [{ ...recording(), status: "stopped" }] }),
      connectExtension: async () => update({ extensionStatus: "확장 로드 완료 · 공식 페이지 감지 확인 중", ready: true }),
      login: async () => update({ loginStatus: "테스트 로그인 요청 · 실제 계정은 사용하지 않습니다" }),
      logout: async () => update({ loginStatus: "로그아웃됨" }),
      setViewport: async (next) => { setViewport(next); return ok(undefined); },
      requestControl: async (action) => update({ pendingControl: { id: `fixture-${Date.now()}`, action, channelId: "a".repeat(32), expiresAt: Date.now() + 20000 } }),
      ackUiAction: async () => update({ pendingUiAction: null }),
      confirmControl: async ({ requestId, approve, rightsAcknowledged }) => {
        const intent = state.pendingControl;
        if (!intent || intent.id !== requestId || intent.expiresAt <= Date.now()) return { ok: false, error: { code: "FIXTURE_EXPIRED", message: "테스트 요청이 만료되었습니다.", retryable: false } };
        if (!approve) return update({ pendingControl: null });
        if (intent.action !== "record_stop" && !rightsAcknowledged) return { ok: false, error: { code: "FIXTURE_RIGHTS", message: "저장 권한을 확인해 주세요.", retryable: false } };
        if (intent.action === "record_start") return update({ pendingControl: null, recordingId: "fixture-recording", status: "recording", recordings: [recording()] });
        if (intent.action === "record_stop") return update({ pendingControl: null, recordingId: null, status: "ready", recordings: [{ ...recording(), status: "stopped" }] });
        return update({ pendingControl: null, lastScreenshot: { id: "fixture-screenshot", channelId: "a".repeat(32), fileName: "local-preview-only.png", createdAt: Date.now() } });
      },
      openInstaller: async () => ok(undefined), openFolder: async () => ok(undefined), openSegment: async () => ok(undefined),
      openMerged: async () => ok(undefined), retryMerge: async () => ok(state),
    };
  }, [scenario]);
  const clip = viewport?.clip;
  return <div style={{ height: "100dvh", display: "flex", flexDirection: "column" }}>
    <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 12, padding: "8px 16px", background: "#263547", color: "#fff", fontSize: 12 }}>
      <strong>로컬 UI 테스트 · 실제 방송·계정·녹화 없음</strong>
      <label>상태 <select aria-label="테스트 상태" value={scenario} onChange={(event) => setScenario(event.target.value as Scenario)}><option value="initial">처음 연결</option><option value="ready">준비됨 (권한 체크 후 활성)</option><option value="recording">녹화 중</option><option value="connecting">연결 중 (비활성)</option><option value="error">오류</option><option value="request-start">플레이어 녹화 요청</option><option value="request-stop">플레이어 중지 요청</option><option value="request-screenshot">플레이어 화면 저장 요청</option></select></label>
      <label><input type="checkbox" checked={privacy} onChange={(event) => setPrivacy(event.target.checked)} /> 프라이버시</label>
      <button onClick={() => setView((current) => current === "live" ? "recordings" : "live")}>시청 / 보관함</button>
    </div>
    <div className="app-shell streaming-shell" style={{ flex: 1, minHeight: 0, gridTemplateColumns: "minmax(0, 1fr)" }}>
      <main className="streaming-workspace is-official-view">
        <OfficialBrowserPanel key={scenario} api={api} runtime="tauri" active view={view} privacyMode={privacy} />
      </main>
    </div>
    {viewport?.visible ? <div aria-label="로컬 가상 시청 영역" style={{ position: "fixed", zIndex: 2, left: viewport.x, top: viewport.y, width: viewport.width, height: viewport.height, display: "grid", gridTemplateColumns: "minmax(0, 1fr) minmax(200px, 28%)", pointerEvents: "none", clipPath: clip ? `inset(${clip.y}px ${viewport.width - clip.x - clip.width}px ${viewport.height - clip.y - clip.height}px ${clip.x}px)` : undefined }}>
      <div style={{ position: "relative", display: "grid", placeContent: "center", textAlign: "center", background: "linear-gradient(135deg,#17394b,#112524)", color: "#c7f6e3" }}><strong>영상 자리 · 로컬 테스트</strong><small>실제 영상이 아닙니다</small><div style={{ position: "absolute", bottom: 16, right: 16, display: "flex", gap: 6, pointerEvents: "auto" }}><button onClick={() => void api.requestControl("record_start")}>녹화</button><button onClick={() => void api.requestControl("record_stop")}>중지</button><button onClick={() => void api.requestControl("screenshot")}>화면 저장</button></div></div>
      <div style={{ display: "flex", flexDirection: "column", justifyContent: "space-between", padding: 18, borderLeft: "1px solid #344351", background: "#101921", color: "#d8e3eb", fontSize: 12 }}><strong>채팅 자리</strong><p style={{ margin: 0, padding: 10, border: "1px solid #435b6b", borderRadius: 6 }}>채팅 입력 영역 · 위치 확인용</p></div>
    </div> : null}
  </div>;
}

// Preserve the fixture's root if a component edit propagates through Vite HMR.
const root = import.meta.hot?.data.root ?? createRoot(document.getElementById("root")!);
if (import.meta.hot) import.meta.hot.data.root = root;
root.render(<Fixture />);
