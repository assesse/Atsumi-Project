import { invoke } from "@tauri-apps/api/core";

export type SoopChannel = {
  channelId: string;
  name: string;
  title: string | null;
  image: string | null;
  status: "live" | "offline" | "unknown";
  viewers: number | null;
};

export type SoopApi = {
  inspect(input: string): Promise<SoopChannel>;
  search?(request: SoopSearchRequest): Promise<SoopSearchPage>;
  home?(): Promise<SoopHomePage>;
  thumbnail?(key: string, videoId?: string): Promise<ArrayBuffer>;
};

export type SoopSearchKind = "all" | "live" | "video" | "channel";
export type SoopSearchRequest = { kind: SoopSearchKind; keyword: string; page: number; channelId?: string };
export type SoopSearchItem = { id: string; kind: Exclude<SoopSearchKind, "all">; title: string; channelId: string; channelName: string; thumbnailKey: string | null; views: number | null; duration: string | null; restricted: boolean; section?: "streamers" | "live" | "related" | "catch"; tags?: string[]; favoriteCount?: number | null };
export type SoopSearchPage = { items: SoopSearchItem[]; page: number; hasMore: boolean; warnings?: string[] };
export type SoopHomePage = { sections: { title: string; items: SoopSearchItem[] }[]; personalized: boolean; notice: string | null };

export function createSoopApi(runtime: "tauri" | "browser-mock"): SoopApi {
  return {
    home: () => runtime === "tauri" ? invoke<SoopHomePage>("soop_home") : Promise.reject(new Error("메인은 데스크톱 앱에서 사용할 수 있습니다.")),
    search: request => runtime === "tauri" ? invoke<SoopSearchPage>("soop_search", { request }) : Promise.reject(new Error("검색은 데스크톱 앱에서 사용할 수 있습니다.")),
    thumbnail: (key, videoId) => runtime === "tauri" ? invoke<ArrayBuffer>("soop_thumbnail", { key, videoId }) : Promise.reject(new Error("미리보기를 불러올 수 없습니다.")),
    inspect: (input) => runtime === "tauri"
      ? invoke<SoopChannel>("soop_channel_inspect", { input })
      : Promise.reject(new Error("채널 정보는 데스크톱 앱에서 확인할 수 있습니다.")),
  };
}
