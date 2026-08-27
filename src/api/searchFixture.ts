import rawSearchFixture from "../../src-tauri/fixtures/search_galleries.json";
import { galleryId, type Language } from "../core/types";
import { normalizeTokenValue } from "../search/searchTokens";
import type { GalleryDetail, GalleryPage, GallerySummary, SearchRequest } from "./contracts";

type FixtureGallery = {
  id: number;
  title: string;
  artist: string;
  group: string | null;
  pages: number;
  language: Language;
  tags: string[];
  series?: string[];
  characters?: string[];
  publishedRank: number;
  popularity: number;
  related: number[];
};

export type SearchFixtureResult = {
  request: SearchRequest;
  items: GallerySummary[];
};

const fixture = rawSearchFixture as FixtureGallery[];
const utf8 = new TextEncoder();
const u64Mask = 0xffff_ffff_ffff_ffffn;
const languageRank: Record<Language, number> = {
  korean: 0,
  japanese: 1,
  chinese: 2,
  english: 3,
};

const compareRustStrings = (left: string, right: string): number => {
  const leftBytes = utf8.encode(left);
  const rightBytes = utf8.encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    const difference = leftBytes[index]! - rightBytes[index]!;
    if (difference) return difference;
  }
  return leftBytes.length - rightBytes.length;
};

const normalizedTags = (values: string[]): string[] =>
  [...new Set(values.map((value) => value.trim().toLowerCase()))].sort(compareRustStrings);

export function normalizeSearchRequest(request: SearchRequest): SearchRequest {
  return {
    text: request.text.trim().toLowerCase().replace(/\\_/g, "_"),
    includeTags: normalizedTags(request.includeTags),
    excludeTags: normalizedTags(request.excludeTags),
    languages: [...new Set(request.languages)].sort((left, right) => languageRank[left] - languageRank[right]),
    sort: request.sort,
    pageSize: request.pageSize,
  };
}

export function searchRequestKey(request: SearchRequest): string {
  return JSON.stringify(normalizeSearchRequest(request));
}

const fnv1a = (bytes: Uint8Array, offset: bigint): bigint => {
  let hash = offset;
  for (const byte of bytes) {
    hash = ((hash ^ BigInt(byte)) * 0x0000_0100_0000_01b3n) & u64Mask;
  }
  return hash;
};

export function searchFixtureQueryId(request: SearchRequest): string {
  const canonical = utf8.encode(searchRequestKey(request));
  const first = fnv1a(canonical, 0xcbf2_9ce4_8422_2325n).toString(16).padStart(16, "0");
  const second = fnv1a(canonical, 0x8422_2325_cbf2_9ce4n).toString(16).padStart(16, "0");
  return `fixture-${first}${second}`;
}

const normalizedFixtureTags = (gallery: FixtureGallery): string[] => normalizedTags(gallery.tags);

const searchableText = (gallery: FixtureGallery): string => {
  const artist = gallery.artist.toLowerCase();
  const values = [gallery.title.toLowerCase(), artist, `artist:${normalizeTokenValue(artist)}`];
  if (gallery.group) {
    const group = gallery.group.toLowerCase();
    values.push(group, `group:${normalizeTokenValue(group)}`);
  }
  for (const series of normalizedTags(gallery.series ?? [])) {
    values.push(series, `series:${series.replace(/\s+/g, "_")}`);
  }
  for (const character of normalizedTags(gallery.characters ?? [])) {
    values.push(character, `character:${character.replace(/\s+/g, "_")}`);
  }
  values.push(...normalizedFixtureTags(gallery));
  return values.join(" ");
};

const toSummary = (gallery: FixtureGallery): GallerySummary => ({
  id: galleryId(gallery.id),
  title: gallery.title.trim(),
  artist: gallery.artist.trim(),
  ...(gallery.group?.trim() ? { group: gallery.group.trim() } : {}),
  pages: gallery.pages,
  language: gallery.language,
  tags: normalizedFixtureTags(gallery),
  series: normalizedTags(gallery.series ?? []),
  characters: normalizedTags(gallery.characters ?? []),
  publishedRank: gallery.publishedRank,
  popularity: gallery.popularity,
  thumbnailKey: `fixture-gallery-${gallery.id}-cover`,
  thumbnailWidth: 512,
  thumbnailHeight: 512,
});

