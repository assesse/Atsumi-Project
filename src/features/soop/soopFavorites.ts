import { normalizeSoopChannel } from "./soopLinks";
import type { SoopChannel } from "../../api/soop";

export const SOOP_FAVORITES_KEY = "atsumi.soop.favorites.v1";
export const SOOP_FAVORITES_LIMIT = 100;
export type SoopFavorite = Pick<SoopChannel, "channelId" | "name">;

export function readSoopFavorites(): SoopFavorite[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(SOOP_FAVORITES_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    return parsed.flatMap((entry: unknown) => {
      if (!entry || typeof entry !== "object") return [];
      const record = entry as Record<string, unknown>;
      if (typeof record.channelId !== "string" || typeof record.name !== "string") return [];
      const channelId = normalizeSoopChannel(record.channelId);
      if (!channelId || seen.has(channelId)) return [];
      seen.add(channelId);
      return [{ channelId, name: record.name.replace(/[\u0000-\u001f\u007f]/g, "").slice(0, 120) || channelId }];
    }).slice(0, SOOP_FAVORITES_LIMIT);
  } catch { return []; }
}

export function writeSoopFavorites(favorites: SoopFavorite[]): boolean {
  try {
    // Keep only channel identity, never images, session state or remote responses.
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify(favorites.slice(0, SOOP_FAVORITES_LIMIT).map(({ channelId, name }) => ({ channelId, name }))));
    return true;
  } catch { return false; }
}
