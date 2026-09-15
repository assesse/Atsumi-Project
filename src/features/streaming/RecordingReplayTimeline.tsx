import { memo, useMemo, useState, type CSSProperties, type PointerEvent } from "react";
import type { ReplayTimeline } from "../../api/replay";
import { formatReplayTime } from "./RecordingReplayModel";

type Bucket = ReplayTimeline["buckets"][number];
type Metric = "uniqueSenderCount" | "viewerCount" | "chatCount";
type Point = { x: number; y: number };
const finiteCount = (value: number | null) => value !== null && Number.isFinite(value) && value >= 0;
const partialViewer = (bucket: Bucket, seconds: number, duration: number) => bucket.viewerCount != null && bucket.viewerCoverageSeconds != null && bucket.viewerCoverageSeconds < Math.min(seconds, duration - bucket.startSeconds) - .01;

/** Visual interpolation only: tooltips always use the original measured bucket. */
export function replayMetricPaths(timeline: ReplayTimeline, duration: number, metric: Metric): string[] {
  if (!Number.isFinite(duration) || duration <= 0 || !Number.isFinite(timeline.bucketSeconds) || timeline.bucketSeconds <= 0) return [];
  // The backend bounds responses to 2,000 buckets; keep the entire recording.
  const buckets = timeline.buckets.slice(0, 2000).filter(b => Number.isFinite(b.startSeconds) && b.startSeconds >= 0 && b.startSeconds < duration);
  const peak = Math.max(1, ...buckets.map(b => finiteCount(b[metric]) ? b[metric]! : 0));
  const continuous = (bucket: Bucket) => finiteCount(bucket[metric]) && !(metric === "viewerCount" && partialViewer(bucket, timeline.bucketSeconds, duration));
  const paths: string[] = [];
  let run: Point[] = [];
  const flush = () => {
    if (!run.length) return;
    const points = run;
    const first = points[0]!, last = points[points.length - 1]!;
    let path = `M${first.x.toFixed(2)},${first.y.toFixed(2)}`;
    for (let i = 1; i < points.length; i++) {
      const previous = points[i - 1]!, point = points[i]!;
      const middle = ((previous.x + point.x) / 2).toFixed(2);
      // Monotone horizontal controls cannot overshoot measured peaks or negatives.
      path += ` C${middle},${previous.y.toFixed(2)} ${middle},${point.y.toFixed(2)} ${point.x.toFixed(2)},${point.y.toFixed(2)}`;
    }
    paths.push(`${path} L${last.x.toFixed(2)},32 L${first.x.toFixed(2)},32 Z`);
    run = [];
  };
  let previousEnd = 0;
  for (let index = 0; index < buckets.length; index++) {
    const bucket = buckets[index]!;
    if (!continuous(bucket) || bucket.startSeconds < previousEnd - .001) { flush(); continue; }
    if (run.length && bucket.startSeconds > previousEnd + .001) flush();
    const start = bucket.startSeconds / duration * 1000;
    const end = Math.min(duration, bucket.startSeconds + timeline.bucketSeconds) / duration * 1000;
    const y = 32 - bucket[metric]! / peak * 28;
    if (!run.length) run.push({ x: start, y });
    run.push({ x: (start + end) / 2, y });
    previousEnd = Math.min(duration, bucket.startSeconds + timeline.bucketSeconds);
    const next = buckets[index + 1];
    if (!next || !continuous(next) || next.startSeconds > previousEnd + .001) {
      run.push({ x: end, y }); flush();
    }
  }
  flush();
  return paths;
}

export function replayBucketAt(timeline: ReplayTimeline | null, seconds: number): Bucket | undefined {
  return timeline?.buckets.find(bucket => seconds >= bucket.startSeconds && seconds < bucket.startSeconds + timeline.bucketSeconds);
}