const stableRandomRank = (id: number): bigint => {
  let value = (BigInt(id) ^ 0x9e37_79b9_7f4a_7c15n) & u64Mask;
  value = ((value ^ (value >> 30n)) * 0xbf58_476d_1ce4_e5b9n) & u64Mask;
  value = ((value ^ (value >> 27n)) * 0x94d0_49bb_1331_11ebn) & u64Mask;
  return (value ^ (value >> 31n)) & u64Mask;
};

export function runSearchFixture(input: SearchRequest): SearchFixtureResult {
  const request = normalizeSearchRequest(input);
  const included = new Set(request.includeTags);
  const excluded = new Set(request.excludeTags);

  const galleries = fixture.filter((gallery) => {
    if (request.languages.length && !request.languages.includes(gallery.language)) return false;
    const tags = new Set(normalizedFixtureTags(gallery));
    if ([...included].some((tag) => !tags.has(tag))) return false;
    if ([...excluded].some((tag) => tags.has(tag))) return false;
    return !request.text || searchableText(gallery).includes(request.text);
  });

  if (request.sort === "recent") {
    galleries.sort((left, right) => right.publishedRank - left.publishedRank || right.id - left.id);
  } else if (request.sort.startsWith("popular")) {
    galleries.sort((left, right) => right.popularity - left.popularity || right.id - left.id);
  } else {
    galleries.sort((left, right) => {
      const leftRank = stableRandomRank(left.id);
      const rightRank = stableRandomRank(right.id);
      return leftRank < rightRank ? -1 : leftRank > rightRank ? 1 : 0;
    });
  }

  return { request, items: galleries.map(toSummary) };
}

export function searchFixturePage(result: SearchFixtureResult, page: number): GalleryPage {
  const totalPages = result.items.length === 0
    ? 0
    : Math.ceil(result.items.length / result.request.pageSize);
  const offset = Math.max(0, page - 1) * result.request.pageSize;
  return {
    page,
    totalPages,
    items: result.items.slice(offset, offset + result.request.pageSize),
  };
}

export function galleryDetailFixture(galleryIdValue: GallerySummary["id"]): GalleryDetail | undefined {
  const gallery = fixture.find((item) => item.id === galleryIdValue);
  if (!gallery) return undefined;
  const related = gallery.related
    .map((id) => fixture.find((item) => item.id === id))
    .filter((item): item is FixtureGallery => item !== undefined)
    .map(toSummary);
  return {
    ...toSummary(gallery),
    tags: normalizedFixtureTags(gallery),
    related,
    pageDimensions: Array.from({ length: gallery.pages }, (_, index) => ({
      sourcePage: index + 1,
      width: 512,
      height: 512,
    })),
  };
}

export function searchRequestValidationError(request: SearchRequest): { field: string; reason: string } | null {
  if (utf8.encode(request.text.trim().toLowerCase()).length > 500) {
    return { field: "text", reason: "must be at most 500 bytes" };
  }
  if (!Number.isInteger(request.pageSize) || request.pageSize < 1 || request.pageSize > 200) {
    return { field: "pageSize", reason: "must be between 1 and 200" };
  }
  for (const [field, tags] of [
    ["includeTags", request.includeTags],
    ["excludeTags", request.excludeTags],
  ] as const) {
    if (tags.length > 100) return { field, reason: "must contain at most 100 tags" };
    for (const tag of tags) {
      const normalized = tag.trim().toLowerCase();
      if (!normalized) return { field, reason: "must not contain empty tags" };
      if (utf8.encode(normalized).length > 200) {
        return { field, reason: "each tag must be at most 200 bytes" };
      }
    }
  }
  return null;
}
