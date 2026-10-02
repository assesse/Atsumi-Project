import { invoke } from "@tauri-apps/api/core";
import { backend } from "./backend";
export const popularityPeriods = ["today", "week", "month", "year"] as const;
export type PopularityPeriod = typeof popularityPeriods[number];
export type DownloadSort = "recent" | `popular_${PopularityPeriod}`;
export type PopularitySnapshot = { period: PopularityPeriod; fetchedAt: string; ranks: Record<string, number | null>; warning?: string | null };
export type PopularityRanks = Partial<Record<PopularityPeriod, Record<string, number | null>>>;
export async function getDownloadPopularity(period: PopularityPeriod): Promise<PopularitySnapshot> {
  if (backend.runtime !== "tauri") return { period, fetchedAt: "", ranks: {} };
  return invoke("download_popularity_snapshot", { period });
}
