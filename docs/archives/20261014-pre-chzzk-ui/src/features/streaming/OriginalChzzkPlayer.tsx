import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { ReplaySession, ReplayTimeline } from "../../api/replay";
import { replayMetricPaths } from "./RecordingReplayTimeline";

export const PLAYER_CHANNEL = "atsumi-replay-player-v1";
export type OriginalPlayerState = { time: number; duration: number; paused: boolean; seeking: boolean; aspect: number };
export type OriginalPlayerHandle = { seek(time: number): void; toggle(): void; mute(): void; pause(): void };
type Props = {
  runtime: "tauri" | "browser-mock"; mediaUrl: string; duration: number; privacy: boolean; timeline: ReplayTimeline | null; wide?: boolean; fullscreen?: boolean;
  recording?: { title: string; channelName?: string | null; recordedAt?: number; profileImage?: string | null };
  parts?: ReplaySession["parts"];
  onTail?(): void;
  onSourceReady?(url: string): void;
  onState(state: OriginalPlayerState): void; onError(message: string): void; onFullscreen(): void; onWide(): void; onClose(): void;
  onChannel?(): void;
};
export function validPlayerState(value: unknown): value is OriginalPlayerState {
  if (!value || typeof value !== "object") return false;
  const v = value as OriginalPlayerState;
  return Number.isFinite(v.time) && v.time >= 0 && v.time <= 604800 && Number.isFinite(v.duration) && v.duration >= 0 && v.duration <= 604800 && typeof v.paused === "boolean" && typeof v.seeking === "boolean" && Number.isFinite(v.aspect) && v.aspect >= .05 && v.aspect <= 20;
}
/** The original SDK runs in an opaque frame: no account cookies, IPC or parent DOM access. */
export const OriginalChzzkPlayer = forwardRef<OriginalPlayerHandle, Props>(function OriginalChzzkPlayer(props, ref) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [nonce] = useState(() => crypto.randomUUID().replaceAll("-", ""));
  const [ready, setReady] = useState(false);
  const readyRef = useRef(false);
  const current = useRef(props); current.current = props;
  const src = useMemo(() => `${props.runtime === "tauri" ? convertFileSrc("frame.html", "atsumi-player") : new URL("/original-player/frame.html", location.href).href}#${nonce}`, [props.runtime, nonce]);
  const send = useCallback((type: string, data?: unknown) => frame.current?.contentWindow?.postMessage({ channel: PLAYER_CHANNEL, nonce, type, data }, "*"), [nonce]);
  useImperativeHandle(ref, () => ({ seek: time => send("seek", time), toggle: () => send("toggle"), mute: () => send("mute"), pause: () => send("privacy", true) }), [send]);
  useEffect(() => {
    const ownedWindow = frame.current?.contentWindow;
    const timeout = setTimeout(() => { if (!readyRef.current) current.current.onError("원본 플레이어를 불러오지 못했습니다. 다시보기를 닫고 다시 열어 주세요."); }, 20000);
    const receive = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow || event.origin !== "null") return;
      const message = event.data;
      if (!message || message.channel !== PLAYER_CHANNEL || message.nonce !== nonce) return;
      switch (message.type) {
        case "ready": readyRef.current = true; clearTimeout(timeout); setReady(true); break;
        case "state": if (validPlayerState(message.data)) current.current.onState(message.data); break;
        case "error": current.current.onError("원본 플레이어에서 저장 영상을 열지 못했습니다. 녹화 파일과 재생 환경을 확인해 주세요."); break;
        case "fullscreen": current.current.onFullscreen(); break;
        case "wide": current.current.onWide(); break;
        case "close": current.current.onClose(); break;
        case "channel": if (!current.current.privacy) current.current.onChannel?.(); break;
        case "tail": if (!current.current.privacy) current.current.onTail?.(); break;
        case "source": if (message.data === current.current.mediaUrl) current.current.onSourceReady?.(message.data); break;
      }
    };
    window.addEventListener("message", receive);
    return () => { ownedWindow?.postMessage({ channel: PLAYER_CHANNEL, nonce, type: "dispose" }, "*"); clearTimeout(timeout); window.removeEventListener("message", receive); };
  }, [nonce, send]);
  useEffect(() => { if (ready) send("init", { url: props.mediaUrl, duration: props.duration, privacy: current.current.privacy, recording: current.current.recording, ...(props.parts?.length ? { parts: props.parts } : {}) }); }, [ready, props.mediaUrl, props.duration, props.parts, send]); // Metadata and privacy changes never reload the source.
  useEffect(() => { if (ready) send("metadata", props.recording); }, [ready, props.recording?.title, props.recording?.channelName, props.recording?.recordedAt, props.recording?.profileImage, send]);
  useEffect(() => { if (ready) send("privacy", props.privacy); }, [ready, props.privacy, send]);
  useEffect(() => { if (ready) send("presentation", { wide: !!props.wide, fullscreen: !!props.fullscreen }); }, [ready, props.wide, props.fullscreen, send]);
  useEffect(() => {
    if (!ready) return;
    const timeline = props.timeline;
    const participantCounts = timeline?.buckets.some(b => b.uniqueSenderCount != null) ?? false;
    send("metrics", timeline ? { ...timeline, buckets: timeline.buckets.slice(0, 2000), chatPaths: replayMetricPaths(timeline, props.duration, participantCounts ? "uniqueSenderCount" : "chatCount"), viewerPaths: replayMetricPaths(timeline, props.duration, "viewerCount"), participantCounts } : null);
  }, [ready, props.timeline, props.duration, send]);
  return <iframe ref={frame} className="recording-replay-original-player" title="CHZZK 원본 다시보기 플레이어" src={src}
    sandbox="allow-scripts" allow="autoplay; fullscreen; picture-in-picture" allowFullScreen referrerPolicy="no-referrer" />;
});
