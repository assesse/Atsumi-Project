export type YouTubeTarget = { kind: "video" | "playlist" | "channel"; id: string; url: string; startSeconds?: number };
export type YouTubeMetadata = YouTubeTarget & { title: string; author: string; authorUrl?: string; thumbnail?: string; warning?: string; isLive?: boolean };
export type YouTubeFavorite = YouTubeMetadata & { savedAt: number };
export const YOUTUBE_FAVORITES_KEY = "atsumi.youtube.favorites.v1";
const videoId = /^[\w-]{11}$/;
const playlistId = /^[\w-]{10,128}$/;
const hosts = new Set(["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "www.youtu.be", "www.youtube-nocookie.com"]);
const channelPath = /^\/(?:channel\/[\w-]{24}|@[\p{L}\p{N}_.-]{1,100}|(?:c|user)\/[\w.-]{1,100})\/?$/u;
const channelEntryPath = /^\/(channel\/UC[\w-]{22}|@[\p{L}\p{N}_.-]{1,100})\/(?:live|streams)\/?$/u;
function startTime(value: string | null): number | undefined {
  if (!value) return undefined;
  const plain = /^\d+s?$/.test(value) ? Number(value.replace(/s$/, "")) : NaN;
  const parts = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/.exec(value);
  const seconds = Number.isFinite(plain) ? plain : parts ? Number(parts[1] ?? 0) * 3600 + Number(parts[2] ?? 0) * 60 + Number(parts[3] ?? 0) : 0;
  return seconds > 0 && seconds <= 604800 ? Math.floor(seconds) : undefined;
}
export function parseYouTubeTarget(input: string): YouTubeTarget | null {
  const value = input.trim();
  if (videoId.test(value)) return { kind: "video", id: value, url: `https://www.youtube.com/watch?v=${value}` };
  let parsed: URL;
  try { parsed = new URL(/^https?:\/\//i.test(value) ? value : `https://${value}`); } catch { return null; }
  if (!["https:", "http:"].includes(parsed.protocol) || !hosts.has(parsed.hostname) || parsed.port || parsed.username || parsed.password) return null;
  let path: string;
  try { path = decodeURIComponent(parsed.pathname); } catch { return null; }
  const id = parsed.hostname.endsWith("youtu.be") ? path.slice(1).split("/")[0] : path === "/watch" ? parsed.searchParams.get("v") : /^\/(?:shorts|live|embed)\//.test(path) ? path.split("/")[2] : null;
  if (id && videoId.test(id)) {
    const seconds = startTime(parsed.searchParams.get("t") ?? parsed.searchParams.get("start"));
    return { kind: "video", id, url: `https://www.youtube.com/watch?v=${id}`, ...(seconds ? { startSeconds: seconds } : {}) };
  }
  const list = parsed.searchParams.get("list");
  if ((path === "/playlist" || path === "/embed/videoseries") && list && playlistId.test(list)) return { kind: "playlist", id: list, url: `https://www.youtube.com/playlist?list=${list}` };
  // A channel's current-live entry is still a channel favorite, not the video
  // currently selected by YouTube. Do not accept URL-normalized separators.
  const channelEntry = !value.includes("\\") && !/%2f|%5c/i.test(parsed.pathname) ? channelEntryPath.exec(path) : null;
  if (channelEntry) return { kind: "channel", id: channelEntry[1]!, url: `https://www.youtube.com/${channelEntry[1]}` };
  if (channelPath.test(path)) { const clean = path.replace(/\/$/, ""); return { kind: "channel", id: clean.slice(1), url: `https://www.youtube.com${clean}` }; }
  return null;
}
export const targetKey = (target: YouTubeTarget) => `${target.kind}:${target.id}`;
export function youTubeEmbedUrl(target: YouTubeTarget, origin: string): string | null {
  if (target.kind === "channel") return null;
  const base = target.kind === "video" ? `https://www.youtube.com/embed/${target.id}` : "https://www.youtube.com/embed/videoseries";
  const url = new URL(base);
  url.searchParams.set("enablejsapi", "1"); url.searchParams.set("playsinline", "1"); url.searchParams.set("autoplay", "1"); url.searchParams.set("rel", "0");
  if (target.kind === "playlist") url.searchParams.set("list", target.id);
  if (target.startSeconds) url.searchParams.set("start", String(target.startSeconds));
  if (/^https?:\/\//.test(origin)) url.searchParams.set("origin", origin);
  return url.href;
}
const playerErrors: Record<number, string> = {
  2: "영상 주소를 확인해 주세요.", 5: "이 환경에서 영상을 재생하지 못했습니다.",
  100: "삭제되었거나 비공개인 영상입니다.", 101: "이 영상은 YouTube에서만 볼 수 있습니다.",
  150: "이 영상은 YouTube에서만 볼 수 있습니다.", 153: "YouTube가 앱의 재생 요청을 허용하지 않았습니다. YouTube에서 열어 주세요.",
};
export const playerErrorMessage = (code: number) => playerErrors[code] ?? "영상을 불러오지 못했습니다. YouTube에서 확인해 주세요.";
export function readYouTubeFavorites(storage: Pick<Storage, "getItem">): YouTubeFavorite[] {
  try {
    const raw: unknown = JSON.parse(storage.getItem(YOUTUBE_FAVORITES_KEY) ?? "[]");
    if (!Array.isArray(raw)) return [];
    const seen = new Set<string>();
    return raw.slice(0, 100).flatMap((item: unknown): YouTubeFavorite[] => {
      if (!item || typeof item !== "object") return [];
      const saved = item as Record<string, unknown>, target = typeof saved.url === "string" ? parseYouTubeTarget(saved.url) : null;
      if (!target || seen.has(targetKey(target))) return [];
      seen.add(targetKey(target));
      const authorTarget = typeof saved.authorUrl === "string" ? parseYouTubeTarget(saved.authorUrl) : null;
      return [{ ...target, title: typeof saved.title === "string" ? saved.title.slice(0, 300) : target.id, author: typeof saved.author === "string" ? saved.author.slice(0, 160) : "", authorUrl: authorTarget?.kind === "channel" ? authorTarget.url : undefined, ...(saved.isLive === true ? { isLive: true } : {}), savedAt: typeof saved.savedAt === "number" && Number.isFinite(saved.savedAt) ? saved.savedAt : 0 }];
    });
  } catch { return []; }
}
export function saveYouTubeFavorites(storage: Pick<Storage, "setItem">, favorites: YouTubeFavorite[]): void {
  // Do not persist remote URLs, image data, or transient errors in preferences.
  storage.setItem(YOUTUBE_FAVORITES_KEY, JSON.stringify(favorites.slice(0, 100).map(({ thumbnail: _thumbnail, warning: _warning, ...favorite }) => favorite)));
}
