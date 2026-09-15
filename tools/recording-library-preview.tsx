import { useState } from "react";
import { createRoot } from "react-dom/client";
import type { BrowserRecording } from "../src/api/officialBrowser";
import { RecordingLibrary } from "../src/features/streaming/RecordingLibrary";
import "../src/styles.css";
import "../src/features/streaming/StreamingWorkspace.css";
import "../src/features/streaming/OfficialBrowserPanel.css";

// Synthetic metadata only: no native commands, recording paths or media loads.
const titles = ["여름 산책 · 함께 보는 작은 이야기", "밤하늘 도서관 | 새로운 모험을 시작합니다", "주말 게임 방송 — 아주 긴 제목에서도 카드와 선택한 녹화의 레이아웃을 확인합니다", "비 오는 오후의 조용한 음악", "친구들과 마지막 라운드", "다시 보고 싶은 장면들"];
const records: BrowserRecording[] = Array.from({ length: 33 }, (_, index) => {
  const status = index === 0 ? "recording" : index % 8 === 0 ? "interrupted" : index % 7 === 0 ? "failed" : "stopped";
  const complete = status === "stopped";
  const storedBytes = Math.floor(1024 ** 3 * (1 + index * .37));
  return { id: `synthetic-${index}`, channelId: (index % 2 ? "a" : "b").repeat(32), title: titles[index % titles.length]!, startedAt: Date.UTC(2026, 8, 12, 10, 30) - index * 3600000, updatedAt: Date.UTC(2026, 8, 12, 11, 30), status, mimeType: "video/webm", outputDir: "", segmentCount: 2, bytesWritten: storedBytes, durationSeconds: 5000 + index * 750, lastError: status === "failed" ? "합성 예시: 저장 장치 연결을 확인해 주세요." : null, captureChat: true, chatStatus: index % 7 === 0 ? "partial" : "stopped", chatCount: 4200 + index * 351, segments: [{ index: 0, file: "synthetic-0.webm", bytes: 1000, durationSeconds: 20 }, { index: 1, file: "synthetic-1.webm", bytes: 1000, durationSeconds: 20 }], merge: complete ? { status: "complete", segmentCount: 2, updatedAt: 1, file: `merged-${"c".repeat(32)}.webm`, timelineFile: `merged-${"c".repeat(32)}.timeline.jsonl`, bytes: storedBytes, durationSeconds: 5000 + index * 750 } : undefined };
});

function Preview() {
  const [recordings, setRecordings] = useState(records);
  const [selected, setSelected] = useState<string | null>("synthetic-1");
  const [notice, setNotice] = useState("");
  const noop = () => setNotice("합성 미리보기 — 실제 파일이나 플레이어를 열지 않았습니다.");
  return <div style={{ display: "grid", gridTemplateColumns: "208px minmax(0, 1fr)", height: "100%" }}>
    <aside style={{ padding: 24, background: "var(--rail)", borderRight: "1px solid var(--line)" }}><strong style={{ fontSize: 22 }}>Atsumi</strong><p style={{ marginTop: 48, color: "var(--muted)", fontSize: 12 }}>라이브 시청·녹화</p><p style={{ padding: "12px 0", color: "var(--primary)", fontSize: 14 }}>녹화 보관함</p><small style={{ color: "var(--muted)", fontSize: 10 }}>합성 데이터 미리보기</small></aside>
    <main className="streaming-workspace is-official-view"><section className="official-browser-panel"><RecordingLibrary recordings={recordings} selectedId={selected} onSelect={setSelected} disabled={false} privacy={false} retrying={false} opening={false} onFolder={noop} onReplay={noop} onOpenMerged={noop} onRetryMerge={noop} onOpenSegment={noop} onDelete={async (ids) => { setRecordings(previous => previous.filter(item => !ids.includes(item.id))); return { deletedIds: ids, failures: [] }; }} stopControl={<button type="button" className="official-browser-stop" onClick={noop}>녹화 중지</button>} /></section><p role="status">{notice}</p></main>
  </div>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