export const RecordingReplayTimeline = memo(function RecordingReplayTimeline({ timeline, time, duration, onSeek }: {
  timeline: ReplayTimeline | null; time: number; duration: number; onSeek: (time: number) => void;
}) {
  const [hoverTime, setHoverTime] = useState<number | null>(null);
  const participantCountsAvailable = timeline?.buckets.some(bucket => finiteCount(bucket.uniqueSenderCount)) ?? false;
  const paths = useMemo(() => timeline ? {
    chat: replayMetricPaths(timeline, duration, participantCountsAvailable ? "uniqueSenderCount" : "chatCount"), viewers: replayMetricPaths(timeline, duration, "viewerCount"),
  } : { chat: [], viewers: [] }, [timeline, duration, participantCountsAvailable]);
  const partialViewers = useMemo(() => {
    if (!timeline || duration <= 0) return [];
    const buckets = timeline.buckets.slice(0, 2000);
    const peak = Math.max(1, ...buckets.map(bucket => finiteCount(bucket.viewerCount) ? bucket.viewerCount! : 0));
    return buckets.filter(bucket => bucket.startSeconds < duration && finiteCount(bucket.viewerCount) && partialViewer(bucket, timeline.bucketSeconds, duration)).map(bucket => ({
      x: (bucket.startSeconds + Math.min(timeline.bucketSeconds, duration - bucket.startSeconds) / 2) / duration * 1000,
      y: 32 - bucket.viewerCount! / peak * 28,
    }));
  }, [timeline, duration]);
  const shownTime = hoverTime ?? time;
  const bucket = replayBucketAt(timeline, Math.min(Math.max(0, duration - .001), shownTime));
  const pointer = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width > 0) setHoverTime(Math.min(duration, Math.max(0, (event.clientX - rect.left) / rect.width * duration)));
  };
  const count = (value: number | null | undefined) => value == null ? "기록 없음" : `${Math.round(value).toLocaleString("ko-KR")}명`;
  const percentage = duration > 0 ? Math.max(0, Math.min(100, time / duration * 100)) : 0;
  return <div className="recording-replay-timeline" onPointerMove={pointer} onPointerLeave={() => setHoverTime(null)}>
    <svg className="recording-replay-activity" data-chat-metric={participantCountsAvailable ? "participants" : "messages"} viewBox="0 0 1000 32" preserveAspectRatio="none" aria-hidden="true">
      {paths.viewers.map((path, index) => <path className="replay-viewer-curve" key={`v${index}`} d={path} />)}
      {paths.chat.map((path, index) => <path className="replay-chat-curve" key={`c${index}`} d={path} />)}
      {partialViewers.map((point, index) => <circle className="replay-viewer-partial" key={`p${index}`} cx={point.x} cy={point.y} r="2" />)}
    </svg>
    <div className="recording-replay-timeline-tooltip" role="tooltip" style={{ "--preview-position": `${Math.max(0, Math.min(100, duration > 0 ? shownTime / duration * 100 : 0))}%` } as CSSProperties}>
      <strong>{formatReplayTime(shownTime)}</strong>
      {timeline ? <><span className="replay-chat-count">채팅 참여자 {count(bucket?.uniqueSenderCount)}</span><span className="replay-viewer-count">평균 시청자 {count(bucket?.viewerCount)}{bucket?.viewerCount != null && bucket.viewerCoverageSeconds != null && bucket.viewerCoverageSeconds < Math.min(timeline.bucketSeconds, duration - bucket.startSeconds) - .01 ? " · 일부 기록" : ""}</span><small>{timeline.bucketSeconds}초 구간 · 채팅 {bucket?.chatCount?.toLocaleString("ko-KR") ?? "기록 없음"}{bucket ? "개" : ""}{participantCountsAvailable ? "" : " · 곡선: 채팅 수"}<br />곡선 높이는 각 지표의 최대값 기준</small></> : null}
    </div>
    <input className="recording-replay-seek" type="range" aria-label="영상 재생 위치" min={0} max={duration || 1} step={0.1}
      value={Math.min(time, duration || 1)} disabled={!duration} style={{ "--played": `${percentage}%` } as CSSProperties}
      onChange={event => onSeek(Number(event.target.value))} onFocus={() => setHoverTime(null)}
      aria-valuetext={`${formatReplayTime(time)} / ${formatReplayTime(duration)}`} />
  </div>;
});
