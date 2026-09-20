import { describe, expect, it, vi } from "vitest";
import source from "../../../public/original-player/recording-source.js?raw";

type Part = { index: number; startSeconds: number; durationSeconds: number; url: string };
type Snapshot = { url: string; duration: number; parts: Part[] };
type Adapter = { update(next: Snapshot): void; dispose(): void };
const { recordingSource, attachRecordingSource } = new Function(source.replaceAll("export function", "function") + ";return {recordingSource,attachRecordingSource};")() as {
  recordingSource(data: unknown, allowed: (url: string) => boolean): Snapshot | null;
  attachRecordingSource(video: FakeMedia, source: Snapshot, onTail: () => void): Adapter;
};
const snapshot = (count = 2, url = "local") => recordingSource({ url, duration: count * 30, parts: Array.from({ length: count }, (_, index) => ({ index, startSeconds: index * 30, durationSeconds: 30 })) }, () => true)!;
class FakeMedia extends EventTarget {
  clock = 0; isPaused = true; isEnded = false; readyState = 4; src = "local/part/0";
  volume = .35; muted = true; playbackRate = 1.5;
  get currentTime() { return this.clock; } set currentTime(n: number) { this.clock = n; }
  get duration() { return 30; }
  get buffered() { return { length: 1, start: (_index: number) => 0, end: (_index: number) => 25 }; }
  get seekable() { return this.buffered; }
  get seeking() { return false; }
  get ended() { return this.isEnded; }
  get paused() { return this.isPaused; }
  async play() { this.isPaused = false; this.dispatchEvent(new Event("play")); }
  pause() { this.isPaused = true; this.dispatchEvent(new Event("pause")); }
  load() { this.clock = 0; this.readyState = 0; this.isEnded = false; }
  loaded() { this.readyState = 4; this.dispatchEvent(new Event("loadedmetadata")); }
  finish() { this.clock = 30; this.isPaused = true; this.isEnded = true; this.dispatchEvent(new Event("ended")); }
}
const fixture = () => { const video = new FakeMedia(), tail = vi.fn(); const adapter = attachRecordingSource(video, snapshot(), tail); return { video, tail, adapter }; };

describe("single recording source for the original player", () => {
  it("accepts a complete file or an ordered contiguous snapshot, never arbitrary part URLs", () => {
    const one = recordingSource({ url: "owned", duration: 30 }, url => url === "owned");
    expect(one?.parts).toEqual([{ index: 0, startSeconds: 0, durationSeconds: 30, url: "owned" }]);
    const p = snapshot().parts.map(({ url: _url, ...value }) => value);
    const valid = { url: "owned", duration: 60, parts: p };
    for (const data of [null, { ...valid, duration: Infinity }, { ...valid, duration: 59 }, { ...valid, parts: [] }, { ...valid, parts: [p[1], p[0]] }, { ...valid, parts: [p[0], { ...p[1], startSeconds: 31 }] }, { ...valid, parts: [{ ...p[0], index: -1 }, p[1]] }]) expect(recordingSource(data, () => true)).toBeNull();
    expect(recordingSource(valid, () => false)).toBeNull();
    expect(recordingSource({ ...valid, parts: [{ ...p[0], url: "evil" }, p[1]] }, () => true)?.parts[0]?.url).toBe("owned/part/0");
  });
  it("maps original seekbar and keyboard seeks to global time, preserving paused state", () => {
    const { video } = fixture(); video.currentTime = 45;
    expect(video.src).toBe("local/part/1"); expect(video.currentTime).toBe(45); expect(video.seeking).toBe(true);
    video.loaded(); expect(video.clock).toBe(15); expect(video.currentTime).toBe(45); expect(video.paused).toBe(true);
    video.currentTime = 5; video.loaded(); expect(video.src).toBe("local/part/0"); expect(video.clock).toBe(5);
    expect(video.duration).toBe(60); expect(video.buffered.end(0)).toBe(25);
    expect(video.seekable.end(0)).toBe(60);
  });
  it("advances at the boundary without an ended event or resetting playback settings", async () => {
    const { video, tail } = fixture(), ended = vi.fn(); video.addEventListener("ended", ended);
    await video.play(); video.finish(); video.loaded();
    expect(video.src).toBe("local/part/1"); expect(video.currentTime).toBe(30); expect(video.paused).toBe(false);
    expect(ended).not.toHaveBeenCalled(); expect(tail).not.toHaveBeenCalled();
    expect([video.volume, video.muted, video.playbackRate]).toEqual([.35, true, 1.5]);
    expect(video.buffered.start(0)).toBe(30); expect(video.buffered.end(0)).toBe(55);
    video.finish(); expect(ended).toHaveBeenCalledOnce(); expect(tail).toHaveBeenCalledOnce();
  });
  it("keeps global position through appended ranges and the final single-file transition", async () => {
    const { video, adapter } = fixture(); await video.play(); video.currentTime = 45; video.loaded();
    adapter.update(snapshot(3, "extended")); expect(video.currentTime).toBe(45); video.loaded();
    expect(video.clock).toBe(15); expect(video.duration).toBe(90); expect(video.paused).toBe(false);
    video.pause(); video.currentTime = 22;
    adapter.update(recordingSource({ url: "completed", duration: 30 }, () => true)!); video.loaded();
    expect(video.src).toBe("completed"); expect(video.clock).toBe(22); expect(video.paused).toBe(true);
  });
  it("honors a pause/privacy request made during a source switch", async () => {
    const { video } = fixture(); await video.play(); video.currentTime = 45; video.pause(); video.loaded();
    expect(video.paused).toBe(true); expect(video.currentTime).toBe(45);
  });
  it("keeps only the latest rapid seek and ignores disposed callbacks", () => {
    const { video, adapter, tail } = fixture(); video.currentTime = 40; video.currentTime = 5; video.currentTime = 35; video.loaded();
    expect(video.clock).toBe(5); expect(video.currentTime).toBe(35);
    adapter.dispose(); video.finish(); expect(tail).not.toHaveBeenCalled();
    adapter.update(snapshot(3, "late")); expect(video.src).toBe("local/part/1");
  });
});
