import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { parseYouTubeTarget } from "../features/youtube/model";
import { normalizePinterestInput } from "../features/pinterest/types";
import { normalizeSoopChannel } from "../features/soop/soopLinks";

export type SupportedLinkRequest = {
  id: string;
  source: "pixiv" | "danbooru" | "pinterest" | "hitomi" | "youtube" | "soop" | "chzzk" | "x";
  kind: "artwork" | "artist" | "post" | "pin" | "gallery" | "video" | "channel" | "playlist" | "profile";
  targetId: string;
  url: string;
};

const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/;
const decimal = /^[1-9]\d{0,11}$/;

/** Native indexed selection is the authority. This parser validates only the
 * bounded canonical delivery shape; it never accepts arbitrary open requests. */
export function isSupportedLinkRequest(value: unknown): value is SupportedLinkRequest {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const row = value as Record<string, unknown>;
  if (Object.keys(row).length !== 5 || typeof row.id !== "string" || !uuid.test(row.id) || typeof row.targetId !== "string" || !row.targetId || row.targetId.length > 256 || typeof row.url !== "string" || row.url.length > 2048) return false;
  if (row.source === "pixiv" && decimal.test(row.targetId) && (row.kind === "artwork" || row.kind === "artist")) return row.url === `https://www.pixiv.net/${row.kind === "artist" ? "users" : "artworks"}/${row.targetId}`;
  if (row.source === "danbooru" && row.kind === "post" && decimal.test(row.targetId)) return row.url === `https://danbooru.donmai.us/posts/${row.targetId}`;
  if (row.source === "pinterest" && row.kind === "pin") return normalizePinterestInput(row.targetId) === row.targetId && row.url === `https://www.pinterest.com/pin/${row.targetId}/`;
  if (row.source === "hitomi" && row.kind === "gallery" && decimal.test(row.targetId)) return row.url === `https://hitomi.la/galleries/${row.targetId}.html`;
  if (row.source === "youtube") { const parsed = parseYouTubeTarget(row.url); return !!parsed && parsed.id === row.targetId && parsed.kind === row.kind && parsed.url === row.url; }
  if (row.source === "soop") return row.kind === "channel" ? normalizeSoopChannel(row.targetId) === row.targetId && row.url === `https://play.sooplive.com/${row.targetId}` : row.kind === "video" && /^[1-9]\d{0,19}$/.test(row.targetId) && row.url === `https://vod.sooplive.com/player/${row.targetId}`;
  if (row.source === "chzzk" && row.kind === "channel") return /^[a-f0-9]{32}$/.test(row.targetId) && row.url === `https://chzzk.naver.com/live/${row.targetId}`;
  if (row.source === "x") {
    const post = /^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/[1-9]\d{4,24}(?:\/(?:photo|video)\/[1-4])?$/.test(row.targetId);
    const profile = /^\/([A-Za-z0-9_]{1,15})$/.exec(row.targetId);
    const validProfile = profile && !/^(home|explore|search|notifications|messages|i|compose|settings|login|logout|signup|tos|privacy|help|about|download|jobs|communities|premium|grok|hashtag|intent|account|accounts)$/i.test(profile[1]!);
    return (post && row.kind === "post" || validProfile && row.kind === "profile") === true && row.url === `https://x.com${row.targetId}`;
  }
  return false;
}

export async function openSupportedXTarget(id: string, generation: number): Promise<void> {
  if (!uuid.test(id) || !Number.isSafeInteger(generation) || generation < 0) throw new Error("선택한 X 링크를 다시 열어 주세요.");
  await invoke("media_search_navigation_open_x", { id, generation });
}

/** Listen first, then recover a delivery emitted before the main listener was
 * ready. The native pending value is acknowledged only after composition kept
 * it in state, so a lazy workspace cannot lose the selected destination. */
export async function subscribeSupportedLinks(accept: (request: SupportedLinkRequest) => void): Promise<() => void> {
  let disposed = false;
  let eventRevision = 0;
  const seen = new Set<string>();
  const receive = (value: unknown) => {
    if (disposed || !isSupportedLinkRequest(value)) return;
    if (!seen.has(value.id)) {
      seen.add(value.id);
      if (seen.size > 64) seen.delete(seen.values().next().value!);
      accept(value);
    }
    void invoke("media_search_navigation_ack", { id: value.id }).catch(() => {});
  };
  const unlisten = await listen<unknown>("atsumi:open-supported-link", event => { if (isSupportedLinkRequest(event.payload)) eventRevision++; receive(event.payload); });
  const pendingRevision = eventRevision;
  void invoke<unknown>("media_search_navigation_pending").then(value => { if (pendingRevision === eventRevision) receive(value); }, () => {});
  return () => { disposed = true; unlisten(); };
}
