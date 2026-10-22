import type { BrowserRecording } from "../../api/officialBrowser";

export type RecordingGroup = {
  key: string;
  broadcastKey: string | null;
  latest: BrowserRecording;
  recordings: BrowserRecording[];
  active: boolean;
};

/** Titles can change during a broadcast; missing IDs must never group by title/channel alone. */
export function groupRecordings(recordings: readonly BrowserRecording[]): RecordingGroup[] {
  const groups = new Map<string, RecordingGroup>();
  for (const recording of recordings) {
    const broadcastKey = recording.broadcastKey?.trim() || null;
    const key = broadcastKey ? JSON.stringify([recording.channelId, broadcastKey]) : JSON.stringify([recording.id]);
    let group = groups.get(key);
    if (!group) {
      group = { key, broadcastKey, latest: recording, recordings: [], active: false };
      groups.set(key, group);
    }
    group.recordings.push(recording);
    if (recording.startedAt > group.latest.startedAt) group.latest = recording;
    group.active ||= recording.status === "recording";
  }
  for (const group of groups.values()) group.recordings.sort((a, b) => a.startedAt - b.startedAt);
  return [...groups.values()].sort((a, b) => b.latest.startedAt - a.latest.startedAt);
}
