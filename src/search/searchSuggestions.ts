import type { SearchHistoryEntry, SearchRequest, TagSuggestion } from "../api/contracts";
import { canonicalSearchToken, searchTokenKind } from "./searchTokens";

export type SearchSuggestion = Readonly<{
  type: "HISTORY" | "ARTIST" | "GROUP" | "TAG" | "FEMALE" | "MALE";
  token: string;
  label: string;
  extra: string;
  favorite?: boolean;
  galleryCount?: number;
  historyUseCount?: number;
  lastUsedAt?: string;
  request?: SearchRequest;
}>;

const readable = (value: string) => value.replaceAll("_", " ");
const tagToken = (value: string) => canonicalSearchToken(value, searchTokenKind(value) ?? "tag");

export function historyDisplayToken(entry: SearchHistoryEntry): string {
  if (entry.text.trim()) return entry.text;
  const include = entry.includeTags.at(0);
  if (include) return tagToken(include);
  const exclude = entry.excludeTags.at(0);
  return exclude ? `-${tagToken(exclude)}` : "";
}

export function buildSearchSuggestionCatalog(history: readonly SearchHistoryEntry[]): SearchSuggestion[] {
  const entries = new Map<string, SearchSuggestion>();
  for (const entry of history) {
    const token = historyDisplayToken(entry); if (!token) continue;
    const key = JSON.stringify([entry.text, entry.includeTags, entry.excludeTags, entry.languages, entry.sort]);
    const existing = entries.get(key); if (existing && (existing.historyUseCount ?? 0) >= entry.useCount) continue;
    const conditions = entry.includeTags.length + entry.excludeTags.length;
    entries.set(key, { type: "HISTORY", token, label: token, extra: `최근 검색 · ${entry.useCount}회${conditions ? ` · 태그 조건 ${conditions}개` : ""}`, historyUseCount: entry.useCount, lastUsedAt: entry.lastUsedAt, request: { text: entry.text, includeTags: [...entry.includeTags], excludeTags: [...entry.excludeTags], languages: [...entry.languages], sort: entry.sort, pageSize: entry.pageSize } });
  }
  return [...entries.values()].sort((a,b) => (b.historyUseCount ?? 0) - (a.historyUseCount ?? 0) || (b.lastUsedAt ?? "").localeCompare(a.lastUsedAt ?? "")).slice(0,4);
}

export function catalogSuggestion(entry: TagSuggestion): SearchSuggestion {
  const type = entry.namespace === "artist"
    ? "ARTIST"
    : entry.namespace === "group"
      ? "GROUP"
      : entry.namespace === "female"
        ? "FEMALE"
        : entry.namespace === "male"
          ? "MALE"
          : "TAG";
  return { type, token: entry.token, label: readable(entry.name), extra: entry.galleryCount.toLocaleString(), favorite: entry.favorite, galleryCount: entry.galleryCount };
}
