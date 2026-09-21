import { createRoot } from "react-dom/client";
import { AutoRecordingPanel } from "../src/features/streaming/AutoRecordingPanel";
import type { AutoRecordingApi, AutoRecordingSnapshot } from "../src/api/autoRecording";
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
function Preview() {
  return <main className="streaming-workspace" style={{ minHeight: "100vh", background: "#17151c" }}>
    <AutoRecordingPanel runtime="tauri" api={api} />
  </main>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
