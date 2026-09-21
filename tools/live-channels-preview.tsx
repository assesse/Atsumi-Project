import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ConnectionSetup, type ConnectionMode } from "../src/features/streaming/ConnectionSetup";
import { LiveChannelPicker } from "../src/features/streaming/LiveChannelPicker";
import type { LiveChannelsApi, LiveFavorite } from "../src/api/liveChannels";
import type { AutoRecordingApi } from "../src/api/autoRecording";
import "../src/styles.css";
import "../src/features/streaming/StreamingWorkspace.css";
import "../src/features/streaming/OfficialBrowserPanel.css";
import "../src/features/streaming/MadoWorkspace.css";

// In-memory UI fixture only. No native commands, media or account requests.
const names = ["도서관 라이브", "밤의 라디오", "오후의 게임 방송", "아주 긴 채널 이름도 목록 너비를 넘어가지 않도록 표시하는 방송"];
const ids = ["a", "b", "c", "d"].map(value => value.repeat(32));
let favorites: LiveFavorite[] = [{ channelId: ids[0]!, channelName: names[0]! }, { channelId: ids[2]!, channelName: names[2]! }, { channelId: ids[3]!, channelName: names[3]! }];
const saved = (): LiveFavorite[] => favorites.map(row => ({ ...row }));
const api: LiveChannelsApi = { snapshot: async () => ({ ok: true, data: saved() }), set: async (input, favorite) => { favorites = favorites.filter(row => row.channelId !== input); if (favorite) favorites.push({ channelId: input, channelName: names[ids.indexOf(input)] ?? "입력한 채널" }); return { ok: true, data: saved() }; }, profile: async (id) => ({ ok: true, data: { channelName: names[ids.indexOf(id)] ?? "채널", image: null } }) };
const autoApi: AutoRecordingApi = {
  snapshot: async () => ({ ok: true, data: { captureChat: true, error: null, channels: ids.slice(0, 2).map((channelId, index) => ({ channelId, channelName: names[index]!, enabled: true, status: "recording", recordingId: `test-${index}`, checkedAt: 0, message: null })) } }),
  add: async () => { throw new Error("Preview must not change recording rules"); }, update: async () => { throw new Error("Preview must not change recording rules"); },
};
function Preview() {
  const [mode, setMode] = useState<ConnectionMode>("general"), [inputs, setInputs] = useState(["", "", "", ""]), [notice, setNotice] = useState("");
  return <main className="streaming-workspace mado-workspace" style={{ minHeight: "100dvh", margin: 0, padding: 24 }}>
    <p>합성 채널만 사용하는 로컬 미리보기</p><p role="status">{notice}</p>
    <div className="official-browser-dialog-backdrop" role="dialog" aria-modal="true" data-native-preserve-video="true" aria-label="연결 설정">
      <ConnectionSetup mode={mode} onMode={setMode} modeDisabled={false} receiverBacked
        channelPicker={<LiveChannelPicker runtime="tauri" inputs={inputs} multiple={mode === "mado"} privacy={false} disabled={false} onInputs={setInputs} api={api} autoApi={autoApi} />}
        inputs={inputs} onInput={(index, value) => setInputs(rows => rows.map((row, i) => i === index ? value : row))} disabled={false} pending={false}
        onConnect={() => setNotice(`${inputs.slice(0, mode === "general" ? 1 : 4).filter(Boolean).length}개 선택됨 · 실제 연결하지 않음`)}
        auth="signed_out" accountDisabled={false} onLogin={() => {}} onLogout={() => {}} onRefresh={() => {}} onGrid={() => {}} gridDisabled={false} gridStatus="로컬 테스트" onInstaller={() => {}} installerDisabled
        options={<div className="mado-layout-options"><button type="button" aria-pressed="true">4화면4챗</button><button type="button">1화면4챗</button></div>} />
    </div>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
