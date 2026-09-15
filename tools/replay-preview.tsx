import { createRoot } from "react-dom/client";
import { RecordingReplay } from "../src/features/streaming/RecordingReplay";
import type { ReplayApi, ReplayMessage } from "../src/api/replay";
import "../src/styles.css";

const texts = ["오늘 방송도 잘 봤어요 💚", "{:smile:} 이 장면 다시 보러 왔습니다", "색감 너무 좋네요", "ㅋㅋㅋㅋㅋㅋ", "채팅과 영상이 같이 움직여요", "좋은 하루 보내세요!", "합성 데이터로 만든 UI 미리보기입니다."];
const messages: ReplayMessage[] = Array.from({ length: 1000 }, (_, sequence) => ({ sequence, sender: ["초록별", "여름바람", "구름산책", "다시보는사람"][sequence % 4]!, text: texts[sequence % texts.length]!, offsetSeconds: sequence / 3 - 200, mediaTimeSeconds: sequence / 3 - 200, serverTime: null, receivedAt: 1_800_000_000_000 + sequence * 1000, broadcastOffsetSeconds: sequence % 4 ? sequence + 7200 : null, syncQuality: "receive_time_approximate", rich: { nicknameColor: ["#85dcb4", "#f2acbf", "#a3bffa", "#e6cf94"][sequence % 4], badges: sequence % 3 ? [] : [{ kind: "subscription", title: "구독", imageUrl: "" }], emojis: [] } }));
let offset = 0;
const api: ReplayApi = {
  open: async () => ({ ok: true, data: { token: "synthetic-only", recordingId: "synthetic-only", title: "늦여름, 조용한 오후의 이야기 · 합성 미리보기", recordedAt: 1789272000000, durationSeconds: 30, mimeType: "video/mp4", chatStatus: "stopped", indexState: "ready", syncQuality: "receive_time_approximate", manualOffsetSeconds: offset, warnings: [] } }),
  chatAt: async (_token, time, generation) => ({ ok: true, data: { generation, items: messages.filter((message) => message.mediaTimeSeconds + offset <= time).slice(-200), previousCursor: "older", nextCursor: null, indexState: "ready", syncQuality: "receive_time_approximate", warnings: [] } }),
  chatPage: async (_token, _cursor, generation) => ({ ok: true, data: { generation, items: messages.slice(0, 200), previousCursor: null, nextCursor: "newer", indexState: "ready", syncQuality: "receive_time_approximate", warnings: [] } }),
  chatSearch: async (_token, query, field, _cursor, generation) => ({ ok: true, data: { generation, items: messages.filter(message => (field !== "nickname" && message.text.includes(query)) || (field !== "body" && message.sender.includes(query))).slice(-200), previousCursor: null, nextCursor: null, indexState: "ready", syncQuality: "receive_time_approximate", warnings: [] } }),
  timeline: async () => ({ ok: true, data: { bucketSeconds: 2, indexState: "ready", viewerMetricStatus: "partial", buckets: Array.from({ length: 120 }, (_, index) => ({ startSeconds: index * 2, chatCount: Math.round(20 + 65 * Math.sin(index * .18) ** 2 + 100 * Math.exp(-(((index - 77) / 8) ** 2))), uniqueSenderCount: Math.round(8 + 16 * Math.sin(index * .18) ** 2 + 20 * Math.exp(-(((index - 77) / 8) ** 2))), viewerCount: index >= 40 && index < 45 ? null : Math.round(300 + 100 * Math.sin(index * .035) + index * 2) })) } }),
  setOffset: async (_token, value) => { offset = value; return { ok: true, data: value }; },
  close: async () => ({ ok: true, data: undefined }),
  // An explicitly provided generated fixture can be served by local Vite.
  mediaUrl: () => new URLSearchParams(window.location.search).get("synthetic") === "1" ? "/.runtime/chzzk-original-player/synthetic.mp4" : "",
};
createRoot(document.getElementById("root")!).render(<RecordingReplay recordingId="synthetic-only" runtime="browser-mock" privacyMode={false} liveRecording={false} api={api} onClose={() => {}} />);
