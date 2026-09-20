import { createRoot } from "react-dom/client";
import { useState } from "react";
import { AutoRecordingPanel } from "../src/features/streaming/AutoRecordingPanel";
import { AutoRecordingLiveView } from "../src/features/streaming/AutoRecordingLiveView";
import type { AutoWatchApi, AutoWatchTarget } from "../src/api/autoWatch";
import type { AutoRecordingApi, AutoRecordingSnapshot } from "../src/api/autoRecording";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot, type OfficialBrowserApi } from "../src/api/officialBrowser";
import "../src/styles.css";
import "../src/features/streaming/StreamingWorkspace.css";

// Isolated in-memory UI fixture: no real channel requests, files or recordings.
let state: AutoRecordingSnapshot = { captureChat: true, error: null, channels: [
  { channelId: "a".repeat(32), channelName: "도서관 라이브", enabled: true, checkedAt: Date.now(), status: "recording", recordingId: "fixture-recording", message: null },
  { channelId: "b".repeat(32), channelName: "밤의 라디오", enabled: true, checkedAt: Date.now(), status: "waiting", recordingId: null, message: null },
  { channelId: "c".repeat(32), channelName: "오후의 게임 방송", enabled: false, checkedAt: Date.now(), status: "disabled", recordingId: null, message: null },
] };
const result = () => ({ ok: true as const, data: structuredClone(state) });
const api: AutoRecordingApi = {
  snapshot: async () => result(),
  add: async (input) => { state.channels.push({ channelId: "fixture-" + state.channels.length, channelName: input, enabled: true, checkedAt: Date.now(), status: "waiting", recordingId: null, message: null }); return result(); },
  update: async (id, change) => {
    if (change.action === "remove") state.channels = state.channels.filter(channel => channel.channelId !== id);
    else state.channels = state.channels.map(channel => channel.channelId === id ? { ...channel,
      enabled: change.action === "enabled" ? change.enabled : channel.enabled,
      status: change.action === "stop" ? "skipped" : change.enabled ? "waiting" : "disabled", recordingId: null } : channel);
    return result();
  },
};
let audioEnabled = true;
const watchSnapshot = () => ({ ok: true as const, data: { recording: true, status: "recording", error: null, audioEnabled, chatCount: 456 } });
const watchApi: AutoWatchApi = {
  runtime: "tauri",
  open: async (channelId, recordingId) => ({ ok: true, data: { kind: "automatic", watchId: "fixture-watch", channelId, recordingId, channelName: "도서관 라이브", epoch: 1 } }),
  snapshot: async () => watchSnapshot(),
  setViewport: async () => ({ ok: true, data: undefined }),
  setAudio: async (_, enabled) => { audioEnabled = enabled; return watchSnapshot(); },
  close: async () => ({ ok: true, data: undefined }),
};
// This page exercises the shared trusted UI only; native player/chat rendering
// is verified separately with the offline WebView and presentation tests.
const liveApi: OfficialBrowserApi = { ...createOfficialBrowserApi("browser-mock"), runtime: "tauri",
  snapshot: async () => ({ ok: true, data: { ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: true,
    ready: true, channelId: "a".repeat(32), recordingId: "fixture-recording", status: "recording", viewportEpoch: 1 } }),
  setViewport: async () => ({ ok: true, data: undefined }),
};
function Preview() {
  const [target, setTarget] = useState<AutoWatchTarget | null>(null);
  return <main className="streaming-workspace" style={{ minHeight: "100vh", background: "#17151c" }}>
    {target ? <AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={watchApi} liveApi={liveApi} onLeave={() => setTarget(null)} />
      : <AutoRecordingPanel runtime="tauri" api={api} watchApi={watchApi} onWatch={setTarget} />}
  </main>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
