import { invoke } from "@tauri-apps/api/core";
import { parseYouTubeTarget, type YouTubeMetadata, type YouTubeTarget } from "./model";

export interface YouTubeApi {
  metadata(target: YouTubeTarget, signal?: AbortSignal): Promise<YouTubeMetadata>;
  openExternal(url: string): Promise<void>;
  search?(request: YouTubeSearchRequest): Promise<YouTubeSearchPage>;
  thumbnail?(id: string): Promise<string | null>;
}
export type YouTubeSearchItem = YouTubeMetadata & { duration?: string; views?: string; published?: string; isLive?: boolean };
export interface YouTubeSearchRequest { query: string; filter: 'all' | 'video' | 'channel' | 'live'; sort: 'relevance' | 'popular'; channel?: string; cursor?: string }
export interface YouTubeSearchPage { items: YouTubeSearchItem[]; cursor?: string | null }
export function createYouTubeApi(runtime: "tauri" | "browser-mock"): YouTubeApi {
  return {
    async search(request) {
      if (runtime !== 'tauri') throw new Error('검색은 Atsumi 앱에서 사용할 수 있습니다.');
      return invoke<YouTubeSearchPage>('youtube_search', { request });
    },
    async thumbnail(id) {
      if (runtime !== 'tauri') return null;
      return invoke<string | null>('youtube_thumbnail', { id });
    },
    async metadata(target, signal) {
      if (runtime === "tauri") return invoke<YouTubeMetadata>("youtube_metadata", { input: target.url });
      if (target.kind !== "video") return { ...target, title: target.kind === "channel" ? target.id.replace(/^@/, "") : "재생목록", author: "" };
      const response = await fetch(`https://www.youtube.com/oembed?url=${encodeURIComponent(target.url)}&format=json`, { signal, credentials: "omit", referrerPolicy: "strict-origin-when-cross-origin" });
      if (!response.ok) throw new Error("영상 정보를 가져오지 못했습니다. 공개 영상인지 확인해 주세요.");
      const data: unknown = await response.json();
      if (!data || typeof data !== "object") throw new Error("영상 정보를 확인하지 못했습니다.");
      const value = data as Record<string, unknown>;
      const authorTarget = typeof value.author_url === "string" ? parseYouTubeTarget(value.author_url) : null;
      return { ...target, title: typeof value.title === "string" ? value.title.slice(0, 300) : target.id, author: typeof value.author_name === "string" ? value.author_name.slice(0, 160) : "", authorUrl: authorTarget?.kind === "channel" ? authorTarget.url : undefined };
    },
    async openExternal(url) {
      const target = parseYouTubeTarget(url);
      if (!target) throw new Error("YouTube 주소를 확인해 주세요.");
      if (runtime === "tauri") await invoke("youtube_open_external", { input: target.url });
      else window.open(target.url, "_blank", "noopener,noreferrer");
    },
  };
}
