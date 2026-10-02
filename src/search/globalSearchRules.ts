import type { GallerySummary, SearchRequest } from "../api/contracts";
import { normalizeTokenValue } from "./searchTokens";

const ruleToken = (value: string): string => normalizeTokenValue(value).replace(/^tag:/, "");

/** Exact metadata matching, including namespace and space/underscore aliases. */
export const matchesGlobalSearchRules = (
  gallery: Pick<GallerySummary, "artist" | "tags" | "language"> & Partial<Pick<GallerySummary, "artists" | "group" | "series" | "characters">>,
  includeTags: readonly string[],
  excludeTags: readonly string[],
): boolean => {
  if (!includeTags.length && !excludeTags.length) return true;
  const tokens = new Set([
    ...gallery.tags,
    ...[gallery.artist, ...(gallery.artists ?? [])].map((name) => `artist:${name}`),
    ...(gallery.group ? [`group:${gallery.group}`] : []),
    ...(gallery.series ?? []).map((name) => `series:${name}`),
    ...(gallery.characters ?? []).map((name) => `character:${name}`),
    `language:${gallery.language}`,
  ].map(ruleToken));
  const excluded = new Set(excludeTags.map(ruleToken));
  return ![...excluded].some((tag) => tokens.has(tag))
    && includeTags.map(ruleToken).every((tag) => excluded.has(tag) || tokens.has(tag));
};

export const normalizeGlobalSearchTag = (value: string): string =>
  value.trim().toLocaleLowerCase();

export const normalizeGlobalSearchTagList = (values: string[]): string[] =>
  [...new Set(values.map(normalizeGlobalSearchTag).filter(Boolean))]
    .sort((left, right) => left.localeCompare(right));

export const parseGlobalSearchTagInput = (value: string): string[] =>
  normalizeGlobalSearchTagList(value.split(/[\r\n,]+/));

/** Global exclusions win when a one-off search asks for the same tag. */
export const applyGlobalSearchRules = (
  request: SearchRequest,
  searchIncludeTags: string[],
  searchExcludeTags: string[],
): SearchRequest => {
  const excludeTags = normalizeGlobalSearchTagList([
    ...request.excludeTags,
    ...searchExcludeTags,
  ]);
  const excluded = new Set(excludeTags);
  const includeTags = normalizeGlobalSearchTagList([
    ...searchIncludeTags,
    ...request.includeTags,
  ]).filter((tag) => !excluded.has(tag));
  return { ...request, includeTags, excludeTags };
};
