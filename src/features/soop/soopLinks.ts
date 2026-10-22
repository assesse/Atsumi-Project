const CHANNEL_ID = /^[a-z0-9_-]{1,40}$/i;
const CHANNEL_HOSTS = new Set([
  "play.sooplive.com", "play.sooplive.co.kr", "play.afreecatv.com",
  "ch.sooplive.com", "ch.sooplive.co.kr", "bj.sooplive.co.kr", "bj.afreecatv.com",
  "www.sooplive.com", "www.sooplive.co.kr",
]);

/** Only channel/player links are accepted; arbitrary URLs never reach the backend. */
export function normalizeSoopChannel(input: string): string | null {
  const value = input.trim();
  if (CHANNEL_ID.test(value)) return value.toLowerCase();
  if (value.length > 2048) return null;
  try {
    const url = new URL(/^https?:\/\//i.test(value) ? value : `https://${value}`);
    if (!CHANNEL_HOSTS.has(url.hostname.toLowerCase()) || !["http:", "https:"].includes(url.protocol)
      || url.username || url.password || url.port) return null;
    let parts = url.pathname.split("/").filter(Boolean);
    if (url.hostname.startsWith("www.")) {
      if (parts[0] !== "station" || (parts.length !== 2 && !(parts.length === 3 && parts[2] === "board"))) return null;
      parts = parts.slice(1, 2);
    }
    const id = parts[0];
    if (!id || !CHANNEL_ID.test(id)) return null;
    if (url.hostname.startsWith("play.")) {
      if (parts.length > 3 || (parts[1] && parts[1] !== "embed" && !/^\d{1,20}$/.test(parts[1]))
        || (parts[2] && (parts[2] !== "embed" || parts[1] === "embed"))) return null;
    } else if (parts.length !== 1) return null;
    return id.toLowerCase();
  } catch { return null; }
}

export const soopPlayerUrl = (channelId: string) => {
  const channel = normalizeSoopChannel(channelId);
  if (!channel) throw new Error("SOOP 채널 ID를 확인해 주세요.");
  return `https://play.sooplive.com/${channel}/embed`;
};

export const soopChannelUrl = (channelId: string) => {
  const channel = normalizeSoopChannel(channelId);
  if (!channel) throw new Error("SOOP 채널 ID를 확인해 주세요.");
  return `https://play.sooplive.com/${channel}`;
};
