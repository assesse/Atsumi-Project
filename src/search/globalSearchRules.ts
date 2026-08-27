import type { SearchRequest } from "../api/contracts";

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
