import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SideRail } from "./SideRail";
import { RecordingLibrary } from "../features/streaming/RecordingLibrary";
import type { BrowserRecording } from "../api/officialBrowser";
import { auditLayouts, layoutBrowser } from "../test/offlineLayoutBrowser";
import appCss from "../styles.css?raw";
import inkCss from "../atsumi-ink.css?raw";
import streamingCss from "../features/streaming/StreamingWorkspace.css?raw";
import panelCss from "../features/streaming/OfficialBrowserPanel.css?raw";
import libraryCss from "../features/streaming/RecordingLibrary.css?raw";
import profileCss from "../features/streaming/RecordedChannel.css?raw";

const noop = () => {};
const records: BrowserRecording[] = [0, 1, 2].map(index => ({
  id: `fixture-${index}`, channelId: "fixture-channel", title: index === 2 ? "주말 음악 방송" : "방송 제목이 변경되어도 같은 방송 ID로 묶이는 녹화",
  broadcastKey: index === 2 ? "id:2" : "id:1", startedAt: 1791200000000 + index * 1000, updatedAt: 1791200001000,
  status: index === 1 ? "interrupted" : "stopped", mimeType: "video/mp4", outputDir: "", segmentCount: 1,
  bytesWritten: 1024 ** 3, durationSeconds: 5400, segments: [], lastError: null,
  merge: { status: "complete", segmentCount: 1, updatedAt: 1, bytes: 1024 ** 3, durationSeconds: 5400,
    file: `merged-${"a".repeat(32)}.mp4`, timelineFile: `merged-${"a".repeat(32)}.timeline.jsonl` },
}));
const scenes = [false, true].flatMap(collapsed => (["hitomi", "chzzk"] as const).map(source => ({
  name: `${source}-${collapsed ? "collapsed" : "expanded"}`,
  html: renderToStaticMarkup(<div id="root"><div className={`app-shell streaming-shell${collapsed ? " sidebar-collapsed" : ""}`}>
    <SideRail source={source} view={source === "chzzk" ? "recordings" : "explore"} collapsed={collapsed} autoFindCount={0} attentionCount={0}
      privacyMode onPrivacyModeToggle={noop} onSettings={noop} onNavigate={noop} onSourceChange={noop} onToggle={noop} />
    <main className="streaming-workspace">{source === "chzzk" ? <RecordingLibrary recordings={records} selectedId="fixture-0" disabled={false} privacy={false}
      retrying={false} openingFolder={false} onSelect={noop} onFolder={noop} onReplay={noop} onRetryMerge={noop} onDelete={async () => ({ deletedIds: [], failures: [] })} />
      : <h1>갤러리 탐색</h1>}</main>
  </div></div>),
})));

describe.skipIf(!layoutBrowser)("sidebar boundary and grouped recording layout", () => {
  it.each([[920, 650], [1440, 900]])("keeps controls inside the viewport at %ix%i in both rail states", async (width, height) => {
    const results = await auditLayouts(scenes, appCss + inkCss + streamingCss + panelCss + libraryCss + profileCss, width, height, `sidebar-recordings-${width}`);
    expect(results.flatMap(result => result.issues.map(issue => `${result.name}: ${issue}`))).toEqual([]);
  }, 60000);
});
