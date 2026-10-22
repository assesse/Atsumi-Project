import { describe, expect, it } from "vitest";
import { parseYouTubeTarget, playerErrorMessage, readYouTubeFavorites, saveYouTubeFavorites, youTubeEmbedUrl, YOUTUBE_FAVORITES_KEY } from "./model";

describe("YouTube link model", () => {
  it.each(["M7lc1UVf-VE", "https://www.youtube.com/watch?v=M7lc1UVf-VE&si=tracker", "https://youtu.be/M7lc1UVf-VE", "youtube.com/shorts/M7lc1UVf-VE", "https://m.youtube.com/live/M7lc1UVf-VE", "https://www.youtube-nocookie.com/embed/M7lc1UVf-VE"])("normalizes %s", value => {
    expect(parseYouTubeTarget(value)).toEqual({ kind: "video", id: "M7lc1UVf-VE", url: "https://www.youtube.com/watch?v=M7lc1UVf-VE" });
  });
  it.each(["https://www.youtube.com.evil.com/watch?v=M7lc1UVf-VE", "https://user@youtube.com/watch?v=M7lc1UVf-VE", "https://youtube.com:444/watch?v=M7lc1UVf-VE", "javascript:alert(1)", "file:///C:/Users", "https://youtube.com/redirect?q=https://evil.com", "https://youtube.com/watch?v=short", "https://evil.com/M7lc1UVf-VE"])("rejects %s", value => expect(parseYouTubeTarget(value)).toBeNull());
  it("keeps timestamps without tracking and embeds with the actual client origin", () => {
    const target = parseYouTubeTarget("https://youtu.be/M7lc1UVf-VE?t=1h2m3s&si=tracking")!;
    expect(target.startSeconds).toBe(3723);
    const url = new URL(youTubeEmbedUrl(target, "http://tauri.localhost")!);
    expect(url.origin).toBe("https://www.youtube.com"); expect(url.searchParams.get("origin")).toBe("http://tauri.localhost"); expect(url.searchParams.get("start")).toBe("3723"); expect(url.searchParams.has("si")).toBe(false);
  });
  it("supports local channel bookmarks and playlists", () => {
    expect(parseYouTubeTarget("https://youtube.com/@%EC%9C%A4%ED%82%A4%ED%82%A4")).toMatchObject({ kind: "channel", id: "@윤키키" });
    const list = parseYouTubeTarget("https://youtube.com/playlist?list=PL590L5WQmH8fJ54F369BLDSqIwcs-TCfs")!;
    expect(list.kind).toBe("playlist"); expect(youTubeEmbedUrl(list, location.origin)).toContain("list=PL590L5WQmH8fJ54F369BLDSqIwcs-TCfs");
    expect(youTubeEmbedUrl(parseYouTubeTarget("https://youtube.com/@GoogleDevelopers")!, location.origin)).toBeNull();
  });
  it.each(['live', 'streams'])('normalizes the exact channel %s entry to its channel, not a video', suffix => {
    for (const base of ['channel/UC83RKJs4eKHVE9v0YUyqzgg', '@KittenAcademy', '@%EC%9C%A4%ED%82%A4%ED%82%A4']) {
      const channel = parseYouTubeTarget(`https://www.youtube.com/${base}`);
      expect(channel?.kind).toBe('channel');
      expect(parseYouTubeTarget(`https://www.youtube.com/${base}/${suffix}`)).toEqual(channel);
      expect(parseYouTubeTarget(`https://www.youtube.com/${base}/${suffix}/?si=share`)).toEqual(channel);
    }
  });
  it.each([
    'https://www.youtube.com/@example/live/more', 'https://www.youtube.com/@example/streams/more',
    'https://www.youtube.com/@example/videos', 'https://www.youtube.com/@example/livestream',
    'https://www.youtube.com/channel/AB83RKJs4eKHVE9v0YUyqzgg/live', 'https://www.youtube.com/channel/UCshort/live',
    'https://www.youtube.com/c/example/live', 'https://www.youtube.com/user/example/streams',
    'https://www.youtube.com/@example%2Flive', 'https://www.youtube.com/@example%5Clive',
    'https://www.youtube.com/@example\\live', 'https://www.youtube.com.evil.test/@example/live',
    'https://name@www.youtube.com/@example/live', 'https://www.youtube.com:444/@example/live',
  ])('rejects unsupported or ambiguous channel entry %s', value => expect(parseYouTubeTarget(value)).toBeNull());
  it('preserves saved video and channel kinds without writing or migrating favorites on read', () => {
    const stored = JSON.stringify([
      { kind: 'video', id: 'M7lc1UVf-VE', url: 'https://www.youtube.com/watch?v=M7lc1UVf-VE', title: 'Saved video', author: 'Channel', isLive: true, savedAt: 10 },
      { kind: 'channel', id: '@example', url: 'https://www.youtube.com/@example', title: 'Saved channel', author: '', savedAt: 20 },
    ]);
    const storage = { getItem: () => stored };
    expect(readYouTubeFavorites(storage).map(item => [item.kind, item.id, item.savedAt])).toEqual([['video', 'M7lc1UVf-VE', 10], ['channel', '@example', 20]]);
    expect(storage.getItem()).toBe(stored);
  });
  it("does not persist thumbnails or trust malformed saved URLs", () => {
    localStorage.clear(); const target = parseYouTubeTarget("M7lc1UVf-VE")!;
    saveYouTubeFavorites(localStorage, [{ ...target, title: "Demo", author: "Creator", savedAt: 1, thumbnail: "data:image/jpeg;base64,AAA", warning: "Transient" }]);
    expect(localStorage.getItem(YOUTUBE_FAVORITES_KEY)).not.toContain("thumbnail"); expect(localStorage.getItem(YOUTUBE_FAVORITES_KEY)).not.toContain("Transient");
    expect(readYouTubeFavorites(localStorage)).toHaveLength(1);
    localStorage.setItem(YOUTUBE_FAVORITES_KEY, JSON.stringify([{ url: "javascript:alert(1)", title: "Bad" }, { ...target, title: "Good", authorUrl: "https://evil.com" }, { ...target, title: "Duplicate" }]));
    expect(readYouTubeFavorites(localStorage)).toEqual([{ ...target, title: "Good", author: "", authorUrl: undefined, savedAt: 0 }]);
  });
  it("distinguishes restricted embeds from missing client identification", () => {
    expect(playerErrorMessage(153)).toContain("앱의 재생 요청"); expect(playerErrorMessage(150)).toContain("YouTube에서만"); expect(playerErrorMessage(100)).toContain("비공개");
  });
});
